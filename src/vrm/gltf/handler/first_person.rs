//! `on_gltf_primitive`: the `VRMC_vrm.firstPerson` classification and the
//! `auto` split.
//!
//! # What the specification asks for
//!
//! [`VRMC_vrm.firstPerson`](https://github.com/vrm-c/vrm-specification/blob/master/specification/VRMC_vrm-1.0/firstPerson.md)
//! gives every mesh one of four annotations, and states that **a mesh without an
//! annotation is `auto`**. `auto` means: look at which bones the mesh's vertices
//! are weighted to, hide from a first-person camera whatever hangs off the head
//! bone, and keep the rest.
//!
//! # Why the split happens here
//!
//! A split has to change the `Mesh` asset itself, and `user_mesh` is the only
//! hook that can. The hook is called once per `(mesh, primitive)`, *before*
//! `bevy_gltf` reads the vertices
//! (`crates/bevy_gltf/src/loader/mod.rs:733-817`), which is why the joints and
//! weights are read here from the buffer data rather than from a built `Mesh`.
//! The head half is registered as a labeled asset here and spawned as a
//! sibling in [`apply_first_person`](super::scene), because the mesh entities
//! only exist by the time `on_scene_completed` runs.
//!
//! The label is added with the *root* `LoadContext` and read back with the scene
//! one; that works because `LoadContext::begin_labeled_asset`
//! (`bevy_asset/src/loader.rs:441`) keeps `asset_path` unchanged, so both
//! contexts resolve the same asset path.
//!
//! # Morph targets
//!
//! `user_mesh` replaces the whole mesh, and `bevy_gltf` only reads
//! `morph_targets` in the `else` branch (`loader/mod.rs:794-815`) — a handler
//! that supplies a mesh silently loses them. Rather than lose a morph-target
//! mesh's expressions, the morph deltas are read from the same buffers and
//! rebuilt for the compacted vertex list. If that cannot be done exactly, the
//! primitive is **not** split and is classified [`AutoClass::Both`], which is
//! the conservative direction: hiding geometry that turns out to be the body
//! would make an avatar lose its torso.

use bevy::asset::{Handle, LoadContext, RenderAssetUsages};
use bevy::math::Vec3;
use bevy::mesh::morph::MorphAttributes;
use bevy::mesh::{Indices, Mesh, PrimitiveTopology, VertexAttributeValues};
use bevy::platform::collections::{HashMap, HashSet};

use super::VrmLoadState;
use crate::error::vrm_warn;
use crate::vrm::gltf::extensions::vrmc_vrm::FirstPersonFlag;

/// Classification/split results keyed by (glTF mesh index, primitive index).
#[derive(Clone, Default)]
pub struct FirstPersonPlan {
    /// How each primitive ended up being classified.
    pub classes: HashMap<(usize, usize), AutoClass>,
    /// Label of the head sub-mesh asset, for the primitives that were split.
    /// The head copy is spawned in [`apply_first_person`](super::scene).
    pub head_mesh_labels: HashMap<(usize, usize), String>,
}

/// What `firstPerson` decided about one primitive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AutoClass {
    /// Visible in both views: layer 0.
    Both,
    /// Entirely on head bones: hidden from a first-person camera.
    ThirdPersonOnly,
    /// Mixed: split into a body part (both) and a head part (third-person only).
    Split,
}

/// The glTF `Mode` -> bevy topology mapping, mirroring `bevy_gltf`'s
/// `loader/gltf_ext/mesh.rs::primitive_topology`.
///
/// Returns `None` for the modes bevy rejects outright, in which case the
/// primitive is left alone.
fn primitive_topology(mode: gltf::mesh::Mode) -> Option<PrimitiveTopology> {
    match mode {
        gltf::mesh::Mode::Points => Some(PrimitiveTopology::PointList),
        gltf::mesh::Mode::Lines => Some(PrimitiveTopology::LineList),
        gltf::mesh::Mode::LineStrip => Some(PrimitiveTopology::LineStrip),
        gltf::mesh::Mode::Triangles => Some(PrimitiveTopology::TriangleList),
        gltf::mesh::Mode::TriangleStrip => Some(PrimitiveTopology::TriangleStrip),
        _ => None,
    }
}

