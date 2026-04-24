pub(super) const SHADOW_SHADER: &str = r#"
struct ViewportUniform {
    resolution: vec2<f32>,
    time: f32,
    _padding: f32,
};

@group(0) @binding(0)
var<uniform> viewport: ViewportUniform;

struct VertexInput {
    @builtin(vertex_index) vertex_id: u32,
    @location(0) draw_bounds: vec4<f32>,
    @location(1) shadow_bounds: vec4<f32>,
    @location(2) color: vec4<f32>,
    @location(3) params: vec4<f32>,
    @location(4) clip_bounds: vec4<f32>,
    @location(5) clip_radii: vec4<f32>,
};

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) @interpolate(flat) shadow_bounds: vec4<f32>,
    @location(1) @interpolate(flat) color: vec4<f32>,
    @location(2) @interpolate(flat) params: vec4<f32>,
    @location(3) @interpolate(flat) clip_bounds: vec4<f32>,
    @location(4) @interpolate(flat) clip_radii: vec4<f32>,
};

@vertex
fn vs_shadow(input: VertexInput) -> VertexOutput {
    let unit = vec2<f32>(
        f32(input.vertex_id & 1u),
        f32((input.vertex_id >> 1u) & 1u)
    );
    let pixel_pos = input.draw_bounds.xy + unit * input.draw_bounds.zw;
    let ndc = pixel_pos / viewport.resolution * vec2<f32>(2.0, -2.0) + vec2<f32>(-1.0, 1.0);

    var out: VertexOutput;
    out.position = vec4<f32>(ndc, 0.0, 1.0);
    out.shadow_bounds = input.shadow_bounds;
    out.color = input.color;
    out.params = input.params;
    out.clip_bounds = input.clip_bounds;
    out.clip_radii = input.clip_radii;
    return out;
}

// Attempt to approximate erf using a polynomial fit.
// Abramowitz & Stegun 7.1.26 — max error < 1.5e-7, which is more than
// enough for visual blur.
fn erf_approx(x: f32) -> f32 {
    let sign = select(-1.0, 1.0, x >= 0.0);
    let a = abs(x);
    let t = 1.0 / (1.0 + 0.3275911 * a);
    let t2 = t * t;
    let t3 = t2 * t;
    let t4 = t3 * t;
    let t5 = t4 * t;
    let poly = 0.254829592 * t - 0.284496736 * t2 + 1.421413741 * t3
             - 1.453152027 * t4 + 1.061405429 * t5;
    return sign * (1.0 - poly * exp(-a * a));
}

// Integral of 1D Gaussian from -inf to x with given sigma.
fn gauss_integral(x: f32, sigma: f32) -> f32 {
    return 0.5 + 0.5 * erf_approx(x / (sigma * 1.4142135));
}

// Rounded-rect SDF (distance from point p to the rounded rect centered at
// origin with given half_size and corner_radius).
fn rounded_rect_sdf(p: vec2<f32>, half_size: vec2<f32>, radius: f32) -> f32 {
    let q = abs(p) - half_size + vec2<f32>(radius);
    return length(max(q, vec2<f32>(0.0))) + min(max(q.x, q.y), 0.0) - radius;
}

fn shadow_pick_corner_radius(p: vec2<f32>, radii: vec4<f32>) -> f32 {
    if (p.x < 0.0) {
        return select(radii.w, radii.x, p.y < 0.0);
    } else {
        return select(radii.z, radii.y, p.y < 0.0);
    }
}

fn shadow_clip_alpha(
    pixel_pos: vec2<f32>,
    clip_bounds: vec4<f32>,
    clip_radii: vec4<f32>,
) -> f32 {
    if (clip_radii.x <= 0.0 && clip_radii.y <= 0.0 && clip_radii.z <= 0.0 && clip_radii.w <= 0.0) {
        return 1.0;
    }
    let clip_half = clip_bounds.zw * 0.5;
    let clip_center = clip_bounds.xy + clip_half;
    let cp = pixel_pos - clip_center;
    let cr = shadow_pick_corner_radius(cp, clip_radii);
    let clip_sdf = rounded_rect_sdf(cp, clip_half, cr);
    return saturate(0.5 - clip_sdf);
}

