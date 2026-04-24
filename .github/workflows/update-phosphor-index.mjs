import { createPrivateKey, sign } from "node:crypto";
import { createHash } from "node:crypto";
import { spawnSync } from "node:child_process";
import { cp, mkdir, readdir, readFile, rm, writeFile } from "node:fs/promises";
import { existsSync } from "node:fs";
import path from "node:path";

const registryUrl =
  "https://raw.githubusercontent.com/neovim-treesitter/treesitter-parser-registry/main/registry.json";
const commitUrl =
  "https://api.github.com/repos/neovim-treesitter/treesitter-parser-registry/commits/main";
const platform = process.env.PHOSPHOR_PACK_PLATFORM ?? "windows-x86_64";
const treeSitterAbi = Number(process.env.PHOSPHOR_TREE_SITTER_ABI ?? "15");
const packBaseUrl =
  process.env.PHOSPHOR_PACK_BASE_URL ??
  "https://blob.diffygui.com/phosphor-packs";
const packOutputRoot =
  process.env.PHOSPHOR_PACK_OUTPUT_DIR ?? ".phosphor-pack-out";
const indexOutputPath =
  process.env.PHOSPHOR_INDEX_OUTPUT ??
  `assets/phosphor-index.${platform}.json`;
const shouldBuildPacks = process.env.PHOSPHOR_SKIP_PACK_BUILD !== "1";

const headers = {
  Accept: "application/vnd.github+json",
  "User-Agent": "diffy-phosphor-index-updater",
};

if (process.env.GITHUB_TOKEN) {
  headers.Authorization = `Bearer ${process.env.GITHUB_TOKEN}`;
}

const commitResponse = await fetch(commitUrl, { headers });
if (!commitResponse.ok) {
  throw new Error(`failed to fetch registry commit: ${commitResponse.status}`);
}
const commit = await commitResponse.json();

const registryResponse = await fetch(registryUrl, { headers });
if (!registryResponse.ok) {
  throw new Error(`failed to fetch registry: ${registryResponse.status}`);
}
const registry = await registryResponse.json();

const upstreamLanguages = Object.entries(registry)
  .filter(([language, entry]) => {
    return (
      !language.startsWith("$") &&
      entry &&
      typeof entry === "object" &&
      !Array.isArray(entry)
    );
  })
  .map(([language, entry]) => ({
    language,
    filetypes: [...(entry.filetypes ?? [])].sort(),
    requires: [...(entry.requires ?? [])].sort(),
    source: entry.source ?? {},
  }))
  .sort((left, right) => left.language.localeCompare(right.language));

const commonLanguageNames = new Set([
  "bash",
  "c",
  "cpp",
  "go",
  "javascript",
  "json",
  "python",
  "rust",
  "toml",
  "typescript",
  "tsx",
]);

const languageCount = upstreamLanguages.length;

if (languageCount === 0) {
  throw new Error("registry did not contain any language entries");
}

const languageNames = upstreamLanguages
  .map((entry) => entry.language)
  .sort();

const buildErrors = [];
const packs = shouldBuildPacks
  ? await buildPacks(registry, upstreamLanguages, commit.sha, buildErrors)
  : [];

const index = {
  payload: {
    schema_version: 1,
    generated_from: `${registryUrl}?ref=${commit.sha}`,
    generated_at: commit.commit?.committer?.date ?? new Date(0).toISOString(),
    platform,
    tree_sitter_abi: treeSitterAbi,
    packs,
    pack_build_error_count: buildErrors.length,
    upstream_language_count: languageCount,
    upstream_languages: languageNames,
    upstream_registry: upstreamLanguages,
  },
  signature: "",
};

function canonicalJson(value) {
  if (value === null) {
    return "null";
  }
  if (typeof value === "boolean") {
    return value ? "true" : "false";
  }
  if (typeof value === "number") {
    if (!Number.isFinite(value)) {
      throw new Error("canonical JSON cannot encode non-finite numbers");
    }
    return JSON.stringify(value);
  }
  if (typeof value === "string") {
    return JSON.stringify(value);
  }
  if (Array.isArray(value)) {
    return `[${value.map((item) => canonicalJson(item)).join(",")}]`;
  }
  if (typeof value === "object") {
    return `{${Object.keys(value)
      .sort()
      .map((key) => `${JSON.stringify(key)}:${canonicalJson(value[key])}`)
      .join(",")}}`;
  }
  throw new Error(`canonical JSON cannot encode ${typeof value}`);
}

