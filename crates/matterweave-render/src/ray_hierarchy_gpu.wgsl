// Optional packed-occupancy (BlockMask) traversal candidate for issue 42.
//
// Geometry, entry, tie and step rules are deliberately identical to the retained
// `ray_reference.wgsl`; only memory access changes. Every visited in-crop cell first
// resolves its block from the packed occupancy words (binding 3), fetched once per block
// entry and reused for the rest of that block:
//
// - an empty block still costs one whole-block fetch, but no cell bit test and no
//   material read;
// - a set bit gates the material read, and a bit is set exactly when the material word
//   is nonzero, so the hits are the same cells, materials, normals and distances.
//
// `crates/matterweave-render/src/ray_hierarchy_gpu.rs` owns the byte-level binding
// contract; this file owns the kernel. The shader has no `max_distance` input: it walks
// the clipped near/far segment only. The CPU `max_distance == 0` inside-solid query is
// therefore not expressible here (the retained reference discards a zero-length segment).
struct RayUniform {
    inverse_view_projection: mat4x4<f32>,
    view_projection: mat4x4<f32>,
    eye: vec4<f32>,
    origin: vec4<i32>,
    dimensions: vec4<u32>,
    sun: vec4<f32>,
};
struct HierarchyUniform {
    // Block extent in cells.
    shape: vec4<u32>,
    // Blocks per axis, ceil(dimensions / shape).
    block_dims: vec4<u32>,
    // x = words per block (at most 16, the cached_words bound), y = bits per block,
    // z and w reserved zero.
    misc: vec4<u32>,
};
@group(0) @binding(0) var<uniform> camera: RayUniform;
@group(0) @binding(1) var<storage, read> materials: array<u32>;
@group(0) @binding(2) var<storage, read> palette: array<vec4<f32>>;
@group(0) @binding(3) var<storage, read> occupancy: array<u32>;
@group(0) @binding(4) var<uniform> hierarchy: HierarchyUniform;
// One block's words are cached in registers (`words_per_block <= 16`, enforced by the
// upload contract); `NO_BLOCK` cannot collide with a real block index.
const NO_BLOCK: u32 = 0xffffffffu;
struct VertexOutput {
    @builtin(position) clip: vec4<f32>,
    @location(0) ndc: vec2<f32>,
};
@vertex fn vs_main(@builtin(vertex_index) index: u32) -> VertexOutput {
    // Draw exactly three vertices, no vertex buffers, culling disabled.
    let xy = vec2(f32((index << 1u) & 2u), f32(index & 2u)) * 2.0 - vec2(1.0);
    var out: VertexOutput;
    out.clip = vec4(xy, 0.0, 1.0);
    // This varying remains in pre-Naga clip coordinates. Naga flips position Y
    // only: interpolation thus supplies the correctly unflipped inverse-projection
    // input, without viewport dimensions or a second Y flip in the fragment.
    out.ndc = xy;
    return out;
}
struct FragmentOutput {
    @location(0) color: vec4<f32>,
    @builtin(frag_depth) depth: f32,
};
@fragment fn fs_main(v: VertexOutput) -> FragmentOutput {
    let near_h = camera.inverse_view_projection * vec4(v.ndc, 0.0, 1.0);
    let far_h = camera.inverse_view_projection * vec4(v.ndc, 1.0, 1.0);
    if near_h.w == 0.0 || far_h.w == 0.0 { discard; }
    var ray_origin = near_h.xyz / near_h.w;
    var far_point = far_h.xyz / far_h.w;
    // Inverse-projection arithmetic can move an exactly parallel grid-plane
    // ray by a few f32 ULPs on mobile GPUs. Stabilize only coordinates whose
    // BOTH segment endpoints lie within eight relative ULPs of the same integer
    // plane. Other rays, including near-boundary offsets, retain their slope.
    for (var axis = 0u; axis < 3u; axis += 1u) {
        let plane = round(ray_origin[axis]);
        let tolerance = 0.00000095367431640625 * max(1.0, abs(plane));
        if abs(ray_origin[axis] - plane) <= tolerance && abs(far_point[axis] - plane) <= tolerance {
            ray_origin[axis] = plane;
            far_point[axis] = plane;
        }
    }
    let segment = far_point - ray_origin;
    let ray_length = length(segment);
    if ray_length == 0.0 { discard; }
    let direction = segment / ray_length;
    let lower = vec3<f32>(camera.origin.xyz);
    let dims = camera.dimensions.xyz;
    let upper = lower + vec3<f32>(dims);

    // Slab clipping never divides by zero. Parallel rays on the upper face
    // are outside the half-open box; rays on its lower face are inside.
    var entry = 0.0;
    var exit = ray_length;
    var slab_near = vec3(-1.0);
    for (var axis = 0u; axis < 3u; axis += 1u) {
        if direction[axis] == 0.0 {
            if ray_origin[axis] < lower[axis] || ray_origin[axis] >= upper[axis] { discard; }
        } else {
            let a = (lower[axis] - ray_origin[axis]) / direction[axis];
            let b = (upper[axis] - ray_origin[axis]) / direction[axis];
            slab_near[axis] = min(a, b);
            entry = max(entry, min(a, b));
            exit = min(exit, max(a, b));
        }
    }
    // Zero-length AABB contact is not geometry (including corner-only touches).
    if entry >= exit { discard; }
    let start = ray_origin + direction * entry;
    var local = start - lower;
    let step = vec3<i32>(sign(direction));
    var normal = vec3(0.0);
    let starts_inside = all(ray_origin >= lower) && all(ray_origin < upper);
    var first = true;
    for (var axis = 0u; axis < 3u; axis += 1u) {
        // Snap only known slab-entry planes, never bias the entire ray.
        if !starts_inside && slab_near[axis] == entry {
            local[axis] = select(f32(dims[axis]), 0.0, step[axis] > 0);
        }
    }
    var cell = vec3<i32>(floor(local));
    for (var axis = 0u; axis < 3u; axis += 1u) {
        // An external entry can tie an INTERNAL grid plane on another axis.
        // Cross all of those together before inspecting the first inside cell.
        // An origin already inside instead checks floor(origin) first, like World.
        if !starts_inside && step[axis] != 0 && local[axis] == floor(local[axis]) {
            if step[axis] < 0 { cell[axis] -= 1; }
            if first { normal[axis] = -f32(step[axis]); first = false; }
        }
    }
    let shape = hierarchy.shape.xyz;
    let block_dims = hierarchy.block_dims.xyz;
    let words_per_block = hierarchy.misc.x;
    // The block cache: all words of the current block, fetched once per block entry.
    var cached_words: array<u32, 16>;
    var cached_block = NO_BLOCK;
    var cached_any = false;
    var distance = entry;
    // A monotonic ray crosses at most sum(dimensions) grid planes. Ties step
    // simultaneously. One extra iteration includes the initial cell.
    let crossing_bound = dims.x + dims.y + dims.z + 1u;
    for (var iteration = 0u; iteration < crossing_bound; iteration += 1u) {
        if any(cell < vec3(0)) || any(cell >= vec3<i32>(dims)) { discard; }
        let c = vec3<u32>(cell);
        let index = c.x + dims.x * (c.y + dims.y * c.z);
        let block = c / shape;
        let block_index = block.x + block_dims.x * (block.y + block_dims.y * block.z);
        if block_index != cached_block {
            cached_block = block_index;
            cached_any = false;
            let base = block_index * words_per_block;
            for (var word = 0u; word < words_per_block; word += 1u) {
                let value = occupancy[base + word];
                cached_words[word] = value;
                cached_any = cached_any || value != 0u;
            }
        }
        if cached_any {
            let local_cell = c % shape;
            let bit = local_cell.x + shape.x * (local_cell.y + shape.y * local_cell.z);
            let occupied = (cached_words[bit / 32u] & (1u << (bit % 32u))) != 0u;
            if occupied {
                let material = materials[index];
                if material != 0u {
                    let world = ray_origin + direction * distance;
                    let clip = camera.view_projection * vec4(world, 1.0);
                    if clip.w == 0.0 { discard; }
                    let depth = clip.z / clip.w;
                    // The clipped ray segment already limits hits to near/far. Permit
                    // small re-projection roundoff at either plane, never an arbitrary
                    // behind-camera hit, then emit legal Vulkan depth.
                    if depth < -0.00001 || depth > 1.00001 { discard; }
                    let sunlight = max(dot(normal, camera.sun.xyz), 0.0);
                    let ambient = 0.28 + 0.12 * max(normal.y, 0.0);
                    let lit = palette[material].xyz * (ambient + sunlight * camera.sun.w);
                    let fog = 1.0 - exp(-length(world - camera.eye.xyz) * 0.013);
                    var out: FragmentOutput;
                    out.color = vec4(mix(lit, vec3(0.16, 0.24, 0.29), fog), 1.0);
                    out.depth = clamp(depth, 0.0, 1.0);
                    return out;
                }
            }
        }
        // Recompute from integer planes instead of accumulating tDelta error.
        // A finite sentinel beyond the clipped interval avoids infinity/NaN math.
        var next = vec3(ray_length + 1.0);
        for (var axis = 0u; axis < 3u; axis += 1u) {
            if step[axis] != 0 {
                let boundary = lower[axis] + f32(cell[axis]) + select(0.0, 1.0, step[axis] > 0);
                next[axis] = (boundary - ray_origin[axis]) / direction[axis];
            }
        }
        distance = min(next.x, min(next.y, next.z));
        if distance > exit { discard; }
        normal = vec3(0.0);
        first = true;
        for (var axis = 0u; axis < 3u; axis += 1u) {
            if next[axis] == distance && step[axis] != 0 {
                cell[axis] += step[axis];
                if first { normal[axis] = -f32(step[axis]); first = false; }
            }
        }
    }
    discard;
}
