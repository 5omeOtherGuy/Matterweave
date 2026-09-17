struct Camera {
    view_proj: mat4x4<f32>,
    eye: vec4<f32>,
};
var<push_constant> camera: Camera;
// Layout is defined once in Rust: crates/matterweave-render/src/shader_contract.rs.
// `indirect_dimensions.w` and `reflection_dimensions.w` are enable flags.
// `reflection_params` is (trace step bound, surface offset, 0, 0).
// `atmosphere` is (sky rgb, fog density per metre).
struct Lighting {
    view_proj: mat4x4<f32>,
    sun: vec4<f32>,
    // enabled, normalized depth bias, inverse map size, unused
    params: vec4<f32>,
    indirect_origin: vec4<i32>,
    indirect_dimensions: vec4<u32>,
    reflection_origin: vec4<i32>,
    reflection_dimensions: vec4<u32>,
    reflection_params: vec4<f32>,
    atmosphere: vec4<f32>,
    // (wind direction x, wind direction z, strength in metres, time in seconds)
    wind: vec4<f32>,
    // (player x, y, z, push radius in metres); radius 0 disables the push
    player: vec4<f32>,
    // (coarse-surface water shading enabled, ripple time in seconds, depth in
    // metres assumed for a coarse flooded cell, unused)
    water: vec4<f32>,
    // (aerial-perspective strength 0..1, 0, 0, 0). Zero keeps the
    // constant-colour fade; a nonzero strength shades distance with the
    // directional in-scatter of `aerial_perspective` below, and with the
    // distance face split and per-voxel tone variation further down.
    aerial: vec4<f32>,
};
@group(0) @binding(0) var<uniform> lighting: Lighting;
@group(0) @binding(1) var shadow_map: texture_depth_2d;
@group(0) @binding(2) var shadow_sampler: sampler_comparison;
@group(0) @binding(3) var<storage, read> indirect_faces: array<vec4<f32>>;
// Bounded reflection source: one u32 material per cell, x + dims.x * (y + dims.y * z).
@group(0) @binding(4) var<storage, read> reflection_materials: array<u32>;
// 256 vec4s: rgb reflectance plus mirror strength in w.
@group(0) @binding(5) var<storage, read> reflection_palette: array<vec4<f32>>;
// Colour a reflected ray terminates against; identical to the fog target and to
// the cleared background, so the horizon and the world's end are one colour.
fn sky_color() -> vec3<f32> { return lighting.atmosphere.xyz; }
// -- Aerial perspective -------------------------------------------------------
//
// The old fog blended every distance to one constant colour, so at kilometres a
// surface facing away from the sun sat within a couple of luminance levels of
// the lit face beside it and the voxel stepping the rings now carry was
// invisible. Distance is two terms:
//
//   colour = surface * exp(-density * distance)
//          + in_scatter * (1 - exp(-density * distance))
//
// The first is what the surface sends through the air. The second is what the
// air scatters toward the eye in its place, and unlike a fog colour it is not
// constant: it is the sky's scattering colour in the view direction, dimmed
// when looking away from the sun and grown toward it. That sun-angle term is
// what keeps an away-facing surface dark at four kilometres while a lit one
// stays bright, and it costs a normalize, a dot and a square.

/// Fraction of the sky's scattering the air returns along a ray looking away
/// from the sun. Sunlight scattered toward the eye is strongly peaked about the
/// sun direction, so with the sun high a horizontal ray away from it carries
/// only this fraction. Low enough that shadowed rock and soil stay dark at
/// kilometres, high enough that distance is a soft blue veil rather than a
/// darkening.
const AERIAL_FLOOR: f32 = 0.30;

/// In-scattered skylight along a view ray, before extinction.
///
/// `view` points from the surface toward the eye, so the ray runs along
/// `-view`. The colour is the horizon-to-zenith gradient `sky.wgsl` draws,
/// without its sun disc and glow - those are direct sunlight, not the path's
/// own scattering - and the strength is a forward-scattering lobe about the
/// sun. `lighting.aerial.x` blends the lobe in; at zero this returns the
/// horizon colour itself, so a sample that never asks for the directional
/// model fades to exactly the colour it did before.
fn aerial_inscatter(view: vec3<f32>) -> vec3<f32> {
    let base = sky_color();
    let ray = -view;
    let zenith = base * vec3(0.52, 0.66, 1.0);
    let sky = mix(base, zenith, smoothstep(0.0, 0.55, ray.y));
    let sun = normalize(lighting.sun.xyz);
    let forward = max(dot(ray, sun), 0.0);
    let phase = AERIAL_FLOOR + (1.0 - AERIAL_FLOOR) * forward * forward;
    return sky * mix(1.0, phase, lighting.aerial.x);
}

/// Two-term distance shading: what the surface sends through the path, plus
/// what the path scatters in its place.
fn aerial_perspective(lit: vec3<f32>, view: vec3<f32>, distance: f32) -> vec3<f32> {
    let transmittance = exp(-distance * lighting.atmosphere.w);
    return mix(lit, aerial_inscatter(view), 1.0 - transmittance);
}

