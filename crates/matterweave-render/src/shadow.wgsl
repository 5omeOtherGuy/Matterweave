struct Camera { view_proj: mat4x4<f32> };
var<push_constant> camera: Camera;
@vertex fn vs_main(@location(0) position: vec3<f32>) -> @builtin(position) vec4<f32> {
    return camera.view_proj * vec4(position, 1.0);
}
