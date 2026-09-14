// Upsample of the reduced-resolution cloud target into the frame, drawn right
// after the sky dome and before any opaque geometry. The target holds
// premultiplied scattered light with `1 - transmittance` in alpha, so the
// pipeline blends it with ONE / ONE_MINUS_SRC_ALPHA and four bilinear taps at
// half-texel offsets soften the reduced resolution without a second pass.
struct Composite {
    // (inverse frame width, inverse frame height, cloud texel width, cloud texel height)
    texel: vec4<f32>,
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
    let offset = composite.texel.zw * 0.5;
    var sum = textureSample(cloud_texture, cloud_sampler, uv + vec2(-offset.x, -offset.y));
    sum = sum + textureSample(cloud_texture, cloud_sampler, uv + vec2(offset.x, -offset.y));
    sum = sum + textureSample(cloud_texture, cloud_sampler, uv + vec2(-offset.x, offset.y));
    sum = sum + textureSample(cloud_texture, cloud_sampler, uv + vec2(offset.x, offset.y));
    return sum * 0.25;
}
