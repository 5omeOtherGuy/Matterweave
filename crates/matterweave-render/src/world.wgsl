struct Camera {
    view_proj: mat4x4<f32>,
    eye: vec4<f32>,
};
var<push_constant> camera: Camera;
struct Lighting {
    view_proj: mat4x4<f32>,
    sun: vec4<f32>,
    // enabled, normalized depth bias, inverse map size, unused
    params: vec4<f32>,
};
@group(0) @binding(0) var<uniform> lighting: Lighting;
@group(0) @binding(1) var shadow_map: texture_depth_2d;
@group(0) @binding(2) var shadow_sampler: sampler_comparison;
struct Input {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) color: vec3<f32>,
};
struct Output {
    @builtin(position) clip: vec4<f32>,
    @location(0) world: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) color: vec3<f32>,
};
@vertex fn vs_main(v: Input) -> Output {
    var out: Output;
    out.clip = camera.view_proj * vec4(v.position, 1.0);
    out.world = v.position;
    out.normal = v.normal;
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
            // Explicit integer fetch separates stored depth/raster precision
            // from the comparison sampler's coordinate and Dref behavior.
            let sample_texel = vec2<i32>(floor(tap_uv / lighting.params.z));
            let map_size = vec2<i32>(textureDimensions(shadow_map));
            var stored_depth = 1.0;
            if all(sample_texel >= vec2<i32>(0)) && all(sample_texel < map_size) {
                stored_depth = textureLoad(shadow_map, sample_texel, 0);
            }
            visible += select(0.0, 1.0, sample_depth <= stored_depth);
        }
    }
    // Fade the finite XY coverage rather than exposing a hard moving map boundary.
    let edge = smoothstep(0.85, 0.98, max(abs(ndc.x), abs(ndc.y)));
    return mix(visible / 9.0, 1.0, edge);
}
@fragment fn fs_main(v: Output) -> @location(0) vec4<f32> {
    let sunlight = max(dot(normalize(v.normal), lighting.sun.xyz), 0.0);
    let ambient = 0.28 + 0.12 * max(v.normal.y, 0.0);
    let lit = v.color * (ambient + sunlight * lighting.sun.w * shadow_visibility(v.world, v.normal));
    let fog = 1.0 - exp(-distance(v.world, camera.eye.xyz) * 0.013);
    return vec4(mix(lit, vec3(0.16, 0.24, 0.29), fog), 1.0);
}