function openSshEd25519PrivateKeyToPkcs8(privateKey) {
  const body = privateKey
    .replace("-----BEGIN OPENSSH PRIVATE KEY-----", "")
    .replace("-----END OPENSSH PRIVATE KEY-----", "")
    .replace(/\s+/g, "");
  const bytes = Buffer.from(body, "base64");
  const reader = new BinaryReader(bytes);
  const magic = reader.readBytes("openssh-key-v1\0".length).toString("utf8");
  if (magic !== "openssh-key-v1\0") {
    throw new Error("private key is not in OpenSSH format");
  }
  const cipherName = reader.readString().toString("utf8");
  const kdfName = reader.readString().toString("utf8");
  reader.readString();
  const keyCount = reader.readUInt32();
  if (cipherName !== "none" || kdfName !== "none" || keyCount !== 1) {
    throw new Error("private key must be a single unencrypted OpenSSH key");
  }
  reader.readString();
  const privateBlock = new BinaryReader(reader.readString());
  const checkA = privateBlock.readUInt32();
  const checkB = privateBlock.readUInt32();
  if (checkA !== checkB) {
    throw new Error("OpenSSH private key checkints did not match");
  }
  const keyType = privateBlock.readString().toString("utf8");
  if (keyType !== "ssh-ed25519") {
    throw new Error("private key is not ssh-ed25519");
  }
  privateBlock.readString();
  const privateValue = privateBlock.readString();
  if (privateValue.length !== 64) {
    throw new Error("unexpected ssh-ed25519 private key length");
  }
  return Buffer.concat([
    Buffer.from("302e020100300506032b657004220420", "hex"),
    privateValue.subarray(0, 32),
  ]);
}

class BinaryReader {
  constructor(bytes) {
    this.bytes = bytes;
    this.offset = 0;
  }

  readUInt32() {
    this.#require(4);
    const value = this.bytes.readUInt32BE(this.offset);
    this.offset += 4;
    return value;
  }

  readString() {
    const length = this.readUInt32();
    return this.readBytes(length);
  }

  readBytes(length) {
    this.#require(length);
    const value = this.bytes.subarray(this.offset, this.offset + length);
    this.offset += length;
    return value;
  }

  #require(length) {
    if (this.offset + length > this.bytes.length) {
      throw new Error("unexpected end of OpenSSH key data");
    }
  }
}

async function buildPacks(registry, upstreamLanguages, registryRevision, buildErrors) {
  if (platform !== "windows-x86_64") {
    throw new Error(`pack builder does not support ${platform} yet`);
  }

  const workDir = path.join(".phosphor-pack-work");
  const packRoot = path.join(packOutputRoot, platform);
  await rm(workDir, { recursive: true, force: true });
  await rm(packRoot, { recursive: true, force: true });
  await mkdir(workDir, { recursive: true });

  const built = [];
  const packLanguages = upstreamLanguages
    .filter((entry) => entry.source)
    .map((entry) => ({
      language: entry.language,
      extensions: entry.filetypes,
      symbol: `tree_sitter_${entry.language}`,
      common: commonLanguageNames.has(entry.language),
    }));

  for (const packLanguage of packLanguages) {
    const registryEntry = registry[packLanguage.language];
    if (!registryEntry?.source) {
      throw new Error(`registry is missing ${packLanguage.language}`);
    }
    try {
      built.push(await buildPack(packLanguage, registryEntry, registryRevision, workDir, packRoot));
    } catch (error) {
      buildErrors.push({
        language: packLanguage.language,
        message: error.message,
      });
      console.warn(`::warning title=Skipped ${packLanguage.language}::${error.message}`);
    }
  }

  await rm(workDir, { recursive: true, force: true });
  return built.sort((left, right) => left.language.localeCompare(right.language));
}

async function buildPack(packLanguage, registryEntry, registryRevision, workDir, packRoot) {
  const source = normalizeSource(registryEntry.source);
  const languageWorkDir = path.join(workDir, packLanguage.language);
  const parserRepoDir = path.join(languageWorkDir, "parser");
  const queryRepoDir = path.join(languageWorkDir, "queries");
  await mkdir(languageWorkDir, { recursive: true });

  cloneRepo(source.parserUrl, parserRepoDir);
  cloneRepo(source.queryUrl ?? source.parserUrl, queryRepoDir);

  const parserRevision = gitRevParse(parserRepoDir);
  const queryRevision = gitRevParse(queryRepoDir);
  const version = `${parserRevision.slice(0, 12)}-${queryRevision.slice(0, 12)}`;
  const parserDir = path.join(parserRepoDir, source.parserLocation ?? "");
  const highlightsSource = await findQueryFile(queryRepoDir, "highlights.scm");
  const injectionsSource = await findOptionalQueryFile(queryRepoDir, "injections.scm");
  const packDir = path.join(packRoot, packLanguage.language, version);
  await mkdir(packDir, { recursive: true });

  const parserPath = "parser.dll";
  await compileWindowsParser(parserDir, path.join(packDir, parserPath));

  const highlightsPath = "highlights.scm";
  await cp(highlightsSource, path.join(packDir, highlightsPath));

  let injections = null;
  if (injectionsSource) {
    const injectionsPath = "injections.scm";
    await cp(injectionsSource, path.join(packDir, injectionsPath));
    injections = {
      path: injectionsPath,
      sha256: await sha256File(path.join(packDir, injectionsPath)),
    };
  }

  const localParser = {
    path: parserPath,
    sha256: await sha256File(path.join(packDir, parserPath)),
  };
  const highlights = {
    path: highlightsPath,
    sha256: await sha256File(path.join(packDir, highlightsPath)),
  };
  const packSource = {
    registry_url: `${registryUrl}?ref=${registryRevision}`,
    parser_url: source.parserUrl,
    query_url: source.queryUrl ?? source.parserUrl,
    revision: `${parserRevision}:${queryRevision}`,
  };
  const manifest = {
    schema_version: 1,
    language: packLanguage.language,
    version,
    platform,
    tree_sitter_abi: treeSitterAbi,
    symbol: packLanguage.symbol,
    parser: localParser,
    highlights,
    injections,
    extensions: packLanguage.extensions,
    source: packSource,
  };
  const manifestPath = "manifest.json";
  await writeFile(
    path.join(packDir, manifestPath),
    `${JSON.stringify(manifest, null, 2)}\n`,
  );
  const manifestFile = {
    path: manifestPath,
    sha256: await sha256File(path.join(packDir, manifestPath)),
  };
  const remoteBase = `${packBaseUrl}/${platform}/${packLanguage.language}/${version}`;

  return {
    language: packLanguage.language,
    version,
    common: packLanguage.common,
    extensions: packLanguage.extensions,
    symbol: packLanguage.symbol,
    manifest: withUrl(manifestFile, remoteBase),
    parser: withUrl(localParser, remoteBase),
    highlights: withUrl(highlights, remoteBase),
    injections: injections ? withUrl(injections, remoteBase) : null,
    source: packSource,
  };
}

