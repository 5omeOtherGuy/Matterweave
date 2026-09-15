// Full-resolution presentation of the reduced-resolution world target: one
// bilinear fetch per output pixel. The HUD is drawn after this in the same
// pass, so text never passes through the filter.
//
// `texel` is the inverse of the *present* extent, not the target's own: the
// fragment position is in present pixels and has to map the whole present range
// onto the target's 0..1 range. Sampling by the target's dimension instead
// would magnify the world and clip its right and bottom edges.
struct Upscale {
    texel: vec2<f32>,
};
var<push_constant> upscale: Upscale;
@group(0) @binding(0) var world_color: texture_2d<f32>;
@group(0) @binding(1) var world_sampler: sampler;

@vertex fn vs_main(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    let x = f32(i32(index & 1u) * 4 - 1);
    let y = f32(i32(index >> 1u) * 4 - 1);
    return vec4(x, y, 1.0, 1.0);
}

@fragment fn fs_main(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    return textureSample(world_color, world_sampler, frag.xy * upscale.texel);
}
