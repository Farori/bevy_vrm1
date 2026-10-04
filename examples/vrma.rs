//! This example shows how to animate a VRM model using VRMA.
//!
//! The avatar is spawned through the load-time glTF pipeline with
//! [`spawn_vrm`], and the `.vrma` is attached from that call's `configure`
//! closure. That is the whole migration: the closure is handed the VRM root -
//! the one entity the scene asset marks with [`Initialized`] - so the
//! `VrmaHandle` it spawns is a child of an avatar that is *already* initialized.
//! `VrmaPlugin` skips any handle whose parent lacks that marker
//! (`src/vrma/initialize.rs:38-44`), so a VRMA parented anywhere else would sit
//! there with its handle forever.
//!
//! [`spawn_vrm`] also settles *when*. The closure runs the moment the avatar is
//! instantiated, so this example never assumes an entity id and needs no
//! `Added<Initialized>` system to find the root.
//!
//! # Which of the two loads finishes first does not matter
//!
//! The `.vrma` asset load is started right here, in `Startup`, in parallel with
//! the `.vrm`; only the `VrmaHandle` *component* is deferred to `configure`.
//! Both orders converge on the same state: the handle is alive from `configure`
//! on, so the VRMA side finds `Assets<VrmaAsset>` already populated when the
//! asset wins the race, and finds an initialized parent when the avatar does.
//!
//! When the VRMA animation player is set up, a [`LoadedVrma`] trigger is fired.
//! You cannot play animations until this load is complete.
//!
//! # Known gap
//!
//! The clip's *humanoid bone* tracks play: the retarget resolves the destination
//! bone by [`VrmBone`], which the pipeline writes on every bone
//! (`src/vrm/gltf/handler/nodes.rs:39`), and the animation graph, its mask
//! groups and the `AnimationPlayer` on `Vrm::ROOT_BONE` are all built from
//! components a pipeline scene does carry.
//!
//! Its *expression* tracks do not. `VRMA_01.vrma` drives `happy`, `aa` and
//! `blinkRight` alongside the bones, but both consumers look the avatar's
//! expressions up through the legacy subtree named `VRMC_vrm.expressions`
//! (`play_expression_animations`, `src/vrma/animation/play.rs:152`, and
//! `apply_regenerate_expression_clips`,
//! `src/vrma/animation/animation_graph.rs:492`), and only the legacy loader
//! spawns that subtree (`src/vrm/expressions.rs:452`). The pipeline writes
//! `VrmExpressionWeights` / `ExpressionMorphBinds` on the root instead, but
//! nothing yet moves a `.vrma`'s expression curves onto them.

use bevy::animation::RepeatAnimation;
use bevy::prelude::*;
use bevy_vrm1::prelude::*;
use std::time::Duration;

fn main() {
    App::new()
        // `VrmGltfPlugin` claims the `vrm` extension for `Handle<Gltf>`, which is
        // what `spawn_vrm` loads, so it has to be in the app. `VrmaPlugin` is
        // untouched by the migration and still owns the `.vrma` loader.
        .add_plugins((DefaultPlugins, VrmPlugin, VrmGltfPlugin, VrmaPlugin))
        .add_systems(
            Startup,
            (spawn_directional_light, spawn_camera, spawn_avatar),
        )
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

fn spawn_camera(mut commands: Commands) {
    commands.spawn((
        Camera3d::default(),
        // A default camera does not render the exclusive first-person layers,
        // and `firstPerson: auto` put this avatar's head copies on one of them.
        third_person_camera_layers(),
        Transform::from_xyz(0., 0.8, 2.5),
    ));
}

fn spawn_avatar(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
) {
    // Started here rather than inside `configure`, so the `.vrma` and the `.vrm`
    // load in parallel. The handle is moved into the closure, which keeps it
    // alive until `configure` runs, so the asset cannot be dropped in between.
    let vrma = asset_server.load("vrma/VRMA_01.vrma");
    spawn_vrm(&mut commands, "vrm/AliciaSolid.vrm", move |root| {
        // You need to spawn VRMA as a child of the VRM you want to retarget.
        // `root` is that VRM, resolved by `spawn_vrm` and not assumed here.
        root.with_children(|child| {
            child.spawn(VrmaHandle(vrma)).observe(apply_play_vrma);
        });
    });
}

/// When the VRMA animation player is set up, a [`LoadedVrma`] trigger is fired.
/// You cannot play animations until this load is complete.
///
/// The trigger's target is the VRMA entity, so a *scoped* observer registered on
/// that same entity sees it. Its `vrma` field is the entity to play, which is why
/// the closure above registers this observer rather than a global one.
fn apply_play_vrma(
    trigger: On<LoadedVrma>,
    mut commands: Commands,
) {
    commands.trigger(PlayVrma {
        repeat: RepeatAnimation::Forever,
        transition_duration: Duration::ZERO,
        vrma: trigger.vrma,
        reset_spring_bones: false,
    });
}
