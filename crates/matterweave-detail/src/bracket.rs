//! Shelf fungus attached to its original woody stump. Source geometry includes
//! three asymmetric brackets, rounded growth bands and recessed underside pores.
use crate::{material, DetailVolume, Result, Scale, SCALE_FINE_M};
pub fn bracket_fungus(id: &str) -> Result<DetailVolume> {
    let mut v = DetailVolume::new(id, Scale::new(SCALE_FINE_M)?);
    for y in 0..=34 {
        for x in -2i32..=2 {
            for z in -2i32..=2 {
                if x * x + z * z <= 5 {
                    v.set([x, y, z], material::FLORA_FUNNEL_STIPE)?;
                }
            }
        }
    }
    for (index, (level, radius)) in [(8, 12i32), (18, 10), (27, 8)].into_iter().enumerate() {
        for x in 0..=radius {
            for z in -radius..=radius {
                let zz = z + if index == 1 { 1 } else { 0 };
                let r2 = x * x + z * z;
                if r2 > radius * radius {
                    continue;
                }
                let crown = 3 - r2 * 3 / (radius * radius + 1);
                for dy in 0..=crown {
                    // Recessed pore openings occupy only the underside layer;
                    // cap flesh above remains a continuous solid shelf.
                    if dy == 0 && x > 3 && x % 3 == 1 && z.rem_euclid(3) == 1 {
                        continue;
                    }
                    let m = if dy == 0 {
                        material::MUSHROOM_GILL
                    } else if r2 > (radius - 1) * (radius - 1) {
                        material::MUSHROOM_RIM
                    } else if ((r2 / 9) + index as i32) % 3 == 0 {
                        material::FLORA_FUNNEL_RIM
                    } else {
                        material::MUSHROOM_CAP
                    };
                    v.set([x + 1, level + dy, zz], m)?;
                }
            }
        }
    }
    Ok(v)
}
