//! This module handles humanoid bones.
//! Refer to [here](https://docs.unity3d.com/ja/2019.4/ScriptReference/HumanBodyBones.html) for the list of humanoid bones.
//!
//! Marker components are written for each bone while the VRM(A) loads. For
//! example, the entity of the hips bone will have [`Hips`] inserted.
//! Additionally, a component that holds the entity will be inserted into the VRM(A) entity.
//!
//! For a `.vrm` these are written into the scene
//! [`WorldAsset`](bevy::world_serialization::WorldAsset) by the load-time glTF
//! pipeline ([`scene::insert_humanoid_bone_holders`](crate::vrm::gltf::handler::scene));
//! a `.vrma` builds its own source rig through
//! [`HumanoidBoneRegistry::new`], which the VRMA retarget reads.

mod bones;

use crate::prelude::*;
use crate::vrm::VrmBone;
use crate::vrm::gltf::extensions::VrmNode;
use crate::vrm::humanoid_bone::bones::BonesPlugin;
use bevy::app::{App, Plugin};
use bevy::asset::{Assets, Handle};
use bevy::gltf::GltfNode;
use bevy::platform::collections::HashMap;
use bevy::prelude::*;

pub mod prelude {
    pub use crate::vrm::humanoid_bone::bones::*;
}

/// The humanoid bones a model declares, as `(bone, glTF node name)` pairs.
///
/// It is the avatar-side half of the VRMA retarget: a `.vrma` file carries node
/// *references*, not names, so the source rig's bones are looked up by the node
/// name its own `vrmc_vrm_animation.humanoid` block points at. Nothing about a
/// `.vrm`'s bones is stored here — the load-time pipeline resolves those by
/// glTF node index while the file loads and writes [`VrmBone`] plus the
/// `<Bone>BoneEntity` holders straight into the scene asset.
#[derive(Component, Deref, Reflect, Default)]
pub(crate) struct HumanoidBoneRegistry(HashMap<VrmBone, Name>);

impl HumanoidBoneRegistry {
    /// Resolves the node names a humanoid map refers to, dropping every entry
    /// whose node is missing from the file or not yet loaded.
    pub fn new(
        bones: &HashMap<String, VrmNode>,
        node_assets: &Assets<GltfNode>,
        nodes: &[Handle<GltfNode>],
    ) -> Self {
        Self(
            bones
                .iter()
                .filter_map(|(name, target_node)| {
                    let node_handle = nodes.get(target_node.node)?;
                    let node = node_assets.get(node_handle)?;
                    Some((VrmBone(name.clone()), Name::new(node.name.clone())))
                })
                .collect(),
        )
    }
}

/// Registers the bone markers and the bone-entity holders for apps that load a
/// VRM without [`VrmGltfPlugin`](crate::prelude::VrmGltfPlugin).
///
/// [`crate::vrm::gltf::VrmGltfPlugin`] registers the same types itself, because
/// the scene spawn pipeline drops any component whose type is not registered
/// (`ReflectComponent::apply_or_insert_mapped`). [`register_bone_components`]
/// guards against the duplicate when an app holds both plugins.
pub(super) struct VrmHumanoidBonePlugin;

impl Plugin for VrmHumanoidBonePlugin {
    fn build(
        &self,
        app: &mut App,
    ) {
        app.register_type::<HumanoidBoneRegistry>();
        register_bone_components(app);
    }
}

macro_rules! insert_bone {
    (
        $commands: expr,
        $vrm_entity: expr,
        $bone_entity: expr,
        $bone_name: expr,
        $($bone: ident),+$(,)?
    ) => {

        match $bone_name.0.to_uppercase(){
            $(
                x if x == stringify!($bone).to_uppercase() => {
                    paste::paste!{
                        $commands.entity($vrm_entity).insert([<$bone BoneEntity>]($bone_entity));
                    }
                    $commands.entity($bone_entity).insert($bone);
                }
            )+
            _ => {

            }
        }
    };
}

