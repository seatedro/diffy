#!/usr/bin/env python3
import argparse
import io
import json
import re
import tarfile
import urllib.request
from datetime import datetime, timezone
from pathlib import Path

MODE_DARK_TOKENS = {"dark", "night", "moon", "frappe", "mocha", "macchiato"}
MODE_LIGHT_TOKENS = {"light", "day", "dawn", "latte"}
STRIP_TOKENS = MODE_DARK_TOKENS | MODE_LIGHT_TOKENS
DEFAULT_ZON_URL = "https://raw.githubusercontent.com/ghostty-org/ghostty/main/build.zig.zon"


def fetch_bytes(url: str) -> bytes:
    req = urllib.request.Request(url, headers={"User-Agent": "diffy-theme-generator/1.0"})
    with urllib.request.urlopen(req) as resp:
        return resp.read()


def extract_archive_url(zon_text: str) -> str:
    match = re.search(r"iterm2_themes\s*=\s*\.?\{[\s\S]*?url\s*=\s*\"([^\"]+)\"", zon_text)
    if not match:
        raise RuntimeError("unable to find iterm2_themes url in build.zig.zon")
    return match.group(1)


def hex_to_rgb(hex_value: str) -> tuple[int, int, int]:
    value = hex_value.strip()
    if value.startswith("#"):
        value = value[1:]
    if len(value) != 6:
        raise ValueError(value)
    return int(value[0:2], 16), int(value[2:4], 16), int(value[4:6], 16)


def rgb_to_hex(rgb: tuple[float, float, float]) -> str:
    r = max(0, min(255, int(round(rgb[0]))))
    g = max(0, min(255, int(round(rgb[1]))))
    b = max(0, min(255, int(round(rgb[2]))))
    return f"#{r:02x}{g:02x}{b:02x}"


def mix(a: tuple[int, int, int], b: tuple[int, int, int], t: float) -> tuple[float, float, float]:
    return (
        a[0] * (1.0 - t) + b[0] * t,
        a[1] * (1.0 - t) + b[1] * t,
        a[2] * (1.0 - t) + b[2] * t,
    )


def rel_lum(rgb: tuple[int, int, int]) -> float:
    r, g, b = rgb[0] / 255.0, rgb[1] / 255.0, rgb[2] / 255.0
    return 0.2126 * r + 0.7152 * g + 0.0722 * b


def tokenize(name: str) -> list[str]:
    return [token for token in re.split(r"[\s_\-]+", name.lower()) if token]


def canonical_base(name: str) -> str:
    words = re.split(r"([\s_\-]+)", name)
    output: list[str] = []
    for word in words:
        low = word.lower()
        if re.fullmatch(r"[\s_\-]+", word):
            output.append(" ")
            continue
        if low in STRIP_TOKENS:
            continue
        output.append(word)
    base = "".join(output)
    base = re.sub(r"\s+", " ", base).strip(" -_")
    return base if base else name


def mode_hint(name: str) -> str | None:
    tokens = set(tokenize(name))
    dark_count = len(tokens & MODE_DARK_TOKENS)
    light_count = len(tokens & MODE_LIGHT_TOKENS)
    if dark_count > light_count:
        return "dark"
    if light_count > dark_count:
        return "light"
    return None


def choose_variant(candidates: list[dict], target_mode: str) -> dict:
    preferred = [c for c in candidates if (c["is_dark"] if target_mode == "dark" else not c["is_dark"])]
    pool = preferred if preferred else candidates

    def score(theme: dict) -> tuple[float, float, int, str]:
        hint = mode_hint(theme["name"])
        hint_score = 2 if hint == target_mode else (1 if hint is None else 0)
        luminance_score = (1.0 - theme["lum"]) if target_mode == "dark" else theme["lum"]
        return hint_score, luminance_score, -len(theme["name"]), theme["name"].lower()

    return max(pool, key=score)


