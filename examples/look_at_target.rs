//! This example shows how to make a VRM model look at a specific entity.
//! The VRM model will track a red cube as its target.
//! The cube can be freely moved by dragging it with the mouse.
//!
//! `LookAt::Target` goes on the VRM root, which under the pipeline is the entity
//! the scene asset instantiates; `Initialized` marks it, so the example attaches
//! the component when that marker appears instead of assuming an entity id.
//!
//! # Known gap
//!
//! `LookAt` is evaluated against the `HeadBoneEntity`, `LeftEyeBoneEntity` and
//! `RightEyeBoneEntity` on the same entity, and only the legacy loader writes
//! those. Until the pipeline resolves them from `VRMC_vrm.humanoid` too, the
//! avatar does not follow the cube.

use bevy::gltf::GltfAssetLabel;
use bevy::prelude::*;
use bevy_vrm1::prelude::*;

/// The entity the avatar looks at, published when the cube is spawned and read
/// once the avatar's root exists.
#[derive(Resource, Default)]
struct LookTarget(Option<Entity>);

fn main() {
    App::new()
        .add_plugins((DefaultPlugins, VrmPlugin, VrmGltfPlugin, MeshPickingPlugin))
        .init_resource::<LookTarget>()
        .add_systems(Startup, (spawn_camera, spawn_vrm, spawn_directional_light))
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

fn spawn_camera(mut commands: Commands) {
    commands.spawn((
        Camera3d::default(),
        // A default camera does not render the exclusive first-person layers.
        third_person_camera_layers(),
        Transform::from_xyz(0.0, 1.0, 3.5),
    ));
}

fn spawn_vrm(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut target: ResMut<LookTarget>,
    asset_server: Res<AssetServer>,
) {
    let cube = commands
        .spawn((
            Mesh3d(meshes.add(Cuboid::from_size(Vec3::ONE / 4.))),
            MeshMaterial3d(
                materials.add(StandardMaterial::from_color(Color::linear_rgb(1., 0., 0.))),
            ),
            Transform::from_xyz(0.5, 1., 1.),
        ))
        .observe(apply_drag_move_cube)
        .id();
    target.0 = Some(cube);
    commands.spawn(WorldAssetRoot(
        asset_server.load(GltfAssetLabel::Scene(0).from_asset("vrm/AliciaSolid.vrm")),
    ));
}

/// Puts `LookAt::Target` on the scene root as soon as the handler has finished
/// it.
fn attach_look_at(
    mut commands: Commands,
    target: Res<LookTarget>,
    vrms: Query<Entity, Added<Initialized>>,
) {
    let Some(target) = target.0 else {
        return;
    };
    for vrm in vrms.iter() {
        commands.entity(vrm).insert(LookAt::Target(target));
    }
}

fn apply_drag_move_cube(
    trigger: On<Pointer<Drag>>,
    mut transforms: Query<&mut Transform>,
    cameras: Query<(&Camera, &GlobalTransform)>,
) {
    let (camera, camera_gtf) = cameras.single().expect("expected a camera");
    let Ok(ray) = camera.viewport_to_world(camera_gtf, trigger.pointer_location.position) else {
        return;
    };
    let Ok(mut tf) = transforms.get_mut(trigger.event_target()) else {
        return;
    };
    let plane = InfinitePlane3d::new(camera_gtf.back());
    let Some(distance) = ray.intersect_plane(tf.translation, plane) else {
        return;
    };
    tf.translation = ray.get_point(distance);
}