/// Inserts every bone-entity holder of an avatar onto its root, plus each bone's
/// own marker on the bone itself.
///
/// * the root gets one `<Bone>BoneEntity(Entity)` holder per bone
///   ([`HeadBoneEntity`], [`NeckBoneEntity`], …), which is how a system
///   addresses a bone *through* the avatar rather than through the hierarchy:
///   [`track_looking_target`](crate::vrm::look_at::track_looking_target),
///   [`track_body_tracking`](crate::vrm::body_tracking) and the load-time
///   `firstPerson: auto` split all read them;
/// * the bone entity gets its marker ([`Head`], [`LeftEye`], …), which is what a
///   reader uses to recognise a bone without resolving the holder.
///
/// The only caller is the load-time pipeline
/// ([`scene::insert_humanoid_bone_holders`](crate::vrm::gltf::handler::scene)),
/// which writes into the scene [`bevy::world_serialization::WorldAsset`] so a
/// `SceneRoot` is born holding them. A `bones` entry whose name is not one of the
/// 55 Unity humanoid bone names inserts nothing — the macro has always fallen
/// through on those.
///
/// # Why `Commands`, and not `&mut World`
///
/// `bevy_gltf`'s loader has no `Commands` to hand out, so the pipeline borrows
/// the world's own command queue (`World::commands`) and flushes it
/// (`World::flush`) before the world is frozen into a `WorldAsset` — the same
/// two calls `bevy_ecs` documents for exactly this case.
///
/// # The holders follow the avatar
///
/// Writing them into a scene asset is only half the job. The scene spawn
/// pipeline remaps entity references through `Component::map_entities`
/// (`bevy_ecs/src/reflect/component.rs:340`, `:345`, `:353`), which
/// `#[derive(Component)]` generates from `#[entities]` field attributes —
/// `entity_component!` (`src/macros.rs:109-127`) writes that attribute on the
/// holder's field, so an instantiated avatar's holders name its own bones and
/// gaze control, body tracking and the first-person split resolve correctly.
pub(crate) fn insert_bone_holders(
    root: Entity,
    commands: &mut Commands,
    bones: &[(VrmBone, Entity)],
) {
    for (bone, bone_entity) in bones {
        let bone_entity = *bone_entity;
        insert_bone!(
            commands,
            root,
            bone_entity,
            bone,
            Hips,
            RightRingProximal,
            RightThumbDistal,
            RightRingIntermediate,
            RightUpperArm,
            LeftIndexProximal,
            LeftUpperLeg,
            LeftFoot,
            LeftIndexDistal,
            LeftThumbMetacarpal,
            RightLowerArm,
            LeftMiddleDistal,
            RightUpperLeg,
            LeftToes,
            LeftThumbDistal,
            RightShoulder,
            RightThumbMetacarpal,
            Spine,
            LeftLowerLeg,
            LeftShoulder,
            LeftUpperArm,
            UpperChest,
            RightToes,
            RightIndexDistal,
            LeftMiddleProximal,
            RightRingProximal,
            LeftRingDistal,
            LeftThumbMetacarpal,
            LeftIndexIntermediate,
            LeftLittleProximal,
            LeftLittleDistal,
            RightHand,
            RightLittleProximal,
            LeftRingIntermediate,
            RightIndexIntermediate,
            Chest,
            LeftHand,
            RightLittleIntermediate,
            RightFoot,
            RightLowerLeg,
            LeftLittleIntermediate,
            LeftLowerArm,
            RightLittleDistal,
            RightMiddleIntermediate,
            RightMiddleProximal,
            RightThumbMetacarpal,
            Neck,
            Jaw,
            Head,
            LeftEye,
            RightEye,
            LeftMiddleIntermediate,
            RightRingDistal,
            LeftIndexProximal,
            RightIndexProximal,
            RightMiddleDistal,
        );
    }
}

/// Registers the bone markers and the bone-entity holders.
///
/// [`crate::vrm::gltf::VrmGltfPlugin`] calls this because the
/// pipeline writes both families into the scene asset, and the scene spawn
/// pipeline drops any component whose type is not registered
/// (`ReflectComponent::apply_or_insert_mapped`). [`VrmPlugin`] calls it too, so
/// an app holding both plugins reaches this twice. [`BonesPlugin`] is a unique
/// plugin and `add_plugins` panics on a duplicate, hence the guard.
pub(crate) fn register_bone_components(app: &mut App) {
    if !app.is_plugin_added::<BonesPlugin>() {
        app.add_plugins(BonesPlugin);
    }
}

#[cfg(test)]
mod plugin_tests {
    use super::*;

    /// `VrmPlugin + VrmGltfPlugin` both register the bone types, which panicked
    /// with "plugin was already added in application" and broke every example.
    #[test]
    fn test_register_bone_components_is_idempotent() {
        let mut app = App::new();
        register_bone_components(&mut app);
        register_bone_components(&mut app);
        assert!(app.is_plugin_added::<BonesPlugin>());
    }
}
