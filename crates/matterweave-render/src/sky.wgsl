// Sky dome and volumetric clouds. Both fragment entries share one full-screen
// triangle and the group-0 lighting uniform the world pass already binds, so
// the sun and the sky colour cannot drift from the ones the terrain is lit and
// fogged with. Only binding 0 of that set is declared here; the rest of the
// layout stays bound and unused.
//
// `fs_main` draws into the swapchain attachment before any opaque geometry.
// `fs_clouds` draws into the reduced-resolution offscreen target and returns
// premultiplied scattered light in rgb with `1 - transmittance` in alpha.
struct Sky {
    inv_view_proj: mat4x4<f32>,
    // eye xyz, w = cloud animation clock in seconds
    eye: vec4<f32>,
    // (march steps, light steps, inverse target width, inverse target height)
    params: vec4<f32>,
};
var<push_constant> sky: Sky;
// Layout is defined once in Rust: crates/matterweave-render/src/shader_contract.rs.
struct Lighting {
    view_proj: mat4x4<f32>,
    sun: vec4<f32>,
    params: vec4<f32>,
    indirect_origin: vec4<i32>,
    indirect_dimensions: vec4<u32>,
    reflection_origin: vec4<i32>,
    reflection_dimensions: vec4<u32>,
    reflection_params: vec4<f32>,
    atmosphere: vec4<f32>,
    wind: vec4<f32>,
    player: vec4<f32>,
};
@group(0) @binding(0) var<uniform> lighting: Lighting;

// Slab the layer occupies, in world metres.
const CLOUD_BASE: f32 = 900.0;
const CLOUD_TOP: f32 = 1500.0;
// Drift along +x. Thirty metres a minute: a shape crosses its own width in
// minutes, so consecutive frames differ far below what the eye reads as motion.
const DRIFT_M_PER_S: f32 = 0.5;
// Coverage lattice spacing and detail lattice spacing in metres. Both hashes
// wrap, and 64 * 512 = 512 * 64 = 32768 m is the period both share; the clock
// the renderer folds is chosen so a wrap lands exactly on it.
const WEATHER_M: f32 = 512.0;
const WEATHER_CELLS: i32 = 64;
const DETAIL_M: f32 = 64.0;
const DETAIL_CELLS: i32 = 512;
// Extinction per metre of fully dense cloud.
const DENSITY_PER_M: f32 = 0.045;
// Distance the layer is faded out over, so it converges to the sky instead of
// ending in a line at the far intersection of the slab.
const FADE_START_M: f32 = 14000.0;
const FADE_END_M: f32 = 32000.0;
// Longest slab span a single ray integrates. A grazing ray would otherwise
// stretch its fixed step count over tens of kilometres and alias badly.
const MAX_SPAN_M: f32 = 9000.0;
// Metres per light-march step toward the sun.
const LIGHT_STEP_M: f32 = 90.0;

@vertex fn vs_main(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    // One triangle covering the clip square: (-1,-1), (3,-1), (-1,3).
    let x = f32(i32(index & 1u) * 4 - 1);
    let y = f32(i32(index >> 1u) * 4 - 1);
    return vec4(x, y, 1.0, 1.0);
}

// World-space ray through a framebuffer pixel. `params.zw` is the inverse of
// the target this entry draws into, so the reduced-resolution cloud pass and
// the full-resolution sky pass reconstruct the same rays.
fn view_ray(frag: vec2<f32>) -> vec3<f32> {
    let uv = frag * sky.params.zw;
    // Clip y is up in the matrix the app supplies; Naga flips it at the vertex
    // stage, so framebuffer v maps back with 1 - 2v.
    let ndc = vec2(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0);
    let far = sky.inv_view_proj * vec4(ndc, 1.0, 1.0);
    return normalize(far.xyz / far.w - sky.eye.xyz);
}

fn hash_cell(cell: vec3<u32>) -> f32 {
    // Integer avalanche: identical on every device, with no dependence on
    // floating-point precision beyond the f32 lattice coordinate itself.
    var h = cell.x * 374761393u + cell.y * 668265263u + cell.z * 2246822519u;
    h = (h ^ (h >> 13u)) * 1274126177u;
    h = h ^ (h >> 16u);
    return f32(h) * (1.0 / 4294967295.0);
}

