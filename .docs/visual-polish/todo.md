# Visual Polish Plan — Make Diffy Stunning

Gap analysis derived from Zed's rendering stack, GPUI shaders, design system,
and comparing against diffy's current `src/render/`, `src/ui/theme.rs`, and
`src/ui/design.rs`.

## Status

| Priority | Item | Status |
|----------|------|--------|
| **P0** | Real Gaussian blur shadows | **DONE** |
| **P1** | Multi-layer elevation system | **DONE** |
| **P2** | Perceptual 12-step color scales | **DONE** |
| **P3** | Text contrast/gamma tuning | Pending |
| **P4** | Ghost element transparency audit | **DONE** |
| **P5** | Quintic easing + 150ms standard | **DONE** |
| **P6** | Custom auto-hiding scrollbars | Pending |
| **P7** | Per-corner radius (Rust side) | **DONE** |
| **P8** | Per-edge border widths (Rust side) | **DONE** |
| **P9** | Density scaling | Pending |

---

## P0 — Real Gaussian Blur Shadows

**Impact: Massive | Effort: Medium | DONE**

### Problem

`ShadowPrimitive` is rendered as an expanded rounded rect with a solid color.
The fragment shader never blurs anything — the "shadow" is just a bigger opaque
shape behind the element. This is the single biggest reason diffy looks flat.

Current code (`renderer.rs:566-580`):
```rust
let expansion = shadow.blur_radius.max(1.0);
let expanded = Rect {
    x: shadow.rect.x - expansion,
    y: shadow.rect.y - expansion,
    width: shadow.rect.width + expansion * 2.0,
    height: shadow.rect.height + expansion * 2.0,
};
// → draws a solid-color rounded rect, no actual blur
```

### What Zed Does

- Gaussian blur computed per-pixel in the fragment shader via `erf()`
  approximation (error function integral).
- Corner-aware: blur respects rounded corners so it doesn't bleed through.
- Multi-layer: modals use 4 separate shadow draws at different blur/offset
  combos for realistic depth cues.

Key Zed shader functions:
```wgsl
fn gaussian(x: f32, sigma: f32) -> f32 {
    return exp(-(x * x) / (2.0 * sigma * sigma)) / (sqrt(2.0 * M_PI_F) * sigma);
}

fn blur_along_x(x: f32, y: f32, sigma: f32, corner: f32, half_size: vec2) -> f32 {
    let delta = min(half_size.y - corner - abs(y), 0.0);
    let curved = half_size.x - corner + sqrt(max(0.0, corner * corner - delta * delta));
    let integral = 0.5 + 0.5 * erf((x + vec2(-curved, curved)) * (sqrt(0.5) / sigma));
    return integral.y - integral.x;
}
```

### Tasks

- [ ] **P0.1** Write a dedicated shadow fragment shader with Gaussian falloff.
  Pass `blur_radius`, `corner_radius`, quad bounds, and shadow color as
  uniforms/instance data. Compute `erf()`-based blur per-pixel.
- [ ] **P0.2** Create a separate `shadow_pipeline` in `Renderer` (vertex format
  can match `QuadInstance` plus a `blur_radius` field, or reuse the existing
  layout with an extra float).
- [ ] **P0.3** In `flatten_scene`, emit shadow quads to a separate list so they
  render in their own pass before the main quad pass.
- [ ] **P0.4** Support shadow `offset` (x, y) — Zed offsets shadows downward
  to simulate top-down lighting.
- [ ] **P0.5** Validate visually: a modal floating over the editor should cast a
  soft, directional shadow that fades smoothly to transparent.

### Files to touch

- `src/render/renderer.rs` — new pipeline, new instance struct, new shader
- `src/render/scene.rs` — extend `ShadowPrimitive` with offset
- `src/ui/design.rs` — update `Elevation` shadow params

---

## P1 — Multi-Layer Elevation System

**Impact: High | Effort: Low (once P0 lands)**

### Problem

`design.rs` defines four elevation levels (`Surface`, `Raised`, `Popover`,
`Modal`) with shadow parameters, but the renderer can't distinguish them because
there's no real blur. Everything reads as the same depth.

### What Zed Does

Each elevation tier has distinct shadow layers:

| Level | Layers | Details |
|-------|--------|---------|
| Surface | 0 | No shadow |
| Elevated | 2 | (0, 2px) blur 3px α=0.12 + (0, 1px) blur 0px α=0.03-0.06 |
| Modal | 4 | blur 3px + blur 6px + blur 12px + 1px accent contour |