def semantic_from(theme: dict) -> dict:
    palette = theme["palette"]
    background = theme["background"]
    foreground = theme["foreground"]
    is_dark = theme["is_dark"]

    accent = palette[4]
    accent_strong = palette[12]
    success_text = palette[10] if is_dark else palette[2]
    danger_text = palette[9] if is_dark else palette[1]
    warning_text = palette[11] if is_dark else palette[3]

    selection_bg = theme.get("selection_background") or mix(background, accent, 0.22 if is_dark else 0.18)

    app_bg = background
    canvas = mix(background, foreground, 0.03 if is_dark else 0.015)
    panel = mix(background, foreground, 0.08 if is_dark else 0.06)
    panel_strong = mix(background, foreground, 0.14 if is_dark else 0.11)
    panel_tint = mix(panel_strong, accent, 0.25)
    toolbar_bg = canvas

    border_soft = mix(background, foreground, 0.18 if is_dark else 0.20)
    border_strong = mix(background, foreground, 0.30 if is_dark else 0.34)
    divider = border_soft

    text_strong = mix(foreground, (255, 255, 255) if is_dark else (0, 0, 0), 0.12)
    text_base = foreground
    text_muted = mix(foreground, background, 0.25)
    text_faint = mix(foreground, background, 0.45)

    accent_soft = mix(background, accent, 0.26 if is_dark else 0.20)

    success_bg = mix(background, success_text, 0.16 if is_dark else 0.12)
    success_border = mix(background, success_text, 0.34 if is_dark else 0.24)
    danger_bg = mix(background, danger_text, 0.16 if is_dark else 0.12)
    danger_border = mix(background, danger_text, 0.34 if is_dark else 0.24)
    warning_bg = mix(background, warning_text, 0.16 if is_dark else 0.12)
    warning_border = mix(background, warning_text, 0.34 if is_dark else 0.24)

    line_context = canvas
    line_context_alt = mix(background, foreground, 0.045 if is_dark else 0.03)
    line_add = mix(background, success_text, 0.14 if is_dark else 0.10)
    line_add_accent = mix(background, success_text, 0.24 if is_dark else 0.18)
    line_del = mix(background, danger_text, 0.14 if is_dark else 0.10)
    line_del_accent = mix(background, danger_text, 0.24 if is_dark else 0.18)

    return {
        "appBg": rgb_to_hex(app_bg),
        "canvas": rgb_to_hex(canvas),
        "panel": rgb_to_hex(panel),
        "panelStrong": rgb_to_hex(panel_strong),
        "panelTint": rgb_to_hex(panel_tint),
        "toolbarBg": rgb_to_hex(toolbar_bg),
        "borderSoft": rgb_to_hex(border_soft),
        "borderStrong": rgb_to_hex(border_strong),
        "divider": rgb_to_hex(divider),
        "textStrong": rgb_to_hex(text_strong),
        "textBase": rgb_to_hex(text_base),
        "textMuted": rgb_to_hex(text_muted),
        "textFaint": rgb_to_hex(text_faint),
        "accent": rgb_to_hex(accent),
        "accentStrong": rgb_to_hex(accent_strong),
        "accentSoft": rgb_to_hex(accent_soft),
        "successBg": rgb_to_hex(success_bg),
        "successBorder": rgb_to_hex(success_border),
        "successText": rgb_to_hex(success_text),
        "dangerBg": rgb_to_hex(danger_bg),
        "dangerBorder": rgb_to_hex(danger_border),
        "dangerText": rgb_to_hex(danger_text),
        "warningBg": rgb_to_hex(warning_bg),
        "warningBorder": rgb_to_hex(warning_border),
        "warningText": rgb_to_hex(warning_text),
        "selectionBg": rgb_to_hex(selection_bg),
        "selectionBorder": rgb_to_hex(accent),
        "lineContext": rgb_to_hex(line_context),
        "lineContextAlt": rgb_to_hex(line_context_alt),
        "lineAdd": rgb_to_hex(line_add),
        "lineAddAccent": rgb_to_hex(line_add_accent),
        "lineDel": rgb_to_hex(line_del),
        "lineDelAccent": rgb_to_hex(line_del_accent),
        "shadowSm": "#1a000000" if is_dark else "#0a000000",
        "shadowMd": "#33000000" if is_dark else "#15000000",
        "shadowLg": "#4d000000" if is_dark else "#22000000",
    }