// Value noise on a wrapping integer lattice. `cells` is the wrap period in
// lattice units and must be a power of two; the wrap is what lets the drift
// clock fold without the layer jumping. The eight corners are fetched without
// an indexable array: a dynamically indexed local is memory on several
// drivers, and this function is the innermost one in the march.
fn value_noise(p: vec3<f32>, cells: i32) -> f32 {
    let base = floor(p);
    let f = p - base;
    let w = f * f * (3.0 - 2.0 * f);
    let mask = vec3<i32>(cells - 1);
    let lo = vec3<u32>(vec3<i32>(base) & mask);
    let hi = vec3<u32>((vec3<i32>(base) + vec3<i32>(1)) & mask);
    let x00 = mix(hash_cell(vec3(lo.x, lo.y, lo.z)), hash_cell(vec3(hi.x, lo.y, lo.z)), w.x);
    let x10 = mix(hash_cell(vec3(lo.x, hi.y, lo.z)), hash_cell(vec3(hi.x, hi.y, lo.z)), w.x);
    let x01 = mix(hash_cell(vec3(lo.x, lo.y, hi.z)), hash_cell(vec3(hi.x, lo.y, hi.z)), w.x);
    let x11 = mix(hash_cell(vec3(lo.x, hi.y, hi.z)), hash_cell(vec3(hi.x, hi.y, hi.z)), w.x);
    return mix(mix(x00, x10, w.y), mix(x01, x11, w.y), w.z);
}

// Two or three octaves of the same lattice, the third only at the quality
// level that asks for it. Each octave doubles the frequency and the wrap period
// with it, so the whole field keeps one 32768 m period.
fn detail_fbm(p: vec3<f32>, octaves: i32) -> f32 {
    var sum = value_noise(p, DETAIL_CELLS) * 0.5
        + value_noise(p * 2.0, DETAIL_CELLS * 2) * 0.25;
    var total = 0.75;
    if octaves > 2 {
        sum = sum + value_noise(p * 4.0, DETAIL_CELLS * 4) * 0.125;
        total = 0.875;
    }
    // Normalize by the amplitudes actually summed, so both levels stay in 0..1.
    return sum / total;
}

// Coverage: a slow two-dimensional weather field, sampled on the same wrapping
// lattice with y pinned so it varies only across the ground plane.
fn coverage(xz: vec2<f32>) -> f32 {
    let p = vec3(xz.x, 0.0, xz.y) / WEATHER_M;
    let wide = value_noise(p, WEATHER_CELLS);
    let fine = value_noise(p * 2.0, WEATHER_CELLS * 2);
    return clamp((wide * 0.7 + fine * 0.3) * 1.7 - 0.62, 0.0, 1.0);
}

// Extinction per metre at a world point. Zero outside the slab. `octaves` is
// the detail budget; the quality level sets it.
fn cloud_density(position: vec3<f32>, octaves: i32) -> f32 {
    let height = (position.y - CLOUD_BASE) / (CLOUD_TOP - CLOUD_BASE);
    if height <= 0.0 || height >= 1.0 {
        return 0.0;
    }
    let drifted = position + vec3(sky.eye.w * DRIFT_M_PER_S, 0.0, 0.0);
    let cover = coverage(drifted.xz);
    if cover <= 0.0 {
        return 0.0;
    }
    // Rounded base, eroded top: the profile is what makes a slab read as a layer.
    let profile = smoothstep(0.0, 0.22, height) * (1.0 - smoothstep(0.45, 1.0, height));
    let shaped = cover * profile;
    if shaped <= 0.0 {
        return 0.0;
    }
    let detail = detail_fbm(drifted / DETAIL_M, octaves);
    let erosion = (1.0 - detail) * (0.35 + 0.45 * height);
    return clamp(shaped - erosion, 0.0, 1.0) * DENSITY_PER_M;
}

