//! The shaders.

/// Draws the background into a texture, which is then kept until the look changes.
pub const BACKGROUND: &str = r#"
struct Params {
    kind: u32,       // 0 colour, 1 linear gradient, 2 radial gradient, 3 image
    angle: f32,
    stage: vec2f,
    color: vec4f,
    image_size: vec2f,
    _pad: vec2f,
};
@group(0) @binding(0) var<uniform> p: Params;
@group(0) @binding(1) var ramp: texture_2d<f32>;
@group(0) @binding(2) var image: texture_2d<f32>;
@group(0) @binding(3) var linear_sampler: sampler;

@vertex
fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4f {
    let x = f32(i32(i & 1u) * 4 - 1);
    let y = f32(i32(i >> 1u) * 4 - 1);
    return vec4f(x, y, 0.0, 1.0);
}

@fragment
fn fs(@builtin(position) pos: vec4f) -> @location(0) vec4f {
    let uv = pos.xy / p.stage;
    if (p.kind == 0u) {
        return p.color;
    }
    if (p.kind == 1u) {
        // CSS's gradient line: as long as it takes to reach the corners at this angle.
        let direction = vec2f(sin(p.angle), -cos(p.angle));
        let length = abs(p.stage.x * direction.x) + abs(p.stage.y * direction.y);
        let t = dot(pos.xy - p.stage / 2.0, direction) / length + 0.5;
        return textureSampleLevel(ramp, linear_sampler, vec2f(clamp(t, 0.0, 1.0), 0.5), 0.0);
    }
    if (p.kind == 2u) {
        // CSS's default radial gradient reaches the farthest corner.
        let t = length((uv - vec2f(0.5)) * 2.0) / sqrt(2.0);
        return textureSampleLevel(ramp, linear_sampler, vec2f(clamp(t, 0.0, 1.0), 0.5), 0.0);
    }
    // An image, covering the stage without changing shape.
    let scale = max(p.stage.x / p.image_size.x, p.stage.y / p.image_size.y);
    let shown = p.stage / (p.image_size * scale);
    let image_uv = (uv - vec2f(0.5)) * shown + vec2f(0.5);
    return textureSampleLevel(image, linear_sampler, image_uv, 0.0);
}
"#;

/// A one-directional Gaussian blur, run once each way.
pub const BLUR: &str = r#"
struct Params {
    direction: vec2f, // one texel along the blur's axis
    sigma: f32,
    radius: f32,
};
@group(0) @binding(0) var<uniform> p: Params;
@group(0) @binding(1) var source: texture_2d<f32>;
@group(0) @binding(2) var linear_sampler: sampler;

@vertex
fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4f {
    let x = f32(i32(i & 1u) * 4 - 1);
    let y = f32(i32(i >> 1u) * 4 - 1);
    return vec4f(x, y, 0.0, 1.0);
}

@fragment
fn fs(@builtin(position) pos: vec4f) -> @location(0) vec4f {
    let size = vec2f(textureDimensions(source));
    let uv = pos.xy / size;
    var total = vec4f(0.0);
    var weights = 0.0;
    var i = -p.radius;
    loop {
        if (i > p.radius) { break; }
        let weight = exp(-0.5 * i * i / (p.sigma * p.sigma));
        total += textureSampleLevel(source, linear_sampler, uv + p.direction * i, 0.0) * weight;
        weights += weight;
        i += 1.0;
    }
    return total / weights;
}
"#;

/// Everything that's inside the camera: the frame's shadow, then the video in its rounded mask.
///
/// It all happens in one full-screen pass: each pixel is taken back through the camera to find where
/// it falls on the stage. Colours stay in their encoded form throughout, as in the browser Recordly
/// composited in, so a look comes out the same.
pub const FRAME: &str = r#"
struct Params {
    camera: vec4f,      // scale, x, y (in stage pixels), unused
    frame: vec4f,       // x, y, width, height in stage pixels
    video: vec4f,       // the same, for the uncropped video
    corner: vec4f,      // radius in stage pixels, video kind (0 RGBA, 1 NV12), scale for shadows, unused
    shadow0: vec4f,     // y offset, sigma, alpha, unused
    shadow1: vec4f,
    shadow2: vec4f,
    blur_from: vec4f,   // the camera a moment ago (scale, x, y), then how many samples to blur over
};
@group(0) @binding(0) var<uniform> p: Params;
@group(0) @binding(1) var plane0: texture_2d<f32>;
@group(0) @binding(2) var plane1: texture_2d<f32>;
@group(0) @binding(3) var linear_sampler: sampler;