@fragment
fn fs_shadow(input: VertexOutput) -> @location(0) vec4<f32> {
    let sigma = input.params.x;
    let corner_radius = input.params.y;
    let half_size = input.shadow_bounds.zw * 0.5;
    let center = input.shadow_bounds.xy + half_size;
    let p = input.position.xy - center;

    // For the blurred shadow, we compute the convolution of the rounded-rect
    // indicator function with a 2D Gaussian. For a box (no rounding), this
    // factors into the product of two 1D Gaussian integrals. For rounded
    // corners we use a hybrid: compute the box integral and multiply by a
    // smooth SDF-based corner correction.

    // Separable box blur integral.
    let ax = gauss_integral(p.x + half_size.x, sigma)
           - gauss_integral(p.x - half_size.x, sigma);
    let ay = gauss_integral(p.y + half_size.y, sigma)
           - gauss_integral(p.y - half_size.y, sigma);
    var alpha = ax * ay;

    // Corner correction: fade out the corners that the box integral
    // over-estimates. We sample the SDF and use the sigma to smooth it.
    if (corner_radius > 0.0) {
        let sdf = rounded_rect_sdf(p, half_size, corner_radius);
        // Outside the rounded rect, attenuate based on how far outside.
        // The smoothstep range is proportional to sigma for a soft edge.
        let corner_fade = 1.0 - smoothstep(-sigma * 0.5, sigma * 1.5, sdf);
        alpha = alpha * corner_fade;
    }

    alpha = alpha * shadow_clip_alpha(input.position.xy, input.clip_bounds, input.clip_radii);

    let final_alpha = input.color.a * alpha;
    if (final_alpha < 0.001) {
        discard;
    }
    return vec4<f32>(input.color.rgb * final_alpha, final_alpha);
}
"#;

// ---------------------------------------------------------------------------
// SDF quad shader
// ---------------------------------------------------------------------------

pub(super) const QUAD_SHADER: &str = r#"
struct ViewportUniform {
    resolution: vec2<f32>,
    time: f32,
    _padding: f32,
};

@group(0) @binding(0)
var<uniform> viewport: ViewportUniform;

struct VertexInput {
    @builtin(vertex_index) vertex_id: u32,
    @location(0) bounds: vec4<f32>,
    @location(1) background: vec4<f32>,
    @location(2) border_color: vec4<f32>,
    @location(3) corner_radii: vec4<f32>,
    @location(4) border_widths: vec4<f32>,
    @location(5) clip_bounds: vec4<f32>,
    @location(6) clip_radii: vec4<f32>,
};

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) @interpolate(flat) bounds: vec4<f32>,
    @location(1) @interpolate(flat) background: vec4<f32>,
    @location(2) @interpolate(flat) border_color: vec4<f32>,
    @location(3) @interpolate(flat) corner_radii: vec4<f32>,
    @location(4) @interpolate(flat) border_widths: vec4<f32>,
    @location(5) @interpolate(flat) clip_bounds: vec4<f32>,
    @location(6) @interpolate(flat) clip_radii: vec4<f32>,
};

@vertex
fn vs_quad(input: VertexInput) -> VertexOutput {
    let unit = vec2<f32>(
        f32(input.vertex_id & 1u),
        f32((input.vertex_id >> 1u) & 1u)
    );
    let pixel_pos = input.bounds.xy + unit * input.bounds.zw;
    let ndc = pixel_pos / viewport.resolution * vec2<f32>(2.0, -2.0) + vec2<f32>(-1.0, 1.0);

    var out: VertexOutput;
    out.position = vec4<f32>(ndc, 0.0, 1.0);
    out.bounds = input.bounds;
    out.background = input.background;
    out.border_color = input.border_color;
    out.corner_radii = input.corner_radii;
    out.border_widths = input.border_widths;
    out.clip_bounds = input.clip_bounds;
    out.clip_radii = input.clip_radii;
    return out;
}

fn pick_corner_radius(p: vec2<f32>, radii: vec4<f32>) -> f32 {
    // radii: tl, tr, br, bl
    if (p.x < 0.0) {
        return select(radii.w, radii.x, p.y < 0.0);
    } else {
        return select(radii.z, radii.y, p.y < 0.0);
    }
}

fn quad_sdf(p: vec2<f32>, half_size: vec2<f32>, radius: f32) -> f32 {
    let d = abs(p) - half_size + vec2<f32>(radius);
    return length(max(d, vec2<f32>(0.0))) + min(max(d.x, d.y), 0.0) - radius;
}

fn over(below: vec4<f32>, above: vec4<f32>) -> vec4<f32> {
    let a = above.a + below.a * (1.0 - above.a);
    if (a <= 0.0) {
        return vec4<f32>(0.0);
    }
    let c = (above.rgb * above.a + below.rgb * below.a * (1.0 - above.a)) / a;
    return vec4<f32>(c, a);
}