// Sun transmittance through the layer: a short Beer-Lambert march, no bounce.
fn light_transmittance(position: vec3<f32>, sun: vec3<f32>, steps: i32, octaves: i32) -> f32 {
    var optical = 0.0;
    for (var i = 0; i < steps; i = i + 1) {
        let sample = position + sun * (f32(i) + 0.5) * LIGHT_STEP_M;
        // The sun march never reads the finest octave: its result is an
        // exponent, and that detail is invisible through it.  Two octaves is
        // the floor, so this differs from the view march only at quality.
        optical = optical + cloud_density(sample, octaves - 1) * LIGHT_STEP_M;
    }
    return exp(-optical);
}

// Horizon-to-zenith gradient, sun disc and glow, and a ground-side floor.
// Every colour is derived from `atmosphere.sky`, and the value exactly at the
// horizon is that colour, so the far terrain's fog target, the cleared
// background and this dome meet without a seam.
fn sky_dome(dir: vec3<f32>) -> vec3<f32> {
    let base = lighting.atmosphere.xyz;
    let zenith = base * vec3(0.52, 0.66, 1.0);
    let ground = base * 0.42;
    var color = mix(base, zenith, smoothstep(0.0, 0.55, dir.y));
    color = mix(color, ground, smoothstep(0.0, -0.30, dir.y));
    let sun = normalize(lighting.sun.xyz);
    let alignment = max(dot(dir, sun), 0.0);
    let disc = smoothstep(0.9994, 0.9997, alignment);
    let glow = pow(alignment, 256.0) * 0.35 + pow(alignment, 8.0) * 0.06;
    return color + lighting.sun.w * (glow + disc * 4.0);
}

@fragment fn fs_main(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    return vec4(sky_dome(view_ray(frag.xy)), 1.0);
}

@fragment fn fs_clouds(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    let eye = sky.eye.xyz;
    let dir = view_ray(frag.xy);
    // Below the layer looking level or down: the slab is behind the ray.
    if dir.y <= 0.015 {
        return vec4(0.0);
    }
    let enter = max((CLOUD_BASE - eye.y) / dir.y, 0.0);
    let exit = (CLOUD_TOP - eye.y) / dir.y;
    if exit <= enter || enter >= FADE_END_M {
        return vec4(0.0);
    }
    let span = min(exit - enter, MAX_SPAN_M);
    let steps = i32(sky.params.x);
    let light_steps = i32(sky.params.y);
    // Three detail octaves only at the quality level; the cheap level marches
    // two, which is the difference between a soft layer and a crisp one.
    let octaves = select(2, 3, steps > 32);
    let dt = span / f32(steps);
    // Hash dither on the first step. There is no history buffer in this
    // renderer, so this is a static per-pixel offset: it trades visible step
    // banding for a fixed fine-grained noise that does not crawl between
    // frames but also is not averaged away over time.
    let pixel = vec3<u32>(u32(frag.x), u32(frag.y), 0u);
    let jitter = hash_cell(pixel);
    let sun = normalize(lighting.sun.xyz);
    let ambient = mix(lighting.atmosphere.xyz, lighting.atmosphere.xyz * vec3(0.6, 0.7, 1.0), 0.5);
    var transmittance = 1.0;
    var scattered = vec3(0.0);
    for (var i = 0; i < steps; i = i + 1) {
        let t = enter + (f32(i) + jitter) * dt;
        let position = eye + dir * t;
        let density = cloud_density(position, octaves);
        if density > 0.0001 {
            let sun_transmittance = light_transmittance(position, sun, light_steps, octaves);
            // Powder: thick edges facing the sun darken instead of saturating.
            let powder = 1.0 - exp(-density * dt * 6.0);
            let step_transmittance = exp(-density * dt);
            let luminance = vec3(1.0, 0.98, 0.94) * lighting.sun.w * sun_transmittance
                * (0.45 + 0.55 * powder) + ambient * 0.55;
            scattered = scattered + transmittance * (1.0 - step_transmittance) * luminance;
            transmittance = transmittance * step_transmittance;
            if transmittance < 0.02 {
                break;
            }
        }
    }
    // Converge to the sky at the horizon and with distance.
    let fade = smoothstep(0.015, 0.12, dir.y) * (1.0 - smoothstep(FADE_START_M, FADE_END_M, enter));
    return vec4(scattered * fade, (1.0 - transmittance) * fade);
}