@vertex
fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4f {
    let x = f32(i32(i & 1u) * 4 - 1);
    let y = f32(i32(i >> 1u) * 4 - 1);
    return vec4f(x, y, 0.0, 1.0);
}

// Evan Wallace's closed-form approximation of a blurred rounded rectangle.
fn gaussian(x: f32, sigma: f32) -> f32 {
    return exp(-(x * x) / (2.0 * sigma * sigma)) / (2.5066282746 * sigma);
}

fn erf2(x: vec2f) -> vec2f {
    let s = sign(x);
    let a = abs(x);
    var r = 1.0 + (0.278393 + (0.230389 + 0.078108 * (a * a)) * a) * a;
    r = r * r;
    return s - s / (r * r);
}

fn shadow_x(x: f32, y: f32, sigma: f32, corner: f32, half_size: vec2f) -> f32 {
    let delta = min(half_size.y - corner - abs(y), 0.0);
    let curved = half_size.x - corner + sqrt(max(0.0, corner * corner - delta * delta));
    let integral = 0.5 + 0.5 * erf2((vec2f(x) + vec2f(-curved, curved)) * (0.7071067 / sigma));
    return integral.y - integral.x;
}

fn rounded_shadow(lower: vec2f, upper: vec2f, point_in: vec2f, sigma: f32, corner: f32) -> f32 {
    let center = (lower + upper) * 0.5;
    let half_size = (upper - lower) * 0.5;
    let point = point_in - center;
    let low = point.y - half_size.y;
    let high = point.y + half_size.y;
    let start = clamp(-3.0 * sigma, low, high);
    let end = clamp(3.0 * sigma, low, high);
    let step_size = (end - start) / 4.0;
    var y = start + step_size * 0.5;
    var value = 0.0;
    for (var i = 0; i < 4; i += 1) {
        value += shadow_x(point.x, point.y - y, sigma, corner, half_size) * gaussian(y, sigma) * step_size;
        y += step_size;
    }
    return value;
}

// How much of this pixel is inside the frame's squircle: a superellipse with exponent 4.5 (§31).
fn squircle_coverage(point: vec2f, center: vec2f, half_size: vec2f, radius: f32) -> f32 {
    let distance = abs(point - center) - (half_size - vec2f(radius));
    // Straight edges.
    let outside = max(distance.x - radius, distance.y - radius);
    let edge = clamp(0.5 - outside / max(fwidth(outside), 1e-4), 0.0, 1.0);
    if (radius <= 0.5) {
        return edge;
    }
    // Corners, where both axes are into the curved part.
    let q = max(distance, vec2f(0.0)) / radius;
    let v = pow(q.x, 4.5) + pow(q.y, 4.5);
    let curve = clamp(0.5 + (1.0 - v) / max(fwidth(v), 1e-4), 0.0, 1.0);
    if (distance.x > 0.0 && distance.y > 0.0) {
        return min(edge, curve);
    }
    return edge;
}

fn sample_video(uv: vec2f) -> vec3f {
    if (p.corner.y < 0.5) {
        return textureSampleLevel(plane0, linear_sampler, uv, 0.0).rgb;
    }
    // BT.709, limited range, which is what recordings and the decoder use.
    let y = (textureSampleLevel(plane0, linear_sampler, uv, 0.0).r - 16.0 / 255.0) * 255.0 / 219.0;
    let c = (textureSampleLevel(plane1, linear_sampler, uv, 0.0).rg - vec2f(128.0 / 255.0)) * 255.0 / 224.0;
    return vec3f(
        y + 1.5748 * c.y,
        y - 0.1873 * c.x - 0.4681 * c.y,
        y + 1.8556 * c.x,
    );
}