pub(crate) fn process_primitive(
    state: &mut VrmLoadState,
    load_context: &mut LoadContext<'_>,
    gltf_document: &gltf::Gltf,
    gltf_mesh: &gltf::Mesh,
    gltf_primitive: &gltf::Primitive,
    buffer_data: &[Vec<u8>],
    user_mesh: &mut Option<Mesh>,
) {
    // An unannotated mesh is `auto`, so only the head subtree can stop the work
    // here: without it every mesh stays `both`.
    if state.head_subtree.is_empty() {
        return;
    }
    if !is_auto(state, gltf_document, gltf_mesh.index()) {
        return;
    }

    let key = (gltf_mesh.index(), gltf_primitive.index());
    let Some(class) = classify(
        state,
        gltf_document,
        gltf_mesh.index(),
        gltf_primitive,
        buffer_data,
    ) else {
        return;
    };
    state.first_person.classes.insert(key, class);
    if class != AutoClass::Split {
        return;
    }

    let Some(parts) = split(state, gltf_document, gltf_mesh, gltf_primitive, buffer_data) else {
        // The classification was `Split` but the geometry could not be divided.
        // Falling back to `Both` keeps the face visible in both views, which is
        // the conservative direction: hiding geometry that turns out to be the
        // body would make an avatar lose its torso.
        state.first_person.classes.insert(key, AutoClass::Both);
        return;
    };
    let (body, head) = parts;

    // The body half replaces the primitive bevy is about to build.
    *user_mesh = Some(body);

    let label = format!("Mesh{}/Primitive{}/VrmHeadOnly", key.0, key.1);
    load_context.add_labeled_asset::<Mesh>(label.clone(), head);
    state.first_person.head_mesh_labels.insert(key, label);
}

/// Whether this mesh is `auto`: either the specification default for a mesh
/// without an annotation, or an explicit `auto` annotation on a node that
/// instances it.
///
/// The experiment only accepted the explicit form, which silently skipped every
/// `auto` mesh of the many avatars that do not annotate their meshes at all.
fn is_auto(
    state: &VrmLoadState,
    gltf_document: &gltf::Gltf,
    mesh_index: usize,
) -> bool {
    let Some(vrm) = &state.vrm else {
        return false;
    };
    let Some(first_person) = &vrm.first_person else {
        // "when the `firstPerson` property itself does not exist […] you must
        // assume all meshes are annotated as `auto`"
        return true;
    };
    if first_person.mesh_annotations.is_empty() {
        return true;
    }
    let nodes: Vec<_> = gltf_document.document.nodes().collect();
    first_person.mesh_annotations.iter().any(|annotation| {
        annotation.first_person_flag == FirstPersonFlag::Auto
            && nodes
                .get(annotation.node)
                .and_then(|node| node.mesh())
                .map(|mesh| mesh.index())
                == Some(mesh_index)
    })
}

/// Splits the primitive's vertices by which triangles are weighted to the head
/// subtree.
///
/// `None` when the classification cannot be made — no skin, no joints, or a
/// joint format the mesh cannot be read from — in which case the primitive keeps
/// bevy's own mesh and stays visible in both views.
fn classify(
    state: &VrmLoadState,
    gltf_document: &gltf::Gltf,
    mesh_index: usize,
    gltf_primitive: &gltf::Primitive,
    buffer_data: &[Vec<u8>],
) -> Option<AutoClass> {
    let head_slots = head_joint_slots(state, gltf_document, mesh_index)?;
    if head_slots.is_empty() {
        return Some(AutoClass::Both);
    }
    let (joints, weights) = read_skin(gltf_primitive, buffer_data)?;
    let head_vertex: Vec<bool> = joints
        .iter()
        .zip(&weights)
        .map(|(joint, weight)| {
            joint
                .iter()
                .zip(weight)
                .any(|(slot, w)| *w > 0.0 && head_slots.contains(slot))
        })
        .collect();
    Some(
        match head_vertex.iter().filter(|is_head| **is_head).count() {
            0 => AutoClass::Both,
            count if count == head_vertex.len() => AutoClass::ThirdPersonOnly,
            _ => AutoClass::Split,
        },
    )
}