The layering creates a subtle gradient from tight contact shadow (sharp, close)
to diffuse ambient shadow (soft, spread). Single-layer shadows always look
cartoonish.

### Tasks

- [ ] **P1.1** Extend `Elevation` in `design.rs` to emit a `Vec<ShadowSpec>`
  (multiple shadows per level) instead of a single shadow config.
- [ ] **P1.2** Update shadow-emitting code in shell/components to push multiple
  `ShadowPrimitive`s per elevated element.
- [ ] **P1.3** Tune shadow parameters per level:
  - `Raised`: 2 layers — tight contact (blur 3px, offset 2px, α=0.12) + ambient (blur 0px, offset 1px, α=0.05)
  - `Popover`: 3 layers — contact + mid-range (blur 6px) + ambient
  - `Modal`: 4 layers — full stack with outermost blur 12px
- [ ] **P1.4** Visual QA: stack a modal over a popover over the editor and
  confirm each tier reads as a distinct depth plane.

### Files to touch

- `src/ui/design.rs` — `Elevation` shadow definitions
- `src/ui/shell.rs` — where elevated containers are drawn
- `src/ui/components/modal.rs`, `toast.rs`, `picker.rs` — elevated components

---

## P2 — Perceptual 12-Step Color Scales

**Impact: High | Effort: Medium**

### Problem

The palette is hand-picked hex values with small, somewhat arbitrary steps
between background layers (`#111316` → `#16191e` → `#1c1f26`). Surfaces are
hard to distinguish, and the overall feel is muddy rather than layered.

### What Zed Does

12-step color scales per hue generated in a perceptual color space, with alpha
variants for each step. Steps have defined semantic roles:

- Steps 1-2: app backgrounds (barely visible contrast)
- Steps 3-5: component backgrounds (hover/active states, 4.5:1 contrast)
- Steps 6-8: borders (subtle → strong)
- Step 9: most saturated — semantic indicators (error, warning, success)
- Steps 10-12: text/icons (increasing prominence)

Each step also has a light-alpha and dark-alpha variant for transparency-based
layering.

### Tasks

- [ ] **P2.1** Pick a base neutral hue (current palette leans blue-grey ~220°).
  Generate a 12-step Oklch scale from near-black to near-white with even
  perceptual spacing.
- [ ] **P2.2** Generate 12-step scales for accent (blue), error (red), warning
  (yellow/gold), success (green), and info (blue). Each scale needs a light
  and dark variant.
- [ ] **P2.3** Generate alpha variants for each scale (same hue/chroma, varying
  alpha) for use in ghost elements, overlays, and scrim.
- [ ] **P2.4** Map the 12 steps to theme tokens:
  - `background` ← step 1
  - `canvas` / `editor_surface` ← step 2
  - `surface` / `panel` ← step 3
  - `elevated_surface` ← step 4
  - `element_background` ← step 4-5
  - `element_hover` ← step 5
  - `element_active` ← step 6
  - `border_variant` ← step 6
  - `border` ← step 7
  - `border_strong` ← step 8
  - `text_muted` ← step 10
  - `text` ← step 11
  - `text_strong` ← step 12
- [ ] **P2.5** Update `theme.rs` `dark()` and `light()` constructors to use the
  generated scales.
- [ ] **P2.6** Visual QA: verify that each surface tier is distinguishable,
  borders are visible but not harsh, and text contrast meets WCAG AA (4.5:1).

### Files to touch

- `src/ui/theme.rs` — color definitions
- Possibly a new `src/ui/palette.rs` for scale generation utilities

---

## P3 — Text Contrast and Gamma Tuning

**Impact: Medium-High | Effort: Medium-Hard**

### Problem

Text is rendered through `glyphon` with no post-processing. Light text on dark
backgrounds is perceptually thinner than dark text on light backgrounds (a known
optical illusion). Without compensation, body text in dark mode looks washed out
and spindly.

### What Zed Does

Custom shader pipeline adapted from Windows Terminal:
- Per-channel gamma correction with 13 precomputed profiles (γ 1.0–2.2)
- Light-on-dark contrast boost via `light_on_dark_contrast()` function
- Subpixel rendering at 4× subpixel variants (X and Y on macOS)
- sRGB ↔ linear conversion with proper gamma curves

### Tasks

- [ ] **P3.1** Short-term fix: set font weight to `Weight::MEDIUM` (500) for
  dark-mode body text in `attrs_for_font()`. This counteracts perceptual
  thinning with minimal code change.
