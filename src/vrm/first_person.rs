//! Provides support for `VRMC_vrm.firstPerson`.
//!
//! | firstPersonFlag | RenderLayers |
//! |---|---|
//! | `both` | 0 (default) |
//! | `thirdPersonOnly` | `FirstPersonLayers::third_person_only` |
//! | `firstPersonOnly` | `FirstPersonLayers::first_person_only` |
//! | `auto` | mesh is split: head part -> thirdPersonOnly, the rest -> both |

use crate::error::vrm_warn;
use crate::prelude::ChildSearcher;
use crate::prelude::MToonMaterial;
use crate::vrm::Vrm;
use crate::vrm::gltf::extensions::vrmc_vrm::{FirstPerson, FirstPersonFlag};
use crate::vrm::prelude::HeadBoneEntity;
use bevy::app::{App, Plugin, Update};
use bevy::asset::{Assets, Handle};
use bevy::camera::visibility::{Layer, RenderLayers};
use bevy::ecs::lifecycle::Add;
use bevy::ecs::reflect::ReflectComponent;
use bevy::ecs::{
    component::Component,
    entity::Entity,
    event::EntityEvent,
    hierarchy::{ChildOf, Children},
    name::Name,
    observer::On,
    query::{With, Without},
    resource::Resource,
    system::{Commands, Query, Res, ResMut},
};
use bevy::gltf::GltfNode;
use bevy::mesh::morph::MeshMorphWeights;
use bevy::mesh::{Indices, Mesh, Mesh3d, skinning::SkinnedMesh};
use bevy::pbr::MeshMaterial3d;
use bevy::platform::collections::HashSet;
use bevy::prelude::Deref;
use bevy::reflect::Reflect;

pub(crate) struct VrmFirstPersonPlugin;

impl Plugin for VrmFirstPersonPlugin {
    fn build(
        &self,
        app: &mut App,
    ) {
        app.init_resource::<FirstPersonLayers>()
            .register_type::<FirstPersonRegistry>()
            .register_type::<FirstPersonCamera>()
            .register_type::<ThirdPersonCamera>()
            .add_observer(setup_first_person_camera)
            .add_observer(setup_third_person_camera)
            .add_observer(apply_enable_first_person)
            .add_observer(apply_disable_first_person)
            .add_systems(Update, split_auto_meshes);
    }
}

/// Holds `(node name, firstPersonFlag)` pairs from `VRMC_vrm.firstPerson.meshAnnotations`.
#[derive(Component, Deref, Reflect, Default)]
pub struct FirstPersonRegistry(Vec<(Name, FirstPersonFlag)>);

impl FirstPersonRegistry {
    pub fn new(
        first_person: Option<&FirstPerson>,
        node_assets: &Assets<GltfNode>,
        nodes: &[Handle<GltfNode>],
    ) -> Self {
        let Some(fp) = first_person else {
            return Self::default();
        };
        Self(
            fp.mesh_annotations
                .iter()
                .filter_map(|a| {
                    let node = node_assets.get(nodes.get(a.node)?)?;
                    Some((Name::new(node.name.clone()), a.first_person_flag))
                })
                .collect(),
        )
    }
}

/// Render layers used to separate first-person-only and third-person-only meshes.
#[derive(Resource, Clone)]
pub struct FirstPersonLayers {
    pub first_person_only: Layer,
    pub third_person_only: Layer,
}

impl Default for FirstPersonLayers {
    fn default() -> Self {
        Self {
            first_person_only: 7,
            third_person_only: 8,
        }
    }
}

/// Attach to a camera that renders from the avatar's point of view.
#[derive(Component, Debug, Copy, Clone, Reflect)]
#[reflect(Component)]
pub struct FirstPersonCamera;

/// Attach to a camera that observes the avatar from outside.
#[derive(Component, Debug, Copy, Clone, Reflect)]
#[reflect(Component)]
pub struct ThirdPersonCamera;

fn setup_first_person_camera(
    trigger: On<Add, FirstPersonCamera>,
    mut commands: Commands,
    layers: Res<FirstPersonLayers>,
) {
    commands
        .entity(trigger.entity)
        .insert(RenderLayers::default().with(layers.first_person_only));
}

