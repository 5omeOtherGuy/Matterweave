//! Soft biome classification: the weights behind a readable transition.
//!
//! [`crate::landscape::column`] used to pick exactly one biome per column from
//! hard thresholds on the climate and relief fields. That is a fence line: two
//! neighbouring metre-columns a step apart report different ground materials and
//! different vegetation pressures, and walking across the boundary shows the
//! whole palette change in one metre.
//!
//! This module keeps the same thresholds but turns each one into a band. Every
//! band is expressed in the deciding scalar's own unit, so the width is a
//! property of the generator and not of where a boundary happens to fall:
//!
//! | boundary | scalar | band | typical walking distance |
//! | --- | --- | --- | --- |
//! | ocean -> shore | height | 8 m of elevation | measured, see the test |
//! | shore -> inland | height | 8 m of elevation | measured, see the test |
//! | desert | temperature and humidity | 1200 / 1500 of 65535 | tens of metres |
//! | cold / wet | temperature / humidity | 1200 / 1500 of 65535 | tens of metres |
//! | mountain / snow | height | 8 m of elevation | tens of metres |
//! | hills | relief | 3 m of relief | tens of metres |
//!
//! Every band is *centred* on the threshold it replaced, so the hard
//! classification is the middle of its own band and the two agree on where the
//! transition is; only its width is new.
//!
//! The result is a mixture of up to [`MAX_BLEND`] biomes whose weights sum to
//! exactly 256, largest first. It is a pure integer function of the same fields
//! the hard classifier read, so it needs no storage, no second noise stack and
//! no per-frame work, and it is identical on every target.
//!
//! The mixture is used in two places, and both matter:
//!
//! - **material**: [`crate::landscape`] picks one candidate's palette with a
//!   deterministic 1-in-256 roll per 8 m patch, so sand gives way to grass as a
//!   thinning pattern and not as a line. The base colours are the palette's
//!   own; nothing here invents a blended colour that the renderer would have to
//!   know about.
//! - **vegetation**: the flora pressures are interpolated by the weights, which
//!   is what makes a forest edge thin out and a meadow grow flowers as you
//!   approach. A pressure is already a probability, so no dither is needed.

use crate::landscape::{Biome, SEA_LEVEL};

/// Biome candidates a mixture keeps. More than four non-zero weights cannot
/// arise from the exclusive thresholds below; the smallest are dropped and the
/// rest renormalised if they ever do.
pub const MAX_BLEND: usize = 4;

/// Temperature band, in field units of `0..=65535`. The temperature lattice
/// moves 14 units per metre at the median and 118 at the 99th percentile, so
/// this is a transition of tens of metres on foot everywhere the generator puts
/// ground a player can stand on.
pub const TEMPERATURE_BAND: i32 = 3400;
/// Humidity band, in field units. The humidity lattice is the faster of the two,
/// 23 units per metre at the median and 131 at the 99th percentile.
pub const HUMIDITY_BAND: i32 = 4200;
/// Elevation band at the snow line and the mountain threshold, in metres. The
/// ground rises about 0.17 m per metre at the median, so this is seventy metres
/// of walking - and still eight metres where the ground is at its steepest.
pub const HIGH_GROUND_BAND_M: i32 = 16;
/// Elevation band over which the shore's sand gives way inland, in metres.
pub const SHORE_BAND_M: i32 = 16;
/// Relief band around the hill threshold, in metres. Relief is the noisiest of
/// the deciding scalars, which is why its band is the widest in its own units.
pub const RELIEF_BAND_M: i32 = 8;

/// Climate and relief scalars one column is classified from. Every field is the
/// generator's own integer output; nothing here is derived from a camera, a
/// frame or a world edit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Inputs {
    /// Temperature field, `0..=65535`; cold below 26000, hot above 41000.
    pub temperature: i32,
    /// Humidity field, `0..=65535`; dry below 26000, wet above 43000.
    pub humidity: i32,
    /// Shaped surface height in metres.
    pub height: i32,
    /// Signed hill relief in metres, about `-10..=10`.
    pub relief: i32,
}

/// A bounded mixture of biomes. Weights are in 1/256ths and sum to 256.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BiomeMix {
    entries: [(Biome, u16); MAX_BLEND],
    len: u8,
}

impl BiomeMix {
    /// A mixture of one biome at full weight.
    pub fn single(biome: Biome) -> Self {
        Self {
            entries: [(biome, 256), (biome, 0), (biome, 0), (biome, 0)],
            len: 1,
        }
    }

    /// The weighted biomes, largest weight first, omitting zero weights.
    pub fn entries(&self) -> &[(Biome, u16)] {
        &self.entries[..self.len as usize]
    }

