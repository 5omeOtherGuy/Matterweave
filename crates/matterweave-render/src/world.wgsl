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
    // Packed instance record: translation xyz, quarter-turn yaw in w. The
    // identity record (0,0,0,0) leaves non-instanced geometry unchanged.
    @location(3) instance: vec4<f32>,
};
struct Output {
    @builtin(position) clip: vec4<f32>,
    @location(0) world: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) color: vec3<f32>,
};
// Exact quarter-turn yaw about Y: x' = c*x + s*z, z' = -s*x + c*z.
// Must match static_scene.rs rotate_xz and the packed instance record.
fn quarter_rotation(yaw: f32) -> mat2x2<f32> {
    var quarter_cos = array<f32, 4>(1.0, 0.0, -1.0, 0.0);
    var quarter_sin = array<f32, 4>(0.0, 1.0, 0.0, -1.0);
    let q = u32(yaw) & 3u;
    return mat2x2<f32>(
        vec2(quarter_cos[q], -quarter_sin[q]),
        vec2(quarter_sin[q], quarter_cos[q]),
    );
}
@vertex fn vs_main(v: Input) -> Output {
    let rotation = quarter_rotation(v.instance.w);
    let xz = rotation * vec2(v.position.x, v.position.z);
    let world_position = vec3(xz.x, v.position.y, xz.y) + v.instance.xyz;
    let nxz = rotation * vec2(v.normal.x, v.normal.z);
    let world_normal = vec3(nxz.x, v.normal.y, nxz.y);
    var out: Output;
    out.clip = camera.view_proj * vec4(world_position, 1.0);
    out.world = world_position;
    out.normal = world_normal;
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
            visible += textureSampleCompareLevel(shadow_map, shadow_sampler,
                sample_uv, sample_depth);
        }
    }
    // Fade the finite XY coverage rather than exposing a hard moving map boundary.
    let edge = smoothstep(0.85, 0.98, max(abs(ndc.x), abs(ndc.y)));
    return mix(visible / 9.0, 1.0, edge);
}
@fragment fn fs_main(v: Output) -> @location(0) vec4<f32> {
    var normal = normalize(v.normal);
    var color = v.color;
    let enhanced = camera.eye.w < 0.0;
    // Palette ID13 water is the unique source color (0.16,0.34,0.42).
    // Geometry and liquid collision policy remain authoritative CPU voxel data.
    let water = enhanced && distance(v.color, vec3(0.16,0.34,0.42)) < 0.001;
    let view = normalize(camera.eye.xyz-v.world);
    var highlight = vec3(0.0);
    if enhanced {
        let near_detail = 1.0-smoothstep(12.0,30.0,distance(v.world,camera.eye.xyz));
        let grain = sin(v.world.x*6.7+v.world.z*3.1)*sin(v.world.y*7.3-v.world.z*5.3);
        color *= 1.0 + 0.07*near_detail*grain;
        if water && normal.y>0.5 {
            let time = -camera.eye.w-1.0;
            let a = v.world.x*2.8+v.world.z*1.7+time*0.5;
            let b = v.world.x*-1.4+v.world.z*3.3-time*0.8;
            normal = normalize(vec3(-0.045*cos(a)-0.035*cos(b),1.0,-0.03*cos(a)+0.05*cos(b)));
            let fresnel = pow(1.0-max(dot(normal,view),0.0),5.0);
            color = mix(vec3(0.08,0.23,0.27),vec3(0.30,0.44,0.45),fresnel*0.75);
            let half_vector = normalize(view+lighting.sun.xyz);
            highlight = vec3(0.84,0.81,0.62)*pow(max(dot(normal,half_vector),0.0),80.0)*0.55;
        }
    }
    let sunlight = max(dot(normal, lighting.sun.xyz), 0.0);
    let ambient = 0.28 + 0.12 * max(normal.y, 0.0);
    let visibility = shadow_visibility(v.world,v.normal);
    let lit = color * (ambient + sunlight * lighting.sun.w * visibility) + highlight*visibility;
    let fog = 1.0 - exp(-distance(v.world, camera.eye.xyz) * 0.013);
    return vec4(mix(lit, vec3(0.16, 0.24, 0.29), fog), 1.0);
}
