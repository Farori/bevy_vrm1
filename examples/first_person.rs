//! Demonstrates `VRMC_vrm.firstPerson` support.
//!
//! `VrmGltfPlugin` makes the first/third-person decision while the `.vrm`
//! loads: a mesh visible in both views is put on `LAYER_BOTH`, the head copy it
//! splits off an `auto` mesh on `LAYER_THIRD_PERSON_ONLY`, and a
//! `firstPersonOnly` mesh on `LAYER_FIRST_PERSON_ONLY`. Nothing has to be split
//! or triggered at runtime, so choosing a view is only a question of which
//! layers the camera carries - which is this example's lesson.
//!
//! The main window starts in third person. Press `F` to switch its
//! `RenderLayers` between `third_person_camera_layers()` and
//! `first_person_camera_layers()`; in first-person mode the camera also moves to
//! the avatar's eyes and the head disappears from the main window.
//!
//! The small dark viewport in the top-left corner is a camera attached to the
//! head bone, tilted down: it sees the floor, the cubes and the body, but never
//! the head.
//!
//! The `FirstPersonCamera` / `ThirdPersonCamera` markers and the
//! `RequestEnableFirstPerson` / `RequestDisableFirstPerson` triggers belong to
//! the legacy loader and are not used here. The render-layer helpers above are
//! what replaces them.

use bevy::camera::visibility::RenderLayers;
use bevy::camera::{ClearColorConfig, Viewport};
use bevy::gltf::GltfAssetLabel;
use bevy::prelude::*;
use bevy_vrm1::prelude::*;

#[derive(Component)]
struct MainCamera;

#[derive(Resource, Default)]
struct FirstPersonMode {
    enabled: bool,
    saved_camera: Option<Transform>,
}

/// The avatar's head bone, resolved once the scene has been instantiated.
#[derive(Resource, Default)]
struct HeadBone(Option<Entity>);

fn main() {
    App::new()
        .add_plugins((DefaultPlugins, VrmPlugin, VrmGltfPlugin))
        .init_resource::<FirstPersonMode>()
        .init_resource::<HeadBone>()
        .add_systems(Startup, (spawn_main_camera, spawn_scene, spawn_vrm))
        .add_systems(
            Update,
            (
                attach_head_camera,
                (toggle_main_camera, sync_first_person_camera).chain(),
            ),
        )
        .run();
}

fn spawn_scene(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    commands.spawn((
        DirectionalLight {
            shadow_maps_enabled: true,
            ..default()
        },
        // A default light does not shadow the exclusive first-person layers.
        all_vrm_render_layers(),
        Transform::from_xyz(3.0, 3.0, 0.3).looking_at(Vec3::ZERO, Vec3::Y),
    ));
    // Ground.
    commands.spawn((
        Mesh3d(meshes.add(Plane3d::new(Vec3::Y, Vec2::new(30.0, 30.0)))),
        MeshMaterial3d(materials.add(StandardMaterial::from(Color::WHITE))),
    ));
    // Reference cubes in front of the avatar (VRM 1.0 avatars face +Z),
    // so the first-person camera has something to look at.
    for (x, z, color) in [
        (-1.0, 4.5, Color::srgb(0.9, 0.3, 0.3)),
        (0.0, 5.0, Color::srgb(0.3, 0.9, 0.3)),
        (1.0, 4.5, Color::srgb(0.3, 0.3, 0.9)),
    ] {
        commands.spawn((
            Mesh3d(meshes.add(Cuboid::new(0.4, 0.4, 0.4))),
            MeshMaterial3d(materials.add(StandardMaterial::from(color))),
            Transform::from_xyz(x, 0.2, z),
        ));
    }
}

/// The main camera starts in third person.
///
/// No plugin assigns its render layers on the pipeline path - that is the point:
/// a view is chosen by picking the layers, here `third_person_camera_layers()`.
fn spawn_main_camera(mut commands: Commands) {
    commands.spawn((
        Camera3d::default(),
        MainCamera,
        third_person_camera_layers(),
        Transform::from_xyz(0.0, 1.2, 2.5).looking_at(Vec3::new(0.0, 1.0, 0.0), Vec3::Y),
    ));
}

