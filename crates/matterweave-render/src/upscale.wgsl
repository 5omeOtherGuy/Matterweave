// Full-resolution presentation of the reduced-resolution world target: one
// target sample per output pixel through a sharp-bilinear coordinate warp. The
// HUD is drawn after this in the same pass, so text never passes through the
// filter.
//
// The filter has to keep a one-pixel step a step. Plain bilinear blends the two
// target texels either side of an edge, halving its contrast, and at render
// scale 0.8 the distant voxel steps are one to two target pixels wide, so that
// blend is what erases them. The warp takes the middle of every target texel
// whole and interpolates only in a short band around the texel boundary; the
// mapping is continuous in the sample position, so sub-pixel camera motion
// moves an edge smoothly instead of snapping it between texels, and no weight
// leaves 0..1, so the filter never overshoots.
//
// `present_texel` is the inverse of the present extent: the fragment position
// is in present pixels and has to map the whole present range onto the target's
// 0..1 range. `target_texel` is the inverse of the target extent, which turns
// the position into a distance inside one target texel. `params.x` is the
// plateau fraction — the sample stays on the nearest texel centre within that
// fraction of the half-way distance to the boundary — and the renderer owns
// its value; `params.y` is unused.
struct Upscale {
    present_texel: vec2<f32>,
    target_texel: vec2<f32>,
    params: vec2<f32>,
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
    // The sample position in target pixels; texel centres sit at half-integers.
    let target_px = frag.xy * upscale.present_texel / upscale.target_texel;
    // `base` is the nearest texel centre and `d` the signed distance to it in
    // target pixels. Within `params.x * 0.5` of the centre the sample stays at
    // the centre; from there to the texel boundary it moves linearly to the
    // boundary, so the hardware blend across that boundary is the only
    // interpolation the output sees.
    let base = floor(target_px) + vec2<f32>(0.5);
    let d = target_px - base;
    let band = upscale.params.x * 0.5;
    let t = clamp((abs(d) - band) / (0.5 - band), vec2<f32>(0.0), vec2<f32>(1.0));
    return textureSample(
        world_color,
        world_sampler,
        (base + sign(d) * t * 0.5) * upscale.target_texel,
    );
}
