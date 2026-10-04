mod update;

use crate::prelude::ColliderShape;
use crate::vrm::spring_bone::update::SpringBoneUpdatePlugin;
use bevy::app::App;
use bevy::ecs::entity::MapEntities;
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
    ///
    /// Every field is `#[entities]`, so the chain follows the avatar: a spring
    /// built at load time names entities in `bevy_gltf`'s scratch world, and the
    /// scene spawn pipeline rewrites them as it copies the component out of the
    /// `WorldAsset` (`bevy_world_serialization/src/world_asset.rs:191-199` ->
    /// `ReflectComponent::apply_or_insert_mapped` -> `C::map_entities`,
    /// `bevy_ecs/src/reflect/component.rs:340`, `:345`, `:353`). Without the
    /// attributes an instantiated avatar's springs verlet-integrate against the
    /// scratch world, which shares no transforms with it.
    #[entities]
    pub joints: SpringJoints,

    #[entities]
    pub colliders: SpringColliders,

    /// If the spring chain has a center node,
    /// The inertia of the spring bone is evaluated in the [`Center Space`](https://github.com/vrm-c/vrm-specification/tree/master/specification/VRMC_springBone-1.0#center-space).
    #[entities]
    pub center_node: SpringCenterNode,
}

/// The chain of joints, each of which an [`Entity`] in the spring's own world.
///
/// `#[derive(MapEntities)]` is what makes the holder itself mappable, so
/// [`SpringRoot::joints`] can be marked `#[entities]`. `Vec<Entity>` is covered
/// by bevy's own blanket impl (`bevy_ecs/src/entity/map_entities.rs:172-178`).
///
/// `MapEntities` is not in `bevy::prelude`, and `#[derive(MapEntities)]` emits a
/// bare `self.0.map_entities(mapper)` with no `use` of its own
/// (`bevy_ecs/macros/src/lib.rs:216-238`), so the `use bevy::ecs::entity::MapEntities`
/// at the top of this module is what makes the method call resolve.
/// [`SpringColliders`] needs the same import for its hand-written impl.
#[derive(Eq, PartialEq, Debug, Clone, Default, Deref, Reflect, MapEntities)]
#[reflect(Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", reflect(Serialize, Deserialize))]
pub struct SpringJoints(#[entities] pub Vec<Entity>);

/// The colliders of one chain: `(collider entity, its shape)` per collider.
///
/// Hand-written rather than derived, because bevy implements [`MapEntities`] for
/// no tuple type — `bevy_ecs/src/entity/map_entities.rs` covers `Entity`,
/// `Option<T>`, `Vec<T>`, `[T; N]`, `VecDeque<T>`, `SmallVec`, the hash and
/// ordered maps/sets and `()`, and nothing else — so `#[entities]` on
/// `Vec<(Entity, ColliderShape)>` would not compile. Only the entity half of the
/// pair is remapped; the shape is plain data and is left alone.
#[derive(PartialEq, Debug, Clone, Default, Deref, Reflect)]
#[reflect(Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", reflect(Serialize, Deserialize))]
pub struct SpringColliders(pub Vec<(Entity, ColliderShape)>);

impl MapEntities for SpringColliders {
    fn map_entities<M: EntityMapper>(
        &mut self,
        entity_mapper: &mut M,
    ) {
        for (collider, _) in &mut self.0 {
            collider.map_entities(entity_mapper);
        }
    }
}

/// The chain's center node, if it declared one — see [`Center Space`](https://github.com/vrm-c/vrm-specification/tree/master/specification/VRMC_springBone-1.0#center-space).
///
/// Derived like [`SpringJoints`]: `Option<Entity>` is bevy's blanket impl at
/// `bevy_ecs/src/entity/map_entities.rs:68-74`.
#[derive(Eq, PartialEq, Debug, Clone, Default, Deref, Reflect, MapEntities)]
#[reflect(Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", reflect(Serialize, Deserialize))]
pub struct SpringCenterNode(#[entities] pub Option<Entity>);

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
            .add_plugins(SpringBoneUpdatePlugin);
    }
}
