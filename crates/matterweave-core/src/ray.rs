use crate::World;

/// Query distance is capped to bound CPU work, including rays through empty space.
pub const MAX_RAY_DISTANCE: f32 = 4096.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RayHit {
    pub cell: [i32; 3],
    /// Entry face; zero when the origin is inside a solid cell.
    /// At exact corner/edge crossings the lowest crossed axis determines the normal.
    pub normal: [i32; 3],
    pub distance: f32,
    pub material: u8,
}

impl World {
    /// Grid DDA with Euclidean distances, half-open cells and simultaneous boundary ties.
    /// Zero/nonfinite directions, nonfinite origins and negative/nonfinite ranges miss.
    /// Finite ranges above MAX_RAY_DISTANCE are capped. Touching only an edge is not a hit.
    pub fn raycast(
        &self,
        origin: [f32; 3],
        direction: [f32; 3],
        max_distance: f32,
    ) -> Option<RayHit> {
        if !max_distance.is_finite()
            || max_distance < 0.0
            || !origin.iter().chain(direction.iter()).all(|v| v.is_finite())
        {
            return None;
        }
        let norm = direction
            .iter()
            .map(|&v| f64::from(v).powi(2))
            .sum::<f64>()
            .sqrt();
        if norm == 0.0 {
            return None;
        }
        let direction = direction.map(|v| f64::from(v) / norm);
        let origin = origin.map(f64::from);
        if origin
            .iter()
            .any(|v| v.floor() < f64::from(i32::MIN) || v.floor() > f64::from(i32::MAX))
        {
            return None;
        }
        let mut cell = origin.map(|v| v.floor() as i32);
        let step = direction.map(|v| {
            if v > 0.0 {
                1
            } else if v < 0.0 {
                -1
            } else {
                0
            }
        });
        let delta = direction.map(|v| {
            if v == 0.0 {
                f64::INFINITY
            } else {
                v.abs().recip()
            }
        });
        let mut next: [f64; 3] = std::array::from_fn(|axis| {
            if step[axis] == 0 {
                return f64::INFINITY;
            }
            let boundary = f64::from(cell[axis]) + if step[axis] > 0 { 1.0 } else { 0.0 };
            (boundary - origin[axis]) / direction[axis]
        });
        let limit = f64::from(max_distance.min(MAX_RAY_DISTANCE));
        let mut distance = 0.0;
        let mut normal = [0; 3];
        // A normalized ray crosses at most sqrt(3)*range grid planes; this is conservative.
        for _ in 0..(3 * MAX_RAY_DISTANCE as usize + 4) {
            let material = self.get(cell);
            if material != 0 {
                return Some(RayHit {
                    cell,
                    normal,
                    distance: distance as f32,
                    material,
                });
            }
            distance = next.iter().copied().fold(f64::INFINITY, f64::min);
            if distance > limit {
                return None;
            }
            normal = [0; 3];
            let mut first = true;
            for axis in 0..3 {
                if next[axis] == distance {
                    cell[axis] = cell[axis].checked_add(step[axis])?;
                    next[axis] += delta[axis];
                    if first {
                        normal[axis] = -step[axis];
                        first = false;
                    }
                }
            }
        }
        None
    }
}