fn setup_third_person_camera(
    trigger: On<Add, ThirdPersonCamera>,
    mut commands: Commands,
    layers: Res<FirstPersonLayers>,
) {
    commands
        .entity(trigger.entity)
        .insert(RenderLayers::default().with(layers.third_person_only));
}

/// Applies first-person render layers to all meshes of the target VRM.
///
/// ```no_run
/// # use bevy::prelude::*;
/// # use bevy_vrm1::prelude::*;
/// fn enable(mut commands: Commands, vrm: Entity) {
///     commands.entity(vrm).trigger(RequestEnableFirstPerson);
/// }
/// ```
#[derive(EntityEvent)]
pub struct RequestEnableFirstPerson(pub Entity);

/// Makes all meshes of the target VRM visible to all cameras again.
#[derive(EntityEvent)]
pub struct RequestDisableFirstPerson(pub Entity);

/// A mesh entity waiting for the `auto` head split.
#[derive(Component)]
pub(crate) struct PendingAutoSplit;

/// Attached to the original mesh entity after the `auto` split.
#[derive(Component)]
pub(crate) struct AutoSplitDone {
    head_entity: Entity,
}

fn apply_enable_first_person(
    trigger: On<RequestEnableFirstPerson>,
    mut commands: Commands,
    registries: Query<&FirstPersonRegistry>,
    searcher: ChildSearcher,
    layers: Res<FirstPersonLayers>,
    children: Query<&Children>,
    meshes: Query<(), With<Mesh3d>>,
) {
    let vrm = trigger.event_target();
    let Ok(registry) = registries.get(vrm) else {
        return;
    };

    // Mesh entities already covered by an explicit annotation.
    let mut covered = HashSet::new();
    for (name, flag) in registry.iter() {
        let Some(node) = searcher.find_from_name(vrm, name.as_str()) else {
            continue;
        };
        for mesh_entity in descendants_with_mesh(node, &children, &meshes) {
            covered.insert(mesh_entity);
            apply_flag(&mut commands, mesh_entity, *flag, &layers);
        }
    }

    // Per spec: meshes without an annotation are treated as `auto`.
    for mesh_entity in descendants_with_mesh(vrm, &children, &meshes) {
        if !covered.contains(&mesh_entity) {
            apply_flag(&mut commands, mesh_entity, FirstPersonFlag::Auto, &layers);
        }
    }
}

fn apply_disable_first_person(
    trigger: On<RequestDisableFirstPerson>,
    mut commands: Commands,
    children: Query<&Children>,
    meshes: Query<(), With<Mesh3d>>,
) {
    let vrm = trigger.event_target();
    for mesh_entity in descendants_with_mesh(vrm, &children, &meshes) {
        commands
            .entity(mesh_entity)
            .insert(RenderLayers::default())
            .remove::<PendingAutoSplit>();
    }
}

fn apply_flag(
    commands: &mut Commands,
    mesh_entity: Entity,
    flag: FirstPersonFlag,
    layers: &FirstPersonLayers,
) {
    match flag {
        FirstPersonFlag::Both => {
            commands.entity(mesh_entity).insert(RenderLayers::default());
        }
        FirstPersonFlag::ThirdPersonOnly => {
            commands
                .entity(mesh_entity)
                .insert(RenderLayers::layer(layers.third_person_only));
        }
        FirstPersonFlag::FirstPersonOnly => {
            commands
                .entity(mesh_entity)
                .insert(RenderLayers::layer(layers.first_person_only));
        }
        FirstPersonFlag::Auto => {
            commands.entity(mesh_entity).insert(PendingAutoSplit);
        }
    }
}

