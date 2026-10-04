//! `BodyTracking` spreads the gaze of a [`LookAt`] over the spine, chest, neck
//! and head bones instead of leaving it to the eyes alone.
//!
//! Both components go on the VRM root, and [`spawn_vrm`]'s `configure` closure
//! is handed that root directly: no `Added<Initialized>` system, no lookup.
//! `BodyTrackingPlugin` observes the insert of `BodyTracking` and adds the
//! `SmoothedGaze` state it keeps, so nothing else has to be set up here.
//!
//! # Known gap
//!
//! `BodyTracking` reads the same `HeadBoneEntity`, `NeckBoneEntity`,
//! `ChestBoneEntity` and `SpineBoneEntity` components as `LookAt`, and only the
//! legacy loader writes those, so the bones stay in their rest pose for now.

use bevy::prelude::*;
use bevy_vrm1::prelude::*;

fn main() {
    App::new()
        .add_plugins((DefaultPlugins, VrmPlugin, VrmGltfPlugin))
        .add_systems(Startup, (spawn_camera_and_avatar, spawn_directional_light))
        .run();
}

fn spawn_camera_and_avatar(mut commands: Commands) {
    commands.spawn((
        Camera3d::default(),
        // A default camera does not render the exclusive first-person layers.
        third_person_camera_layers(),
        Transform::from_xyz(0.0, 1.3, 1.0),
    ));
    spawn_vrm(&mut commands, "vrm/AliciaSolid.vrm", |root| {
        root.insert((LookAt::Cursor, BodyTracking::default()));
    });
}

fn spawn_directional_light(mut commands: Commands) {
    commands.spawn((
        DirectionalLight {
            illuminance: 10000.0,
            ..default()
        },
        // A default light does not shadow the exclusive first-person layers.
        all_vrm_render_layers(),
        Transform::from_rotation(Quat::from_euler(EulerRot::XYZ, -1.0, 1.0, 0.0)),
    ));
}
