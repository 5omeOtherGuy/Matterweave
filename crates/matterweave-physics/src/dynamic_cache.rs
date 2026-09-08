//! CPU-only dynamic geometry cache. GPU upload ownership stays with the caller.
use super::{append_box, Physics, FIXED_DT};
use matterweave_core::{Mesh, Vertex};
use rapier3d::prelude::Pose;
use std::mem::{size_of, swap};

#[derive(PartialEq)]
struct RenderBody {
    pose: Pose,
    dimensions: [u8; 3],
    material: u8,
}

/// Retains geometry while its complete render inputs are unchanged.
///
/// Backend identities, sleeping flags and fixed-step revisions are not sufficient
/// invalidation keys: geometry can change during sub-step interpolation, and
/// backend handles can change while visible data remains identical.
/// This cache compares ordered poses, dimensions and materials exactly. It uses
/// Rapier/glam interpolation and the existing box mesher, not a second mesher.
#[derive(Default)]
pub struct DynamicMeshCache {
    previous: Vec<RenderBody>,
    next: Vec<RenderBody>,
    mesh: Mesh,
    initialized: bool,
}

impl DynamicMeshCache {
    /// Returns true only when CPU geometry was rebuilt (including first use).
    /// Does not step or mutate physics. Render-input scratch capacity is reused.
    ///
    /// A caller must still upload after renderer recreation, and retain an
    /// upload-dirty flag until an upload succeeds. A false result says nothing
    /// about GPU residency. `Mesh::revision` remains informational, as on the
    /// uncached dynamic path; it cannot identify fractional interpolation changes.
    pub fn update(&mut self, physics: &Physics) -> bool {
        let alpha = (physics.accumulator / FIXED_DT).clamp(0., 1.);
        self.next.clear();
        self.next.extend(physics.objects.iter().map(|object| {
            let current = physics.bodies[object.handle].position();
            // Interpolating identical quaternions can introduce rounding noise.
            // The physical pose is already the exact stationary endpoint.
            let pose = if object.previous == *current {
                *current
            } else {
                Pose::from_parts(
                    object.previous.translation.lerp(current.translation, alpha),
                    object.previous.rotation.slerp(current.rotation, alpha),
                )
            };
            RenderBody {
                pose,
                dimensions: object.dimensions,
                material: object.material,
            }
        }));
        if self.initialized && self.previous == self.next {
            return false;
        }
        swap(&mut self.previous, &mut self.next);
        self.mesh.vertices.clear();
        self.mesh.indices.clear();
        self.mesh.revision = physics.mesh_revision;
        for body in &self.previous {
            append_box(&mut self.mesh, body.pose, body.dimensions, body.material);
        }
        self.initialized = true;
        true
    }

    pub fn mesh(&self) -> &Mesh {
        &self.mesh
    }

    /// Retained vector payload capacities only, excluding allocator metadata,
    /// the owning struct and GPU memory. Physics bounds object count at 64.
    pub fn retained_bytes(&self) -> usize {
        self.mesh.vertices.capacity() * size_of::<Vertex>()
            + self.mesh.indices.capacity() * size_of::<u32>()
            + (self.previous.capacity() + self.next.capacity()) * size_of::<RenderBody>()
    }
}
