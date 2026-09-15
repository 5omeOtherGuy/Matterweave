//! Scratch probe (temporary): prints the spawn camera's neighbourhood.
use matterweave_core::landscape::{self, Biome, FloraKind, FLORA_CELL_M, TREE_CELL_M};

const SEED: u64 = 20260913;

fn spawn() -> ([f32; 3], f32) {
    for gz in (-24..24).rev() {
        for gx in -24..24 {
            let (x, z) = (gx * 64, gz * 64);
            let ground = landscape::height_at(SEED, x, z);
            if !(2..=14).contains(&ground) {
                continue;
            }
            for (dx, dz) in [
                (1, 0),
                (-1, 0),
                (0, 1),
                (0, -1),
                (1, 1),
                (-1, -1),
                (1, -1),
                (-1, 1),
            ] {
                if landscape::height_at(SEED, x + dx * 64, z + dz * 64) < -2 {
                    let yaw = (dx as f32).atan2(dz as f32);
                    return ([x as f32, ground as f32 + 8., z as f32], yaw);
                }
            }
        }
    }
    ([0., 40., 0.], 0.6)
}

fn main() {
    let (mut eye, mut yaw) = spawn();
    let args: Vec<String> = std::env::args().collect();
    if let Some(spec) = args.get(1) {
        let values: Vec<f32> = spec
            .split(',')
            .filter_map(|part| part.trim().parse().ok())
            .collect();
        if values.len() >= 4 {
            eye = [values[0], values[1], values[2]];
            yaw = values[3];
            println!("eye override {eye:?} yaw {yaw}");
        }
    }
    println!("spawn eye {eye:?} yaw {yaw}");
    let (ex, ez) = (eye[0] as i32, eye[2] as i32);
    for (name, x, z) in [
        ("eye", ex, ez),
        ("ahead16", ex + (16.0 * yaw.sin()) as i32, ez + (16.0 * yaw.cos()) as i32),
        ("ahead64", ex + (64.0 * yaw.sin()) as i32, ez + (64.0 * yaw.cos()) as i32),
        ("left64", ex - (64.0 * yaw.cos()) as i32, ez + (64.0 * yaw.sin()) as i32),
        ("right64", ex + (64.0 * yaw.cos()) as i32, ez - (64.0 * yaw.sin()) as i32),
        ("back64", ex - (64.0 * yaw.sin()) as i32, ez - (64.0 * yaw.cos()) as i32),
    ] {
        let c = landscape::column(SEED, x, z);
        println!(
            "{name:>8} ({x},{z}) h {} water {} biome {} surface {}",
            c.height,
            c.water_level,
            c.biome.name(),
            c.surface
        );
    }
    // Biome census in a disc around the eye.
    let mut counts = std::collections::BTreeMap::new();
    let mut ground = std::collections::BTreeMap::new();
    for x in (ex - 160)..=(ex + 160) {
        for z in (ez - 160)..=(ez + 160) {
            if (x - ex).abs().max((z - ez).abs()) > 160 {
                continue;
            }
            let c = landscape::column(SEED, x, z);
            *counts.entry(c.biome.name()).or_insert(0usize) += 1;
            if !c.flooded() {
                *ground.entry(c.height).or_insert(0usize) += 1;
            }
        }
    }
    println!("biomes within 160 m: {counts:?}");
    // Trees within 160 m, binned by distance and kind.
    let mut trees = Vec::new();
    let first = (ex - 160).div_euclid(TREE_CELL_M);
    let last = (ex + 160).div_euclid(TREE_CELL_M);
    let first_z = (ez - 160).div_euclid(TREE_CELL_M);
    let last_z = (ez + 160).div_euclid(TREE_CELL_M);
    for cz in first_z..=last_z {
        for cx in first..=last {
            if let Some(site) = landscape::tree_cell(SEED, cx, cz) {
                let d = (site.x - ex).abs().max((site.z - ez).abs());
                if d <= 160 {
                    trees.push((d, site));
                }
            }
        }
    }
    trees.sort_by_key(|(d, _)| *d);
    println!("trees within 160 m: {}", trees.len());
    let mut bins = std::collections::BTreeMap::new();
    for (d, _) in &trees {
        *bins.entry((d / 20) * 20).or_insert(0usize) += 1;
    }
    println!("tree distance bins (m): {bins:?}");
    for (d, site) in trees.iter().take(8) {
        println!("  tree d={d} {site:?} biome {}", landscape::biome_at(SEED, site.x, site.z).name());
    }
    // Land inside the spawn camera's forward cone, 20..=200 m, by biome.
    let forward = [yaw.sin(), yaw.cos()];
    {
        let mut cone = std::collections::BTreeMap::new();
        let mut cone_land = 0usize;
        for r in (20..=200).step_by(2) {
            for deg in -35..=35 {
                let a = (deg as f32).to_radians();
                let dx = forward[0] * a.cos() - forward[1] * a.sin();
                let dz = forward[0] * a.sin() + forward[1] * a.cos();
                let x = ex + (dx * r as f32) as i32;
                let z = ez + (dz * r as f32) as i32;
                let c = landscape::column(SEED, x, z);
                *cone.entry(c.biome.name()).or_insert(0usize) += 1;
                if !c.flooded() {
                    cone_land += 1;
                }
            }
        }
        println!("forward cone samples: {cone:?} (land {cone_land})");
    }
    // Trees inside the spawn camera's forward cone, 20..=200 m.
    let mut in_cone = Vec::new();
    for (d, site) in &trees {
        if !(20..=200).contains(d) {
            continue;
        }
        let dx = (site.x - ex) as f32;
        let dz = (site.z - ez) as f32;
        let len = (dx * dx + dz * dz).sqrt();
        if len < 1.0 {
            continue;
        }
        let cos = (dx * forward[0] + dz * forward[1]) / len;
        if cos >= (35.0f32).to_radians().cos() {
            in_cone.push((len, cos, site.x, site.z, site.y));
        }
    }
    in_cone.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
    println!(
        "trees in the forward 70-degree cone beyond 20 m: {}",
        in_cone.len()
    );
    // Trees actually inside the frame for the device (2.2) and host (16:9)
    // aspect ratios, using the sample's 65-degree vertical FOV and its yaw/pitch.
    {
        let pitch = -0.05f32;
        let fwd = [
            yaw.sin() * pitch.cos(),
            pitch.sin(),
            yaw.cos() * pitch.cos(),
        ];
        let right = [yaw.cos(), 0.0, -yaw.sin()];
        let up = [
            right[1] * fwd[2] - right[2] * fwd[1],
            right[2] * fwd[0] - right[0] * fwd[2],
            right[0] * fwd[1] - right[1] * fwd[0],
        ];
        let tan_y = (65.0f32 * 0.5).to_radians().tan();
        for aspect in [3168.0f32 / 1440.0, 16.0 / 9.0] {
            let tan_x = tan_y * aspect;
            let mut visible = 0usize;
            for (_, site) in &trees {
                let v = [
                    site.x as f32 - eye[0],
                    site.y as f32 + 1.5 - eye[1],
                    site.z as f32 - eye[2],
                ];
                let z = v[0] * fwd[0] + v[1] * fwd[1] + v[2] * fwd[2];
                if z <= 20.0 || z >= 200.0 {
                    continue;
                }
                let xs = (v[0] * right[0] + v[2] * right[2]) / (z * tan_x);
                let ys = (v[0] * up[0] + v[1] * up[1] + v[2] * up[2]) / (z * tan_y);
                if xs.abs() <= 1.0 && ys.abs() <= 1.0 {
                    visible += 1;
                }
            }
            println!(
                "trees visible in frame at aspect {aspect:.2}, 20-200 m: {visible}"
            );
        }
    }
    for row in in_cone.iter().take(10) {
        println!("  len {:.0} cos {:.2} ({},{}) y {}", row.0, row.1, row.2, row.3, row.4);
    }
    // Time a cold plan (the first plan a run makes) and a warm one.
    {
        use matterweave_core::landscape::{plan_flora, LANDSCAPE_FLORA_TIERS};
        let t0 = std::time::Instant::now();
        let plan = plan_flora(SEED, eye, &LANDSCAPE_FLORA_TIERS, 12_000, 700);
        let cold = t0.elapsed().as_secs_f64() * 1000.0;
        let t1 = std::time::Instant::now();
        let _ = plan_flora(SEED, eye, &LANDSCAPE_FLORA_TIERS, 12_000, 700);
        let warm = t1.elapsed().as_secs_f64() * 1000.0;
        println!(
            "cold plan {:.2} ms ({} sites, {} trees, {} dropped), warm {:.2} ms",
            cold,
            plan.sites.len(),
            plan.trees.len(),
            plan.dropped,
            warm
        );
    }
    // Ground cover in the 16 m square at the eye.
    let mut s16 = 0usize;
    let mut s32 = 0usize;
    let c0 = (ex).div_euclid(FLORA_CELL_M);
    let z0 = (ez).div_euclid(FLORA_CELL_M);
    for cz in z0..z0 + 8 {
        for cx in c0..c0 + 8 {
            s16 += landscape::flora_cell(SEED, cx, cz).len();
        }
    }
    for cz in (z0 - 8)..(z0 + 16) {
        for cx in (c0 - 8)..(c0 + 16) {
            s32 += landscape::flora_cell(SEED, cx, cz).len();
        }
    }
    println!("flora sites in 16 m square {s16}, in 32 m square {s32}");
    // Kind census in the 16 m square.
    let mut kinds = std::collections::BTreeMap::new();
    for cz in z0..z0 + 8 {
        for cx in c0..c0 + 8 {
            for site in landscape::flora_cell(SEED, cx, cz).sites() {
                *kinds.entry(site.kind.name()).or_insert(0usize) += 1;
            }
        }
    }
    println!("kinds in 16 m square: {kinds:?}");
    let _ = Biome::ALL;
    let _ = FloraKind::Cactus;

    // Top-down map around the eye: 2 m/px, 512x512 px = 1x1 km, trees in red,
    // the eye in magenta.
    {
        let px = 512i32;
        let step = 2i32;
        let mut img = vec![0u8; (px * px * 3) as usize];
        for py in 0..px {
            for pxi in 0..px {
                let x = ex + (pxi - px / 2) * step;
                let z = ez + (py - px / 2) * step;
                let c = landscape::column(SEED, x, z);
                let base = if c.flooded() {
                    [30u8, 70, 120]
                } else {
                    match c.biome {
                        Biome::Beach => [220, 205, 150],
                        Biome::Desert => [215, 185, 110],
                        Biome::Plains => [110, 155, 80],
                        Biome::Forest => [50, 100, 55],
                        Biome::Swamp => [80, 115, 85],
                        Biome::Hills => [100, 130, 75],
                        Biome::Mountain => [130, 120, 110],
                        Biome::Snow => [235, 240, 245],
                        Biome::Tundra => [165, 175, 160],
                        Biome::Ocean => [30, 70, 120],
                    }
                };
                let s = (base[0] as f32 * 0.6 + c.height as f32 * 2.0).clamp(20.0, 245.0) as u8;
                let s2 = (base[1] as f32 * 0.6 + c.height as f32 * 2.0).clamp(20.0, 245.0) as u8;
                let s3 = (base[2] as f32 * 0.6 + c.height as f32 * 2.0).clamp(20.0, 245.0) as u8;
                let o = ((py * px + pxi) * 3) as usize;
                img[o] = if c.flooded() { base[0] } else { s };
                img[o + 1] = if c.flooded() { base[1] } else { s2 };
                img[o + 2] = if c.flooded() { base[2] } else { s3 };
            }
        }
        for cz in (ez - 500).div_euclid(8)..=(ez + 500).div_euclid(8) {
            for cx in (ex - 500).div_euclid(8)..=(ex + 500).div_euclid(8) {
                if let Some(site) = landscape::tree_cell(SEED, cx, cz) {
                    let pxi = (site.x - ex) / step + px / 2;
                    let py = (site.z - ez) / step + px / 2;
                    if (0..px).contains(&pxi) && (0..py).contains(&py) {
                        let o = ((py * px + pxi) * 3) as usize;
                        img[o] = 255;
                        img[o + 1] = 40;
                        img[o + 2] = 40;
                    }
                }
            }
        }
        let mut f = std::fs::File::create("/tmp/spawn_map.ppm").unwrap();
        use std::io::Write;
        write!(f, "P6\n{px} {px}\n255\n").unwrap();
        f.write_all(&img).unwrap();
        println!("wrote /tmp/spawn_map.ppm");
    }
}
