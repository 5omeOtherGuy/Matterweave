//! Landscape flora gallery: builds every landscape prototype, prints real
//! meshed numbers, and fails nonzero over cap.
//!
//! Usage: `cargo run -p matterweave-detail --example landscape_flora_gallery`
//! Prints one `id, cells, vertices, triangles, class` line per prototype plus
//! budget verdicts. The later slicing worker and the lead use this output as
//! the geometry budget input, so the numbers are real meshed numbers through
//! the existing detail mesh path ([`DetailScene::prototype_mesh`]), not
//! estimates.
//!
//! Exit status is 1 when any cell or triangle cap is exceeded.

use matterweave_detail::*;
use std::process::ExitCode;

fn cell_cap(id: &str) -> usize {
    if id.starts_with("grass_tuft") {
        GRASS_CELL_CAP
    } else if id.starts_with("flower_") {
        FLOWER_CELL_CAP
    } else if id == "fern" {
        FERN_CELL_CAP
    } else if id == "shrub" {
        SHRUB_CELL_CAP
    } else if id == "cactus" {
        CACTUS_CELL_CAP
    } else {
        TREE_CELL_CAP
    }
}

fn tri_cap(id: &str) -> usize {
    if id.starts_with("grass_tuft") {
        GRASS_TRI_CAP
    } else if id.starts_with("flower_") {
        FLOWER_TRI_CAP
    } else if id == "fern" {
        FERN_TRI_CAP
    } else if id == "shrub" {
        SHRUB_TRI_CAP
    } else if id == "cactus" {
        CACTUS_TRI_CAP
    } else {
        TREE_TRI_CAP
    }
}

fn main() -> ExitCode {
    let mut scene = DetailScene::new();
    for id in LANDSCAPE_FLORA_SPECIES {
        match landscape_prototype(id) {
            Ok(volume) => {
                if let Err(error) = scene.add_prototype(volume) {
                    eprintln!("FAIL {id}: {error}");
                    return ExitCode::from(1);
                }
            }
            Err(error) => {
                eprintln!("FAIL {id}: {error}");
                return ExitCode::from(1);
            }
        }
    }
    println!("id, cells, vertices, triangles, class");
    let mut over = false;
    for id in LANDSCAPE_FLORA_SPECIES {
        let volume = scene.prototype(id).expect("added prototype");
        let cells = volume.occupied_cells();
        let mesh = scene.prototype_mesh(id, Lod::Source).expect("mesh builds");
        let vertices = mesh.vertices.len();
        let triangles = mesh.indices.len() / 3;
        let class = landscape_flora_class(id);
        println!("{id}, {cells}, {vertices}, {triangles}, {class}");
        if cells > cell_cap(id) {
            eprintln!("OVER CELL CAP {id}: {cells} > {}", cell_cap(id));
            over = true;
        }
        if triangles > tri_cap(id) {
            eprintln!("OVER TRI CAP {id}: {triangles} > {}", tri_cap(id));
            over = true;
        }
    }
    if over {
        return ExitCode::from(1);
    }
    println!(
        "all {} prototypes within cell and triangle caps",
        LANDSCAPE_FLORA_SPECIES.len()
    );
    ExitCode::SUCCESS
}