// Anti-aliased rounded-clip alpha. Returns 1.0 when no rounded clip is
// active (all radii zero), otherwise fades with the clip-rect SDF.
fn rounded_clip_alpha(
    pixel_pos: vec2<f32>,
    clip_bounds: vec4<f32>,
    clip_radii: vec4<f32>,
) -> f32 {
    if (clip_radii.x <= 0.0 && clip_radii.y <= 0.0 && clip_radii.z <= 0.0 && clip_radii.w <= 0.0) {
        return 1.0;
    }
    let clip_half = clip_bounds.zw * 0.5;
    let clip_center = clip_bounds.xy + clip_half;
    let cp = pixel_pos - clip_center;
    let cr = pick_corner_radius(cp, clip_radii);
    let clip_sdf = quad_sdf(cp, clip_half, cr);
    return saturate(0.5 - clip_sdf);
}

@fragment
fn fs_quad(input: VertexOutput) -> @location(0) vec4<f32> {
    let half_size = input.bounds.zw * 0.5;
    let center = input.bounds.xy + half_size;
    let p = input.position.xy - center;

    let corner_radius = pick_corner_radius(p, input.corner_radii);
    let outer_sdf = quad_sdf(p, half_size, corner_radius);

    let aa = 0.5;
    let outer_alpha = saturate(aa - outer_sdf);
    if (outer_alpha <= 0.0) {
        discard;
    }

    let clip_alpha = rounded_clip_alpha(input.position.xy, input.clip_bounds, input.clip_radii);
    if (clip_alpha <= 0.0) {
        discard;
    }

    let max_border = max(
        max(input.border_widths.x, input.border_widths.y),
        max(input.border_widths.z, input.border_widths.w)
    );

    var color: vec4<f32>;
    if (max_border > 0.0) {
        let bw = max_border;
        let inner_half = half_size - vec2<f32>(bw);
        let inner_radius = max(0.0, corner_radius - bw);
        let inner_sdf = quad_sdf(p, inner_half, inner_radius);
        let fill_blend = saturate(aa - inner_sdf);
        let blended = over(input.background, input.border_color);
        color = mix(blended, input.background, fill_blend);
    } else {
        color = input.background;
    }

    let final_alpha = color.a * outer_alpha * clip_alpha;
    return vec4<f32>(color.rgb * final_alpha, final_alpha);
}
"#;

// ---------------------------------------------------------------------------
// Procedural effect shader — noise gradient + linear gradient
// ---------------------------------------------------------------------------

pub(super) const EFFECT_SHADER: &str = r#"
struct ViewportUniform {
    resolution: vec2<f32>,
    time: f32,
    _padding: f32,
};

@group(0) @binding(0)
var<uniform> viewport: ViewportUniform;

struct VertexInput {
    @builtin(vertex_index) vertex_id: u32,
    @location(0) bounds: vec4<f32>,
    @location(1) color_a: vec4<f32>,
    @location(2) color_b: vec4<f32>,
    @location(3) params: vec4<f32>,   // [effect_type, param1, param2, corner_radius]
    @location(4) clip_bounds: vec4<f32>,
    @location(5) clip_radii: vec4<f32>,
};

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) @interpolate(flat) bounds: vec4<f32>,
    @location(1) @interpolate(flat) color_a: vec4<f32>,
    @location(2) @interpolate(flat) color_b: vec4<f32>,
    @location(3) @interpolate(flat) params: vec4<f32>,
    @location(4) @interpolate(flat) clip_bounds: vec4<f32>,
    @location(5) @interpolate(flat) clip_radii: vec4<f32>,
};

@vertex
fn vs_effect(input: VertexInput) -> VertexOutput {
    let unit = vec2<f32>(
        f32(input.vertex_id & 1u),
        f32((input.vertex_id >> 1u) & 1u)
    );
    let pixel_pos = input.bounds.xy + unit * input.bounds.zw;
    let ndc = pixel_pos / viewport.resolution * vec2<f32>(2.0, -2.0) + vec2<f32>(-1.0, 1.0);

    var out: VertexOutput;
    out.position = vec4<f32>(ndc, 0.0, 1.0);
    out.bounds = input.bounds;
    out.color_a = input.color_a;
    out.color_b = input.color_b;
    out.params = input.params;
    out.clip_bounds = input.clip_bounds;
    out.clip_radii = input.clip_radii;
    return out;
}

