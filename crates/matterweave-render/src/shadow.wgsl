struct Camera { view_proj: mat4x4<f32> };
var<push_constant> camera: Camera;
// Packed instance record: translation xyz, quarter-turn yaw in w. The identity
// record (0,0,0,0) leaves non-instanced casters unchanged. Must match world.wgsl
// and static_scene.rs rotate_xz exactly so casters and receivers agree.
//
// The fourth wind slot carries the per-instance scale, and nothing else: this
// pass deliberately does not run `world.wgsl`'s wind displacement, so a swaying
// plant casts its rest-pose shadow. It does apply the scale, because a caster
// whose shadow stayed prototype-sized while the plant is drawn 25% larger or
// smaller is simply wrong; `0` marks a record that is not flora, exactly as it
// does in `world.wgsl`.
fn quarter_rotation(yaw: f32) -> mat2x2<f32> {
    var quarter_cos = array<f32, 4>(1.0, 0.0, -1.0, 0.0);
    var quarter_sin = array<f32, 4>(0.0, 1.0, 0.0, -1.0);
    let q = u32(yaw) & 3u;
    return mat2x2<f32>(
        vec2(quarter_cos[q], -quarter_sin[q]),
        vec2(quarter_sin[q], quarter_cos[q]),
    );
}
@vertex fn vs_main(
    @location(0) position: vec3<f32>,
    @location(3) instance: vec4<f32>,
    @location(4) wind: vec4<f32>,
) -> @builtin(position) vec4<f32> {
    let rotation = quarter_rotation(instance.w);
    // A zero scale marks a record that is not flora, which draws unscaled.
    let scale = select(1.0, wind.w, wind.w > 0.0);
    let local = position * scale;
    let xz = rotation * vec2(local.x, local.z);
    let world_position = vec3(xz.x, local.y, xz.y) + instance.xyz;
    return camera.view_proj * vec4(world_position, 1.0);
}
