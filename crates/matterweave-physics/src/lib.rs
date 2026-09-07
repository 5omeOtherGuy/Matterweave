//! Fixed-step voxel physics. Backend handles never cross the public boundary.
use matterweave_core::{Mesh, Vertex, World};
use rapier3d::{control::KinematicCharacterController, prelude::*};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const FIXED_DT: f32 = 1.0 / 60.0;
pub const MAX_BODIES: usize = 64;
const EYE_OFFSET: f32 = 0.65;
const HALF_SEGMENT: f32 = 0.55;
const RADIUS: f32 = 0.30;
const VOXEL_SIZE: f32 = 0.5;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct BodySnapshot {
    pub position: [f32; 3],
    /// Unit quaternion, x/y/z/w.
    pub rotation: [f32; 4],
    pub velocity: [f32; 3],
    pub angular_velocity: [f32; 3],
    /// Solid rectangular voxel volume. Fracture preserves each half-meter voxel.
    pub dimensions: [u8; 3],
    pub material: u8,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PhysicsSnapshot {
    pub version: u32,
    pub eye: [f32; 3],
    pub bodies: Vec<BodySnapshot>,
}
struct VoxelBody {
    handle: RigidBodyHandle,
    dimensions: [u8; 3],
    material: u8,
    previous: Pose,
}
struct Held {
    handle: RigidBodyHandle,
    joint: ImpulseJointHandle,
}

pub struct Physics {
    pipeline: PhysicsPipeline,
    islands: IslandManager,
    broad: BroadPhaseBvh,
    narrow: NarrowPhase,
    bodies: RigidBodySet,
    colliders: ColliderSet,
    joints: ImpulseJointSet,
    multibody: MultibodyJointSet,
    ccd: CCDSolver,
    terrain: BTreeMap<[i32; 3], (u64, ColliderHandle)>,
    objects: Vec<VoxelBody>,
    character: RigidBodyHandle,
    character_collider: ColliderHandle,
    anchor: RigidBodyHandle,
    held: Option<Held>,
    center: Vector,
    vertical_velocity: f32,
    grounded: bool,
    accumulator: f32,
    mesh_revision: u64,
    flying: bool,
    jump_pending: bool,
}
impl Physics {
    pub fn new(world: &World) -> Self {
        let mut bodies = RigidBodySet::new();
        let center = Vector::new(12.0, 18.0 - EYE_OFFSET, 28.0);
        let character =
            bodies.insert(RigidBodyBuilder::kinematic_position_based().translation(center));
        let mut colliders = ColliderSet::new();
        let character_collider = colliders.insert_with_parent(
            ColliderBuilder::capsule_y(HALF_SEGMENT, RADIUS).friction(0.0),
            character,
            &mut bodies,
        );
        let anchor = bodies.insert(RigidBodyBuilder::kinematic_position_based());
        let mut result = Self {
            pipeline: PhysicsPipeline::new(),
            islands: IslandManager::new(),
            broad: BroadPhaseBvh::new(),
            narrow: NarrowPhase::new(),
            bodies,
            colliders,
            joints: ImpulseJointSet::new(),
            multibody: MultibodyJointSet::new(),
            ccd: CCDSolver::new(),
            terrain: BTreeMap::new(),
            objects: Vec::new(),
            character,
            character_collider,
            anchor,
            held: None,
            center,
            vertical_velocity: 0.0,
            grounded: false,
            accumulator: 0.0,
            mesh_revision: 0,
            flying: false,
            jump_pending: false,
        };
        result.sync_world(world);
        result
    }
    /// Publishes exact solid-voxel collision synchronously. No visual mesh/LOD is consulted.
    pub fn sync_world(&mut self, world: &World) {
        let keys = world.chunk_keys();
        let removed: Vec<_> = self
            .terrain
            .keys()
            .filter(|key| !keys.contains(key))
            .copied()
            .collect();
        for key in removed {
            let (_, handle) = self.terrain.remove(&key).unwrap();
            self.colliders
                .remove(handle, &mut self.islands, &mut self.bodies, true);
        }
        for key in keys {
            let revision = world.chunk_revision(key).unwrap();
            if self
                .terrain
                .get(&key)
                .is_some_and(|entry| entry.0 == revision)
            {
                continue;
            }
            if let Some((_, handle)) = self.terrain.remove(&key) {
                self.colliders
                    .remove(handle, &mut self.islands, &mut self.bodies, true);
            }
            let shapes = solid_boxes(world, key);
            if !shapes.is_empty() {
                let handle = self
                    .colliders
                    .insert(ColliderBuilder::compound(shapes).friction(0.8));
                self.terrain.insert(key, (revision, handle));
            }
        }
    }
    pub fn character_eye(&self) -> [f32; 3] {
        (self.center + Vector::Y * EYE_OFFSET).to_array()
    }
    pub fn grounded(&self) -> bool {
        self.grounded
    }
    pub fn held(&self) -> bool {
        self.held.is_some()
    }
    pub fn body_count(&self) -> usize {
        self.objects.len()
    }
    /// Rejects nonfinite/out-of-range positions and existing solid colliders.
    /// Call sync_world before teleporting into newly loaded terrain.
    pub fn teleport(&mut self, eye: [f32; 3]) -> bool {
        if !valid_vector(eye, 16384.0) {
            return false;
        }
        let center = Vector::from_array(eye) - Vector::Y * EYE_OFFSET;
        let shape = SharedShape::capsule_y(HALF_SEGMENT, RADIUS);
        let pos = Pose::from_translation(center);
        // Direct narrow-phase checks include newly inserted chunks before the next step.
        if self.colliders.iter().any(|(handle, collider)| {
            handle != self.character_collider
                && rapier3d::parry::query::intersection_test(
                    &pos,
                    shape.as_ref(),
                    collider.position(),
                    collider.shape(),
                )
                .unwrap_or(true)
        }) {
            return false;
        }
        self.release();
        self.center = center;
        self.bodies[self.character].set_translation(center, true);
        self.bodies[self.character].set_next_kinematic_translation(center);
        self.vertical_velocity = 0.0;
        self.grounded = false;
        self.accumulator = 0.0;
        self.jump_pending = false;
        true
    }
    /// Sets the free-flight camera pose without collision rejection or resetting fixed time.
    /// Use before step_objects; regular teleport is the validated return-to-walking path.
    pub fn set_flying_eye(&mut self, eye: [f32; 3]) -> bool {
        if !valid_vector(eye, 16384.0) {
            return false;
        }
        self.center = Vector::from_array(eye) - Vector::Y * EYE_OFFSET;
        self.bodies[self.character].set_translation(self.center, true);
        self.bodies[self.character].set_next_kinematic_translation(self.center);
        self.vertical_velocity = 0.0;
        self.grounded = false;
        self.jump_pending = false;
        true
    }
    pub fn intersects_character_cell(&self, cell: [i32; 3]) -> bool {
        let min = self.center - Vector::new(RADIUS, HALF_SEGMENT + RADIUS, RADIUS);
        let max = self.center + Vector::new(RADIUS, HALF_SEGMENT + RADIUS, RADIUS);
        (0..3).all(|a| (cell[a] as f32) < max[a] && (cell[a] as f32 + 1.0) > min[a])
    }
    /// Input is horizontal meters/second. At most six fixed steps are simulated per call.
    /// Invalid dt is ignored; stalls discard excess time instead of accelerating physics.
    pub fn step(&mut self, dt: f32, horizontal_velocity: [f32; 3], jump: bool) -> usize {
        if !dt.is_finite() || dt <= 0.0 {
            return 0;
        }
        let mut velocity = if valid_vector(horizontal_velocity, 128.0) {
            Vector::from_array(horizontal_velocity)
        } else {
            Vector::ZERO
        };
        velocity.y = 0.0;
        velocity = velocity.clamp_length_max(12.0);
        self.accumulator = (self.accumulator + dt.min(0.1)).min(0.1);
        let mut count = 0;
        self.jump_pending |= jump;
        while self.accumulator + 1e-7 >= FIXED_DT && count < 6 {
            self.accumulator = (self.accumulator - FIXED_DT).max(0.0);
            let jump = std::mem::take(&mut self.jump_pending);
            self.fixed_step(velocity, jump);
            count += 1;
        }
        count
    }
    /// Advances dynamic bodies while leaving the character pose under the caller's control.
    pub fn step_objects(&mut self, dt: f32) -> usize {
        self.flying = true;
        self.colliders[self.character_collider].set_enabled(false);
        let count = self.step(dt, [0.0; 3], false);
        self.flying = false;
        self.colliders[self.character_collider].set_enabled(true);
        count
    }
    fn fixed_step(&mut self, horizontal: Vector, jump: bool) {
        if self
            .held
            .as_ref()
            .is_some_and(|held| self.bodies[held.handle].translation().distance(self.center) > 12.0)
        {
            self.release();
        }
        for object in &mut self.objects {
            let body = &mut self.bodies[object.handle];
            object.previous = *body.position();
            let delta = body.translation() - self.center;
            // Resident terrain extends at least 48 m horizontally. Preserve distant bodies
            // before their support is evicted; they resume when the player returns.
            if body.translation().y < -32.0 {
                // Retain fallen voxel data without unbounded acceleration below the world.
                let mut position = body.translation();
                position.y = -32.0;
                body.set_translation(position, true);
                body.set_linvel(Vector::ZERO, true);
                body.set_angvel(Vector::ZERO, true);
            }
            body.set_enabled(
                delta.x.abs() < 40.0 && delta.z.abs() < 40.0 && body.translation().y > -32.0,
            );
        }
        self.pipeline.step(
            Vector::new(0.0, -20.0, 0.0),
            &IntegrationParameters {
                dt: FIXED_DT,
                ..Default::default()
            },
            &mut self.islands,
            &mut self.broad,
            &mut self.narrow,
            &mut self.bodies,
            &mut self.colliders,
            &mut self.joints,
            &mut self.multibody,
            &mut self.ccd,
            &(),
            &(),
        );
        // Keep engine-produced states inside the persistence contract even after a
        // large spring/impact impulse. This slice has no high-speed projectile workload.
        for object in &self.objects {
            let body = &mut self.bodies[object.handle];
            let velocity = body.linvel();
            let angular = body.angvel();
            if velocity.length_squared() > 128.0 * 128.0 {
                body.set_linvel(velocity.clamp_length_max(128.0), true);
            }
            if angular.length_squared() > 64.0 * 64.0 {
                body.set_angvel(angular.clamp_length_max(64.0), true);
            }
        }
        if self.flying {
            self.mesh_revision = self.mesh_revision.wrapping_add(1);
            return;
        }
        let controller = KinematicCharacterController::default();
        let shape = SharedShape::capsule_y(HALF_SEGMENT, RADIUS);
        if jump && self.grounded {
            self.vertical_velocity = 8.0;
        }
        self.vertical_velocity = (self.vertical_velocity - 20.0 * FIXED_DT).max(-35.0);
        let desired = (horizontal + Vector::Y * self.vertical_velocity) * FIXED_DT;
        let mut collisions = Vec::new();
        let filter = QueryFilter::default().exclude_rigid_body(self.character);
        let queries = self.broad.as_query_pipeline(
            self.narrow.query_dispatcher(),
            &self.bodies,
            &self.colliders,
            filter,
        );
        let movement = controller.move_shape(
            FIXED_DT,
            &queries,
            shape.as_ref(),
            &Pose::from_translation(self.center),
            desired,
            |c| collisions.push(c),
        );
        self.center += movement.translation;
        self.center.x = self.center.x.clamp(-255.5, 255.5);
        self.center.z = self.center.z.clamp(-255.5, 255.5);
        self.grounded = movement.grounded;
        if (movement.grounded && self.vertical_velocity < 0.0)
            || (desired.y > 0.0 && movement.translation.y < desired.y * 0.5)
        {
            self.vertical_velocity = 0.0;
        }
        let mut queries = self.broad.as_query_pipeline_mut(
            self.narrow.query_dispatcher(),
            &mut self.bodies,
            &mut self.colliders,
            filter,
        );
        controller.solve_character_collision_impulses(
            FIXED_DT,
            &mut queries,
            shape.as_ref(),
            75.0,
            &collisions,
        );
        self.bodies[self.character].set_next_kinematic_translation(self.center);
        self.mesh_revision = self.mesh_revision.wrapping_add(1);
    }
    /// Places a clearly visible stack and loose crates on the actual generated terrain.
    pub fn spawn_demo(&mut self, world: &World) {
        if !self.objects.is_empty() {
            return;
        }
        for (x, z, levels) in [
            (9, 17, 3),
            (10, 17, 3),
            (11, 17, 3),
            (14, 20, 1),
            (15, 18, 1),
        ] {
            let ground = (-16..48)
                .rev()
                .find(|&y| world.get([x, y, z]) != 0)
                .map_or(1.0, |y| y as f32 + 1.0);
            for level in 0..levels {
                self.insert_body(&BodySnapshot {
                    position: [
                        x as f32 + 0.5,
                        ground + 0.52 + level as f32 * 1.02,
                        z as f32 + 0.5,
                    ],
                    rotation: [0.0, 0.0, 0.0, 1.0],
                    velocity: [0.0; 3],
                    angular_velocity: [0.0; 3],
                    dimensions: [2; 3],
                    material: if level % 2 == 0 { 8 } else { 7 },
                });
            }
        }
    }
    fn insert_body(&mut self, snapshot: &BodySnapshot) {
        let pose = Pose::from_parts(
            Vector::from_array(snapshot.position),
            Rotation::from_array(snapshot.rotation).normalize(),
        );
        let body = RigidBodyBuilder::dynamic()
            .pose(pose)
            .linvel(Vector::from_array(snapshot.velocity))
            .angvel(Vector::from_array(snapshot.angular_velocity))
            .linear_damping(0.15)
            .angular_damping(0.3)
            .ccd_enabled(true);
        let handle = self.bodies.insert(body);
        let size = snapshot.dimensions.map(|n| n as f32 * VOXEL_SIZE * 0.5);
        self.colliders.insert_with_parent(
            ColliderBuilder::cuboid(size[0], size[1], size[2])
                .density(3.0)
                .friction(0.7)
                .restitution(0.1),
            handle,
            &mut self.bodies,
        );
        self.objects.push(VoxelBody {
            handle,
            dimensions: snapshot.dimensions,
            material: snapshot.material,
            previous: pose,
        });
        self.mesh_revision = self.mesh_revision.wrapping_add(1);
    }
    /// Whether a dynamic voxel object is aimed at before any terrain obstruction.
    pub fn has_target(
        &self,
        world: &World,
        origin: [f32; 3],
        direction: [f32; 3],
        range: f32,
    ) -> bool {
        self.pick(world, origin, direction, range).is_some()
    }
    fn pick(
        &self,
        world: &World,
        origin: [f32; 3],
        direction: [f32; 3],
        range: f32,
    ) -> Option<usize> {
        let direction = normalized(direction)?;
        if !valid_vector(origin, 16384.0) || !range.is_finite() || range <= 0.0 {
            return None;
        }
        let limit = world
            .raycast(origin, direction.to_array(), range.min(12.0))
            .map_or(range.min(12.0), |hit| hit.distance);
        let ray = Ray::new(Vector::from_array(origin), direction);
        self.objects
            .iter()
            .enumerate()
            .filter_map(|(i, object)| {
                let body = &self.bodies[object.handle];
                let collider = &self.colliders[body.colliders()[0]];
                collider
                    .shape()
                    .cast_ray(body.position(), &ray, limit, true)
                    .map(|distance| (i, distance))
            })
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(i, _)| i)
    }
    pub fn grab(
        &mut self,
        world: &World,
        origin: [f32; 3],
        direction: [f32; 3],
        range: f32,
    ) -> bool {
        if self.held.is_some() {
            self.release();
            return true;
        }
        let Some(index) = self.pick(world, origin, direction, range) else {
            return false;
        };
        let handle = self.objects[index].handle;
        let position = self.bodies[handle].translation();
        self.bodies[self.anchor].set_translation(position, true);
        let joint = self.joints.insert(
            self.anchor,
            handle,
            SpringJointBuilder::new(0.0, 140.0, 28.0).contacts_enabled(false),
            true,
        );
        self.held = Some(Held { handle, joint });
        self.update_grab(origin, direction);
        true
    }
    pub fn update_grab(&mut self, origin: [f32; 3], direction: [f32; 3]) {
        if self.held.is_none() || !valid_vector(origin, 16384.0) {
            return;
        }
        if let Some(direction) = normalized(direction) {
            self.bodies[self.anchor]
                .set_next_kinematic_translation(Vector::from_array(origin) + direction * 3.0);
        }
    }
    pub fn release(&mut self) {
        if let Some(held) = self.held.take() {
            self.joints.remove(held.joint, true);
        }
    }
    pub fn throw(&mut self, direction: [f32; 3]) -> bool {
        let Some(direction) = normalized(direction) else {
            return false;
        };
        let Some(held) = self.held.take() else {
            return false;
        };
        self.joints.remove(held.joint, true);
        let body = &mut self.bodies[held.handle];
        body.apply_impulse(direction * body.mass() * 13.0, true);
        body.set_linvel(body.linvel().clamp_length_max(128.0), true);
        true
    }
    /// Splits a solid voxel volume into physical half-meter voxels, preserving mass and
    /// each voxel's center/rotation. Refuses the operation if the body cap would be exceeded.
    pub fn break_body(
        &mut self,
        world: &World,
        origin: [f32; 3],
        direction: [f32; 3],
        range: f32,
    ) -> bool {
        let Some(index) = self.pick(world, origin, direction, range) else {
            return false;
        };
        let object = &self.objects[index];
        let count = object
            .dimensions
            .iter()
            .map(|&n| n as usize)
            .product::<usize>();
        if count <= 1 || self.objects.len() - 1 + count > MAX_BODIES {
            return false;
        }
        let dimensions = object.dimensions;
        let material = object.material;
        let body = &self.bodies[object.handle];
        let pose = *body.position();
        let velocity = body.linvel();
        let angular = body.angvel();
        if self
            .held
            .as_ref()
            .is_some_and(|held| held.handle == object.handle)
        {
            self.release();
        }
        let removed = self.objects.remove(index);
        self.bodies.remove(
            removed.handle,
            &mut self.islands,
            &mut self.colliders,
            &mut self.joints,
            &mut self.multibody,
            true,
        );
        for x in 0..dimensions[0] {
            for y in 0..dimensions[1] {
                for z in 0..dimensions[2] {
                    let local = Vector::new(
                        x as f32 + 0.5 - dimensions[0] as f32 * 0.5,
                        y as f32 + 0.5 - dimensions[1] as f32 * 0.5,
                        z as f32 + 0.5 - dimensions[2] as f32 * 0.5,
                    ) * VOXEL_SIZE;
                    let offset = pose.rotation * local;
                    self.insert_body(&BodySnapshot {
                        position: (pose.translation + offset).to_array(),
                        rotation: pose.rotation.to_array(),
                        velocity: (velocity + angular.cross(offset) + offset * 1.5)
                            .clamp_length_max(128.0)
                            .to_array(),
                        angular_velocity: angular.to_array(),
                        dimensions: [1; 3],
                        material,
                    });
                }
            }
        }
        true
    }
    pub fn snapshot(&self) -> PhysicsSnapshot {
        PhysicsSnapshot {
            version: 1,
            eye: self.character_eye(),
            bodies: self
                .objects
                .iter()
                .map(|object| {
                    let body = &self.bodies[object.handle];
                    BodySnapshot {
                        position: body.translation().to_array(),
                        rotation: body.rotation().to_array(),
                        velocity: body.linvel().to_array(),
                        angular_velocity: body.angvel().to_array(),
                        dimensions: object.dimensions,
                        material: object.material,
                    }
                })
                .collect(),
        }
    }
    /// Validates everything before changing any live body. Held constraints are transient.
    pub fn restore(&mut self, snapshot: &PhysicsSnapshot) -> Result<(), String> {
        if snapshot.version != 1
            || !valid_vector(snapshot.eye, 16384.0)
            || snapshot.bodies.len() > MAX_BODIES
        {
            return Err("invalid physics snapshot header".into());
        }
        for body in &snapshot.bodies {
            let norm: f32 = body.rotation.iter().map(|v| v * v).sum();
            if !valid_vector(body.position, 16384.0)
                || !valid_vector(body.velocity, 128.0)
                || !valid_vector(body.angular_velocity, 128.0)
                || !body.rotation.iter().all(|v| v.is_finite())
                || (norm - 1.0).abs() > 0.01
                || body.dimensions.iter().any(|&n| !(1..=2).contains(&n))
                || body.material == 0
            {
                return Err("invalid voxel body snapshot".into());
            }
        }
        self.release();
        for object in self.objects.drain(..) {
            self.bodies.remove(
                object.handle,
                &mut self.islands,
                &mut self.colliders,
                &mut self.joints,
                &mut self.multibody,
                true,
            );
        }
        for body in &snapshot.bodies {
            self.insert_body(body);
        }
        // Restoring the authoritative snapshot may place the character touching a body;
        // retain its saved pose instead of applying teleport's interactive rejection.
        self.center = Vector::from_array(snapshot.eye) - Vector::Y * EYE_OFFSET;
        self.bodies[self.character].set_translation(self.center, true);
        self.bodies[self.character].set_next_kinematic_translation(self.center);
        self.vertical_velocity = 0.0;
        self.grounded = false;
        self.accumulator = 0.0;
        self.jump_pending = false;
        Ok(())
    }
    /// Interpolated, world-space mesh for bounded small dynamic objects.
    pub fn dynamic_mesh(&self) -> Mesh {
        let mut mesh = Mesh {
            revision: self.mesh_revision,
            ..Mesh::default()
        };
        let alpha = (self.accumulator / FIXED_DT).clamp(0.0, 1.0);
        for object in &self.objects {
            let current = self.bodies[object.handle].position();
            let pose = Pose::from_parts(
                object.previous.translation.lerp(current.translation, alpha),
                object.previous.rotation.slerp(current.rotation, alpha),
            );
            append_box(&mut mesh, pose, object.dimensions, object.material);
        }
        mesh
    }
}
fn valid_vector(v: [f32; 3], bound: f32) -> bool {
    v.iter().all(|x| x.is_finite() && x.abs() <= bound)
}
fn normalized(v: [f32; 3]) -> Option<Vector> {
    if !valid_vector(v, 1e8) {
        return None;
    }
    Vector::from_array(v).try_normalize()
}
fn solid_boxes(world: &World, key: [i32; 3]) -> Vec<(Pose, SharedShape)> {
    let mut solid = [false; 4096];
    let index = |x: usize, y: usize, z: usize| x + y * 16 + z * 256;
    for z in 0..16 {
        for y in 0..16 {
            for x in 0..16 {
                solid[index(x, y, z)] = world.get([
                    key[0] * 16 + x as i32,
                    key[1] * 16 + y as i32,
                    key[2] * 16 + z as i32,
                ]) != 0;
            }
        }
    }
    let mut shapes = Vec::new();
    for z in 0..16 {
        for y in 0..16 {
            for x in 0..16 {
                if !solid[index(x, y, z)] {
                    continue;
                }
                let mut end_x = x + 1;
                while end_x < 16 && solid[index(end_x, y, z)] {
                    end_x += 1;
                }
                let mut end_z = z + 1;
                while end_z < 16 && (x..end_x).all(|xx| solid[index(xx, y, end_z)]) {
                    end_z += 1;
                }
                let mut end_y = y + 1;
                while end_y < 16
                    && (z..end_z).all(|zz| (x..end_x).all(|xx| solid[index(xx, end_y, zz)]))
                {
                    end_y += 1;
                }
                for zz in z..end_z {
                    for yy in y..end_y {
                        for xx in x..end_x {
                            solid[index(xx, yy, zz)] = false;
                        }
                    }
                }
                let half =
                    Vector::new((end_x - x) as f32, (end_y - y) as f32, (end_z - z) as f32) * 0.5;
                let center = Vector::new(
                    (key[0] * 16 + x as i32) as f32,
                    (key[1] * 16 + y as i32) as f32,
                    (key[2] * 16 + z as i32) as f32,
                ) + half;
                shapes.push((
                    Pose::from_translation(center),
                    SharedShape::cuboid(half.x, half.y, half.z),
                ));
            }
        }
    }
    shapes
}
fn append_box(mesh: &mut Mesh, pose: Pose, dimensions: [u8; 3], material: u8) {
    let half = Vector::from_array(dimensions.map(|n| n as f32 * VOXEL_SIZE * 0.5));
    let color = if material == 7 {
        [0.25, 0.8, 0.8]
    } else {
        [0.9, 0.43, 0.15]
    };
    for (normal, u, v) in [
        (Vector::X, Vector::Y, Vector::Z),
        (-Vector::X, Vector::Y, -Vector::Z),
        (Vector::Y, Vector::Z, Vector::X),
        (-Vector::Y, Vector::X, Vector::Z),
        (Vector::Z, Vector::X, Vector::Y),
        (-Vector::Z, -Vector::X, Vector::Y),
    ] {
        let base = mesh.vertices.len() as u32;
        for (du, dv) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
            let local = (normal + u * du + v * dv) * half;
            mesh.vertices.push(Vertex {
                position: (pose.translation + pose.rotation * local).to_array(),
                normal: (pose.rotation * normal).to_array(),
                color,
            });
        }
        mesh.indices
            .extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }
}
