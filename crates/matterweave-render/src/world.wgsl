struct Camera {
    view_proj: mat4x4<f32>,
    eye: vec4<f32>,
};
var<push_constant> camera: Camera;
// Layout is defined once in Rust: crates/matterweave-render/src/shader_contract.rs.
// `indirect_dimensions.w` and `reflection_dimensions.w` are enable flags.
// `reflection_params` is (trace step bound, surface offset, 0, 0).
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
};
@group(0) @binding(0) var<uniform> lighting: Lighting;
@group(0) @binding(1) var shadow_map: texture_depth_2d;
@group(0) @binding(2) var shadow_sampler: sampler_comparison;
@group(0) @binding(3) var<storage, read> indirect_faces: array<vec4<f32>>;
// Bounded reflection source: one u32 material per cell, x + dims.x * (y + dims.y * z).
@group(0) @binding(4) var<storage, read> reflection_materials: array<u32>;
// 256 vec4s: rgb reflectance plus mirror strength in w.
@group(0) @binding(5) var<storage, read> reflection_palette: array<vec4<f32>>;
// Colour a reflected ray terminates against; identical to the fog target.
const REFLECTION_BACKGROUND = vec3<f32>(0.16, 0.24, 0.29);
struct Input {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) color: vec3<f32>,
    // Packed instance record: translation xyz, quarter-turn yaw in w. The
    // identity record (0,0,0,0) leaves non-instanced geometry unchanged.
    @location(3) instance: vec4<f32>,
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
@vertex fn vs_main(v: Input) -> Output {
    let rotation = quarter_rotation(v.instance.w);
    let xz = rotation * vec2(v.position.x, v.position.z);
    let world_position = vec3(xz.x, v.position.y, xz.y) + v.instance.xyz;
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
// volume. A miss terminates against REFLECTION_BACKGROUND at the volume exit distance.
// Reflected hits use documented simple shading: no shadow lookup, no second bounce.
fn specular_reflection(world_pos: vec3<f32>, normal: vec3<f32>, eye: vec3<f32>) -> ReflectionSample {
    var out: ReflectionSample;
    out.hit = false;
    out.color = REFLECTION_BACKGROUND;
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
@fragment fn fs_main(v: Output) -> @location(0) vec4<f32> {
    var normal = normalize(v.normal);
    var color = v.color;
    let enhanced = camera.eye.w < 0.0;
    // Palette ID13 water is the unique source color (0.16,0.34,0.42).
    // Geometry and liquid collision policy remain authoritative CPU voxel data.
    let water = enhanced && distance(v.color, vec3(0.16,0.34,0.42)) < 0.001;
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
    let sunlight = max(dot(normal, lighting.sun.xyz), 0.0);
    let ambient = 0.28 + 0.12 * max(normal.y, 0.0);
    let visibility = shadow_visibility(v.world,v.normal);
    let indirect = indirect_diffuse(v.world, v.normal);
    var lit = color * (vec3(ambient + sunlight * lighting.sun.w * visibility) + indirect) + highlight*visibility;
    // Opt-in single specular bounce. With every mirror strength zero this block is
    // skipped and the result is bit-identical to nonreflective rendering.
    var path_length = distance(v.world, camera.eye.xyz);
    let mirror = reflection_mirror(v.world, v.normal);
    if mirror > 0.0 {
        let sample = specular_reflection(v.world, normal, camera.eye.xyz);
        path_length = path_length + sample.distance;
        lit = mix(lit, sample.color, mirror);
    }
    let fog = 1.0 - exp(-path_length * 0.013);
    return vec4(mix(lit, REFLECTION_BACKGROUND, fog), 1.0);
}