fn spawn_vrm(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
) {
    commands.spawn(WorldAssetRoot(
        asset_server.load(GltfAssetLabel::Scene(0).from_asset("vrm/AliciaSolid.vrm")),
    ));
}

/// Attaches a picture-in-picture camera to the head bone once the avatar has
/// been instantiated.
///
/// `Initialized` is the pipeline's readiness signal - it is written onto the
/// scene root inside the scene asset - and the head bone is found through the
/// `VrmBone` the handler puts on every humanoid bone.
fn attach_head_camera(
    mut commands: Commands,
    mut head: ResMut<HeadBone>,
    vrms: Query<Entity, Added<Initialized>>,
    bones: Query<(Entity, &VrmBone)>,
) {
    if vrms.is_empty() {
        return;
    }
    let Some((head_bone, _)) = bones.iter().find(|(_, bone)| bone.0 == "head") else {
        return;
    };
    head.0 = Some(head_bone);
    commands.entity(head_bone).with_children(|spawner| {
        spawner.spawn((
            Camera3d::default(),
            Camera {
                // Render on top of the main camera.
                order: 1,
                viewport: Some(Viewport {
                    physical_position: UVec2::new(10, 10),
                    physical_size: UVec2::new(400, 300),
                    ..default()
                }),
                // A distinct background makes the viewport clearly visible.
                clear_color: ClearColorConfig::Custom(Color::srgb(0.05, 0.05, 0.15)),
                ..default()
            },
            // A first-person view: the ordinary scene plus whatever the file
            // marked `firstPersonOnly`, but never the head copies.
            first_person_camera_layers(),
            // At eye level, tilted down so the body is in view.
            Transform::from_xyz(0.0, 0.06, 0.0)
                .looking_to(Dir3::new(Vec3::new(0.0, -0.4, 1.0)).unwrap(), Vec3::Y),
        ));
    });
}

/// Press `F` to toggle the main camera between the external third-person view
/// and a true first-person view from the avatar's eyes: the render layers decide
/// which of the avatar's meshes the camera can see, and the transform follows.
fn toggle_main_camera(
    keys: Res<ButtonInput<KeyCode>>,
    mut mode: ResMut<FirstPersonMode>,
    mut cameras: Query<(&mut Transform, &mut RenderLayers), With<MainCamera>>,
) {
    if !keys.just_pressed(KeyCode::KeyF) {
        return;
    }
    mode.enabled = !mode.enabled;
    for (mut transform, mut render_layers) in cameras.iter_mut() {
        if mode.enabled {
            // Remember the external view to restore it later.
            mode.saved_camera = Some(*transform);
            *render_layers = first_person_camera_layers();
        } else {
            if let Some(saved) = mode.saved_camera.take() {
                *transform = saved;
            }
            *render_layers = third_person_camera_layers();
        }
    }
}

/// While first-person mode is enabled, the main camera follows the head bone
/// every frame (so it also tracks animations).
fn sync_first_person_camera(
    mode: Res<FirstPersonMode>,
    head: Res<HeadBone>,
    globals: Query<&GlobalTransform>,
    mut cameras: Query<&mut Transform, With<MainCamera>>,
) {
    if !mode.enabled {
        return;
    }
    let Some(head) = head.0 else {
        return;
    };
    let Ok(head_global) = globals.get(head) else {
        return;
    };
    // Eye position slightly above the head joint; VRM 1.0 avatars face +Z.
    // GlobalTransform is one frame behind, which is fine for a demo.
    let eye = head_global.transform_point(Vec3::new(0.0, 0.06, 0.0));
    let Ok(forward) = Dir3::new(head_global.rotation() * Vec3::Z) else {
        return;
    };
    for mut transform in cameras.iter_mut() {
        *transform = Transform::from_translation(eye).looking_to(forward, Vec3::Y);
    }
}
