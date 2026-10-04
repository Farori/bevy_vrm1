//! Avatar forward-orientation policy.
//!
//! VRM 1.0 models face +Z in glTF space while Bevy's forward is -Z. Bevy can
//! rotate a scene itself, through
//! [`bevy::gltf::convert_coordinates::GltfConvertCoordinates`]; this module
//! decides, once per load, what the resulting orientation is, so that nothing
//! downstream has to guess.
//!
//! # Where Bevy applies the conversion
//!
//! `bevy_gltf` computes the scene root's conversion transform up front
//! (`bevy_gltf/src/loader/mod.rs:1027`, `convert_coordinates::scene_conversion_transform`
//! at `bevy_gltf/src/convert_coordinates.rs:93`) and spawns `world_root_id`
//! with it (`bevy_gltf/src/loader/mod.rs:1029-1040`). Only afterwards does it
//! call the extension hooks: `on_scene_completed` runs at
//! `bevy_gltf/src/loader/mod.rs:1113-1120`, and `WorldAsset::new(world)` closes
//! the scratch world at `bevy_gltf/src/loader/mod.rs:1122`.
//!
//! The ordering matters: **rest poses captured in `on_scene_completed` already
//! include Bevy's conversion**, so applying a second rotation there would double
//! it. That is why this module only classifies the orientation and never
//! rewrites a transform.
//!
//! # Why [`VrmForwardPolicy::KeepGltfForward`] is the default
//!
//! Both conversion flags are off by default — see
//! [`bevy::gltf::convert_coordinates::GltfConvertCoordinates`], whose fields
//! default to `false` — which is what vanilla `bevy_gltf` does and what
//! `VrmGltfPlugin`'s inner loader inherits. Switching the default would silently
//! rotate every already-working avatar, so the orientation is *described*, not
//! *changed*.
//!
//! `rotate_meshes` is reported as unsupported rather than honoured: it rebuilds
//! mesh assets and their skinned bind poses, and rebaking inverse bind poses
//! against rewritten vertex data would invalidate the rest poses that spring
//! bones and constraints are measured against.

use bevy::prelude::*;

use crate::error::vrm_warn;

/// How a loaded scene is oriented.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VrmForwardPolicy {
    /// The loader already rotated the scene entity, so the avatar's glTF +Z
    /// forward now matches Bevy's `Transform::forward`.
    AlreadyConverted,
    /// Keep the raw glTF orientation: the avatar faces +Z, matching vanilla
    /// `bevy_gltf` with conversion disabled, which is what `VrmGltfPlugin`'s
    /// loader inherits from `GltfPlugin`.
    #[default]
    KeepGltfForward,
    /// `rotate_meshes` was requested, which is not supported for `.vrm` /
    /// `.vrma`. The scene entity is still converted, so the orientation is the
    /// same as [`AlreadyConverted`](Self::AlreadyConverted) and only the meshes
    /// were left in glTF space; the mismatch is reported at load time.
    UnsupportedRotateMeshes,
}

/// Classifies the scene orientation from the conversion flags.
///
/// The flags come from `GltfLoaderSettings.convert_coordinates`, falling back to
/// the [`GltfPlugin::convert_coordinates`](bevy::gltf::GltfPlugin) defaults.
#[must_use]
pub fn resolve_forward_policy(
    rotate_scene_entity: bool,
    rotate_meshes: bool,
) -> VrmForwardPolicy {
    if rotate_meshes {
        vrm_warn!(
            "VRM: GltfConvertCoordinates.rotate_meshes is not supported for .vrm/.vrma; \
             treating the scene as already converted"
        );
        VrmForwardPolicy::UnsupportedRotateMeshes
    } else if rotate_scene_entity {
        VrmForwardPolicy::AlreadyConverted
    } else {
        VrmForwardPolicy::KeepGltfForward
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_conversion_keeps_gltf_forward() {
        assert_eq!(
            resolve_forward_policy(false, false),
            VrmForwardPolicy::KeepGltfForward
        );
    }

    #[test]
    fn scene_conversion_skips_own_rotation() {
        assert_eq!(
            resolve_forward_policy(true, false),
            VrmForwardPolicy::AlreadyConverted
        );
    }

    #[test]
    fn rotate_meshes_is_unsupported() {
        assert_eq!(
            resolve_forward_policy(false, true),
            VrmForwardPolicy::UnsupportedRotateMeshes
        );
    }

    /// `rotate_meshes` is the more specific report: it wins over
    /// `rotate_scene_entity`, which is also set in practice whenever a caller
    /// enables both. Reporting `AlreadyConverted` there would hide the fact that
    /// the meshes were never converted.
    #[test]
    fn rotate_meshes_takes_precedence_over_scene_conversion() {
        assert_eq!(
            resolve_forward_policy(true, true),
            VrmForwardPolicy::UnsupportedRotateMeshes
        );
    }
}