// ---- Simplex noise (2D) ----

fn mod289_v3(x: vec3<f32>) -> vec3<f32> {
    return x - floor(x * (1.0 / 289.0)) * 289.0;
}

fn mod289_v2(x: vec2<f32>) -> vec2<f32> {
    return x - floor(x * (1.0 / 289.0)) * 289.0;
}

fn permute(x: vec3<f32>) -> vec3<f32> {
    return mod289_v3(((x * 34.0) + vec3<f32>(10.0)) * x);
}

fn simplex_noise(v: vec2<f32>) -> f32 {
    let C = vec4<f32>(
        0.211324865405187,   // (3.0 - sqrt(3.0)) / 6.0
        0.366025403784439,   // 0.5 * (sqrt(3.0) - 1.0)
        -0.577350269189626,  // -1.0 + 2.0 * C.x
        0.024390243902439    // 1.0 / 41.0
    );

    // First corner.
    var i = floor(v + dot(v, C.yy));
    let x0 = v - i + dot(i, C.xx);

    // Other corners.
    let i1 = select(vec2<f32>(0.0, 1.0), vec2<f32>(1.0, 0.0), x0.x > x0.y);
    var x12 = x0.xyxy + C.xxzz;
    x12 = vec4<f32>(x12.xy - i1, x12.zw);

    // Permutations.
    i = mod289_v2(i);
    let p = permute(permute(
        i.y + vec3<f32>(0.0, i1.y, 1.0))
      + i.x + vec3<f32>(0.0, i1.x, 1.0));

    var m = max(vec3<f32>(0.5) - vec3<f32>(
        dot(x0, x0),
        dot(x12.xy, x12.xy),
        dot(x12.zw, x12.zw)
    ), vec3<f32>(0.0));
    m = m * m;
    m = m * m;

    // Gradients.
    let x_ = 2.0 * fract(p * C.www) - vec3<f32>(1.0);
    let h = abs(x_) - vec3<f32>(0.5);
    let ox = floor(x_ + vec3<f32>(0.5));
    let a0 = x_ - ox;

    // Approximate normalisation.
    m = m * (vec3<f32>(1.79284291400159) - vec3<f32>(0.85373472095314) * (a0 * a0 + h * h));

    // Compute final noise value at P.
    let g = vec3<f32>(
        a0.x * x0.x + h.x * x0.y,
        a0.y * x12.x + h.y * x12.y,
        a0.z * x12.z + h.z * x12.w
    );

    return 130.0 * dot(m, g);
}

// ---- Rounded-rect SDF for masking ----

fn effect_sdf(p: vec2<f32>, half_size: vec2<f32>, radius: f32) -> f32 {
    let q = abs(p) - half_size + vec2<f32>(radius);
    return length(max(q, vec2<f32>(0.0))) + min(max(q.x, q.y), 0.0) - radius;
}

fn effect_pick_corner_radius(p: vec2<f32>, radii: vec4<f32>) -> f32 {
    if (p.x < 0.0) {
        return select(radii.w, radii.x, p.y < 0.0);
    } else {
        return select(radii.z, radii.y, p.y < 0.0);
    }
}

fn effect_clip_alpha(
    pixel_pos: vec2<f32>,
    clip_bounds: vec4<f32>,
    clip_radii: vec4<f32>,
) -> f32 {
    if (clip_radii.x <= 0.0 && clip_radii.y <= 0.0 && clip_radii.z <= 0.0 && clip_radii.w <= 0.0) {
        return 1.0;
    }
    let clip_half = clip_bounds.zw * 0.5;
    let clip_center = clip_bounds.xy + clip_half;
    let cp = pixel_pos - clip_center;
    let cr = effect_pick_corner_radius(cp, clip_radii);
    let clip_sdf = effect_sdf(cp, clip_half, cr);
    return saturate(0.5 - clip_sdf);
}

// ---- Fragment shader ----