fn split_auto_meshes(
    mut commands: Commands,
    mut mesh_assets: ResMut<Assets<Mesh>>,
    already_split: Query<(Entity, &AutoSplitDone), With<PendingAutoSplit>>,
    unskinned: Query<
        Entity,
        (
            With<PendingAutoSplit>,
            Without<SkinnedMesh>,
            Without<AutoSplitDone>,
        ),
    >,
    pending: Query<
        (
            Entity,
            &Mesh3d,
            &SkinnedMesh,
            &ChildOf,
            Option<&MeshMaterial3d<MToonMaterial>>,
            Option<&MeshMorphWeights>,
            Option<&Name>,
        ),
        (With<PendingAutoSplit>, Without<AutoSplitDone>),
    >,
    parents: Query<&ChildOf>,
    vrms: Query<&HeadBoneEntity, With<Vrm>>,
    children: Query<&Children>,
    layers: Res<FirstPersonLayers>,
) {
    // Re-enabling after a previous split: just restore the layers.
    for (entity, done) in already_split.iter() {
        commands
            .entity(entity)
            .insert(RenderLayers::default())
            .remove::<PendingAutoSplit>();
        commands
            .entity(done.head_entity)
            .insert(RenderLayers::layer(layers.third_person_only));
    }

    // Meshes without a skin cannot be weighted to the head bone: treat as `both`.
    for entity in unskinned.iter() {
        commands
            .entity(entity)
            .insert(RenderLayers::default())
            .remove::<PendingAutoSplit>();
    }

    for (entity, mesh3d, skinned, child_of, material, morph_weights, name) in pending.iter() {
        // The VRM root is the ancestor holding `Vrm` + `HeadBoneEntity`.
        // It may not be initialized yet; retry on the next frame.
        let Some(head) = find_head_bone(entity, &parents, &vrms) else {
            continue;
        };

        // The head bone and all of its descendants.
        let mut head_set = HashSet::new();
        collect_descendants(head, &children, &mut head_set);

        // Joint indices of this skin that point into the head subtree.
        let head_joint_ids = skinned
            .joints
            .iter()
            .enumerate()
            .filter(|(_, joint)| head_set.contains(*joint))
            .map(|(index, _)| index as u16)
            .collect::<HashSet<u16>>();

        // The mesh asset may not be loaded yet; retry on the next frame.
        let Some(mesh) = mesh_assets.get(&mesh3d.0) else {
            continue;
        };
        match classify_auto_mesh(mesh, &head_joint_ids) {
            // Nothing is weighted to the head: keep visible everywhere.
            AutoSplit::Both => {
                commands
                    .entity(entity)
                    .insert(RenderLayers::default())
                    .remove::<PendingAutoSplit>();
            }
            // The whole mesh belongs to the head (face, eyes, hair):
            // hide it from first-person cameras, no split needed.
            AutoSplit::ThirdPersonOnly => {
                commands
                    .entity(entity)
                    .insert(RenderLayers::layer(layers.third_person_only))
                    .remove::<PendingAutoSplit>();
            }
            // Mixed weights: split into a head part and the rest.
            AutoSplit::Split(parts) => {
                let (head_mesh, rest_mesh) = *parts;
                let head_handle = mesh_assets.add(head_mesh);
                let rest_handle = mesh_assets.add(rest_mesh);

                // The head part is a new sibling entity under the same glTF node,
                // visible only to third-person cameras.
                let head_name = name.map(Name::as_str).unwrap_or("mesh");
                let mut head_commands = commands.spawn((
                    Name::new(format!("{head_name}.headSplit")),
                    ChildOf(child_of.parent()),
                    Mesh3d(head_handle),
                    skinned.clone(),
                    RenderLayers::layer(layers.third_person_only),
                ));
                if let Some(material) = material {
                    head_commands.insert(material.clone());
                }
                if let Some(morph_weights) = morph_weights {
                    head_commands.insert(morph_weights.clone());
                }
                let head_entity = head_commands.id();

                // The original entity keeps only the non-head part and stays visible everywhere.
                commands
                    .entity(entity)
                    .insert((
                        Mesh3d(rest_handle),
                        RenderLayers::default(),
                        AutoSplitDone { head_entity },
                    ))
                    .remove::<PendingAutoSplit>();
            }
        }
    }
}

