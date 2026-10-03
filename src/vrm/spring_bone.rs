pub(crate) mod initialize;
pub mod registry;
mod update;

use crate::prelude::ColliderShape;
use crate::vrm::spring_bone::initialize::SpringBoneInitializePlugin;
use crate::vrm::spring_bone::registry::SpringBoneRegistryPlugin;
use crate::vrm::spring_bone::update::SpringBoneUpdatePlugin;
use bevy::app::App;
use bevy::math::{Mat4, Quat, Vec3};
use bevy::prelude::*;

/// The component that holds the spring bone state of each Joint
///
/// Implement the method described in the  [Official documentation](https://github.com/vrm-c/vrm-specification/blob/master/specification/VRMC_springBone-1.0/README.ja.md#%E5%88%9D%E6%9C%9F%E5%8C%96)
#[derive(PartialEq, Debug, Clone, Default, Reflect, Component)]
#[reflect(Default, Component)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", reflect(Serialize, Deserialize))]
pub(crate) struct SpringJointState {
    prev_tail: Vec3,
    current_tail: Vec3,
    bone_axis: Vec3,
    bone_length: f32,
    initial_local_matrix: Mat4,
    initial_local_rotation: Quat,
}

impl SpringJointState {
    /// Seeds the simulation state of one joint from the rest pose.
    ///
    /// The load-time pipeline (`vrm::gltf::handler::scene::build_spring_chains`)
    /// builds the chains while the glTF file loads, where the scene is still in
    /// its rest pose and every rest transform is known, so the state is
    /// initialised right there and the runtime systems never have to.
    ///
    /// * `bone_axis` — the direction from the joint to the next one, in the
    ///   joint's own rest frame. The caller passes it already normalised
    ///   (`Vec3::normalize_or_zero`), because a zero-length bone axis would make
    ///   the Verlet step produce `NaN`.
    /// * `bone_length` — the distance to the next joint; the simulation keeps it
    ///   constant.
    /// * `tail` — the next joint's rest position, in center space when the
    ///   spring declares a center node and in world space otherwise. This is
    ///   both `current_tail` and `prev_tail`, i.e. the chain starts at rest with
    ///   no inertia.
    /// * `rest` — the joint's own rest transform, which the solver needs as the
    ///   reference its delta rotation is applied to.
    pub(crate) fn from_rest(
        bone_axis: Vec3,
        bone_length: f32,
        tail: Vec3,
        rest: Transform,
    ) -> Self {
        Self {
            prev_tail: tail,
            current_tail: tail,
            bone_axis,
            bone_length,
            initial_local_matrix: rest.to_matrix(),
            initial_local_rotation: rest.rotation,
        }
    }

    /// Resets the velocity by setting `prev_tail` to `current_tail`.
    ///
    /// This eliminates the inertia term `(current_tail - prev_tail)` in
    /// the Verlet integration, preventing spring bones from bouncing
    /// after sudden bone movements (e.g. animation transitions).
    pub(crate) fn reset_velocity(&mut self) {
        self.prev_tail = self.current_tail;
    }
}

#[derive(Component, Debug, Clone, PartialEq, Default, Reflect)]
#[reflect(Component, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", reflect(Serialize, Deserialize))]
pub struct SpringRoot {
    /// Represents a list of entity of spring joints belonging to the spring chain.
    /// This component is inserted into the root entity of the chain.
    pub joints: SpringJoints,

    pub colliders: SpringColliders,

    /// If the spring chain has a center node,
    /// The inertia of the spring bone is evaluated in the [`Center Space`](https://github.com/vrm-c/vrm-specification/tree/master/specification/VRMC_springBone-1.0#center-space).
    pub center_node: SpringCenterNode,
}

#[derive(Eq, PartialEq, Debug, Clone, Default, Deref, Reflect)]
#[reflect(Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", reflect(Serialize, Deserialize))]
pub struct SpringJoints(pub Vec<Entity>);

#[derive(PartialEq, Debug, Clone, Default, Deref, Reflect)]
#[reflect(Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", reflect(Serialize, Deserialize))]
pub struct SpringColliders(pub Vec<(Entity, ColliderShape)>);

#[derive(Eq, PartialEq, Debug, Clone, Default, Deref, Reflect)]
#[reflect(Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", reflect(Serialize, Deserialize))]
pub struct SpringCenterNode(pub Option<Entity>);

#[derive(Component, Debug, Copy, Clone, Default, PartialEq, Reflect)]
#[reflect(Default, Component)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", reflect(Serialize, Deserialize))]
pub struct SpringJointProps {
    pub drag_force: f32,
    pub gravity_dir: Vec3,
    pub gravity_power: f32,
    pub hit_radius: f32,
    pub stiffness: f32,
}
pub struct VrmSpringBonePlugin;

impl Plugin for VrmSpringBonePlugin {
    fn build(
        &self,
        app: &mut App,
    ) {
        app.register_type::<SpringRoot>()
            .register_type::<SpringJointState>()
            .register_type::<SpringJoints>()
            .register_type::<SpringColliders>()
            .register_type::<SpringCenterNode>()
            .register_type::<SpringJointProps>()
            .add_plugins((
                SpringBoneInitializePlugin,
                SpringBoneRegistryPlugin,
                SpringBoneUpdatePlugin,
            ));
    }
}