/// The joint slots of the mesh's skin that point into the head subtree.
///
/// The skin is indexed by **mesh index**, not looked up from "the first node
/// that references this mesh": see
/// [`collect_mesh_skins`](super::root::collect_mesh_skins) for why the hook
/// cannot see the entity's own node and how an ambiguous mesh is handled.
fn head_joint_slots(
    state: &VrmLoadState,
    gltf_document: &gltf::Gltf,
    mesh_index: usize,
) -> Option<HashSet<u16>> {
    let skin_index = state.mesh_skins.get(&mesh_index).copied().flatten()?;
    let skin = gltf_document.document.skins().nth(skin_index)?;
    Some(
        skin.joints()
            .enumerate()
            .filter(|(_, joint)| state.head_subtree.contains(&joint.index()))
            .map(|(slot, _)| slot as u16)
            .collect(),
    )
}

/// `JOINTS_0` and `WEIGHTS_0` as `u16` joint slots and `f32` weights.
///
/// `into_u16` widens the glTF-permitted `UNSIGNED_BYTE` layout as well, so a
/// file that stores `JOINTS_0` compactly is classified like any other instead of
/// falling back to `Both`.
fn read_skin<'r>(
    gltf_primitive: &gltf::Primitive<'r>,
    buffer_data: &[Vec<u8>],
) -> Option<(Vec<[u16; 4]>, Vec<[f32; 4]>)> {
    let reader = gltf_primitive.reader(|buffer| buffer_data.get(buffer.index()).map(Vec::as_slice));
    let (Some(joints), Some(weights)) = (reader.read_joints(0), reader.read_weights(0)) else {
        return None;
    };
    let joints: Vec<[u16; 4]> = joints.into_u16().collect();
    let weights: Vec<[f32; 4]> = weights.into_f32().collect();
    if joints.len() != weights.len() {
        vrm_warn!(format!(
            "VRM firstPerson: JOINTS_0 has {} entries and WEIGHTS_0 has {}; treating the mesh as \
             visible in both views",
            joints.len(),
            weights.len()
        ));
        return None;
    }
    Some((joints, weights))
}

/// Per-vertex data of one primitive, plus the per-vertex head flag.
struct PrimitiveData<'a> {
    topology: PrimitiveTopology,
    positions: &'a [[f32; 3]],
    normals: Option<&'a [[f32; 3]]>,
    tex_coords_0: Option<&'a [[f32; 2]]>,
    tex_coords_1: Option<&'a [[f32; 2]]>,
    colors: Option<&'a [[f32; 4]]>,
    tangents: Option<&'a [[f32; 4]]>,
    joints: &'a [[u16; 4]],
    weights: &'a [[f32; 4]],
    morph_targets: Vec<(Option<Vec<Vec3>>, Option<Vec<Vec3>>, Option<Vec<Vec3>>)>,
    indices: Vec<u32>,
    head_vertex: Vec<bool>,
}