/// Walks up the hierarchy to the VRM root and returns its head bone entity.
fn find_head_bone(
    mesh_entity: Entity,
    parents: &Query<&ChildOf>,
    vrms: &Query<&HeadBoneEntity, With<Vrm>>,
) -> Option<Entity> {
    let mut current = mesh_entity;
    loop {
        if let Ok(head) = vrms.get(current) {
            return Some(head.0);
        }
        current = parents.get(current).ok()?.parent();
    }
}

fn collect_descendants(
    entity: Entity,
    children: &Query<&Children>,
    output: &mut HashSet<Entity>,
) {
    output.insert(entity);
    if let Ok(entity_children) = children.get(entity) {
        for child in entity_children {
            collect_descendants(*child, children, output);
        }
    }
}

/// Returns `root` and all of its descendants that have a `Mesh3d`.
fn descendants_with_mesh(
    root: Entity,
    children: &Query<&Children>,
    meshes: &Query<(), With<Mesh3d>>,
) -> Vec<Entity> {
    let mut found = Vec::new();
    let mut stack = vec![root];
    while let Some(entity) = stack.pop() {
        if meshes.contains(entity) {
            found.push(entity);
        }
        if let Ok(entity_children) = children.get(entity) {
            stack.extend(entity_children.iter().copied());
        }
    }
    found
}

/// Classification of an `auto` mesh relative to the head bone subtree.
enum AutoSplit {
    /// No triangles are weighted to the head: visible everywhere.
    Both,
    /// Every triangle is weighted to the head (face, eyes, hair):
    /// the whole mesh must be hidden from first-person cameras.
    ThirdPersonOnly,
    /// Mixed weights: (head part, rest part), split by triangle, index-only.
    Split(Box<(Mesh, Mesh)>),
}

fn classify_auto_mesh(
    mesh: &Mesh,
    head_joint_ids: &HashSet<u16>,
) -> AutoSplit {
    use bevy::mesh::VertexAttributeValues;

    if head_joint_ids.is_empty() {
        return AutoSplit::Both;
    }
    let Some(joint_indices) = read_joint_indices(mesh) else {
        return AutoSplit::Both;
    };
    let Some(VertexAttributeValues::Float32x4(joint_weights)) =
        mesh.attribute(Mesh::ATTRIBUTE_JOINT_WEIGHT)
    else {
        return AutoSplit::Both;
    };

    let is_head_vertex: Vec<bool> = joint_indices
        .iter()
        .zip(joint_weights)
        .map(|(idx, w)| (0..4).any(|k| w[k] > 0.0 && head_joint_ids.contains(&idx[k])))
        .collect();

    let Some(indices) = mesh.indices() else {
        return AutoSplit::Both;
    };
    let indices: Vec<u32> = indices.iter().map(|i| i as u32).collect();
    let (mut head, mut rest) = (Vec::new(), Vec::new());
    for tri in indices.as_chunks::<3>().0 {
        let target = if tri.iter().any(|&v| is_head_vertex[v as usize]) {
            &mut head
        } else {
            &mut rest
        };
        target.extend_from_slice(tri);
    }

    match (head.is_empty(), rest.is_empty()) {
        (true, _) => AutoSplit::Both,
        (false, true) => AutoSplit::ThirdPersonOnly,
        (false, false) => {
            let mut head_mesh = mesh.clone();
            head_mesh.insert_indices(Indices::U32(head));
            let mut rest_mesh = mesh.clone();
            rest_mesh.insert_indices(Indices::U32(rest));
            AutoSplit::Split(Box::new((head_mesh, rest_mesh)))
        }
    }
}