// -- Distance face shading and tone -------------------------------------------
//
// Past the near field two things that made voxel steps read at forty metres have
// both gone: the atmosphere has removed most of the surface contrast, and a
// coarse cell carries one flat colour while its neighbours quantise to the same
// handful of values. Two distance-shaded terms put them back, and both fade in
// over the band the brief fixes, so nothing inside 60 m changes from them. Both
// belong to the aerial-perspective model, so a sample that does not ask for it
// keeps the shading it always had:
//
//   * the face split: the sun and the sky ambient are weighted by how much sky
//     a face sees, so a distant wall keeps about a third of the direct sun and a
//     quarter of the ambient a top beside it gets;
//   * the tone jitter: a deterministic per-voxel variation of the material
//     tone, fixed to the world rather than the screen so it cannot crawl.
//
// The tone jitter is the far half of fidelity-spec A3: the palette variation a
// near chunk carries per voxel at meshing time is carried into the coarse ring
// shader as a hash on the world position instead, one cell per authoritative
// metre and never finer than a pixel.

/// Metres at which the face split and the tone jitter begin and are full.
const FACE_SPLIT_START_M: f32 = 60.0;
const FACE_SPLIT_END_M: f32 = 300.0;
const TONE_START_M: f32 = 60.0;
const TONE_END_M: f32 = 200.0;
/// Sky ambient a vertical face keeps once the split is full, and the fraction
/// of the direct sun it keeps. A horizontal face is unchanged: 0.40 ambient and
/// the full sun, at every distance.
const DISTANT_AMBIENT_FLOOR: f32 = 0.10;
const DISTANT_AMBIENT_UP: f32 = 0.30;
const DISTANT_SUN_FLOOR: f32 = 0.35;
/// Peak tone variation at full distance, as a fraction of the material colour.
/// The variation is split: a correlated luminance term, which is what puts a
/// break of several levels between neighbouring cells and keeps a distant
/// surface from reading as one flat sheet, plus a gentler per-channel
/// remainder so the mottle is colour rather than grey noise.
const TONE_JITTER: f32 = 0.20;
const TONE_CHROMA: f32 = 0.12;

/// Deterministic per-cell tone in `0..=1` per channel.
///
/// `footprint` is the world-space size of one pixel; at distance a pixel covers
/// many metres, so the cell grows with it and the pattern stays no finer than
/// about two pixels. Integer hash, so the same world cell is the same tone on
/// every device and in every frame.
fn voxel_tone(world: vec3<f32>, footprint: f32) -> vec3<f32> {
    let grain = max(footprint * 2.0, 1.0);
    let cell = vec3<u32>(vec3<i32>(floor(world / grain)));
    var h = cell.x * 374761393u ^ cell.y * 668265263u ^ cell.z * 2246822519u;
    h = (h ^ (h >> 13u)) * 1274126177u;
    h = h ^ (h >> 16u);
    return vec3<f32>(
        f32(h & 0xffu),
        f32((h >> 8u) & 0xffu),
        f32((h >> 16u) & 0xffu),
    ) * (1.0 / 255.0);
}

