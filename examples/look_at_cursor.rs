//! This example demonstrates the `LookAt` functionality of VRM.
//!
//! By using [`LookAt::Cursor`], the VRM model will look at the cursor position.
//!
//!
//! Alternatively, VRM can also look at a specific target by using [`LookAt::Target`].
//! Please refer to `examples/look_at_target.rs` for more details.
//!
//! `VrmGltfPlugin` writes the file's `VRMC_vrm.lookAt` onto the scene root as
//! `LookAtProperties` while the file loads, so the example only has to put the
//! `LookAt` component on that same root. `Initialized`, written by the same
//! handler, is how the example finds the root rather than assuming an id: the
//! `WorldAssetRoot` entity it spawned is only the avatar's *parent* and carries
//! none of the VRM data.
//!
//! # Known gap
//!
//! `LookAt` is evaluated against the `HeadBoneEntity`, `LeftEyeBoneEntity` and
//! `RightEyeBoneEntity` on the same entity, and only the legacy loader writes
//! those. Until the pipeline resolves them from `VRMC_vrm.humanoid` too, the
//! avatar does not follow the cursor.

use bevy::gltf::GltfAssetLabel;
use bevy::prelude::*;
use bevy_vrm1::prelude::*;

fn main() {
    App::new()
        .add_plugins((DefaultPlugins, VrmPlugin, VrmGltfPlugin))
        .add_systems(Startup, (spawn_camera_and_vrm, spawn_directional_light))
        .add_systems(Update, attach_look_at)
        .run();
}
fn spawn_directional_light(mut commands: Commands) {
    commands.spawn((
        DirectionalLight {
            shadow_maps_enabled: true,
            ..default()
        },
        // A default light does not shadow the exclusive first-person layers.
        all_vrm_render_layers(),
        Transform::from_xyz(3.0, 3.0, 0.3).looking_at(Vec3::ZERO, Vec3::Y),
    ));
}

fn spawn_camera_and_vrm(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
) {
    commands.spawn((
        Camera3d::default(),
        // A default camera does not render the exclusive first-person layers.
        third_person_camera_layers(),
        Transform::from_xyz(0.0, 1.3, 1.),
    ));
    commands.spawn(WorldAssetRoot(
        asset_server.load(GltfAssetLabel::Scene(0).from_asset("vrm/AliciaSolid.vrm")),
    ));
}

/// Puts `LookAt` on the scene root as soon as the handler has finished it.
fn attach_look_at(
    mut commands: Commands,
    vrms: Query<Entity, Added<Initialized>>,
) {
    for vrm in vrms.iter() {
        commands.entity(vrm).insert(LookAt::Cursor);
    }
}