function normalizeSource(source) {
  if (source.type === "self_contained") {
    return {
      parserUrl: source.url,
      queryUrl: source.url,
      parserLocation: source.parser_location,
    };
  }
  return {
    parserUrl: source.parser_url,
    queryUrl: source.queries_url,
    parserLocation: source.parser_location,
  };
}

function cloneRepo(url, destination) {
  if (!url) {
    throw new Error("missing repository URL");
  }
  run("git", ["clone", "--depth", "1", url, destination]);
}

function gitRevParse(cwd) {
  return run("git", ["rev-parse", "HEAD"], { cwd }).stdout.trim();
}

async function compileWindowsParser(parserDir, outputPath) {
  const srcDir = path.join(parserDir, "src");
  const sources = [
    path.join(srcDir, "parser.c"),
    path.join(srcDir, "scanner.c"),
    path.join(srcDir, "scanner.cc"),
  ].filter((file) => existsSync(file));

  if (!sources.some((file) => file.endsWith("parser.c"))) {
    throw new Error(`missing parser.c in ${srcDir}`);
  }

  run("cl", [
    "/nologo",
    "/LD",
    "/O2",
    `/I${srcDir}`,
    `/Fe:${outputPath}`,
    ...sources,
    "/link",
    "/NOLOGO",
  ]);
}

async function findQueryFile(root, filename) {
  const found = await findOptionalQueryFile(root, filename);
  if (!found) {
    throw new Error(`missing ${filename} under ${root}`);
  }
  return found;
}

async function findOptionalQueryFile(root, filename) {
  const entries = await walk(root);
  return entries
    .filter((entry) => path.basename(entry) === filename)
    .sort((left, right) => queryRank(left) - queryRank(right) || left.localeCompare(right))[0];
}

async function walk(root) {
  const out = [];
  for (const entry of await readdir(root, { withFileTypes: true })) {
    if (entry.name === ".git") {
      continue;
    }
    const entryPath = path.join(root, entry.name);
    if (entry.isDirectory()) {
      out.push(...await walk(entryPath));
    } else if (entry.isFile()) {
      out.push(entryPath);
    }
  }
  return out;
}

function queryRank(file) {
  const normalized = file.replaceAll(path.sep, "/");
  if (normalized.endsWith("/queries/highlights.scm")) {
    return 0;
  }
  if (normalized.endsWith("/highlights.scm")) {
    return 1;
  }
  return 2;
}

function withUrl(file, remoteBase) {
  return {
    url: `${remoteBase}/${file.path}`,
    path: file.path,
    sha256: file.sha256,
  };
}

async function sha256File(file) {
  return createHash("sha256").update(await readFile(file)).digest("hex");
}

function run(command, args, options = {}) {
  const result = spawnSync(command, args, {
    cwd: options.cwd,
    encoding: "utf8",
    stdio: ["ignore", "pipe", "pipe"],
  });
  if (result.status !== 0) {
    throw new Error(
      `${command} ${args.join(" ")} failed\n${result.stdout}\n${result.stderr}`,
    );
  }
  return result;
}

if (process.env.PHOSPHOR_PACK_INDEX_PRIVATE_KEY) {
  const privateKey = createPrivateKey({
    key: openSshEd25519PrivateKeyToPkcs8(
      process.env.PHOSPHOR_PACK_INDEX_PRIVATE_KEY,
    ),
    format: "der",
    type: "pkcs8",
  });
  index.signature = sign(null, Buffer.from(canonicalJson(index.payload)), privateKey).toString(
    "hex",
  );
}

await mkdir(path.dirname(indexOutputPath), { recursive: true });
await writeFile(
  indexOutputPath,
  `${JSON.stringify(index, null, 2)}\n`,
);
