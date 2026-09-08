// Comparison-only raster path for docs/performance/renderer-comparison.md.
// Deliberately minimal: exposed-surface mesh in, matched sun/ambient/fog out.
// No shadow map, no indirect lighting, no water/grain enhancement and no
// instancing, so its output is directly comparable with ray_reference.wgsl on
// the same camera, palette and depth range.
struct Camera {
    view_proj: mat4x4<f32>,
    eye: vec4<f32>,
    // Normalized direction to sun xyz, intensity w (same as ray_reference.wgsl).
    sun: vec4<f32>,
};
var<push_constant> camera: Camera;
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
@fragment fn fs_main(v: Output) -> @location(0) vec4<f32> {
    let normal = normalize(v.normal);
    let sunlight = max(dot(normal, camera.sun.xyz), 0.0);
    let ambient = 0.28 + 0.12 * max(normal.y, 0.0);
    let lit = v.color * (ambient + sunlight * camera.sun.w);
    let fog = 1.0 - exp(-distance(v.world, camera.eye.xyz) * 0.013);
    return vec4(mix(lit, vec3(0.16, 0.24, 0.29), fog), 1.0);
}