/// Reads `Mesh::ATTRIBUTE_JOINT_INDEX` as one row of four `u16` joint indices
/// per vertex, whatever integer format bevy stored the attribute in.
///
/// glTF permits `JOINTS_0` to be `UNSIGNED_BYTE` as well as `UNSIGNED_SHORT`, and
/// while bevy's glTF loader widens the former to `Uint16x4`, a `Mesh` assembled by
/// any other route can still hold `Uint8x4`. Requiring `Uint16x4` alone would
/// silently leave the head geometry visible from first-person cameras on those
/// avatars, so both spellings are widened here.
fn read_joint_indices(mesh: &Mesh) -> Option<Vec<[u16; 4]>> {
    use bevy::mesh::VertexAttributeValues;

    let values = mesh.attribute(Mesh::ATTRIBUTE_JOINT_INDEX)?;
    Some(match values {
        VertexAttributeValues::Uint16x4(rows) => rows.clone(),
        VertexAttributeValues::Uint8x4(rows) => rows
            .iter()
            .map(|row| {
                [
                    u16::from(row[0]),
                    u16::from(row[1]),
                    u16::from(row[2]),
                    u16::from(row[3]),
                ]
            })
            .collect(),
        unsupported => {
            let variant = joint_index_variant_name(unsupported);
            // The split retries on every frame until the skin resolves, so a bare
            // warning would flood the log. `once!` is the same one-shot guard that
            // `bevy::log::warn_once!` uses internally.
            bevy::utils::once!(vrm_warn!(
                "[VRMC_vrm.firstPerson] `Mesh::ATTRIBUTE_JOINT_INDEX` stored as `{variant}`; \
                 treating the mesh as `both` because its joint weights cannot be resolved"
            ));
            return None;
        }
    })
}