/// Reads the primitive and divides it into `(body, head)`.
///
/// Both halves keep the source topology, every attribute the source carried, and
/// the morph targets; only the index list and the vertex set shrink, which is
/// what makes the split lossless.
fn split(
    state: &VrmLoadState,
    gltf_document: &gltf::Gltf,
    gltf_mesh: &gltf::Mesh,
    gltf_primitive: &gltf::Primitive,
    buffer_data: &[Vec<u8>],
) -> Option<(Mesh, Mesh)> {
    let topology = primitive_topology(gltf_primitive.mode())?;
    let head_slots = head_joint_slots(state, gltf_document, gltf_mesh.index())?;
    let (joints, weights) = read_skin(gltf_primitive, buffer_data)?;

    let reader = gltf_primitive.reader(|buffer| buffer_data.get(buffer.index()).map(Vec::as_slice));
    let positions: Vec<[f32; 3]> = reader.read_positions()?.collect();
    let normals: Option<Vec<[f32; 3]>> = reader.read_normals().map(|values| values.collect());
    let tex_coords_0: Option<Vec<[f32; 2]>> =
        reader.read_tex_coords(0).map(|uv| uv.into_f32().collect());
    let tex_coords_1: Option<Vec<[f32; 2]>> =
        reader.read_tex_coords(1).map(|uv| uv.into_f32().collect());
    let colors: Option<Vec<[f32; 4]>> = reader
        .read_colors(0)
        .map(|values| values.into_rgba_f32().collect());
    let tangents: Option<Vec<[f32; 4]>> = reader.read_tangents().map(|values| values.collect());
    let indices: Vec<u32> = match reader.read_indices() {
        Some(read) => read.into_u32().collect(),
        None => (0..positions.len() as u32).collect(),
    };
    // Morph deltas are per-vertex, exactly like the base attributes.
    let morph_targets: Vec<_> = reader
        .read_morph_targets()
        .map(|(positions, normals, tangents)| {
            (
                positions.map(|d| d.map(Vec3::from).collect::<Vec<_>>()),
                normals.map(|d| d.map(Vec3::from).collect::<Vec<_>>()),
                tangents.map(|d| d.map(Vec3::from).collect::<Vec<_>>()),
            )
        })
        .collect();

    let head_vertex: Vec<bool> = joints
        .iter()
        .zip(&weights)
        .map(|(joint, weight)| {
            joint
                .iter()
                .zip(weight)
                .any(|(slot, w)| *w > 0.0 && head_slots.contains(slot))
        })
        .collect();

    let data = PrimitiveData {
        topology,
        positions: &positions,
        normals: normals.as_deref(),
        tex_coords_0: tex_coords_0.as_deref(),
        tex_coords_1: tex_coords_1.as_deref(),
        colors: colors.as_deref(),
        tangents: tangents.as_deref(),
        joints: &joints,
        weights: &weights,
        morph_targets,
        indices,
        head_vertex,
    };

    let body = build_submesh(&data, false)?;
    let head = build_submesh(&data, true)?;

    // `bevy_gltf` copies the target names out of the mesh's `extras`
    // (`loader/mod.rs:808-814`); without them `MeshMorphWeights` loses the
    // names the expression system looks morph targets up by.
    let names = gltf_mesh
        .extras()
        .as_ref()
        .and_then(|extras| serde_json::from_str::<MorphTargetNames>(extras.get()).ok());
    let names = names.map(|names| names.target_names);

    Some((
        with_morph_target_names(body, names.clone()),
        with_morph_target_names(head, names),
    ))
}

/// `targetNames` of a glTF mesh's `extras`, mirroring bevy's private
/// `MorphTargetNames`.
#[derive(serde::Deserialize)]
struct MorphTargetNames {
    #[serde(rename = "targetNames")]
    target_names: Vec<String>,
}

fn with_morph_target_names(
    mut mesh: Mesh,
    names: Option<Vec<String>>,
) -> Mesh {
    if let Some(names) = names {
        // `set_morph_target_names` panics on a length mismatch, and a name list
        // that does not match the target count is malformed exporter output.
        if mesh
            .morph_targets()
            .is_some_and(|targets| targets.len() == names.len())
        {
            mesh.set_morph_target_names(names);
        } else {
            vrm_warn!(
                "VRM firstPerson: `extras.targetNames` does not match the number of morph \
                 targets; the split mesh keeps its targets unnamed"
            );
        }
    }
    mesh
}

/// One source vertex, gathered for the compacted vertex list.
///
/// The channels the primitive does not carry stay `None` and are not written to
/// the sub-mesh, so the split never invents an attribute. A channel that *is*
/// carried but shorter than the position array is a malformed accessor: that is
/// reported by the `None` from [`compact_vertex`], never by silently padding.
struct CompactedVertex {
    mapped: u32,
    position: [f32; 3],
    normals: Option<[f32; 3]>,
    tex_coord_0: Option<[f32; 2]>,
    tex_coord_1: Option<[f32; 2]>,
    color: Option<[f32; 4]>,
    tangent: Option<[f32; 4]>,
    joint: [u16; 4],
    weight: [f32; 4],
}

