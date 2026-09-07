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
    let slope = 1.0 - max(dot(normalize(normal), lighting.sun.xyz), 0.0);
    let depth = ndc.z - lighting.params.y * (1.0 + 2.0 * slope);
    var visible = 0.0;
    for (var y = -1; y <= 1; y += 1) {
        for (var x = -1; x <= 1; x += 1) {
            visible += textureSampleCompareLevel(shadow_map, shadow_sampler,
                uv + vec2(f32(x), f32(y)) * lighting.params.z, depth);
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
