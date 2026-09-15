// Upsample of the reduced-resolution cloud target into the frame, drawn after
// the sky dome with a depth test that keeps it on background pixels only. The
// target holds premultiplied scattered light with `1 - transmittance` in alpha,
// so the pipeline blends it with ONE / ONE_MINUS_SRC_ALPHA and four bilinear
// taps at half-texel offsets soften the reduced resolution without a second
// pass.
//
// The target is a frame behind: the cloud march needs the depth buffer this
// frame's opaque pass just wrote, so it runs after the frame pass and the
// composite can only read the previous frame's result. `homography` maps this
// frame's screen position back to where that result put the same world
// direction, which cancels the lag for rotation exactly and leaves only the
// small translation parallax between the cloud layer and the far plane.
struct Composite {
    // (inverse frame width, inverse frame height, cloud texel width, cloud texel height)
    texel: vec4<f32>,
    // Column-major 3x3 homography, current frame UV -> previous target UV.
    h0: vec4<f32>,
    h1: vec4<f32>,
    h2: vec4<f32>,
};
var<push_constant> composite: Composite;
@group(0) @binding(0) var cloud_texture: texture_2d<f32>;
@group(0) @binding(1) var cloud_sampler: sampler;

@vertex fn vs_main(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    let x = f32(i32(index & 1u) * 4 - 1);
    let y = f32(i32(index >> 1u) * 4 - 1);
    return vec4(x, y, 1.0, 1.0);
}

@fragment fn fs_main(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    let uv = frag.xy * composite.texel.xy;
    let h = mat3x3<f32>(composite.h0.xyz, composite.h1.xyz, composite.h2.xyz);
    let mapped = h * vec3(uv, 1.0);
    let uv_previous = mapped.xy / mapped.z;
    let offset = composite.texel.zw * 0.5;
    var sum = textureSample(cloud_texture, cloud_sampler, uv_previous + vec2(-offset.x, -offset.y));
    sum = sum + textureSample(cloud_texture, cloud_sampler, uv_previous + vec2(offset.x, -offset.y));
    sum = sum + textureSample(cloud_texture, cloud_sampler, uv_previous + vec2(-offset.x, offset.y));
    sum = sum + textureSample(cloud_texture, cloud_sampler, uv_previous + vec2(offset.x, offset.y));
    return sum * 0.25;
}