/// Copies one source vertex into the compacted lists and records its new index.
fn compact_vertex(
    data: &PrimitiveData<'_>,
    index: u32,
    sources: &mut Vec<usize>,
    remap: &mut HashMap<u32, u32>,
) -> Option<CompactedVertex> {
    let source = index as usize;
    let mapped = sources.len() as u32;
    remap.insert(index, mapped);
    sources.push(source);
    Some(CompactedVertex {
        mapped,
        position: *data.positions.get(source)?,
        normals: element(data.normals, source)?,
        tex_coord_0: element(data.tex_coords_0, source)?,
        tex_coord_1: element(data.tex_coords_1, source)?,
        color: element(data.colors, source)?,
        tangent: element(data.tangents, source)?,
        joint: *data.joints.get(source)?,
        weight: *data.weights.get(source)?,
    })
}

/// One element of an optional vertex channel.
///
/// `Some(None)` is "the primitive does not carry this channel", `Some(Some(v))`
/// is the element, and `None` is "the channel is there but too short", i.e.
/// malformed exporter output. Collapsing the last case into the first would let
/// a broken attribute array through as a *missing* one.
fn element<T: Copy>(
    values: Option<&[T]>,
    index: usize,
) -> Option<Option<T>> {
    match values {
        None => Some(None),
        Some(values) => values.get(index).copied().map(Some),
    }
}

/// Partitions the triangles by head membership and compacts the vertices each
/// half actually uses.
///
/// Returns `None` when the primitive has fewer than three indices per triangle,
/// or an attribute array that disagrees with the positions — the caller then
/// leaves the primitive unsplit.
fn build_submesh(
    data: &PrimitiveData<'_>,
    keep_head: bool,
) -> Option<Mesh> {
    // Triangles are consumed three at a time. Anything else (a line list, a
    // point cloud) has no such notion, so it is not divided.
    if !data.indices.len().is_multiple_of(3) {
        return None;
    }

    // Source vertex -> destination vertex, and the source indices in output
    // order (needed to rebuild the per-vertex morph deltas).
    let mut remap: HashMap<u32, u32> = HashMap::default();
    let mut sources: Vec<usize> = Vec::new();
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut tex_coords_0 = Vec::new();
    let mut tex_coords_1 = Vec::new();
    let mut colors = Vec::new();
    let mut tangents = Vec::new();
    let mut joints = Vec::new();
    let mut weights = Vec::new();
    let mut indices: Vec<u32> = Vec::new();

    for triangle in data.indices.as_chunks::<3>().0 {
        let is_head = triangle.iter().any(|index| {
            data.head_vertex
                .get(*index as usize)
                .copied()
                .unwrap_or(false)
        });
        if is_head != keep_head {
            continue;
        }
        for &index in triangle {
            if let Some(&mapped) = remap.get(&index) {
                indices.push(mapped);
                continue;
            }
            // A vertex index with no position - or a shorter attribute array than
            // the position one - is a malformed accessor. The whole split is
            // refused rather than half-applied.
            let compacted = compact_vertex(data, index, &mut sources, &mut remap)?;
            positions.push(compacted.position);
            if let Some(value) = compacted.normals {
                normals.push(value);
            }
            if let Some(value) = compacted.tex_coord_0 {
                tex_coords_0.push(value);
            }
            if let Some(value) = compacted.tex_coord_1 {
                tex_coords_1.push(value);
            }
            if let Some(value) = compacted.color {
                colors.push(value);
            }
            if let Some(value) = compacted.tangent {
                tangents.push(value);
            }
            joints.push(compacted.joint);
            weights.push(compacted.weight);
            indices.push(compacted.mapped);
        }
    }

    let mut mesh = Mesh::new(
        data.topology,
        // `on_gltf_primitive` does not receive the loader settings, and
        // `RenderAssetUsages::default()` is exactly
        // `GltfLoaderSettings::load_meshes`'s default
        // (`bevy_gltf/src/loader/mod.rs:223`).
        RenderAssetUsages::default(),
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    if !normals.is_empty() {
        mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    }
    if !tex_coords_0.is_empty() {
        mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, tex_coords_0);
    }
    if !tex_coords_1.is_empty() {
        mesh.insert_attribute(Mesh::ATTRIBUTE_UV_1, tex_coords_1);
    }
    if !colors.is_empty() {
        mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
    }
    if !tangents.is_empty() {
        mesh.insert_attribute(Mesh::ATTRIBUTE_TANGENT, tangents);
    }
    mesh.insert_attribute(
        Mesh::ATTRIBUTE_JOINT_INDEX,
        VertexAttributeValues::Uint16x4(joints),
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_JOINT_WEIGHT, weights);
    if !indices.is_empty() {
        mesh.insert_indices(Indices::U32(indices));
    }

    if !data.morph_targets.is_empty() {
        // The morph buffer is weight-major:
        // `morph_targets[target * vertex_count + vertex]`
        // (`crates/bevy_pbr/src/render/morph.wgsl`, `get_morph_target`), which
        // is the order `bevy_gltf` produces by walking the vertices of one
        // target at a time (`crates/bevy_gltf/src/loader/mod.rs:797-807`).
        let vertex_count = sources.len();
        let mut rebuilt = Vec::with_capacity(data.morph_targets.len() * vertex_count);
        for (position, normal, tangent) in &data.morph_targets {
            for source in &sources {
                rebuilt.push(MorphAttributes::new(
                    component(position, *source),
                    component(normal, *source),
                    component(tangent, *source),
                ));
            }
        }
        mesh.set_morph_targets(rebuilt);
    }

    Some(mesh)
}