def parse_theme_archive(archive_bytes: bytes) -> list[dict]:
    themes: list[dict] = []
    with tarfile.open(fileobj=io.BytesIO(archive_bytes), mode="r:*") as archive:
        for member in archive.getmembers():
            if not member.isfile() or not member.name.startswith("ghostty/"):
                continue
            theme_name = member.name[len("ghostty/"):]
            file_obj = archive.extractfile(member)
            if file_obj is None:
                continue
            raw = file_obj.read().decode("utf-8", errors="ignore")
            data: dict[str, str] = {}
            palette: dict[int, str] = {}
            for line in raw.splitlines():
                line = line.strip()
                if not line or line.startswith("#") or "=" not in line:
                    continue
                key, value = [part.strip() for part in line.split("=", 1)]
                if key == "palette" and "=" in value:
                    idx_raw, color = [part.strip() for part in value.split("=", 1)]
                    if idx_raw.isdigit():
                        palette[int(idx_raw)] = color
                else:
                    data[key] = value

            if len(palette) < 16 or "background" not in data or "foreground" not in data:
                continue

            palette_rgb = [hex_to_rgb(palette[index]) for index in range(16)]
            background = hex_to_rgb(data["background"])
            foreground = hex_to_rgb(data["foreground"])
            luminance = rel_lum(background)

            themes.append(
                {
                    "name": theme_name,
                    "background": background,
                    "foreground": foreground,
                    "palette": palette_rgb,
                    "selection_background": hex_to_rgb(data["selection-background"]) if "selection-background" in data else None,
                    "lum": luminance,
                    "is_dark": luminance < 0.5,
                }
            )

    return themes


def generate_payload(archive_url: str, zon_url: str, themes: list[dict]) -> dict:
    grouped: dict[str, list[dict]] = {}
    for theme in themes:
        grouped.setdefault(canonical_base(theme["name"]), []).append(theme)

    entries: list[dict] = []
    for base in sorted(grouped.keys(), key=lambda value: value.lower()):
        variants = grouped[base]
        dark_variant = choose_variant(variants, "dark")
        light_variant = choose_variant(variants, "light")
        entries.append(
            {
                "name": base,
                "darkVariant": dark_variant["name"],
                "lightVariant": light_variant["name"],
                "dark": semantic_from(dark_variant),
                "light": semantic_from(light_variant),
            }
        )

    return {
        "upstream": {
            "source": "ghostty-org/ghostty (iterm2_themes dependency)",
            "buildZigZonUrl": zon_url,
            "archiveUrl": archive_url,
            "generatedAt": datetime.now(timezone.utc).isoformat(),
            "themeFileCount": len(themes),
            "baseThemeCount": len(entries),
        },
        "themes": entries,
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--output", required=True)
    parser.add_argument("--zon-url", default=DEFAULT_ZON_URL)
    parser.add_argument("--archive-url")
    args = parser.parse_args()

    archive_url = args.archive_url
    if not archive_url:
        zon_text = fetch_bytes(args.zon_url).decode("utf-8", errors="ignore")
        archive_url = extract_archive_url(zon_text)

    archive_bytes = fetch_bytes(archive_url)
    themes = parse_theme_archive(archive_bytes)
    payload = generate_payload(archive_url, args.zon_url, themes)

    out_path = Path(args.output)
    out_path.parent.mkdir(parents=True, exist_ok=True)
    out_path.write_text(json.dumps(payload, separators=(",", ":"), ensure_ascii=False), encoding="utf-8")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