/// Names the `VertexAttributeValues` variant bevy stored the joint indices in,
/// so the diagnostic above identifies the offending format.
///
/// The match is exhaustive so that a `VertexAttributeValues` variant added by a
/// future bevy release is a compile error here rather than another silent skip.
fn joint_index_variant_name(values: &bevy::mesh::VertexAttributeValues) -> &'static str {
    use bevy::mesh::VertexAttributeValues as Values;

    match values {
        Values::Uint8(_) => "Uint8",
        Values::Uint8x2(_) => "Uint8x2",
        Values::Uint8x4(_) => "Uint8x4",
        Values::Sint8(_) => "Sint8",
        Values::Sint8x2(_) => "Sint8x2",
        Values::Sint8x4(_) => "Sint8x4",
        Values::Unorm8(_) => "Unorm8",
        Values::Unorm8x2(_) => "Unorm8x2",
        Values::Unorm8x4(_) => "Unorm8x4",
        Values::Snorm8(_) => "Snorm8",
        Values::Snorm8x2(_) => "Snorm8x2",
        Values::Snorm8x4(_) => "Snorm8x4",
        Values::Uint16(_) => "Uint16",
        Values::Uint16x2(_) => "Uint16x2",
        Values::Uint16x4(_) => "Uint16x4",
        Values::Sint16(_) => "Sint16",
        Values::Sint16x2(_) => "Sint16x2",
        Values::Sint16x4(_) => "Sint16x4",
        Values::Unorm16(_) => "Unorm16",
        Values::Unorm16x2(_) => "Unorm16x2",
        Values::Unorm16x4(_) => "Unorm16x4",
        Values::Snorm16(_) => "Snorm16",
        Values::Snorm16x2(_) => "Snorm16x2",
        Values::Snorm16x4(_) => "Snorm16x4",
        Values::Float16(_) => "Float16",
        Values::Float16x2(_) => "Float16x2",
        Values::Float16x4(_) => "Float16x4",
        Values::Float32(_) => "Float32",
        Values::Float32x2(_) => "Float32x2",
        Values::Float32x3(_) => "Float32x3",
        Values::Float32x4(_) => "Float32x4",
        Values::Uint32(_) => "Uint32",
        Values::Uint32x2(_) => "Uint32x2",
        Values::Uint32x3(_) => "Uint32x3",
        Values::Uint32x4(_) => "Uint32x4",
        Values::Sint32(_) => "Sint32",
        Values::Sint32x2(_) => "Sint32x2",
        Values::Sint32x3(_) => "Sint32x3",
        Values::Sint32x4(_) => "Sint32x4",
        Values::Float64(_) => "Float64",
        Values::Float64x2(_) => "Float64x2",
        Values::Float64x3(_) => "Float64x3",
        Values::Float64x4(_) => "Float64x4",
        Values::Unorm10_10_10_2(_) => "Unorm10_10_10_2",
        Values::Unorm8x4Bgra(_) => "Unorm8x4Bgra",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::success;
    use crate::tests::TestResult;
    use bevy::asset::RenderAssetUsages;
    use bevy::math::Vec3;
    use bevy::mesh::morph::MorphAttributes;
    use bevy::mesh::{MeshVertexAttribute, PrimitiveTopology, VertexAttributeValues, VertexFormat};

    /// `Mesh::ATTRIBUTE_JOINT_INDEX` occupies built-in attribute id 7: the
    /// built-in ids run upwards from 0 to `Mesh::FIRST_AVAILABLE_CUSTOM_ATTRIBUTE`,
    /// which is 8.
    const JOINT_INDEX_ID: u64 = 7;

    /// Six vertices, each weighted to a single joint: `0..3` to joint 0 and
    /// `3..6` to joint 1.
    const JOINT_INDICES: [[u16; 4]; 6] = [
        [0, 0, 0, 0],
        [0, 0, 0, 0],
        [0, 0, 0, 0],
        [1, 0, 0, 0],
        [1, 0, 0, 0],
        [1, 0, 0, 0],
    ];
    const JOINT_WEIGHTS: [[f32; 4]; 6] = [
        [1.0, 0.0, 0.0, 0.0],
        [1.0, 0.0, 0.0, 0.0],
        [1.0, 0.0, 0.0, 0.0],
        [1.0, 0.0, 0.0, 0.0],
        [1.0, 0.0, 0.0, 0.0],
        [1.0, 0.0, 0.0, 0.0],
    ];

    /// A comparable form of `AutoSplit`: `Split` carries the two index lists.
    #[derive(Debug, PartialEq)]
    enum Classification {
        Both,
        ThirdPersonOnly,
        Split(Vec<u32>, Vec<u32>),
    }

    fn head_joint_ids() -> HashSet<u16> {
        [0u16].into_iter().collect()
    }

    fn mesh_indices(mesh: &Mesh) -> Vec<u32> {
        mesh.indices()
            .map(|indices| indices.iter().map(|index| index as u32).collect())
            .unwrap_or_default()
    }

    fn classify(
        mesh: &Mesh,
        head_joint_ids: &HashSet<u16>,
    ) -> Classification {
        match classify_auto_mesh(mesh, head_joint_ids) {
            AutoSplit::Both => Classification::Both,
            AutoSplit::ThirdPersonOnly => Classification::ThirdPersonOnly,
            AutoSplit::Split(parts) => {
                let (head_mesh, rest_mesh) = *parts;
                Classification::Split(mesh_indices(&head_mesh), mesh_indices(&rest_mesh))
            }
        }
    }

    fn base_mesh(indices: &[u32]) -> Mesh {
        let mut mesh = Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::default(),
        );
        mesh.insert_attribute(
            Mesh::ATTRIBUTE_JOINT_WEIGHT,
            VertexAttributeValues::Float32x4(JOINT_WEIGHTS.to_vec()),
        );
        mesh.insert_indices(Indices::U32(indices.to_vec()));
        mesh
    }

    /// Describes the built-in joint index slot with an explicit format.
    ///
    /// `Mesh::insert_attribute` panics unless the values agree with the
    /// descriptor's format, so a mesh holding anything other than the declared
    /// `Uint16x4` can only be described through a descriptor that matches.
    fn joint_index_attribute(format: VertexFormat) -> MeshVertexAttribute {
        MeshVertexAttribute::new(Mesh::ATTRIBUTE_JOINT_INDEX.name, JOINT_INDEX_ID, format)
    }

    /// A mesh whose joint indices bevy stored as `Uint16x4`.
    fn mesh_uint16(indices: &[u32]) -> Mesh {
        let mut mesh = base_mesh(indices);
        mesh.insert_attribute(
            Mesh::ATTRIBUTE_JOINT_INDEX,
            VertexAttributeValues::Uint16x4(JOINT_INDICES.to_vec()),
        );
        mesh
    }

    /// The same mesh with its joint indices stored as `Uint8x4`.
    fn mesh_uint8(indices: &[u32]) -> Mesh {
        let mut mesh = base_mesh(indices);
        mesh.insert_attribute(
            joint_index_attribute(VertexFormat::Uint8x4),
            VertexAttributeValues::Uint8x4(
                JOINT_INDICES
                    .iter()
                    .map(|row| row.map(|index| index as u8))
                    .collect(),
            ),
        );
        mesh
    }

    #[test]
    fn classify_auto_mesh_splits_uint8_joint_indices() -> TestResult {
        let head = head_joint_ids();
        let expected = Classification::Split(vec![0, 1, 2], vec![3, 4, 5]);

        assert_eq!(classify(&mesh_uint16(&[0, 1, 2, 3, 4, 5]), &head), expected);
        assert_eq!(classify(&mesh_uint8(&[0, 1, 2, 3, 4, 5]), &head), expected);
        success!()
    }

    #[test]
    fn classify_auto_mesh_hides_head_only_uint8_joint_indices() -> TestResult {
        let head = head_joint_ids();

        assert_eq!(
            classify(&mesh_uint16(&[0, 1, 2, 0, 1, 2]), &head),
            Classification::ThirdPersonOnly
        );
        assert_eq!(
            classify(&mesh_uint8(&[0, 1, 2, 0, 1, 2]), &head),
            Classification::ThirdPersonOnly
        );
        success!()
    }

    #[test]
    fn classify_auto_mesh_keeps_body_only_uint8_joint_indices() -> TestResult {
        let head = head_joint_ids();

        assert_eq!(
            classify(&mesh_uint16(&[3, 4, 5, 3, 4, 5]), &head),
            Classification::Both
        );
        assert_eq!(
            classify(&mesh_uint8(&[3, 4, 5, 3, 4, 5]), &head),
            Classification::Both
        );
        success!()
    }

    #[test]
    fn classify_auto_mesh_ignores_mesh_without_head_joints() -> TestResult {
        let no_head_joints: HashSet<u16> = HashSet::default();

        assert_eq!(
            classify(&mesh_uint16(&[0, 1, 2, 3, 4, 5]), &no_head_joints),
            Classification::Both
        );
        assert_eq!(
            classify(&mesh_uint8(&[0, 1, 2, 3, 4, 5]), &no_head_joints),
            Classification::Both
        );
        success!()
    }

    #[test]
    fn classify_auto_mesh_skips_unsupported_joint_index_format() -> TestResult {
        let head = head_joint_ids();
        let mut mesh = base_mesh(&[0, 1, 2, 3, 4, 5]);
        mesh.insert_attribute(
            joint_index_attribute(VertexFormat::Float32x4),
            VertexAttributeValues::Float32x4(
                JOINT_INDICES
                    .iter()
                    .map(|row| row.map(|index| index as f32))
                    .collect(),
            ),
        );

        assert_eq!(classify(&mesh, &head), Classification::Both);
        success!()
    }

    #[test]
    fn classify_auto_mesh_split_preserves_topology_and_morph_targets() -> TestResult {
        let head = head_joint_ids();
        let mut mesh = mesh_uint8(&[0, 1, 2, 3, 4, 5]);
        mesh.set_morph_targets(vec![
            MorphAttributes::from([Vec3::X, Vec3::ZERO, Vec3::ZERO]),
            MorphAttributes::from([Vec3::ZERO, Vec3::ZERO, Vec3::ZERO]),
        ]);

        assert_eq!(
            classify(&mesh, &head),
            Classification::Split(vec![0, 1, 2], vec![3, 4, 5])
        );
        let AutoSplit::Split(parts) = classify_auto_mesh(&mesh, &head) else {
            panic!("expected the uint8 mesh to split");
        };
        for part in [&parts.0, &parts.1] {
            assert_eq!(part.primitive_topology(), PrimitiveTopology::TriangleList);
            assert_eq!(part.morph_targets().map(Vec::len), Some(2));
        }
        success!()
    }
}
