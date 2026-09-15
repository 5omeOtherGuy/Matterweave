//! Pure image comparison for the opt-in render-scale acceptance gate.
//!
//! The gate is the `--scale-check` run of the landscape sample: it captures the
//! real presented frame at native scale and at the configured scale, and these
//! functions turn the two buffers into the numbers the review asks for. Nothing
//! here knows about Vulkan, so the arithmetic is unit-testable.
use std::path::Path;

/// How far a scaled frame is from native, in 8-bit units.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ImageDifference {
    /// Pixels where the largest per-channel absolute difference is strictly
    /// greater than 4/255, as a fraction of all pixels.
    pub over_4_255: f64,
    /// Mean absolute per-channel difference over every pixel, in 0..=255.
    pub mean_absolute: f64,
    /// Pixels whose bytes are identical.
    pub identical: usize,
    pub pixels: usize,
}

impl ImageDifference {
    /// True when the two images are byte-identical, which is the requirement
    /// for the HUD-only pair.
    pub fn bit_identical(&self) -> bool {
        self.identical == self.pixels
    }
}

/// Compare two tightly packed RGBA8 images. Returns `None` when the lengths
/// are not equal or not a multiple of four: a mismatched pair is a bug in the
/// gate, not a large difference.
pub fn image_difference(a: &[u8], b: &[u8]) -> Option<ImageDifference> {
    if a.len() != b.len() || a.len() % 4 != 0 || a.is_empty() {
        return None;
    }
    let mut over = 0usize;
    let mut identical = 0usize;
    let mut total = 0u64;
    for (pa, pb) in a.chunks_exact(4).zip(b.chunks_exact(4)) {
        let mut worst = 0u8;
        let mut all_equal = true;
        for channel in 0..4 {
            let diff = pa[channel].abs_diff(pb[channel]);
            worst = worst.max(diff);
            all_equal &= diff == 0;
            total += u64::from(diff);
        }
        if worst > 4 {
            over += 1;
        }
        if all_equal {
            identical += 1;
        }
    }
    let pixels = a.len() / 4;
    Some(ImageDifference {
        over_4_255: over as f64 / pixels as f64,
        mean_absolute: total as f64 / (pixels * 4) as f64,
        identical,
        pixels,
    })
}

/// Write one binary PPM (P6) beside the run's save, for eyeballing the pair.
pub fn write_ppm(path: &Path, width: u32, height: u32, rgba: &[u8]) -> Result<(), String> {
    let expected = (width as usize) * (height as usize) * 4;
    if rgba.len() != expected {
        return Err(format!(
            "PPM payload is {} bytes, expected {expected}",
            rgba.len()
        ));
    }
    let mut bytes = Vec::with_capacity(expected / 4 * 3 + 32);
    bytes.extend_from_slice(format!("P6\n{width} {height}\n255\n").as_bytes());
    for pixel in rgba.chunks_exact(4) {
        bytes.extend_from_slice(&pixel[..3]);
    }
    std::fs::write(path, bytes).map_err(|error| format!("{}: {error}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_images_are_zero_difference_and_bit_identical() {
        let image = [10u8, 20, 30, 255, 1, 2, 3, 4];
        let difference = image_difference(&image, &image).unwrap();
        assert!(difference.bit_identical());
        assert_eq!(difference.over_4_255, 0.0);
        assert_eq!(difference.mean_absolute, 0.0);
    }

    #[test]
    fn the_threshold_is_strictly_more_than_four() {
        let a = [100u8, 100, 100, 255];
        let b = [104u8, 100, 100, 255];
        let at = image_difference(&a, &b).unwrap();
        assert_eq!(at.over_4_255, 0.0, "4/255 is not over the threshold");
        assert!((at.mean_absolute - 1.0).abs() < 1.0e-9);
        let b = [105u8, 100, 100, 255];
        let over = image_difference(&a, &b).unwrap();
        assert_eq!(over.over_4_255, 1.0);
        assert!((over.mean_absolute - 1.25).abs() < 1.0e-9);
        assert!(!over.bit_identical());
    }

    #[test]
    fn mismatch_and_empty_inputs_are_not_differences() {
        assert!(image_difference(&[0; 4], &[0; 8]).is_none());
        assert!(image_difference(&[0; 5], &[0; 5]).is_none());
        assert!(image_difference(&[], &[]).is_none());
    }
}