@fragment
fn fs_effect(input: VertexOutput) -> @location(0) vec4<f32> {
    let half_size = input.bounds.zw * 0.5;
    let center = input.bounds.xy + half_size;
    let p = input.position.xy - center;
    let corner_radius = input.params.w;

    // Rounded-rect mask.
    let sdf = effect_sdf(p, half_size, corner_radius);
    let mask_self = saturate(0.5 - sdf);
    let mask_clip = effect_clip_alpha(input.position.xy, input.clip_bounds, input.clip_radii);
    let mask = mask_self * mask_clip;
    if (mask <= 0.0) {
        discard;
    }

    // Normalised UV within the element bounds.
    let uv = (input.position.xy - input.bounds.xy) / input.bounds.zw;

    let effect_type = u32(input.params.x);
    var color: vec4<f32>;

    switch (effect_type) {
        // Type 0: Noise gradient — simplex noise blended between two colors.
        case 0u: {
            let scale = input.params.y;
            let noise_coord = input.position.xy * scale + vec2<f32>(viewport.time * 3.0);
            let n = simplex_noise(noise_coord) * 0.5 + 0.5;
            // Layer a second octave for richer texture.
            let n2 = simplex_noise(noise_coord * 2.0 + vec2<f32>(17.3, 31.7)) * 0.5 + 0.5;
            let combined = n * 0.7 + n2 * 0.3;
            // Blend from color_a (top) to color_b (bottom) modulated by noise.
            let gradient = uv.y;
            let t = saturate(gradient + (combined - 0.5) * 0.4);
            color = mix(input.color_a, input.color_b, t);
        }
        // Type 1: Linear gradient with angle.
        case 1u: {
            let angle = input.params.y;
            let dir = vec2<f32>(cos(angle), sin(angle));
            let t = saturate(dot(uv - vec2<f32>(0.5), dir) + 0.5);
            color = mix(input.color_a, input.color_b, t);
        }
        // Type 2: Radial gradient — color_a at center, color_b at edge.
        case 2u: {
            let center = vec2<f32>(0.5, 0.5);
            let d = length((uv - center) * 2.0);
            let t = saturate(d);
            color = mix(input.color_a, input.color_b, t);
        }
        // Type 3: Animated shimmer — diagonal highlight sweep.
        case 3u: {
            let speed = input.params.y;
            // Diagonal position: combine x and y into a single sweep axis.
            let diag = (uv.x + uv.y) * 0.5;
            // Animate the highlight band across the diagonal.
            let phase = fract(viewport.time * speed * 0.3);
            let band_center = phase * 1.6 - 0.3; // sweep from left to right with overshoot
            let band = 1.0 - smoothstep(0.0, 0.15, abs(diag - band_center));
            color = mix(input.color_a, input.color_b, band);
        }
        // Type 4: Vignette — darken/tint edges.
        case 4u: {
            let intensity = input.params.y;
            let center = vec2<f32>(0.5, 0.5);
            let d = length((uv - center) * 2.0);
            let vignette_factor = smoothstep(0.2, 1.2, d) * intensity;
            // Start from transparent, blend toward color_a at edges.
            color = vec4<f32>(input.color_a.rgb, input.color_a.a * vignette_factor);
        }
        // Type 5: Color tint — flat semi-transparent overlay.
        case 5u: {
            color = input.color_a;
        }
        // Fallback: solid color_a.
        default: {
            color = input.color_a;
        }
    }

    let final_alpha = color.a * mask;
    if (final_alpha < 0.001) {
        discard;
    }
    return vec4<f32>(color.rgb * final_alpha, final_alpha);
}
"#;

// ---------------------------------------------------------------------------
// Blit shader — composite an offscreen texture to screen
// ---------------------------------------------------------------------------

pub(super) const BLIT_SHADER: &str = r#"
struct ViewportUniform {
    resolution: vec2<f32>,
    time: f32,
    _padding: f32,
};

@group(0) @binding(0)
var<uniform> viewport: ViewportUniform;

@group(1) @binding(0)
var t_source: texture_2d<f32>;
@group(1) @binding(1)
var s_source: sampler;

struct VertexInput {
    @builtin(vertex_index) vertex_id: u32,
    @location(0) bounds: vec4<f32>,    // screen-space destination [x, y, w, h]
    @location(1) uv_rect: vec4<f32>,   // source UV [u_min, v_min, u_max, v_max]
    @location(2) tint: vec4<f32>,      // tint/opacity
};

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) @interpolate(flat) tint: vec4<f32>,
};

@vertex
fn vs_blit(input: VertexInput) -> VertexOutput {
    let unit = vec2<f32>(
        f32(input.vertex_id & 1u),
        f32((input.vertex_id >> 1u) & 1u)
    );
    let pixel_pos = input.bounds.xy + unit * input.bounds.zw;
    let ndc = pixel_pos / viewport.resolution * vec2<f32>(2.0, -2.0) + vec2<f32>(-1.0, 1.0);

    // Interpolate UV from uv_rect min→max.
    let uv = mix(input.uv_rect.xy, input.uv_rect.zw, unit);

    var out: VertexOutput;
    out.position = vec4<f32>(ndc, 0.0, 1.0);
    out.uv = uv;
    out.tint = input.tint;
    return out;
}