@fragment
fn fs(@builtin(position) pos: vec4f) -> @location(0) vec4f {
    // Back through the camera, to a place on the stage.
    let point = (pos.xy - p.camera.yz) / p.camera.x;

    let lower = p.frame.xy;
    let upper = p.frame.xy + p.frame.zw;
    let radius = min(p.corner.x, min(p.frame.z, p.frame.w) * 0.5);
    let center = (lower + upper) * 0.5;

    // Each shadow layer goes over the last.
    var shadow = 0.0;
    for (var layer = 0; layer < 3; layer += 1) {
        var params = p.shadow0;
        if (layer == 1) { params = p.shadow1; }
        if (layer == 2) { params = p.shadow2; }
        if (params.z > 0.0 && params.y > 0.0) {
            let offset = vec2f(0.0, params.x);
            let amount = rounded_shadow(lower + offset, upper + offset, point, params.y, radius) * params.z;
            shadow = shadow + amount * (1.0 - shadow);
        }
    }

    let coverage = squircle_coverage(point, center, p.frame.zw * 0.5, radius);
    var color = vec3f(0.0);
    if (coverage > 0.0) {
        let samples = i32(p.blur_from.w);
        if (samples > 1) {
            // Motion blur: the picture averaged along where the camera has just been (§32).
            for (var i = 0; i < samples; i += 1) {
                let t = f32(i) / f32(samples - 1);
                let scale = mix(p.camera.x, p.blur_from.x, t);
                let offset = mix(p.camera.yz, p.blur_from.yz, t);
                let moved = (pos.xy - offset) / scale;
                color += sample_video((moved - p.video.xy) / p.video.zw);
            }
            color /= f32(samples);
        } else {
            color = sample_video((point - p.video.xy) / p.video.zw);
        }
    }
    // Premultiplied: the video over its shadow.
    let alpha = coverage + shadow * (1.0 - coverage);
    return vec4f(color * coverage, alpha);
}
"#;

/// A rectangle anywhere on the stage: a texture, a ring, a solid colour, or a window onto a texture
/// that's the same size as the stage, for blur annotations.
pub const QUAD: &str = r#"
struct Quad {
    center: vec2f,
    half_size: vec2f,
    rotation: vec2f,   // cos, sin
    stage: vec2f,
    uv: vec4f,
    color: vec4f,      // tint and opacity
    params: vec4f,     // kind (0 texture, 1 ring, 2 stage window), ring radius, ring stroke (0 fills), unused
};
@group(0) @binding(0) var<uniform> q: Quad;
@group(1) @binding(0) var image: texture_2d<f32>;
@group(1) @binding(1) var linear_sampler: sampler;

struct Out {
    @builtin(position) position: vec4f,
    @location(0) local: vec2f,
    @location(1) uv: vec2f,
};

@vertex
fn vs(@builtin(vertex_index) i: u32) -> Out {
    let corner = vec2f(f32(i & 1u) * 2.0 - 1.0, f32(i >> 1u) * 2.0 - 1.0);
    let local = corner * q.half_size;
    let turned = vec2f(
        local.x * q.rotation.x - local.y * q.rotation.y,
        local.x * q.rotation.y + local.y * q.rotation.x,
    );
    let at = q.center + turned;
    let clip = vec2f(at.x / q.stage.x * 2.0 - 1.0, 1.0 - at.y / q.stage.y * 2.0);
    let uv = mix(q.uv.xy, q.uv.zw, (corner + vec2f(1.0)) * 0.5);
    return Out(vec4f(clip, 0.0, 1.0), local, uv);
}

@fragment
fn fs(in: Out) -> @location(0) vec4f {
    let kind = q.params.x;
    if (kind < 0.5) {
        return textureSampleLevel(image, linear_sampler, in.uv, 0.0) * q.color.a * vec4f(q.color.rgb, 1.0);
    }
    if (kind < 1.5) {
        let distance_out = length(in.local);
        var edge: f32;
        if (q.params.z > 0.0) {
            edge = abs(distance_out - q.params.y) - q.params.z * 0.5;
        } else {
            edge = distance_out - q.params.y;
        }
        let coverage = clamp(0.5 - edge, 0.0, 1.0) * q.color.a;
        return vec4f(q.color.rgb * coverage, coverage);
    }
    // A window onto the blurred stage, at this same place on it.
    let uv = in.position.xy / q.stage;
    let sampled = textureSampleLevel(image, linear_sampler, uv, 0.0);
    return vec4f(sampled.rgb * q.color.a, q.color.a);
}
"#;