- [ ] **P3.2** Evaluate glyphon's `SubpixelBin` support — check whether the
  current setup uses subpixel positioning and whether quality improves by
  explicitly enabling it.
- [ ] **P3.3** Long-term: investigate rendering text to a texture and applying a
  gamma-correction post-process pass before compositing. This would require:
  - A text-only render target
  - A fullscreen composite pass that applies gamma correction
  - Shader with `apply_contrast_and_gamma_correction3()` logic from Zed
- [ ] **P3.4** Evaluate whether bundling a specific font (like IBM Plex Mono or
  Iosevka) with tuned metrics would improve consistency vs. relying on
  system Consolas/Segoe UI.

### Files to touch

- `src/render/renderer.rs` — text preparation, possible post-process pass
- `src/ui/theme.rs` — font weight tokens per mode

---

## P4 — Ghost Element Transparency System

**Impact: Medium | Effort: Low**

### Problem

Interactive states use solid color swaps (e.g., `element_background` →
`element_hover` → `element_active`). These feel jarring because the hover color
is unrelated to the surface beneath — it's a hard jump.

### What Zed Does

Ghost elements use semi-transparent overlays:
- Default: fully transparent
- Hover: 10–11% white (dark mode) / 5–6% black (light mode)
- Active: 15–18% opacity
- Selected: tinted semi-transparent

This means hover states blend naturally over whatever background they sit on.

### Current State

The theme already defines ghost tokens:
```rust
ghost_element_hover: Color::rgba(255, 255, 255, 15),   // ~6%
ghost_element_active: Color::rgba(255, 255, 255, 24),  // ~9%
ghost_element_selected: ...
```

But these may not be used consistently across all interactive elements.

### Tasks

- [ ] **P4.1** Audit every button, sidebar row, toolbar item, and list item.
  Ensure ghost-style elements (those on varying backgrounds) use ghost tokens
  instead of solid `element_hover`.
- [ ] **P4.2** Increase ghost opacity slightly — current 6% hover is very
  subtle. Try 10-11% (α ≈ 26-28) for hover, 15-18% (α ≈ 38-46) for active.
- [ ] **P4.3** Ensure the blending mode in the quad shader handles
  semi-transparent fills correctly over arbitrary backgrounds (the current
  `PREMULTIPLIED_ALPHA_BLENDING` should work, but verify visually).
- [ ] **P4.4** Light mode: use semi-transparent black instead of white for ghost
  overlays.

### Files to touch

- `src/ui/theme.rs` — ghost token values
- `src/ui/components/button.rs` — button hover/active states
- `src/ui/components/list_item.rs` — sidebar row states
- `src/ui/shell.rs` — toolbar button states

---

## P5 — Quintic Easing and 150ms Standard Duration

**Impact: Medium | Effort: Trivial**

### Problem

Current easing is ease-out cubic: `1.0 - (1.0 - t)^3`. This is decent but
slightly abrupt at the start of the animation.

### What Zed Does

- Ease-out quint: `1.0 - (1.0 - t)^5`
- Standard duration: 150ms for hover/press transitions
- Simultaneous position + opacity animation for entrances

The higher exponent means faster initial response (feels snappier) with a
gentler landing (feels smoother).

### Tasks

- [ ] **P5.1** Change easing function in `animation.rs` from
  `1.0 - (1.0 - t).powi(3)` to `1.0 - (1.0 - t).powi(5)`.
- [ ] **P5.2** Standardize transition duration to 150ms for all hover/active
  state changes. Check current defaults and adjust if different.
- [ ] **P5.3** For toast/modal entrance animations, add simultaneous opacity
  fade (0.4 → 1.0) alongside any position animation.

### Files to touch

- `src/ui/animation.rs` — easing function, default duration

---

## P6 — Custom Auto-Hiding Scrollbars

**Impact: Medium | Effort: Low-Medium**

### Problem

No custom scrollbar rendering. Likely using platform default or none.

### What Zed Does

- Semi-transparent rounded scrollbar thumbs using neutral alpha colors
- 3 opacity states: idle (subtle), hover (medium), active (prominent)
- Auto-hide: 1 second after last scroll, then 400ms fade-out
- Small radius for rounded corners

### Tasks

- [ ] **P6.1** Add a `ScrollbarState` struct tracking: scroll offset, content
  height, viewport height, last-scroll timestamp, hover state.
- [ ] **P6.2** Draw scrollbar thumb as a rounded rect with ghost-element colors:
  - Idle: `neutral_alpha.step_3` (barely visible)
  - Hover: `neutral_alpha.step_4`
  - Active: `neutral_alpha.step_5`
