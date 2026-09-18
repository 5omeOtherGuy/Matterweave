//! Host tests for the near-field highlight roll-off in `world.wgsl`.
//!
//! The curve itself cannot run without a device, so it is mirrored here in
//! Rust and the shader source is checked for the constants and the call site.
//! The properties tested are the ones the fix claims: identity below the knee,
//! monotone, bounded short of one for every input, and a single scale factor
//! across the three channels so hue and saturation survive.

/// The shader the renderer compiles, read from source.
fn shader() -> &'static str {
    include_str!("world.wgsl")
}

/// Mirror of `highlight_roll_off` in `world.wgsl`. Kept as literal constants:
/// if the shader's constants move, `the_shader_still_carries_the_roll_off` fails
/// before this copy can disagree with it.
const KNEE: f32 = 0.85;
const CEIL: f32 = 0.99;

fn roll_off(lit: [f32; 3]) -> [f32; 3] {
    let peak = lit[0].max(lit[1]).max(lit[2]);
    if peak <= KNEE {
        return lit;
    }
    let span = CEIL - KNEE;
    let over = (peak - KNEE) / span;
    let shoulder = KNEE + span * (1.0 - (-over).exp());
    let scale = shoulder / peak;
    lit.map(|channel| channel * scale)
}

#[test]
fn the_shader_still_carries_the_roll_off() {
    let world = shader();
    assert!(
        world.contains("const HIGHLIGHT_KNEE: f32 = 0.85;"),
        "the knee this test mirrors must still be the shader's"
    );
    assert!(
        world.contains("const HIGHLIGHT_CEIL: f32 = 0.99;"),
        "the ceiling this test mirrors must still be the shader's"
    );
    assert!(
        world.contains("aerial_perspective(highlight_roll_off(lit), view, path_length)"),
        "the opaque pass must still roll the lit colour off before the fog mix"
    );
}

#[test]
fn below_the_knee_is_unchanged_and_the_curve_never_decreases() {
    let bright = [0.8, 0.6, 0.5];
    assert_eq!(roll_off(bright), bright);
    assert_eq!(roll_off([KNEE, 0.1, 0.0]), [KNEE, 0.1, 0.0]);
    // Continuous at the knee: a hair over it moves a hair at most.
    let just_over = roll_off([KNEE + 1.0e-4, 0.0, 0.0])[0];
    assert!((just_over - KNEE).abs() < 1.0e-3, "{just_over}");
    let mut previous = 0.0;
    for step in 0..=400 {
        let value = step as f32 * 0.01;
        let mapped = roll_off([value, 0.0, 0.0])[0];
        assert!(
            mapped >= previous - 1.0e-6,
            "{value}: {mapped} < {previous}"
        );
        // The asympote is reached once `exp` underflows, so allow the last
        // f32 ulp; the 8-bit check below is the bound that matters.
        assert!(
            mapped <= CEIL + 1.0e-6,
            "{value} mapped to {mapped}, past the ceiling"
        );
        assert!((mapped * 255.0).round() <= 254.0, "{value}: {mapped}");
        previous = mapped;
    }
}

#[test]
fn no_sunlit_albedo_can_reach_the_clamp() {
    // The world spawn's bracket: sky ambient 0.28 + 0.12 * up = 0.40 on a top
    // face, plus the direct sun cos(n, sun) * 0.85. With the sun at
    // [0.35, 0.82, 0.45] the cosine is 0.821, so the bracket is 1.098. Snow's
    // palette colour lifted by the tone family reaches about 1.16 per channel,
    // and the near-field grain another 7%: 1.36 covers every reachable value.
    let bracket = 1.098_f32;
    for albedo in [0.83, 0.86, 0.92, 1.0, 1.16, 1.36] {
        let peak = roll_off([albedo * bracket; 3])[2];
        assert!(peak <= CEIL + 1.0e-6, "albedo {albedo} reached {peak}");
        // The 8-bit target must not round the value back up to 255.
        assert!(
            (peak * 255.0).round() <= 255.0 - 1.0,
            "albedo {albedo}: {peak}"
        );
    }
    // A brighter-than-white input is still bounded: the asymptote is the cap.
    assert!(roll_off([50.0, 50.0, 50.0])[0] <= CEIL + 1.0e-6);
}

#[test]
fn the_shoulder_scales_all_three_channels_by_the_same_factor() {
    // Warm sand and cool snow with the same peak must keep their ratios.
    let sand = [1.05, 0.90, 0.55];
    let snow = [0.95, 1.05, 1.15];
    let mapped_sand = roll_off(sand);
    let mapped_snow = roll_off(snow);
    let sand_scale = mapped_sand[0] / sand[0];
    let snow_scale = mapped_snow[2] / snow[2];
    for channel in 0..3 {
        assert!((mapped_sand[channel] / sand[channel] - sand_scale).abs() < 1.0e-6);
        assert!((mapped_snow[channel] / snow[channel] - snow_scale).abs() < 1.0e-6);
    }
    // Hue is the ordering of the channels, and it is unchanged.
    assert!(mapped_sand[0] > mapped_sand[1] && mapped_sand[1] > mapped_sand[2]);
    assert!(mapped_snow[2] > mapped_snow[1] && mapped_snow[1] > mapped_snow[0]);
}
