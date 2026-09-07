//! Conservative CPU rejection against the Vulkan clip volume: -w <= x,y <= w,
//! 0 <= z <= w. Matrices are column-major, matching the shader and glam arrays.
pub(crate) struct Frustum {
    planes: [[f64; 4]; 6],
}

impl Frustum {
    pub(crate) fn new(matrix: [[f32; 4]; 4]) -> Self {
        let row = |r| matrix.map(|column| f64::from(column[r]));
        let w = row(3);
        let x = row(0);
        let y = row(1);
        let z = row(2);
        Self {
            planes: [
                std::array::from_fn(|i| w[i] + x[i]),
                std::array::from_fn(|i| w[i] - x[i]),
                std::array::from_fn(|i| w[i] + y[i]),
                std::array::from_fn(|i| w[i] - y[i]),
                z,
                std::array::from_fn(|i| w[i] - z[i]),
            ],
        }
    }

    pub(crate) fn intersects(&self, bounds: [[f32; 3]; 2]) -> bool {
        self.planes.iter().all(|p| {
            // Positive support vertex maximizes signed distance to this plane.
            // Keep uncertain/non-finite cases instead of incorrectly hiding geometry.
            let terms: [f64; 3] = std::array::from_fn(|axis| {
                p[axis] * f64::from(bounds[usize::from(p[axis] >= 0.0)][axis])
            });
            let distance = terms.iter().sum::<f64>() + p[3];
            let tolerance =
                1.0e-5 * (terms.iter().map(|v| v.abs()).sum::<f64>() + p[3].abs() + 1.0);
            !distance.is_finite() || distance >= -tolerance
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const IDENTITY: [[f32; 4]; 4] = [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ];

    #[test]
    fn vulkan_near_plane_is_zero_and_intersections_are_kept() {
        let f = Frustum::new(IDENTITY);
        assert!(!f.intersects([[-0.2, -0.2, -0.4], [0.2, 0.2, -0.1]]));
        assert!(f.intersects([[-0.2, -0.2, -0.1], [0.2, 0.2, 0.1]]));
        assert!(f.intersects([[-0.2, -0.2, 1.0], [0.2, 0.2, 1.2]]));
        assert!(!f.intersects([[-0.2, -0.2, 1.1], [0.2, 0.2, 1.2]]));
        assert!(!f.intersects([[1.1, -0.2, 0.1], [1.2, 0.2, 0.2]]));
    }

    #[test]
    fn column_major_translation_moves_the_volume() {
        let mut m = IDENTITY;
        m[3][0] = -10.0;
        let f = Frustum::new(m);
        assert!(f.intersects([[9.5, -0.5, 0.1], [10.5, 0.5, 0.9]]));
        assert!(!f.intersects([[-0.5, -0.5, 0.1], [0.5, 0.5, 0.9]]));
    }

    #[test]
    fn perspective_keeps_camera_inside_and_rejects_behind() {
        // RH perspective, near 1, far 10, 90 degree vertical field of view.
        let f = Frustum::new([
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, -10.0 / 9.0, -1.0],
            [0.0, 0.0, -10.0 / 9.0, 0.0],
        ]);
        assert!(f.intersects([[-16.0; 3], [16.0; 3]]));
        assert!(f.intersects([[-0.5, -0.5, -2.0], [0.5, 0.5, -1.0]]));
        assert!(!f.intersects([[-0.5, -0.5, 1.0], [0.5, 0.5, 2.0]]));
        assert!(!f.intersects([[5.0, -0.5, -2.0], [6.0, 0.5, -1.0]]));
    }

    #[test]
    fn invalid_matrix_does_not_cull() {
        assert!(Frustum::new([[f32::NAN; 4]; 4]).intersects([[0.0; 3], [1.0; 3]]));
    }
}