    /// Weight of one biome in 1/256ths; zero when it is not a candidate.
    pub fn weight(&self, biome: Biome) -> u16 {
        self.entries()
            .iter()
            .find(|(candidate, _)| *candidate == biome)
            .map_or(0, |(_, weight)| *weight)
    }

    /// The heaviest biome. Ties go to the earlier [`Biome::ALL`] entry, so the
    /// answer is a deterministic function of the weights alone.
    pub fn dominant(&self) -> Biome {
        let mut best = self.entries[0];
        for entry in self.entries() {
            if entry.1 > best.1 || (entry.1 == best.1 && order(entry.0) < order(best.0)) {
                best = *entry;
            }
        }
        best.0
    }

    /// Number of distinct biomes with a non-zero weight.
    pub fn len(&self) -> usize {
        self.len as usize
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Whether more than one biome has weight, i.e. this column sits in a
    /// transition band. `floor` excludes the tail of a band that has almost
    /// ended (see the demo's spawn-view check).
    pub fn mixed(&self, floor: u16) -> bool {
        self.entries()
            .iter()
            .filter(|(_, weight)| *weight >= floor)
            .count()
            > 1
    }

    /// Interpolate one per-biome pressure by the weights, rounded to nearest.
    ///
    /// Every biome's pressure is defined, so a mixture is an exact weighted
    /// mean of integer pressures: the result is continuous in the weights by
    /// construction, which is what the acceptance test measures.
    pub fn pressure(&self, of: impl Fn(Biome) -> u8) -> u8 {
        let mut sum = 0u32;
        for &(biome, weight) in self.entries() {
            sum += u32::from(of(biome)) * u32::from(weight);
        }
        ((sum + 128) / 256) as u8
    }

    /// Interpolate one per-biome integer quantity by the weights.
    pub fn blend_i32(&self, of: impl Fn(Biome) -> i32) -> i32 {
        let mut sum = 0i64;
        for &(biome, weight) in self.entries() {
            sum += i64::from(of(biome)) * i64::from(weight);
        }
        ((sum + 128) / 256) as i32
    }
}

/// Position of a biome in [`Biome::ALL`], for deterministic tie-breaking.
fn order(biome: Biome) -> usize {
    Biome::ALL
        .iter()
        .position(|candidate| *candidate == biome)
        .unwrap_or(usize::MAX)
}

/// Integer ramp from `0` at `low` to `256` at `high`, clamped. Duplicated from
/// the landscape's own helper because that one is private to its module and the
/// two must not drift apart silently.
fn ramp(value: i32, low: i32, high: i32) -> i32 {
    let span = (high - low).max(1);
    (((value - low).max(0) as i64 * 256) / span as i64).min(256) as i32
}

fn down(value: i32, low: i32, high: i32) -> i32 {
    256 - ramp(value, low, high)
}

/// The two ends of a band `width` wide centred on `threshold`.
fn band(threshold: i32, width: i32) -> (i32, i32) {
    let half = width / 2;
    (threshold - half, threshold + half)
}

/// `a * b / 256`, the weight-space product of two memberships.
fn mul(a: i32, b: i32) -> i32 {
    (a as i64 * b as i64 / 256) as i32
}

/// Snow line in metres: about 109 m in the coldest regions and 129 m in the
/// warmest, never below 72 m. Unchanged from the hard classifier.
pub fn snow_line(temperature: i32) -> i32 {
    (118 + ((temperature - 32768) as i64 * 46 / 32768) as i32).max(72)
}

/// The soft classification: a mixture of biomes that sums to 256.
///
/// Thresholds and their precedence are the ones the hard classifier used, so
/// the world a mixture renders is the same world; only the last few metres
/// either side of each threshold differ, and there they differ on purpose.
pub fn classify(inputs: Inputs) -> BiomeMix {
    if inputs.height < SEA_LEVEL {
        return BiomeMix::single(Biome::Ocean);
    }
    let snow = ramp(
        inputs.height,
        band(snow_line(inputs.temperature), HIGH_GROUND_BAND_M).0,
        band(snow_line(inputs.temperature), HIGH_GROUND_BAND_M).1,
    );
    let mut rest = 256 - snow;

    // The shore band: sand at the waterline, giving way to inland ground over
    // SHORE_BAND_M of elevation.
    let shore = down(
        inputs.height,
        band(SEA_LEVEL + 1, SHORE_BAND_M).0,
        band(SEA_LEVEL + 1, SHORE_BAND_M).1,
    );
    let shore_weight = mul(rest, shore);
    rest -= shore_weight;
    // Wet shores are bog rather than beach; the coastal band is wider than the
    // open-country wet threshold, as the hard classifier had it.
    let boggy = ramp(
        inputs.humidity,
        band(36000, HUMIDITY_BAND).0,
        band(36000, HUMIDITY_BAND).1,
    )
    .max(ramp(
        inputs.humidity,
        band(43000, HUMIDITY_BAND).0,
        band(43000, HUMIDITY_BAND).1,
    ));
    let swamp = mul(shore_weight, boggy);
    let beach = shore_weight - swamp;

    let hot = ramp(
        inputs.temperature,
        band(41000, TEMPERATURE_BAND).0,
        band(41000, TEMPERATURE_BAND).1,
    );
    let dry = down(
        inputs.humidity,
        band(26000, HUMIDITY_BAND).0,
        band(26000, HUMIDITY_BAND).1,
    );
    let desert = mul(rest, mul(hot, dry));
    rest -= desert;

    let cold = down(
        inputs.temperature,
        band(26000, TEMPERATURE_BAND).0,
        band(26000, TEMPERATURE_BAND).1,
    );
    let tundra = mul(rest, cold);
    rest -= tundra;

    let mountain = mul(
        rest,
        ramp(
            inputs.height,
            band(74, HIGH_GROUND_BAND_M).0,
            band(74, HIGH_GROUND_BAND_M).1,
        ),
    );
    rest -= mountain;

    let wet = ramp(
        inputs.humidity,
        band(43000, HUMIDITY_BAND).0,
        band(43000, HUMIDITY_BAND).1,
    );
    let forest = mul(rest, wet);
    rest -= forest;

    let hills = mul(
        rest,
        ramp(
            inputs.relief.abs(),
            band(5, RELIEF_BAND_M).0,
            band(5, RELIEF_BAND_M).1,
        ),
    );
    let plains = rest - hills;

    let candidates = [
        (Biome::Snow, snow),
        (Biome::Swamp, swamp),
        (Biome::Beach, beach),
        (Biome::Desert, desert),
        (Biome::Tundra, tundra),
        (Biome::Mountain, mountain),
        (Biome::Forest, forest),
        (Biome::Hills, hills),
        (Biome::Plains, plains),
    ];
    let mut weights: [(Biome, u16); 9] = candidates.map(|(biome, weight)| (biome, weight as u16));
    // Largest first, and by enum order between equals, so the mixture is a
    // deterministic function of the weights.
    weights.sort_by_key(|(biome, weight)| (std::cmp::Reverse(*weight), order(*biome)));

    let mut mix = BiomeMix {
        entries: [(Biome::Plains, 0); MAX_BLEND],
        len: 0,
    };
    let mut sum = 0u32;
    for (biome, weight) in weights {
        if weight == 0 || mix.len as usize == MAX_BLEND {
            break;
        }
        mix.entries[mix.len as usize] = (biome, weight);
        mix.len += 1;
        sum += u32::from(weight);
    }
    // Whatever is left over - the candidates past the last kept one - rides on
    // the heaviest entry, so the mixture always sums to exactly 256 and the
    // interpolation of a pressure stays an exact weighted mean.
    if mix.len == 0 {
        return BiomeMix::single(Biome::Plains);
    }
    let remainder = 256u32.saturating_sub(sum.min(256));
    mix.entries[0].1 += remainder as u16;
    mix
}

/// The hard classification this module replaced.
///
/// Kept as the reference a mixture has to agree with away from its bands: the
/// acceptance test asserts that a column whose weights are all in one biome is
/// classified exactly as this function classifies it, so a blend changes the
/// transitions and not the world.
pub fn hard_classify(inputs: Inputs) -> Biome {
    if inputs.height < SEA_LEVEL {
        return Biome::Ocean;
    }
    let cold = inputs.temperature < 26000;
    let hot = inputs.temperature > 41000;
    let wet = inputs.humidity > 43000;
    let dry = inputs.humidity < 26000;
    if inputs.height >= snow_line(inputs.temperature) {
        return Biome::Snow;
    }
    if inputs.height <= SEA_LEVEL + 1 {
        return if wet || inputs.humidity > 36000 {
            Biome::Swamp
        } else {
            Biome::Beach
        };
    }
    if hot && dry {
        return Biome::Desert;
    }
    if cold {
        return Biome::Tundra;
    }
    if inputs.height > 74 {
        return Biome::Mountain;
    }
    if wet {
        return Biome::Forest;
    }
    if inputs.relief.abs() > 5 {
        return Biome::Hills;
    }
    Biome::Plains
}