@fragment
fn fs_blit(input: VertexOutput) -> @location(0) vec4<f32> {
    let tex_color = textureSample(t_source, s_source, input.uv);
    return tex_color * input.tint;
}
"#;

// ---------------------------------------------------------------------------
// Separable Gaussian blur shader — 13-tap kernel
// ---------------------------------------------------------------------------

pub(super) const BLUR_SHADER: &str = r#"
struct ViewportUniform {
    resolution: vec2<f32>,
    time: f32,
    _padding: f32,
};

@group(0) @binding(0)
var<uniform> viewport: ViewportUniform;

@group(1) @binding(0)
var t_source: texture_2d<f32>;
@group(1) @binding(1)
var s_source: sampler;

struct VertexInput {
    @builtin(vertex_index) vertex_id: u32,
    @location(0) bounds: vec4<f32>,       // [x, y, w, h] in pixel coords
    @location(1) uv_rect: vec4<f32>,      // [u_min, v_min, u_max, v_max]
    @location(2) blur_params: vec4<f32>,  // [dir_x, dir_y, sigma, 0]
};

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) @interpolate(flat) blur_params: vec4<f32>,
};

@vertex
fn vs_blur(input: VertexInput) -> VertexOutput {
    let unit = vec2<f32>(
        f32(input.vertex_id & 1u),
        f32((input.vertex_id >> 1u) & 1u)
    );
    let pixel_pos = input.bounds.xy + unit * input.bounds.zw;
    let ndc = pixel_pos / viewport.resolution * vec2<f32>(2.0, -2.0) + vec2<f32>(-1.0, 1.0);

    let uv = mix(input.uv_rect.xy, input.uv_rect.zw, unit);

    var out: VertexOutput;
    out.position = vec4<f32>(ndc, 0.0, 1.0);
    out.uv = uv;
    out.blur_params = input.blur_params;
    return out;
}

@fragment
fn fs_blur(input: VertexOutput) -> @location(0) vec4<f32> {
    let sigma = input.blur_params.z;
    let dir = vec2<f32>(input.blur_params.x, input.blur_params.y);
    let tex_size = vec2<f32>(textureDimensions(t_source));
    // Step size in UV space, scaled up for large blur radii.
    let step_scale = max(1.0, sigma / 6.0);
    let texel = dir / tex_size * step_scale;

    // 13-tap Gaussian kernel (offsets -6..+6).
    var color = vec4<f32>(0.0);
    var total_weight = 0.0;

    let w0 = exp(0.0);
    let w1 = exp(-1.0 / (2.0 * sigma * sigma));
    let w2 = exp(-4.0 / (2.0 * sigma * sigma));
    let w3 = exp(-9.0 / (2.0 * sigma * sigma));
    let w4 = exp(-16.0 / (2.0 * sigma * sigma));
    let w5 = exp(-25.0 / (2.0 * sigma * sigma));
    let w6 = exp(-36.0 / (2.0 * sigma * sigma));

    color += textureSample(t_source, s_source, input.uv + texel * -6.0) * w6;
    color += textureSample(t_source, s_source, input.uv + texel * -5.0) * w5;
    color += textureSample(t_source, s_source, input.uv + texel * -4.0) * w4;
    color += textureSample(t_source, s_source, input.uv + texel * -3.0) * w3;
    color += textureSample(t_source, s_source, input.uv + texel * -2.0) * w2;
    color += textureSample(t_source, s_source, input.uv + texel * -1.0) * w1;
    color += textureSample(t_source, s_source, input.uv)                * w0;
    color += textureSample(t_source, s_source, input.uv + texel *  1.0) * w1;
    color += textureSample(t_source, s_source, input.uv + texel *  2.0) * w2;
    color += textureSample(t_source, s_source, input.uv + texel *  3.0) * w3;
    color += textureSample(t_source, s_source, input.uv + texel *  4.0) * w4;
    color += textureSample(t_source, s_source, input.uv + texel *  5.0) * w5;
    color += textureSample(t_source, s_source, input.uv + texel *  6.0) * w6;

    total_weight = w6 + w5 + w4 + w3 + w2 + w1 + w0 + w1 + w2 + w3 + w4 + w5 + w6;

    return color / total_weight;
}
"#;