/// One component of one vertex's morph-target displacement.
///
/// A channel the target does not declare reads as zero, exactly as
/// `bevy_gltf`'s `PrimitiveMorphAttributesIter::next` fills it.
fn component(
    source: &Option<Vec<Vec3>>,
    index: usize,
) -> Vec3 {
    source
        .as_ref()
        .and_then(|values| values.get(index))
        .copied()
        .unwrap_or(Vec3::ZERO)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::success;
    use crate::tests::TestResult;

    /// Two triangles: `0,1,2` weighted to joint 0 and `3,4,5` weighted to joint
    /// 1, with normals, a uv set and one morph target on top.
    ///
    /// `PrimitiveData` borrows, so the owned vectors live in the fixture and the
    /// borrowed view is handed out by [`Fixture::primitive`].
    struct Fixture {
        positions: Vec<[f32; 3]>,
        normals: Vec<[f32; 3]>,
        tex_coords_0: Vec<[f32; 2]>,
        joints: Vec<[u16; 4]>,
        weights: Vec<[f32; 4]>,
        morph_targets: Vec<(Option<Vec<Vec3>>, Option<Vec<Vec3>>, Option<Vec<Vec3>>)>,
        /// Truncated normals, used by the malformed-input test.
        short_normals: bool,
    }

    impl Fixture {
        fn new() -> Self {
            let positions: Vec<[f32; 3]> = (0..6).map(|i| [i as f32, 0.0, 0.0]).collect();
            let joints: Vec<[u16; 4]> = vec![
                [0, 0, 0, 0],
                [0, 0, 0, 0],
                [0, 0, 0, 0],
                [1, 0, 0, 0],
                [1, 0, 0, 0],
                [1, 0, 0, 0],
            ];
            let weights: Vec<[f32; 4]> = joints
                .iter()
                .map(|joint| {
                    let mut weight = [0.0; 4];
                    weight[joint[0] as usize] = 1.0;
                    weight
                })
                .collect();
            Self {
                normals: positions.iter().map(|_| [0.0, 1.0, 0.0]).collect(),
                tex_coords_0: positions.iter().map(|p| [p[0], 0.0]).collect(),
                positions,
                joints,
                weights,
                morph_targets: vec![(Some(vec![Vec3::X; 6]), Some(vec![Vec3::Y; 6]), None)],
                short_normals: false,
            }
        }

        fn primitive(&self) -> PrimitiveData<'_> {
            let normals: &[[f32; 3]] = if self.short_normals {
                &self.normals[..2]
            } else {
                &self.normals
            };
            PrimitiveData {
                // A strip, not a triangle list: the experiment hardcoded
                // `TriangleList`.
                topology: PrimitiveTopology::TriangleStrip,
                positions: &self.positions,
                normals: Some(normals),
                tex_coords_0: Some(&self.tex_coords_0),
                tex_coords_1: None,
                colors: None,
                tangents: None,
                joints: &self.joints,
                weights: &self.weights,
                morph_targets: self.morph_targets.clone(),
                indices: vec![0, 1, 2, 3, 4, 5],
                head_vertex: vec![false, false, false, true, true, true],
            }
        }
    }

    fn indices_of(mesh: &Mesh) -> Vec<u32> {
        mesh.indices()
            .map(|indices| indices.iter().map(|index| index as u32).collect())
            .unwrap_or_default()
    }

    fn first_joint(mesh: &Mesh) -> u16 {
        match mesh.attribute(Mesh::ATTRIBUTE_JOINT_INDEX) {
            Some(VertexAttributeValues::Uint16x4(rows)) => rows[0][0],
            _ => panic!("the joint indices are kept as `Uint16x4`"),
        }
    }

    /// The split is lossless: both halves keep the source topology, every
    /// attribute the source had, and the morph targets - only the vertex set and
    /// the index list shrink.
    #[test]
    fn the_split_keeps_topology_attributes_and_morph_targets() -> TestResult {
        let fixture = Fixture::new();
        let data = fixture.primitive();

        let body = build_submesh(&data, false).expect("the body half splits");
        let head = build_submesh(&data, true).expect("the head half splits");

        for mesh in [&body, &head] {
            assert_eq!(mesh.primitive_topology(), PrimitiveTopology::TriangleStrip);
            assert_eq!(
                mesh.count_vertices(),
                3,
                "each half keeps only the vertices it uses"
            );
            assert_eq!(indices_of(mesh), vec![0, 1, 2]);
            for attribute in [
                Mesh::ATTRIBUTE_POSITION,
                Mesh::ATTRIBUTE_NORMAL,
                Mesh::ATTRIBUTE_UV_0,
                Mesh::ATTRIBUTE_JOINT_INDEX,
                Mesh::ATTRIBUTE_JOINT_WEIGHT,
            ] {
                assert_eq!(
                    mesh.attribute(attribute).map(|values| values.len()),
                    Some(3),
                    "`{attribute:?}` is carried over"
                );
            }
            // The morph buffer is weight-major and one entry per vertex, so one
            // target over three vertices is three entries.
            let targets = mesh
                .morph_targets()
                .expect("the morph targets are carried over");
            assert_eq!(targets.len(), 3);
            assert_eq!(targets[0].position, Vec3::X);
            assert_eq!(targets[0].normal, Vec3::Y);
            assert_eq!(targets[0].tangent, Vec3::ZERO);
        }

        // The joint weights decide which half a triangle belongs to.
        assert_eq!(first_joint(&body), 0);
        assert_eq!(first_joint(&head), 1);
        success!()
    }

    /// An attribute that is shorter than the position array is malformed
    /// exporter output; the split is refused rather than half-applied, which is
    /// what makes the caller fall back to `Both`.
    #[test]
    fn a_short_attribute_array_refuses_the_split() -> TestResult {
        let mut fixture = Fixture::new();
        fixture.short_normals = true;
        let data = fixture.primitive();

        assert!(build_submesh(&data, false).is_none());
        assert!(build_submesh(&data, true).is_none());
        success!()
    }

    /// An index list that is not made of whole triangles is not divided at all.
    #[test]
    fn an_index_list_that_is_not_triangles_refuses_the_split() -> TestResult {
        let mut fixture = Fixture::new();
        let mut data = fixture.primitive();
        // A borrow-safe copy is not needed: the fixture outlives `data`, so only
        // the index list is replaced.
        let fixture2 = Fixture::new();
        let mut data2 = fixture2.primitive();
        data2.indices = vec![0, 1];
        drop(data);

        assert!(build_submesh(&data2, false).is_none());
        // The fixture is still usable, so the test does not leak a borrow.
        assert_eq!(fixture.morph_targets.len(), 1);
        success!()
    }
}
