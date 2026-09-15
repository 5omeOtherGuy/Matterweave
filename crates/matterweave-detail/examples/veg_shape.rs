//! Scratch shape viewer (temporary): ASCII side/top projections of landscape
//! flora prototypes, plus cell/triangle counts.
//!
//! Usage: `veg_shape [ids...]` (defaults to a few interesting prototypes).

use matterweave_detail::*;

fn side(volume: &DetailVolume, axis: usize, label: &str) {
    let (min, max) = volume.cell_bounds().unwrap();
    let (u_axis, v_axis) = match axis {
        0 => (0usize, 1usize), // x-y
        1 => (2usize, 1usize), // z-y
        _ => (0usize, 2usize), // x-z top
    };
    let (u0, u1) = (min[u_axis], max[u_axis]);
    let (v0, v1) = (min[v_axis], max[v_axis]);
    let mut grid = vec![vec!['.'; (u1 - u0 + 1) as usize]; (v1 - v0 + 1) as usize];
    for (cell, m) in volume.iter_cells() {
        let u = (cell[u_axis] - u0) as usize;
        let v = (cell[v_axis] - v0) as usize;
        let ch = match m {
            0 => '.',
            x if x == material::GRASS_BLADE => 'g',
            x if x == material::GRASS_TIP => 'G',
            x if x == material::TREE_BARK => 'T',
            x if x == material::TREE_LEAF => 'L',
            x if x == material::TREE_NEEDLE => 'N',
            x if x == material::FLOWER_PETAL_RED => 'r',
            x if x == material::FLOWER_PETAL_WHITE => 'w',
            x if x == material::FLOWER_PETAL_YELLOW => 'y',
            x if x == material::FLOWER_HEART => 'h',
            x if x == material::SHRUB_LEAF => 'l',
            x if x == material::SHRUB_STEM => 's',
            x if x == material::CACTUS_BODY => 'c',
            x if x == material::CACTUS_SPINE => 'p',
            x if x == material::FLORA_REED_STEM => 'e',
            x if x == material::FLORA_REED_LEAF => 'f',
            x if x == material::FLORA_ROSETTE_LEAF => 'o',
            x if x == material::FLORA_ROSETTE_HEART => 'q',
            x if x == material::FLORA_ROSETTE_SPOT => 'z',
            _ => '?',
        };
        grid[v][u] = ch;
    }
    println!("{label} (u=[{u0}..{u1}] v=[{v0}..{v1}])");
    for row in grid.iter().rev() {
        println!("  {}", row.iter().collect::<String>());
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let ids: Vec<&str> = if args.is_empty() {
        vec![
            "grass_tuft_s",
            "grass_tuft_m",
            "grass_tuft_l",
            "flower_red_m",
            "flower_yellow_l",
            "tree_broadleaf_m",
            "tree_conifer_m",
        ]
    } else {
        args.iter().map(|s| s.as_str()).collect()
    };
    for id in ids {
        let volume = landscape_prototype(id).unwrap();
        if !is_single_body(&volume) {
            use std::collections::BTreeSet;
            let cells: BTreeSet<[i32; 3]> = volume.iter_cells().map(|(c, _)| c).collect();
            let mut seen: BTreeSet<[i32; 3]> = BTreeSet::new();
            let mut comps: Vec<Vec<[i32; 3]>> = Vec::new();
            for start in &cells {
                if seen.contains(start) {
                    continue;
                }
                let mut stack = vec![*start];
                seen.insert(*start);
                let mut comp = vec![*start];
                while let Some(c) = stack.pop() {
                    for step in [
                        [1, 0, 0],
                        [-1, 0, 0],
                        [0, 1, 0],
                        [0, -1, 0],
                        [0, 0, 1],
                        [0, 0, -1],
                    ] {
                        let n = [c[0] + step[0], c[1] + step[1], c[2] + step[2]];
                        if cells.contains(&n) && seen.insert(n) {
                            stack.push(n);
                            comp.push(n);
                        }
                    }
                }
                comps.push(comp);
            }
            comps.sort_by_key(|c| std::cmp::Reverse(c.len()));
            println!("\n{id}: NOT one body, {} components", comps.len());
            for comp in comps.iter().take(8) {
                let min = comp.iter().fold([i32::MAX; 3], |a, c| a.map(|v| v.min(c[..].iter().copied().max().unwrap_or(v))));
                let _ = min;
                let xs: Vec<i32> = comp.iter().map(|c| c[0]).collect();
                let ys: Vec<i32> = comp.iter().map(|c| c[1]).collect();
                let zs: Vec<i32> = comp.iter().map(|c| c[2]).collect();
                println!(
                    "  {} cells x {}..{} y {}..{} z {}..{}",
                    comp.len(),
                    xs.iter().min().unwrap(),
                    xs.iter().max().unwrap(),
                    ys.iter().min().unwrap(),
                    ys.iter().max().unwrap(),
                    zs.iter().min().unwrap(),
                    zs.iter().max().unwrap()
                );
            }
        }
        let (min, max) = volume.cell_bounds().unwrap();
        let mesh = volume.mesh_local().unwrap();
        println!(
            "\n=== {id}: cells {} bounds {min:?}..{max:?} scale {} m mesh tris {}",
            volume.occupied_cells(),
            volume.scale().metres(),
            mesh.indices.len() / 3
        );
        // Occupancy of the bounding box: 1.0 is a solid block.
        let dims = [
            (max[0] - min[0] + 1) as f64,
            (max[1] - min[1] + 1) as f64,
            (max[2] - min[2] + 1) as f64,
        ];
        let box_cells = dims[0] * dims[1] * dims[2];
        println!(
            "  bbox fill {:.3}  ({} of {} cells)",
            volume.occupied_cells() as f64 / box_cells,
            volume.occupied_cells(),
            box_cells
        );
        if id.starts_with("grass") || id.starts_with("flower") {
            side(&volume, 0, "side x-y");
            side(&volume, 1, "side z-y");
        } else {
            side(&volume, 0, "front x-y");
            side(&volume, 1, "side z-y");
            side(&volume, 2, "top x-z");
        }
        let blend = volume.coarsen(Lod::Half).unwrap();
        let quarter = volume.coarsen(Lod::Quarter).unwrap();
        println!(
            "  coarsen half cells {} quarter cells {}",
            blend.occupied_cells(),
            quarter.occupied_cells()
        );
    }
}
