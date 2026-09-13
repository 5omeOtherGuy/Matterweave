//! Top-down biome and relief map of the landscape generator, as a binary PPM.
//!
//! This is an evidence tool: it reads only the pure generator, so the image is
//! exactly what the render paths derive from. It prints biome coverage, height
//! extremes and the sea fraction next to the image.
//!
//! Usage: `landscape_map [seed] [pixels] [metres-per-pixel] [output.ppm]`

use matterweave_core::landscape::{self, Biome, SEA_LEVEL};
use std::io::Write;

fn biome_color(biome: Biome, flooded: bool, depth: i32) -> [f32; 3] {
    if flooded {
        // Deeper water is darker blue; shallow water is brighter and greener.
        let t = ((-depth) as f32 / 40.0).clamp(0.0, 1.0);
        return [
            0.10 + 0.06 * t,
            0.25 + 0.15 * (1.0 - t),
            0.45 + 0.10 * (1.0 - t),
        ];
    }
    match biome {
        Biome::Ocean => [0.10, 0.22, 0.38],
        Biome::Beach => [0.85, 0.80, 0.60],
        Biome::Desert => [0.85, 0.72, 0.42],
        Biome::Plains => [0.45, 0.62, 0.32],
        Biome::Forest => [0.22, 0.42, 0.24],
        Biome::Swamp => [0.30, 0.45, 0.33],
        Biome::Hills => [0.40, 0.52, 0.30],
        Biome::Mountain => [0.52, 0.48, 0.44],
        Biome::Snow => [0.92, 0.94, 0.96],
        Biome::Tundra => [0.66, 0.70, 0.62],
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let seed: u64 = args.get(1).and_then(|v| v.parse().ok()).unwrap_or(20260913);
    let pixels: i32 = args.get(2).and_then(|v| v.parse().ok()).unwrap_or(1024);
    let step: i32 = args.get(3).and_then(|v| v.parse().ok()).unwrap_or(8);
    let output = args
        .get(4)
        .cloned()
        .unwrap_or_else(|| "landscape-map.ppm".to_string());

    let mut counts = std::collections::BTreeMap::new();
    let mut flooded = 0usize;
    let mut lowest = i32::MAX;
    let mut highest = i32::MIN;
    let mut peak = (0, 0);
    let mut image = Vec::with_capacity((pixels * pixels * 3) as usize);
    let mut summary = Vec::new();

    for py in 0..pixels {
        for px in 0..pixels {
            let x = (px - pixels / 2) * step;
            let z = (py - pixels / 2) * step;
            let column = landscape::column(seed, x, z);
            *counts.entry(column.biome.name()).or_insert(0usize) += 1;
            flooded += usize::from(column.flooded());
            if column.height > highest {
                highest = column.height;
                peak = (x, z);
            }
            lowest = lowest.min(column.height);
            // One-sided difference for a simple north-west light.
            let shade = {
                let east = landscape::height_at(seed, x + step, z);
                let south = landscape::height_at(seed, x, z + step);
                let slope = (column.height - east) as f32 + (column.height - south) as f32;
                (1.0 + slope * 0.02).clamp(0.35, 1.5)
            };
            let base = biome_color(column.biome, column.flooded(), column.height);
            let rgb = base.map(|c| (c * shade).clamp(0.0, 1.0));
            image.extend_from_slice(&[
                (rgb[0] * 255.0) as u8,
                (rgb[1] * 255.0) as u8,
                (rgb[2] * 255.0) as u8,
            ]);
        }
        if py % 64 == 0 {
            summary.push(format!("row {py}/{pixels}"));
        }
    }

    let mut file = std::fs::File::create(&output).expect("create image");
    writeln!(file, "P6\n{pixels} {pixels}\n255").expect("write header");
    file.write_all(&image).expect("write pixels");
    println!(
        "seed {seed}, {pixels}x{pixels} pixels at {step} m, {}x{} m area",
        pixels * step,
        pixels * step
    );
    println!("height range {lowest}..{highest} (sea level {SEA_LEVEL}), peak at {peak:?}");
    println!(
        "flooded {:.1}% of columns",
        flooded as f64 * 100.0 / (pixels as f64 * pixels as f64)
    );
    for (name, count) in &counts {
        println!(
            "{name:>9}: {:>6.2}%",
            *count as f64 * 100.0 / (pixels as f64 * pixels as f64)
        );
    }
    println!("wrote {output}");
}