/// The tone variation one world cell applies to its material colour.
fn tone_variation(world: vec3<f32>, footprint: f32) -> vec3<f32> {
    let t = voxel_tone(world, footprint) - vec3<f32>(0.5);
    let luma = (t.x + t.y + t.z) * (1.0 / 3.0);
    return TONE_JITTER * luma + TONE_CHROMA * (t - vec3<f32>(luma));
}
struct Input {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) color: vec3<f32>,
    // Packed instance record: translation xyz, quarter-turn yaw in w. The
    // identity record (0,0,0,0) leaves non-instanced geometry unchanged.
    @location(3) instance: vec4<f32>,
    // Packed wind record: (sway phase 0..1, bend 0..1, prototype height in
    // metres, per-instance scale). The identity record (0,0,0,0) has scale = 0,
    // which is the early-out in `displace` below *and* the mark a non-flora
    // draw's record carries, so every draw that binds it - every chunk, terrain
    // tile, legacy, dynamic and plain static-scene draw - is rasterized at its
    // own scale from exactly the position it was before this path existed.
    @location(4) wind: vec4<f32>,
};
struct Output {
    @builtin(position) clip: vec4<f32>,
    @location(0) world: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) color: vec3<f32>,
};
// Exact quarter-turn yaw about Y: x' = c*x + s*z, z' = -s*x + c*z.
// Must match static_scene.rs rotate_xz and the packed instance record.
fn quarter_rotation(yaw: f32) -> mat2x2<f32> {
    var quarter_cos = array<f32, 4>(1.0, 0.0, -1.0, 0.0);
    var quarter_sin = array<f32, 4>(0.0, 1.0, 0.0, -1.0);
    let q = u32(yaw) & 3u;
    return mat2x2<f32>(
        vec2(quarter_cos[q], -quarter_sin[q]),
        vec2(quarter_sin[q], quarter_cos[q]),
    );
}
// The one place vegetation is displaced. Two effects share it:
//
//   scale        = wind.w, the instance's uniform scale, which multiplies the
//                  local position in `vs_main` before this runs;
//   h            = clamp(local_y / (height_m * scale), 0, 1), the fraction of
//                  the *drawn* plant height, so the ground contact never moves
//                  and the tip moves most at every scale;
//   sway         = two decorrelated sines of time, the instance phase and the
//                  world position, so neighbours never pulse together;
//   bend_amount  = bend * scale * strength * h^1.5, in metres: a short instance
//                  bows proportionally less than a tall one;
//   push         = (1 - d/radius)^2 * radius * 0.5 metres away from the player
//                  inside the push radius, weighted by the same h. The push
//                  strength is derived from the radius rather than carried in
//                  its own uniform: one number describes how wide the player
//                  parts the field and how far, and the h weighting keeps the
//                  ground contact planted while the tips bow away.
//
// Normals are deliberately left alone: displacement is horizontal and small
// next to a voxel face, and recomputing a normal per vertex would need the
// neighbouring displaced positions, which an instanced vertex does not have.
// Shading therefore follows the rest pose. `shadow.wgsl` applies the same scale
// but not this displacement, so a swaying plant casts its rest-pose shadow at
// its drawn size; both are stated limitations of this slice.
fn displace(local: vec3<f32>, rest: vec3<f32>, wind: vec4<f32>) -> vec3<f32> {
    if wind.w <= 0.0 { return rest; }
    let scale = wind.w;
    let height_m = max(wind.z * scale, 0.01);
    let h = clamp(local.y / height_m, 0.0, 1.0);
    let phase = wind.x;
    let bend = wind.y;
    let time = lighting.wind.w;
    let strength = lighting.wind.z;
    let freq = 1.7;
    let TAU = 6.2831853;
    let sway = sin(time * freq + phase * TAU + rest.x * 0.11 + rest.z * 0.07)
        + 0.5 * sin(time * freq * 1.7 + phase * 3.1);
    let bend_amount = bend * scale * strength * pow(h, 1.5);
    var offset = lighting.wind.xy * sway * bend_amount;
    let radius = lighting.player.w;
    if radius > 0.0 {
        let away = rest.xz - lighting.player.xz;
        let d = length(away);
        if d < radius && d > 1.0e-4 {
            let falloff = 1.0 - d / radius;
            offset = offset + (away / d) * falloff * falloff * radius * 0.5 * h;
        }
    }
    return vec3(rest.x + offset.x, rest.y, rest.z + offset.y);
}
@vertex fn vs_main(v: Input) -> Output {
    let rotation = quarter_rotation(v.instance.w);
    // A zero scale marks a record that is not flora, which draws unscaled.
    let scale = select(1.0, v.wind.w, v.wind.w > 0.0);
    let local = v.position * scale;
    let xz = rotation * vec2(local.x, local.z);
    let rest = vec3(xz.x, local.y, xz.y) + v.instance.xyz;
    let world_position = displace(local, rest, v.wind);
    let nxz = rotation * vec2(v.normal.x, v.normal.z);
    let world_normal = vec3(nxz.x, v.normal.y, nxz.y);
    var out: Output;
    out.clip = camera.view_proj * vec4(world_position, 1.0);
    out.world = world_position;
    out.normal = world_normal;
    out.color = v.color;
    return out;
}
fn shadow_visibility(world: vec3<f32>, normal: vec3<f32>) -> f32 {
    if lighting.params.x < 0.5 { return 1.0; }
    let clip = lighting.view_proj * vec4(world, 1.0);
    let ndc = clip.xyz / clip.w;
    if ndc.z <= 0.0 || ndc.z >= 1.0 || any(abs(ndc.xy) >= vec2(1.0)) { return 1.0; }
    // Naga flips vertex clip Y for Vulkan. Texture coordinates must match that flip.
    let uv = vec2(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
    let receiver_normal = normalize(normal);
    let cos_light = dot(receiver_normal, lighting.sun.xyz);
    // The receiver plane is singular parallel to the sun. Its direct-light
    // contribution is negligible here, so avoid an unbounded depth correction.
    if cos_light <= 0.0001 { return 1.0; }
    let row_x = vec3(lighting.view_proj[0].x, lighting.view_proj[1].x, lighting.view_proj[2].x);
    let row_y = vec3(lighting.view_proj[0].y, lighting.view_proj[1].y, lighting.view_proj[2].y);
    let row_z = vec3(lighting.view_proj[0].z, lighting.view_proj[1].z, lighting.view_proj[2].z);
    // The directional projection has orthogonal rows. These vectors move one UV
    // unit at constant light depth; V reverses the Naga/Vulkan clip-Y adjustment.
    let world_per_u = 2.0 * row_x / dot(row_x, row_x);
    let world_per_v = -2.0 * row_y / dot(row_y, row_y);
    let depth_per_world = -dot(row_z, lighting.sun.xyz);
    let depth_gradient = vec2(dot(receiver_normal, world_per_u), dot(receiver_normal, world_per_v))
        * (depth_per_world / cos_light);
    let bias = lighting.params.y * (1.0 + 2.0 * (1.0 - cos_light));
    var visible = 0.0;
    for (var y = -1; y <= 1; y += 1) {
        for (var x = -1; x <= 1; x += 1) {
            let tap_uv = uv + vec2(f32(x), f32(y)) * lighting.params.z;
            // Nearest sampling reads a texel center, including for the center
            // tap. Compensate both the filter offset and the subtexel phase.
            let sample_uv = (floor(tap_uv / lighting.params.z) + vec2(0.5)) * lighting.params.z;
            let sample_depth = ndc.z + dot(depth_gradient, sample_uv - uv) - bias;
            visible += textureSampleCompareLevel(shadow_map, shadow_sampler,
                sample_uv, sample_depth);
        }
    }
    // Fade the finite XY coverage rather than exposing a hard moving map boundary.
    let edge = smoothstep(0.85, 0.98, max(abs(ndc.x), abs(ndc.y)));
    return mix(visible / 9.0, 1.0, edge);
}
// Piecewise-constant face cache: deliberately no trilinear interpolation across
// thin walls. Lookup the solid side of the unit voxel boundary; greedy quads may
// span many cells. Unsupported/non-axis normals and outside coverage return zero.
fn indirect_diffuse(world: vec3<f32>, normal: vec3<f32>) -> vec3<f32> {
    if lighting.indirect_dimensions.w == 0u { return vec3(0.0); }
    let a = abs(normal);
    var face = 0u;
    if a.x > 0.999 { face = select(1u, 0u, normal.x > 0.0); }
    else if a.y > 0.999 { face = select(3u, 2u, normal.y > 0.0); }
    else if a.z > 0.999 { face = select(5u, 4u, normal.z > 0.0); }
    else { return vec3(0.0); }
    let local = floor(world - normal * 0.001) - vec3<f32>(lighting.indirect_origin.xyz);
    let dims = lighting.indirect_dimensions.xyz;
    if any(local < vec3(0.0)) || any(local >= vec3<f32>(dims)) { return vec3(0.0); }
    let c = vec3<u32>(local);
    let index = ((c.z * dims.y + c.y) * dims.x + c.x) * 6u + face;
    return indirect_faces[index].xyz;
}
struct ReflectionSample {
    color: vec3<f32>,
    distance: f32,
    hit: bool,
};
// Mirror strength of the authoritative voxel at the shaded face. A surface whose
// voxel is outside the source volume, or whose material is nonreflective, is zero.
fn reflection_mirror(world: vec3<f32>, normal: vec3<f32>) -> f32 {
    if lighting.reflection_dimensions.w == 0u { return 0.0; }
    let local = floor(world - normal * lighting.reflection_params.y)
        - vec3<f32>(lighting.reflection_origin.xyz);
    let dims = lighting.reflection_dimensions.xyz;
    if any(local < vec3(0.0)) || any(local >= vec3<f32>(dims)) { return 0.0; }
    let c = vec3<u32>(local);
    let index = c.x + dims.x * (c.y + dims.y * c.z);
    return reflection_palette[reflection_materials[index]].w;
}
// One ideal specular bounce: r = d - 2 * dot(d, n) * n with d = normalize(world - eye).
// The single secondary ray starts at world + n * offset and is clipped to the source
// volume. A miss terminates against the sky colour at the volume exit distance.
// Reflected hits use documented simple shading: no shadow lookup, no second bounce.
fn specular_reflection(world_pos: vec3<f32>, normal: vec3<f32>, eye: vec3<f32>) -> ReflectionSample {
    var out: ReflectionSample;
    out.hit = false;
    out.color = sky_color();
    out.distance = 0.0;
    let incident = normalize(world_pos - eye);
    let direction = incident - 2.0 * dot(incident, normal) * normal;
    if any(incident != incident) || any(direction != direction) { return out; }
    if dot(direction, direction) < 1.0e-12 { return out; }
    let origin = world_pos + normal * lighting.reflection_params.y;
    let lower = vec3<f32>(lighting.reflection_origin.xyz);
    let dims = lighting.reflection_dimensions.xyz;
    let upper = lower + vec3<f32>(dims);
    // Outside the half-open volume, or on its outward-facing shell: a miss.
    if any(origin < lower) || any(origin >= upper) { return out; }
    var exit = 3.0e38;
    for (var axis = 0u; axis < 3u; axis = axis + 1u) {
        if direction[axis] > 0.0 {
            exit = min(exit, (upper[axis] - origin[axis]) / direction[axis]);
        } else if direction[axis] < 0.0 {
            exit = min(exit, (lower[axis] - origin[axis]) / direction[axis]);
        }
    }
    out.distance = max(exit, 0.0);
    if !(exit > 0.0) { return out; }
    var cell = vec3<i32>(floor(origin - lower));
    let stride = vec3<i32>(sign(direction));
    let requested = u32(max(lighting.reflection_params.x, 1.0));
    let bound = min(requested, dims.x + dims.y + dims.z + 1u);
    var distance = 0.0;
    var hit_normal = vec3(0.0);
    for (var iteration = 0u; iteration < bound; iteration = iteration + 1u) {
        if any(cell < vec3(0)) || any(cell >= vec3<i32>(dims)) { return out; }
        let index = u32(cell.x) + dims.x * (u32(cell.y) + dims.y * u32(cell.z));
        let material = reflection_materials[index];
        if material != 0u {
            // Self-intersection: a solid start cell is the surface being shaded.
            if iteration == 0u { return out; }
            let albedo = reflection_palette[material].xyz;
            var term = 0.28 + 0.12 * max(hit_normal.y, 0.0);
            if any(hit_normal != vec3(0.0)) {
                let n = normalize(hit_normal);
                term = term + max(dot(n, lighting.sun.xyz), 0.0) * lighting.sun.w;
            }
            out.hit = true;
            out.color = albedo * term;
            out.distance = distance;
            return out;
        }
        // Recompute from integer planes instead of accumulating tDelta error.
        var next = vec3(exit + 1.0);
        for (var axis = 0u; axis < 3u; axis = axis + 1u) {
            if stride[axis] != 0 {
                let boundary = lower[axis] + f32(cell[axis]) + select(0.0, 1.0, stride[axis] > 0);
                next[axis] = (boundary - origin[axis]) / direction[axis];
            }
        }
        distance = min(next.x, min(next.y, next.z));
        if !(distance < exit) { return out; }
        hit_normal = vec3(0.0);
        var first = true;
        for (var axis = 0u; axis < 3u; axis = axis + 1u) {
            if next[axis] == distance && stride[axis] != 0 {
                cell[axis] = cell[axis] + stride[axis];
                if first {
                    hit_normal[axis] = -f32(stride[axis]);
                    first = false;
                }
            }
        }
    }
    return out;
}
// -- Water --------------------------------------------------------------------
//
// One surface model for both water surfaces a landscape frame can hold:
//
//   * the derived water pass (`fs_water`), whose vertices carry the depth of the
//     bed under their own corner in `color.r` - see `matterweave_core::water`;
//   * the flooded cells of a distance-ring tile, drawn by the opaque pass in the
//     water colour with no bed of their own, shaded at `lighting.water.z` metres
//     and only when `lighting.water.x` is set.
//
// Nothing here reads a texture, a depth image or a second pass. A water pixel is
// six sines, one fresnel and two specular lobes, and it returns *before* the
// nine-tap shadow lookup, the indirect cache read and the reflection probe the
// opaque path would have run, so shading a pixel as water is cheaper than not.
//
// Deliberately absent: shadows on the water (a surface that reflects the sky is
// not readably shadowed at this range), and any reflection of the scene, which
// is the separate tier-B item.

/// Depth in metres a full vertex depth channel encodes. Must equal
/// `matterweave_core::water::WATER_MAX_DEPTH_M`.
const WATER_MAX_DEPTH_M = 32.0;
/// Metres over which the body colour saturates from shallow to deep. Shallow
/// water is most of what a shoreline shows, so the ramp is fast - and it is also
/// what makes the surface relief of a ripple visible as a tint change over a
/// fixed bed rather than a change too small to see.
const WATER_TINT_DEPTH_M = 2.0;
const WATER_SHALLOW = vec3(0.26, 0.45, 0.44);
const WATER_DEEP = vec3(0.012, 0.042, 0.085);
/// Alpha at no depth and at full depth. Shallow water shows its bed, which is
/// what makes the beach meet the sea without a seam; deep water hides it.
const WATER_ALPHA_SHALLOW = 0.45;
const WATER_ALPHA_DEEP = 0.97;

/// The sky in one direction, as the dome pass draws it.
///
/// Water is mostly a mirror at a distance, and a mirror that returns one flat
/// colour is what made the old surface a sheet of paint: the near water reflects
/// the zenith and the far water reflects the horizon, and the difference between
/// them is most of what a grazing angle is supposed to show. The gradient is the
/// one `sky.wgsl` builds from `atmosphere.sky`, repeated here rather than shared
/// because the two passes are separate modules; the sun disc and glow are not
/// repeated, because this surface has its own sun lobe.
fn water_sky(direction: vec3<f32>) -> vec3<f32> {
    let base = lighting.atmosphere.xyz;
    return mix(base, base * vec3(0.52, 0.66, 1.0), smoothstep(0.0, 0.55, direction.y));
}

/// Vertical focal length in pixels the ripple level of detail is sized for: a
/// 65-degree field of view over 768 rows. A phone renders 1440 rows and has
/// nearly twice this, so a wave fades here slightly before the device would have
/// had to drop it - the conservative end of the choice.
const WATER_FOCAL_PX = 600.0;

/// Metres the swell displaces the frame the chop is sampled in. Large enough
/// that short crests bend visibly along the swell, small enough that they stay
/// attached to it.
const WATER_WARP_M = 9.0;

/// One sample of the ripple field: the surface offset in metres, the slope, and
/// the surface curvature in reciprocal metres, which is what focuses light on a
/// shallow bed.
struct Ripple {
    height: f32,
    slope: vec2<f32>,
    curvature: f32,
};

/// Strength of the shallow-water caustic, per metre of depth per unit curvature.
/// A crest focuses the light that passes through it onto the bed and a trough
/// spreads it; the effect is proportional to how far the light travels after
/// being bent, so it belongs to shallow water and disappears in deep water on
/// its own.
const WATER_CAUSTIC = 0.22;
/// Depth in metres past which the caustic stops growing: light that far down has
/// scattered too much to keep a focus.
const WATER_CAUSTIC_DEPTH_M = 4.0;

/// Sample the ripple field at one world point.
///
/// Six waves, spread in direction and at incommensurate wavelengths, so the
/// pattern does not repeat over a session and no crest stands still. Each runs at
/// its own deep-water speed `sqrt(g*k)`, which is why the short waves overtake
/// the long ones instead of the whole field sliding as one image. Both the height
/// and the slope come out of the same sine/cosine pair: the slope tilts the
/// normal, and the height moves the surface up and down over a fixed bed, which
/// is what makes shallow water breathe rather than sit at one tint.
///
/// Each wave is faded out once its wavelength no longer covers the pixel it is
/// being sampled in. A water plane is seen at a grazing angle, so the footprint
/// is set by the foreshortening, not by distance alone: at `view_distance` with
/// the eye `eye_height` above the surface, one pixel covers about
/// `view_distance^2 / (focal * eye_height)` metres along the view. Without this
/// the far sea turns into a corduroy of aliased crests that crawls as the camera
/// moves - measured M2 17.6 against the reference's 1.0 to 5.8.
fn water_ripple(p: vec2<f32>, time: f32, view_distance: f32, eye_height: f32) -> Ripple {
    let footprint = view_distance * view_distance
        / (WATER_FOCAL_PX * max(eye_height, 0.5));
    var dirs = array<vec2<f32>, 6>(
        vec2(0.981, 0.196),
        vec2(-0.422, 0.906),
        vec2(0.570, -0.822),
        vec2(0.110, 0.994),
        vec2(-0.848, -0.530),
        vec2(0.743, 0.669),
    );
    var wavelength = array<f32, 6>(12.0, 6.5, 3.0, 1.4, 0.65, 0.32);
    var wavenumber = array<f32, 6>(0.524, 0.967, 2.094, 4.488, 9.666, 19.635);
    // Amplitude times wavenumber: the peak slope each wave contributes.
    var steepness = array<f32, 6>(0.038, 0.044, 0.068, 0.062, 0.058, 0.044);
    // sqrt(g * k) for g = 9.81: deep-water dispersion, not an invented rate.
    var speed = array<f32, 6>(2.27, 3.08, 4.53, 6.64, 9.74, 13.88);
    var out: Ripple;
    out.height = 0.0;
    out.slope = vec2(0.0);
    out.curvature = 0.0;
    for (var i = 0; i < 6; i = i + 1) {
        let weight = 1.0 - smoothstep(wavelength[i] * 0.10, wavelength[i] * 0.34, footprint);
        if weight <= 0.0 { continue; }
        // The two long waves are the swell; the four short ones are the chop
        // riding on it, and they are sampled in the swell's own displaced frame.
        // Without that warp every crest in the field stays parallel to every
        // other and the middle distance reads as corduroy rather than water.
        let q = select(p + out.slope * WATER_WARP_M, p, i < 2);
        let phase = wavenumber[i] * dot(q, dirs[i]) + time * speed[i];
        let amplitude = steepness[i] * weight / wavenumber[i];
        let rise = sin(phase);
        out.height = out.height + amplitude * rise;
        out.slope = out.slope + dirs[i] * (steepness[i] * weight * cos(phase));
        // Curvature is the first thing to alias, because it weights the
        // shortest waves most: it is faded a good deal earlier than the slope
        // that comes from the same wave.
        out.curvature = out.curvature
            - steepness[i] * weight * weight * weight * wavenumber[i] * rise;
    }
    return out;
}

/// Shade one water pixel. `depth_m` is the water column under it in metres;
/// `opaque` is set for a surface with no bed drawn behind it to blend over.
fn water_surface(world: vec3<f32>, depth_m: f32, opaque: bool) -> vec4<f32> {
    let to_eye = camera.eye.xyz - world;
    let view_distance = length(to_eye);
    let view = to_eye / max(view_distance, 1.0e-4);
    let ripple = water_ripple(world.xz, lighting.water.y, view_distance,
        camera.eye.y - world.y);
    let normal = normalize(vec3(-ripple.slope.x, 1.0, -ripple.slope.y));
    // Body colour: the deeper the column, the less of the bed's light returns.
    // The surface itself rises and falls over a fixed bed, so a crest carries
    // more water than the trough beside it and tints deeper.
    let depth_t = 1.0 - exp(-max(depth_m + ripple.height, 0.0) / WATER_TINT_DEPTH_M);
    let sun = max(dot(normal, lighting.sun.xyz), 0.0);
    let body = mix(WATER_SHALLOW, WATER_DEEP, depth_t) * (0.16 + 0.34 * sun * lighting.sun.w);
    // Schlick fresnel about water's 0.02 normal reflectance: near vertical the
    // eye sees the body, at grazing angles it sees the sky, continuously.
    let facing = clamp(dot(normal, view), 0.0, 1.0);
    let fresnel = 0.02 + 0.98 * pow(1.0 - facing, 5.0);
    var color = mix(body, water_sky(2.0 * facing * normal - view), fresnel);
    // Two lobes, because one cannot do both jobs: the tight one is the glint a
    // crest throws when it happens to face the sun, the broad one is the sheen
    // that makes a near surface read as wet at all. The ripple normal is what
    // breaks either of them into structure instead of one mirror disc.
    let half_vector = normalize(view + lighting.sun.xyz);
    let alignment = max(dot(normal, half_vector), 0.0);
    let glint = pow(alignment, 320.0) * 3.2;
    let sheen = pow(alignment, 36.0) * 0.30;
    color = color
        + vec3(1.0, 0.97, 0.88) * (glint + sheen) * lighting.sun.w * (0.25 + 0.75 * fresnel);
    // Caustics reach the eye as a change in how much bed light comes through,
    // so they are applied to the transmitted fraction rather than added as a
    // colour. Deep water has almost none to modulate, which is why this fades
    // out on its own without a second condition.
    let caustic = clamp(
        1.0 + min(depth_m, WATER_CAUSTIC_DEPTH_M) * ripple.curvature * WATER_CAUSTIC,
        0.2,
        2.2,
    );
    let body_alpha = mix(WATER_ALPHA_SHALLOW, WATER_ALPHA_DEEP, depth_t);
    let alpha = select(
        clamp(1.0 - (1.0 - body_alpha) * caustic, 0.04, 0.995),
        1.0,
        opaque,
    );
    // The far water gets the same per-voxel tone variation as far terrain: its
    // analytic ripple field has faded out by this distance, and without it the
    // sea quantises to one flat colour between the shoreline and the horizon.
    let tone = smoothstep(TONE_START_M, TONE_END_M, view_distance) * lighting.aerial.x;
    if tone > 0.0 {
        let footprint = length(dpdx(world));
        color = color * (1.0 + tone * tone_variation(world, footprint));
    }
    return vec4(aerial_perspective(color, view, view_distance), alpha);
}

// -- Highlight roll-off -------------------------------------------------------
//
// In the near field the lighting bracket is the sum of two terms: the sky
// ambient `0.28 + 0.12 * up` (0.40 on a top face) and the direct sun
// `max(dot(n, sun), 0) * intensity` (0.70 for the landscape sample's sun). Each
// alone is below one; their sum is about 1.10, so a near-white albedo leaves the
// shader above one and the display clamps it to a flat white. Snow is the clear
// case: its palette colour is 0.86..0.92, and once the deterministic per-voxel
// tone of `material::tone` has lifted a cell by up to about a quarter, the
// tinted albedo itself is around 1.0 and every sunlit snow cell loses its
// per-voxel value to the clamp. White flower petals (`FLOWER_PETAL_WHITE`) and
// bright sand cross the same line on their brightest cells.
//
// The fix is a highlight shoulder, not an exposure change: everything up to the
// knee is passed through untouched, and above it the value tends to a ceiling
// short of one. The shoulder is evaluated on the brightest channel and applied
// to all three, so the ratios between the channels - hue and saturation - are
// exactly the material's. The curve is continuous with slope one at the knee,
// so no step appears where the roll-off begins, and it is monotone, so a
// brighter surface still reads brighter. Values below the knee are bit-for-bit
// what they were, which is what keeps the distant band, the water and the
// mid-tone grass unchanged.
//
// Cost: one max-of-three, one compare, one subtract, one multiply, one `exp`,
// one division and three multiplies per opaque pixel. No new pass, no second
// render target, no texture and no uniform: the same fragment shader entry point
// every sample already runs.

/// Lit value at which the shoulder starts. 0.85 is above the brightest mid-tone
/// a near-field surface has (grass at about 0.53, sand at about 0.79) and below
/// the near-white materials the clamp was eating.
const HIGHLIGHT_KNEE: f32 = 0.85;
/// Asymptote of the shoulder: the brightest value the opaque pass can emit.
/// Short of 1.0 by 0.01 so an 8-bit target cannot round up to 255, and high
/// enough that the surface still reads as sunlit rather than dimmed.
const HIGHLIGHT_CEIL: f32 = 0.99;

/// Map one lit colour through the shoulder on its brightest channel.
fn highlight_roll_off(lit: vec3<f32>) -> vec3<f32> {
    let peak = max(lit.r, max(lit.g, lit.b));
    if peak <= HIGHLIGHT_KNEE {
        return lit;
    }
    let span = HIGHLIGHT_CEIL - HIGHLIGHT_KNEE;
    let over = (peak - HIGHLIGHT_KNEE) / span;
    let shoulder = HIGHLIGHT_KNEE + span * (1.0 - exp(-over));
    return lit * (shoulder / peak);
}

/// Shade one opaque surface. The derived water pass has its own entry and its
/// own model; what this function still owns is the *coarse* water of a distance
/// ring, which arrives here as terrain because that is what it is.
fn shade(v: Output) -> vec4<f32> {
    var normal = normalize(v.normal);
    var color = v.color;
    let enhanced = camera.eye.w < 0.0;
    // Palette ID13 water is the unique source color (0.16,0.34,0.42).
    // Geometry and liquid collision policy remain authoritative CPU voxel data.
    let water = distance(v.color, vec3(0.16,0.34,0.42)) < 0.001;
    let view = normalize(camera.eye.xyz-v.world);
    var highlight = vec3(0.0);
    if enhanced {
        let near_detail = 1.0-smoothstep(12.0,30.0,distance(v.world,camera.eye.xyz));
        let grain = sin(v.world.x*6.7+v.world.z*3.1)*sin(v.world.y*7.3-v.world.z*5.3);
        color *= 1.0 + 0.07*near_detail*grain;
        if water && normal.y>0.5 {
            let time = -camera.eye.w-1.0;
            let a = v.world.x*2.8+v.world.z*1.7+time*0.5;
            let b = v.world.x*-1.4+v.world.z*3.3-time*0.8;
            normal = normalize(vec3(-0.045*cos(a)-0.035*cos(b),1.0,-0.03*cos(a)+0.05*cos(b)));
            let fresnel = pow(1.0-max(dot(normal,view),0.0),5.0);
            color = mix(vec3(0.08,0.23,0.27),vec3(0.30,0.44,0.45),fresnel*0.75);
            let half_vector = normalize(view+lighting.sun.xyz);
            highlight = vec3(0.84,0.81,0.62)*pow(max(dot(normal,half_vector),0.0),80.0)*0.55;
        }
    }
    // Flooded coarse terrain: the far sea of a distance ring, drawn by this
    // opaque pass in the water colour. It reads as a surface only where the
    // sample asked for it, and never in the wetland's own enhanced path.
    if water && !enhanced && normal.y > 0.5 && lighting.water.x > 0.5 {
        return water_surface(v.world, lighting.water.z, true);
    }
    var path_length = distance(v.world, camera.eye.xyz);
    // Face-orientation split, faded in over 60-300 m. Up close a top and a wall
    // differ by their material as much as by the light, so the near field keeps
    // the shading it always had; at range the atmosphere has removed what is
    // left of that difference, so how much sky the face can see carries both
    // the ambient and the direct sun. A top is unchanged at every distance,
    // which is what leaves a distant wall clearly darker than the top beside it.
    let up = clamp(normal.y, 0.0, 1.0);
    let split = smoothstep(FACE_SPLIT_START_M, FACE_SPLIT_END_M, path_length)
        * lighting.aerial.x;
    let ambient = mix(0.28 + 0.12 * up, DISTANT_AMBIENT_FLOOR + DISTANT_AMBIENT_UP * up, split);
    let sunlight = max(dot(normal, lighting.sun.xyz), 0.0)
        * mix(1.0, DISTANT_SUN_FLOOR + (1.0 - DISTANT_SUN_FLOOR) * up, split);
    // Per-voxel tone variation, faded in past the near field. Applied to the
    // material before lighting and fog so it shades like the material it
    // varies, and gated to the distance band so the shore keeps its look.
    let tone = smoothstep(TONE_START_M, TONE_END_M, path_length) * lighting.aerial.x;
    // Unconditional derivative: a derivative under non-uniform control flow is
    // undefined, and a silhouette can cross the fade threshold inside a quad.
    let footprint = length(dpdx(v.world));
    if tone > 0.0 {
        color = color * (1.0 + tone * tone_variation(v.world, footprint));
    }
    let visibility = shadow_visibility(v.world,v.normal);
    let indirect = indirect_diffuse(v.world, v.normal);
    var lit = color * (vec3(ambient + sunlight * lighting.sun.w * visibility) + indirect) + highlight*visibility;
    // Opt-in single specular bounce. With every mirror strength zero this block is
    // skipped and the result is bit-identical to nonreflective rendering.
    let mirror = reflection_mirror(v.world, v.normal);
    if mirror > 0.0 {
        let sample = specular_reflection(v.world, normal, camera.eye.xyz);
        path_length = path_length + sample.distance;
        lit = mix(lit, sample.color, mirror);
    }
    return vec4(aerial_perspective(highlight_roll_off(lit), view, path_length), 1.0);
}
@fragment fn fs_main(v: Output) -> @location(0) vec4<f32> {
    return shade(v);
}
// Derived water surface. The vertex colour is the bed depth under this corner,
// not a colour: `matterweave_core::water` owns that encoding and this entry owns
// the palette.
@fragment fn fs_water(v: Output) -> @location(0) vec4<f32> {
    return water_surface(v.world, v.color.r * WATER_MAX_DEPTH_M, false);
}
