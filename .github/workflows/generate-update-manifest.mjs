import { createHash, createPrivateKey, sign } from "node:crypto";
import { readdir, readFile, writeFile } from "node:fs/promises";
import { basename, join } from "node:path";

const releaseDir = process.env.DIFFY_RELEASE_DIR ?? "release";
const version = (process.env.GITHUB_REF_NAME ?? "").replace(/^v/, "");
const tag = process.env.GITHUB_REF_NAME;
const repo = process.env.GITHUB_REPOSITORY;
const privateKey = process.env.DIFFY_UPDATE_PRIVATE_KEY;

if (!version || !tag || !repo) {
  throw new Error("GITHUB_REF_NAME and GITHUB_REPOSITORY are required");
}
if (!privateKey) {
  throw new Error("DIFFY_UPDATE_PRIVATE_KEY is required to sign update manifest");
}

const files = await readdir(releaseDir);
const platforms = {};
const key = createPrivateKey(privateKey.replaceAll("\\n", "\n"));

for (const file of files) {
  const path = join(releaseDir, file);
  const bytes = await readFile(path);
  const sha256 = createHash("sha256").update(bytes).digest("hex");
  const signature = sign(null, bytes, key).toString("hex");
  const url = `https://github.com/${repo}/releases/download/${tag}/${encodeURIComponent(file)}`;
  const entry = { url, signature, sha256, size: bytes.length };

  if (file.match(/^Diffy_.+_aarch64\.dmg$/)) {
    platforms["macos-aarch64"] = { ...entry, format: "dmg" };
  } else if (file.match(/^Diffy_.+_x64\.dmg$/)) {
    platforms["macos-x86_64"] = { ...entry, format: "dmg" };
  } else if (file.match(/^diffy_.+_x64-setup\.exe$/)) {
    platforms["windows-x86_64"] = { ...entry, format: "nsis" };
  } else if (file.match(/^diffy_.+_aarch64-setup\.exe$/)) {
    platforms["windows-aarch64"] = { ...entry, format: "nsis" };
  } else if (file.match(/^diffy_.+_x86_64\.AppImage$/)) {
    platforms["linux-x86_64"] = { ...entry, format: "appimage" };
  } else if (file.match(/^diffy_.+_aarch64\.AppImage$/)) {
    platforms["linux-aarch64"] = { ...entry, format: "appimage" };
  }
}

if (Object.keys(platforms).length === 0) {
  throw new Error("no updateable artifacts found");
}

const payload = {
  version,
  pub_date: new Date().toISOString(),
  channel: version.includes("-") ? "prerelease" : "stable",
  notes: `Diffy ${tag}`,
  minimum_supported_version: "0.1.0",
  platforms,
};

const signature = sign(null, Buffer.from(canonicalJson(payload)), key).toString("hex");
const manifest = { payload, signature };
const outPath = join(releaseDir, "diffy-update.json");
await writeFile(outPath, `${JSON.stringify(manifest, null, 2)}\n`);
console.log(`wrote ${basename(outPath)} for ${Object.keys(platforms).join(", ")}`);

function canonicalJson(value) {
  if (value === null) return "null";
  if (typeof value === "boolean") return value ? "true" : "false";
  if (typeof value === "number") return String(value);
  if (typeof value === "string") return JSON.stringify(value);
  if (Array.isArray(value)) return `[${value.map(canonicalJson).join(",")}]`;
  const keys = Object.keys(value).sort();
  return `{${keys.map((key) => `${JSON.stringify(key)}:${canonicalJson(value[key])}`).join(",")}}`;
}