- [ ] **P6.3** Implement auto-hide: start fade 1000ms after last scroll event,
  fade over 400ms using existing animation system.
- [ ] **P6.4** Width: 6px thumb with 2px padding from edge. Border radius: 3px
  (half width for pill shape).
- [ ] **P6.5** Apply to diff viewport and sidebar file list.

### Files to touch

- New: `src/ui/components/scrollbar.rs`
- `src/ui/shell.rs` — integrate into viewport and sidebar

---

## P7 — Per-Corner Radius Support (Rust Side)

**Impact: Low-Medium | Effort: Trivial**

### Problem

`RoundedRectPrimitive` has a single `radius: f32`. All four corners are always
the same. This prevents compound shapes like tabs (rounded top, square bottom)
or nested containers where inner corners need smaller radii.

### Current State

The shader already handles per-corner radii — `corner_radii: vec4<f32>` and
`pick_corner_radius()` are fully implemented. Only the Rust-side primitive is
limited.

### Tasks

- [ ] **P7.1** Change `RoundedRectPrimitive` from `radius: f32` to
  `corner_radii: [f32; 4]` (tl, tr, br, bl).
- [ ] **P7.2** Add a convenience constructor: `RoundedRectPrimitive::uniform(r)`
  that sets all four to the same value (keeps existing call sites clean).
- [ ] **P7.3** Update `flatten_scene` to pass per-corner values through.
- [ ] **P7.4** Update all existing call sites.

### Files to touch

- `src/render/scene.rs` — primitive definition
- `src/render/renderer.rs` — `flatten_scene` mapping
- All call sites that create `RoundedRectPrimitive`

---

## P8 — Per-Edge Border Widths (Rust Side)

**Impact: Low-Medium | Effort: Trivial**

### Problem

`BorderPrimitive` has a single `width: f32`. All four edges get the same width.
Zed uses per-edge borders for bottom-only panel dividers, left-accent
indicators, etc.

### Current State

Like P7, the shader already handles `border_widths: vec4<f32>` per edge. Only
the Rust side is limited.

### Tasks

- [ ] **P8.1** Change `BorderPrimitive` from `width: f32` to
  `widths: [f32; 4]` (top, right, bottom, left).
- [ ] **P8.2** Add convenience: `BorderPrimitive::uniform(w)` and
  `BorderPrimitive::bottom(w)` etc.
- [ ] **P8.3** Update `flatten_scene` to pass per-edge values through.
- [ ] **P8.4** Update all existing call sites.

### Files to touch

- `src/render/scene.rs` — primitive definition
- `src/render/renderer.rs` — `flatten_scene` mapping
- All call sites that create `BorderPrimitive`

---

## P9 — UI Density Scaling

**Impact: Low | Effort: Low**

### Problem

Spacing is fixed. There's no way for users to choose tighter or more spacious
layouts.

### What Zed Does

Three density tiers that scale all spacing proportionally:
- Compact: (1px, 1px, 2px)
- Default: (2px, 4px, 8px)
- Comfortable: (4px, 6px, 10px)

### Tasks

- [ ] **P9.1** Add a `UiDensity` enum (Compact, Default, Comfortable) to
  settings/theme.
- [ ] **P9.2** Multiply all `Sp::*` constants by a density factor:
  - Compact: 0.75×
  - Default: 1.0×
  - Comfortable: 1.25×
- [ ] **P9.3** Expose density toggle in UI (toolbar or settings).
- [ ] **P9.4** Ensure layout recalculates when density changes.

### Files to touch

- `src/ui/design.rs` — spacing system
- `src/ui/theme.rs` — density setting
- `src/ui/shell.rs` — toolbar toggle

---

## Execution Order

```
P0 (shadows) ──→ P1 (elevation layers)
                          │
P5 (easing) ──────────────┤  ← trivial, do alongside anything
P7 (per-corner radius) ───┤
P8 (per-edge borders) ────┘

P2 (color scales) ──→ P4 (ghost elements)

P3 (text gamma) ── standalone, can start anytime

P6 (scrollbars) ── standalone, after P4 (uses ghost tokens)

P9 (density) ── last, lowest priority
```

Start with **P0 + P5 + P7 + P8** in a single pass (they all touch the renderer
and are mostly independent). Then **P1** (trivial once P0 exists). Then
**P2 + P4** for color system overhaul. **P3** and **P6** whenever there's
bandwidth. **P9** last.
