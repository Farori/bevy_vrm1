//! `on_scene_completed`: the whole VRM initialization, run once per glTF scene
//! before the scratch world is frozen into a
//! [`WorldAsset`](bevy::world_serialization::WorldAsset).
//!
//! `bevy_gltf` calls this hook at `crates/bevy_gltf/src/loader/mod.rs:1112-1120`
//! and only then runs `WorldAsset::new(world)` at `:1122`, so everything written
//! here lands in the asset and is copied — through reflection, entity fields
//! remapped — into every `SceneRoot` instance of it.
//!
//! # Order
//!
//! 1. the forward-orientation policy, which is *classified* here and never
//!    applied (see [`crate::vrm::coords`]);
//! 2. rest poses, because the spring-bone joint state is derived from them;
//! 3. spring-bone chains and colliders;
//! 4. node constraints: node indices resolved to entities and sorted;
//! 5. expressions and look-at on the scene root;
//! 6. animation targets and the player;
//! 7. firstPerson render layers and the head copies;
//! 8. the `Initialized` marker, which is what the rest of the crate treats as
//!    "this scene is a fully initialized VRM".

use bevy::animation::{AnimatedBy, AnimationPlayer};
use bevy::asset::{Handle, LoadContext};
use bevy::camera::primitives::Aabb;
use bevy::camera::visibility::{DynamicSkinnedMeshBounds, NoFrustumCulling, RenderLayers};
use bevy::ecs::entity::Entity;
use bevy::ecs::world::World;
use bevy::mesh::morph::{MeshMorphWeights, MorphWeights};
use bevy::mesh::skinning::SkinnedMesh;
use bevy::mesh::{Mesh, Mesh3d};
use bevy::pbr::{MeshMaterial3d, StandardMaterial};
use bevy::platform::collections::HashMap;
use bevy::prelude::*;

use super::first_person::AutoClass;
use super::{VrmLoadState, bone_target_id};
use crate::error::vrm_warn;
use crate::prelude::{
    ConstraintExecutionOrder, ExpressionMorphBinds, ExpressionSetting, ExpressionSettings,
    Initialized, MToonMaterial, MorphBind, PendingNodeConstraint, RestGlobalTransform,
    RestTransform, RestWorldTransform, Vrm, VrmExpressionWeights, VrmHeadOnly, VrmNodeConstraint,
    both_view_mesh_layers, first_person_only_mesh_layers, third_person_only_mesh_layers,
    vrm_root_animation_target,
};
use crate::vrm::expressions::{ExpressionCategory, ExpressionOverrideType};
use crate::vrm::gltf::extensions::vrmc_spring_bone::ColliderShape;
use crate::vrm::gltf::extensions::vrmc_vrm::FirstPersonFlag;
use crate::vrm::spring_bone::{
    SpringCenterNode, SpringColliders, SpringJointProps, SpringJointState, SpringJoints, SpringRoot,
};
use crate::vrma::RetargetSource;

pub(crate) fn finalize(
    state: &mut VrmLoadState,
    load_context: &mut LoadContext<'_>,
    _scene: &gltf::Scene,
    world_root_id: Entity,
    scene_world: &mut World,
) {
    // The forward policy is decided in `on_root` and deliberately not acted on:
    // `bevy_gltf` already rotated the scene root entity before this hook runs
    // (`loader/mod.rs:1027-1040`), so rest poses captured now include that
    // rotation and a second one would double it.
    insert_rest_transforms(scene_world, world_root_id);
    build_spring_chains(state, scene_world);
    resolve_constraints(state, scene_world, world_root_id);
    build_expressions(state, scene_world, world_root_id);
    build_look_at(state, scene_world, world_root_id);
    setup_animation(state, scene_world, world_root_id);
    apply_first_person(state, load_context, scene_world);

    scene_world.entity_mut(world_root_id).insert(Initialized);
}

/// Snapshots the rest pose of the whole scene.
///
/// Three components, because three consumers read three different frames:
///
/// * [`RestTransform`] / [`RestGlobalTransform`] on every entity with a
///   `Transform` — the local and world pose a bone returns to, used by node
///   constraints and gaze control;
/// * [`RestWorldTransform`] on the scene root — the model entity's own world
///   transform at snapshot time. The VRMA retarget strips this prefix from both
///   sides (`vrma::animation::bone_rotation::strip_world_prefix`) so a
///   retargeted pose does not depend on where the application placed the
///   avatar.
///
/// `GlobalTransform` is not propagated inside the loader's scratch world, so the
/// globals are composed by walking down from the root.
fn insert_rest_transforms(
    world: &mut World,
    root: Entity,
) {
    fn walk(
        world: &mut World,
        entity: Entity,
        parent_global: GlobalTransform,
        out: &mut Vec<(Entity, Transform, GlobalTransform)>,
    ) {
        let Some(local) = world.get::<Transform>(entity).copied() else {
            return;
        };
        let global = parent_global.mul_transform(local);
        out.push((entity, local, global));
        let children: Vec<Entity> = world
            .get::<Children>(entity)
            .map(|children| children.iter().collect())
            .unwrap_or_default();
        for child in children {
            walk(world, child, global, out);
        }
    }

    let mut poses = Vec::new();
    walk(world, root, GlobalTransform::IDENTITY, &mut poses);

    let root_global = poses
        .first()
        .map(|(_, _, global)| *global)
        .unwrap_or(GlobalTransform::IDENTITY);
    for (entity, local, global) in poses {
        world
            .entity_mut(entity)
            .insert((RestTransform(local), RestGlobalTransform(global)));
    }
    world
        .entity_mut(root)
        .insert(RestWorldTransform(root_global));
}

/// Builds the spring-bone chains from the crate's runtime components.
///
/// The runtime systems (`spring_bone::update`) simulate any chain that carries
/// `SpringRoot` + `SpringJointProps` + `SpringJointState`, so building all three
/// here leaves `VrmSpringBonePlugin` with nothing to set up.
fn build_spring_chains(
    state: &VrmLoadState,
    world: &mut World,
) {
    let Some(spring_bone) = state.spring_bone.as_ref() else {
        return;
    };

    // Resolve the colliders once: spec array index -> (entity, shape). A
    // collider node that was never spawned is dropped with a warning; the
    // remaining colliders of the group still apply.
    let mut resolved_colliders: Vec<Option<(Entity, ColliderShape)>> = Vec::new();
    for collider in &spring_bone.colliders {
        let resolved = match state.node_entities.get(&collider.node).copied() {
            Some(entity) => Some((entity, collider.shape)),
            None => {
                vrm_warn!(format!(
                    "VRM springBone: collider refers to a missing node {}, skipping it",
                    collider.node
                ));
                None
            }
        };
        resolved_colliders.push(resolved);
    }

    for spring in &spring_bone.springs {
        // Resolve the joints; a chain that names a node nobody spawned cannot be
        // simulated, so it is skipped as a whole.
        let mut joints: Vec<Entity> = Vec::with_capacity(spring.joints.len());
        let mut props: Vec<SpringJointProps> = Vec::with_capacity(spring.joints.len());
        let mut broken = false;
        for joint in &spring.joints {
            let Some(&entity) = state.node_entities.get(&joint.node) else {
                vrm_warn!(format!(
                    "VRM springBone: joint refers to a missing node {}, skipping the chain",
                    joint.node
                ));
                broken = true;
                break;
            };
            joints.push(entity);
            // The schema materialises the specification defaults for every
            // omitted property (`dragForce` 0.5, `gravityDir` [0,-1,0],
            // `gravityPower` 0.0, `hitRadius` 0.0, `stiffness` 1.0), so a joint
            // only reads as `None` when the file wrote an explicit `null`.
            props.push(SpringJointProps {
                drag_force: joint.drag_force.unwrap_or(0.5),
                gravity_dir: joint.gravity_dir.map(Vec3::from).unwrap_or(Vec3::NEG_Y),
                gravity_power: joint.gravity_power.unwrap_or(0.0),
                hit_radius: joint.hit_radius.unwrap_or(0.0),
                stiffness: joint.stiffness.unwrap_or(1.0),
            });
        }
        if broken {
            continue;
        }
        if joints.len() < 2 {
            vrm_warn!(format!(
                "VRM springBone: spring `{}` has fewer than two joints; skipped",
                spring.name
            ));
            continue;
        }

        let center = spring
            .center
            .and_then(|node| state.node_entities.get(&node).copied());
        let center_rest =
            center.and_then(|entity| world.get::<RestGlobalTransform>(entity).copied());

        for pair in 0..joints.len() - 1 {
            let head = joints[pair];
            let tail = joints[pair + 1];
            let (Some(head_rest_global), Some(tail_rest_global), Some(head_rest_local)) = (
                world.get::<RestGlobalTransform>(head).copied(),
                world.get::<RestGlobalTransform>(tail).copied(),
                world.get::<RestTransform>(head).copied(),
            ) else {
                vrm_warn!(
                    "VRM springBone: a joint of the chain has no rest pose; the chain is skipped"
                );
                break;
            };
            // `reparented_to` instead of reading the tail's local translation:
            // a spring chain is allowed to skip nodes, so the next joint is not
            // necessarily a direct child of this one.
            let tail_in_head = tail_rest_global.0.reparented_to(&head_rest_global.0);
            // The tail is simulated in *center space* when the spring declares
            // one, and in world space otherwise.
            let tail_position = match center_rest {
                Some(center_rest) => tail_rest_global.0.reparented_to(&center_rest.0).translation,
                None => tail_rest_global.0.translation(),
            };
            world.entity_mut(head).insert((
                props[pair],
                SpringJointState::from_rest(
                    tail_in_head.translation.normalize_or_zero(),
                    tail_in_head.translation.length(),
                    tail_position,
                    head_rest_local.0,
                ),
            ));
        }
        // The last joint carries its properties but simulates no pair of its
        // own, because it is the tail of the last segment.
        if let (Some(&last), Some(&last_props)) = (joints.last(), props.last()) {
            world.entity_mut(last).insert(last_props);
        }

        let mut colliders: Vec<(Entity, ColliderShape)> = Vec::new();
        for group_index in spring.collider_groups.iter().flatten() {
            let Some(group) = spring_bone.collider_groups.get(*group_index) else {
                vrm_warn!(format!(
                    "VRM springBone: colliderGroups index {group_index} is out of range; \
                     the group is skipped"
                ));
                continue;
            };
            for collider_index in &group.colliders {
                if let Some(Some((entity, shape))) =
                    resolved_colliders.get(*collider_index as usize)
                {
                    colliders.push((*entity, *shape));
                }
            }
        }

        world.entity_mut(joints[0]).insert(SpringRoot {
            joints: SpringJoints(joints.clone()),
            colliders: SpringColliders(colliders),
            center_node: SpringCenterNode(center),
        });
    }
}

/// Turns the [`PendingNodeConstraint`]s into resolved [`VrmNodeConstraint`]s and
/// sorts the scene's constraint graph.
///
/// The pending component is **removed** again: it exists only because the source
/// node may not have been visited when the destination was, and it would
/// otherwise ship inside the scene asset with node indices that mean nothing in
/// an instantiated copy.
fn resolve_constraints(
    state: &VrmLoadState,
    world: &mut World,
    root: Entity,
) {
    let mut pendings: Vec<(Entity, PendingNodeConstraint)> = Vec::new();
    {
        let mut query = world.query::<(Entity, &PendingNodeConstraint)>();
        for (destination, pending) in query.iter(world) {
            pendings.push((destination, *pending));
        }
    }
    // Sorted by entity so the topological order below is reproducible: query
    // iteration order is not.
    pendings.sort_by_key(|(entity, _)| *entity);

    let mut constraints: Vec<(Entity, Entity)> = Vec::new();
    for (destination, pending) in pendings {
        let mut entity = world.entity_mut(destination);
        entity.remove::<PendingNodeConstraint>();
        let Some(&source) = state.node_entities.get(&pending.source_node) else {
            vrm_warn!(format!(
                "VRM node_constraint: the source node {} was not found; the constraint is \
                     skipped",
                pending.source_node
            ));
            continue;
        };
        entity.insert(VrmNodeConstraint {
            source,
            weight: pending.weight,
            kind: pending.kind,
        });
        constraints.push((destination, source));
    }

    world
        .entity_mut(root)
        .insert(ConstraintExecutionOrder(toposort_constraints(&constraints)));
}

/// Kahn's algorithm over the constraints of one scene.
///
/// A node is a graph vertex when it is the *destination* of some constraint;
/// there is an edge `source -> destination` when the source is itself a
/// destination, i.e. when evaluating the destination requires the source's
/// rotation to be final.
///
/// The specification forbids cycles. One is reported and the remaining
/// constraints are appended unordered rather than dropped, so a malformed
/// exporter costs ordering, not behaviour.
fn toposort_constraints(constraints: &[(Entity, Entity)]) -> Vec<Entity> {
    let destination_index: HashMap<Entity, usize> = constraints
        .iter()
        .enumerate()
        .map(|(index, (destination, _))| (*destination, index))
        .collect();
    let mut indegree = vec![0usize; constraints.len()];
    let mut outgoing: Vec<Vec<usize>> = vec![Vec::new(); constraints.len()];
    for (index, (_, source)) in constraints.iter().enumerate() {
        if let Some(&source_index) = destination_index.get(source) {
            outgoing[source_index].push(index);
            indegree[index] += 1;
        }
    }

    let mut ready: Vec<usize> = (0..constraints.len())
        .filter(|index| indegree[*index] == 0)
        .collect();
    let mut order = Vec::with_capacity(constraints.len());
    while let Some(index) = ready.pop() {
        order.push(constraints[index].0);
        for &next in &outgoing[index] {
            let Some(degree) = indegree.get_mut(next) else {
                continue;
            };
            *degree -= 1;
            if *degree == 0 {
                ready.push(next);
            }
        }
    }

    if order.len() < constraints.len() {
        vrm_warn!(
            "VRM node_constraint: the constraints of this scene contain a dependency cycle; \
             the remaining constraints are evaluated unordered"
        );
        for (index, (destination, _)) in constraints.iter().enumerate() {
            if indegree[index] > 0 {
                order.push(*destination);
            }
        }
    }
    order
}

/// Builds the expression components on the scene root.
///
/// Preset **and** custom expressions are both included: `Expressions::custom`
/// is a first-class part of `VRMC_vrm.expressions` and upstream's runtime path
/// silently dropped it, which lost every custom expression of an avatar.
fn build_expressions(
    state: &VrmLoadState,
    world: &mut World,
    root: Entity,
) {
    let Some(vrm) = &state.vrm else {
        return;
    };
    let Some(expressions) = &vrm.expressions else {
        return;
    };

    let mut weights = VrmExpressionWeights::default();
    let mut binds = ExpressionMorphBinds::default();
    let mut settings = ExpressionSettings::default();

    let preset = expressions
        .preset
        .iter()
        .map(|(name, expression)| (name, expression, true));
    let custom = expressions
        .custom
        .iter()
        .map(|(name, expression)| (name, expression, false));

    for (name, expression, is_preset) in preset.chain(custom) {
        let mut bind_list = Vec::new();
        for bind in expression.morph_target_binds.iter().flatten() {
            let Some(&target) = state.node_entities.get(&bind.node) else {
                vrm_warn!(format!(
                    "VRM expressions: a morph bind of `{name}` refers to the missing node {}",
                    bind.node
                ));
                continue;
            };
            // The bind drives a morph index of a mesh, and only a node that
            // carries `MorphWeights` has any. `bevy_gltf` puts
            // `MorphWeights` on the node and `MeshMorphWeights` on each of its
            // primitives (`loader/mod.rs:1688-1692`, `:1872-1886`), so this is
            // also the check that the node has a mesh at all.
            if !world.get::<MorphWeights>(target).is_some() {
                vrm_warn!(format!(
                    "VRM expressions: node {} of the morph binds of `{name}` carries no morph \
                     weights; the bind is skipped",
                    bind.node
                ));
                continue;
            }
            bind_list.push(MorphBind {
                target,
                index: bind.index,
                weight: bind.weight,
            });
        }
        if !expression.material_color_binds.is_empty()
            || !expression.texture_transform_binds.is_empty()
        {
            // Both bind kinds are parsed by the schema and kept, so a follow-up
            // can apply them without another migration; nothing applies them
            // today, and pretending otherwise would be worse than saying so.
            vrm_warn!(format!(
                "VRM expressions: the material color and texture transform binds of `{name}` \
                 are parsed but not applied"
            ));
        }

        weights.0.insert(name.clone(), 0.0);
        binds.0.insert(name.clone(), bind_list);
        settings.0.insert(
            name.clone(),
            ExpressionSetting {
                is_binary: expression.is_binary,
                category: if is_preset {
                    ExpressionCategory::from_preset_name(name)
                } else {
                    ExpressionCategory::Other
                },
                override_blink: ExpressionOverrideType::parse(&expression.override_blink),
                override_look_at: ExpressionOverrideType::parse(&expression.override_look_at),
                override_mouth: ExpressionOverrideType::parse(&expression.override_mouth),
            },
        );
    }

    world.entity_mut(root).insert((weights, binds, settings));
}

/// Puts the parsed `VRMC_vrm.lookAt` on the scene root.
///
/// The component is the crate's existing [`LookAtProperties`], with the
/// specification defaults already applied by its `Deserialize`
/// (`RangeMap` defaults to `inputMaxValue` 90.0 / `outputScale` 10.0, `type` to
/// `bone`), so there is exactly one look-at component in the crate.
fn build_look_at(
    state: &VrmLoadState,
    world: &mut World,
    root: Entity,
) {
    let Some(vrm) = &state.vrm else {
        return;
    };
    let Some(look_at) = vrm.look_at.clone() else {
        return;
    };

    world.entity_mut(root).insert(look_at);
}

/// Attaches the animation targets and the player.
///
/// # Bone-name ids, not glTF path ids
///
/// Every humanoid bone gets
/// [`AnimationTargetId::from_name`] of its **VRM bone name**, which is the id a
/// baked `.vrma` clip addresses, so one clip plays on any avatar. That is a
/// different id from the path-based one `bevy_gltf` computes for a file's own
/// animations (`loader/mod.rs:1557-1560`); a `.vrm` declares no glTF animation in
/// practice, so no path id exists to clobber, and the two systems are separated
/// by `AnimatedBy` anyway — `bevy_gltf`'s targets point at the animation root it
/// picked, ours point at the scene root.
///
/// # Why the player sits on the scene root
///
/// `bevy_gltf` puts its own `AnimationPlayer` on each *animation root* node
/// (`loader/mod.rs:1089-1095`). That is a different entity from the scene root,
/// and it only exists for files with glTF animations. Putting the VRM player on
/// the scene root instead gives one player per avatar instance — which is what
/// `AnimatedBy(root)` and every mask-group lookup expects — and lets the VRMA
/// commit attach one `AnimationGraphHandle` to it.
///
/// The scene root also carries the synthetic target
/// [`vrm_root_animation_target`], so curves that address the avatar as a whole
/// (expression weights) have something to bind to.
fn setup_animation(
    state: &VrmLoadState,
    world: &mut World,
    root: Entity,
) {
    for (bone, node) in &state.bone_nodes {
        let Some(&entity) = state.node_entities.get(node) else {
            continue;
        };
        world
            .entity_mut(entity)
            .insert((bone_target_id(bone), AnimatedBy(root), RetargetSource));
    }

    world.entity_mut(root).insert((
        vrm_root_animation_target(),
        AnimatedBy(root),
        AnimationPlayer::default(),
    ));

    // The root bone is the parent of `hips`, exactly as the legacy path defines
    // it (`src/vrm/humanoid_bone.rs:154-168`): `ChildSearcher::find_root_bone`
    // and the VRMA retarget both look it up by `Vrm::ROOT_BONE`.
    let hips = state
        .bone_nodes
        .get("hips")
        .and_then(|node| state.node_entities.get(node))
        .copied();
    if let Some(root_bone) = hips.and_then(|hips| world.get::<ChildOf>(hips).map(ChildOf::parent)) {
        world.entity_mut(root_bone).insert((
            Name::new(Vrm::ROOT_BONE),
            AnimationTransitions::default(),
            RetargetSource,
        ));
    }
}

/// Applies the `VRMC_vrm.firstPerson` decisions to the scene.
///
/// Three passes, in this order:
///
/// 1. the per-primitive `auto` classifications recorded while the meshes were
///    built;
/// 2. the explicit node annotations, which *override* the `auto` result — an
///    `auto` annotation is skipped here, because it has already been honoured per
///    primitive and forcing "both" on the whole subtree would cancel it;
/// 3. the head copies, spawned as siblings of the body half.
///
/// Render layers come from [`crate::vrm::components`]; no layer number is
/// hardcoded here.
fn apply_first_person(
    state: &VrmLoadState,
    load_context: &mut LoadContext<'_>,
    world: &mut World,
) {
    apply_first_person_layers(state, world);
    spawn_head_copies(state, load_context, world);
}

/// Passes 1 and 2 of [`apply_first_person`].
fn apply_first_person_layers(
    state: &VrmLoadState,
    world: &mut World,
) {
    for (key, class) in &state.first_person.classes {
        let Some(&entity) = state.primitive_entities.get(key) else {
            continue;
        };
        let layers = match class {
            AutoClass::ThirdPersonOnly => third_person_only_mesh_layers(),
            // A split primitive's body half, and a primitive nothing is weighted
            // to the head by.
            AutoClass::Split | AutoClass::Both => both_view_mesh_layers(),
        };
        world.entity_mut(entity).insert(layers);
    }

    let Some(vrm) = &state.vrm else {
        return;
    };
    let Some(first_person) = &vrm.first_person else {
        return;
    };
    for annotation in &first_person.mesh_annotations {
        let layers = match annotation.first_person_flag {
            FirstPersonFlag::FirstPersonOnly => first_person_only_mesh_layers(),
            FirstPersonFlag::ThirdPersonOnly => third_person_only_mesh_layers(),
            FirstPersonFlag::Both => both_view_mesh_layers(),
            // Handled per primitive in pass 1 and 3.
            FirstPersonFlag::Auto => continue,
        };
        let Some(&entity) = state.node_entities.get(&annotation.node) else {
            vrm_warn!(format!(
                "VRM firstPerson: meshAnnotations names the missing node {}",
                annotation.node
            ));
            continue;
        };
        insert_layers_recursive(world, entity, layers);
    }
}

/// Spawns pass 3 of [`apply_first_person`].
///
/// The copy inherits everything that makes the body half render: the transform,
/// the skin, the material, the morph-weight reference and the bounds. Only
/// `RenderLayers` differs.
fn spawn_head_copies(
    state: &VrmLoadState,
    load_context: &mut LoadContext<'_>,
    world: &mut World,
) {
    for (key, label) in &state.first_person.head_mesh_labels {
        let Some(&source) = state.primitive_entities.get(key) else {
            vrm_warn!(format!(
                "VRM firstPerson: no entity was recorded for mesh {} primitive {}, so its head \
                 part is not spawned",
                key.0, key.1
            ));
            continue;
        };
        let Some(parent) = world.get::<ChildOf>(source).map(ChildOf::parent) else {
            vrm_warn!(format!(
                "VRM firstPerson: the body half of mesh {} primitive {} has no parent",
                key.0, key.1
            ));
            continue;
        };
        let head_mesh: Handle<Mesh> = load_context.get_label_handle(label.clone());
        let name = world
            .get::<Name>(source)
            .map(|name| format!("{name}/VrmHeadOnly"))
            .unwrap_or_else(|| "VrmHeadOnly".to_owned());
        let transform = world.get::<Transform>(source).copied().unwrap_or_default();
        let visibility = world.get::<Visibility>(source).copied().unwrap_or_default();

        // Everything the body half needs in order to render at all. Read before
        // spawning: `World::spawn` mutably borrows the world for as long as the
        // returned `EntityWorldMut` lives.
        //
        // `MeshMorphWeights` is what drives the node's `MorphWeights`, so the
        // head half has to reference the same node or its expressions do nothing.
        let morph_weights = world.get::<MeshMorphWeights>(source).cloned();
        let skinned = world.get::<SkinnedMesh>(source).cloned();
        // The source's `Aabb` is a superset of the head half's, which only costs
        // a little frustum-culling precision; the skinned-mesh bounds markers
        // come along because they are what makes those bounds track the animated
        // pose at all.
        let aabb = world.get::<Aabb>(source).copied();
        let dynamic_bounds = world.get::<DynamicSkinnedMeshBounds>(source).is_some();
        let no_frustum_culling = world.get::<NoFrustumCulling>(source).is_some();
        let material = world.get::<MeshMaterial3d<MToonMaterial>>(source).cloned();
        let fallback_material = world
            .get::<MeshMaterial3d<StandardMaterial>>(source)
            .cloned();

        let mut head = world.spawn((
            Name::new(name),
            Mesh3d(head_mesh),
            transform,
            visibility,
            ChildOf(parent),
            VrmHeadOnly,
            third_person_only_mesh_layers(),
        ));
        if let Some(morph_weights) = morph_weights {
            head.insert(morph_weights);
        }
        if let Some(skinned) = skinned {
            head.insert(skinned);
        }
        if let Some(aabb) = aabb {
            head.insert(aabb);
        }
        if let Some(material) = material {
            head.insert(material);
        } else if let Some(material) = fallback_material {
            head.insert(material);
        }
        if dynamic_bounds {
            head.insert(DynamicSkinnedMeshBounds);
        }
        if no_frustum_culling {
            head.insert(NoFrustumCulling);
        }
    }
}

/// `RenderLayers` is not inherited, so it is inserted on every `Mesh3d` entity of
/// the annotated subtree.
fn insert_layers_recursive(
    world: &mut World,
    entity: Entity,
    layers: RenderLayers,
) {
    let children: Vec<Entity> = world
        .get::<Children>(entity)
        .map(|children| children.iter().collect())
        .unwrap_or_default();
    if world.get::<Mesh3d>(entity).is_some() {
        world.entity_mut(entity).insert(layers.clone());
    }
    for child in children {
        insert_layers_recursive(world, child, layers.clone());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prelude::VrmConstraintKind;
    use crate::success;
    use crate::tests::TestResult;
    use crate::vrm::gltf::extensions::vrmc_vrm::VrmcVrm;
    use bevy::animation::AnimationTargetId;

    /// `VrmcVrm` parsed from a JSON literal, so the tests below exercise the
    /// same schema the loader reads.
    fn vrm(json: &str) -> std::sync::Arc<VrmcVrm> {
        std::sync::Arc::new(serde_json::from_str(json).expect("the fixture should parse"))
    }

    fn state_with(vrm: std::sync::Arc<VrmcVrm>) -> VrmLoadState {
        VrmLoadState {
            vrm: Some(vrm),
            ..Default::default()
        }
    }

    fn node(index: usize) -> Entity {
        Entity::from_raw_u32(index as u32).expect("a valid entity index")
    }

    /// A scene: `root -> child -> grandchild`, each with a `Transform`.
    fn scene_world() -> (World, Entity, Entity, Entity) {
        let mut world = World::new();
        let root = world.spawn(Transform::from_xyz(1.0, 0.0, 0.0)).id();
        let child = world
            .spawn((Transform::from_xyz(0.0, 2.0, 0.0), ChildOf(root)))
            .id();
        let grandchild = world
            .spawn((Transform::from_xyz(0.0, 0.0, 3.0), ChildOf(child)))
            .id();
        (world, root, child, grandchild)
    }

    /// The VRMA retarget strips `RestWorldTransform` from both sides, so all
    /// three rest components have to exist and agree with each other.
    #[test]
    fn rest_transforms_cover_the_whole_scene() -> TestResult {
        let (mut world, root, child, grandchild) = scene_world();

        insert_rest_transforms(&mut world, root);

        assert_eq!(
            world.get::<RestTransform>(root).map(|rest| rest.0),
            Some(Transform::from_xyz(1.0, 0.0, 0.0))
        );
        assert_eq!(
            world.get::<RestGlobalTransform>(child).map(|rest| rest.0),
            Some(GlobalTransform::from(Transform::from_xyz(1.0, 2.0, 0.0)))
        );
        assert_eq!(
            world
                .get::<RestGlobalTransform>(grandchild)
                .map(|rest| rest.0),
            Some(GlobalTransform::from(Transform::from_xyz(1.0, 2.0, 3.0)))
        );
        // The model's own world transform, so a retarget does not depend on
        // where the application placed the avatar.
        assert_eq!(
            world.get::<RestWorldTransform>(root).map(|rest| rest.0),
            world.get::<RestGlobalTransform>(root).map(|rest| rest.0)
        );
        success!()
    }

    /// Sources are evaluated before the destinations that read them.
    #[test]
    fn constraints_are_sorted_source_before_destination() -> TestResult {
        // `(destination, source)`: `a` reads `b`, `b` reads `c`, `c` reads `d`,
        // and `d` is not a destination itself, so the graph is a chain.
        let (a, b, c, d) = (node(1), node(2), node(3), node(4));
        let order = toposort_constraints(&[(a, b), (b, c), (c, d)]);

        let position = |entity: Entity| {
            order
                .iter()
                .position(|candidate| *candidate == entity)
                .unwrap_or_else(|| panic!("every constraint is listed"))
        };
        assert!(position(b) < position(a), "a reads b");
        assert!(position(c) < position(b), "b reads c");
        assert!(
            !order.contains(&d),
            "only destinations are listed; a source that nothing constrains is not"
        );
        success!()
    }

    /// The specification forbids cycles. One is reported, not dropped: the
    /// remaining constraints are appended unordered.
    #[test]
    fn a_constraint_cycle_still_lists_every_destination() -> TestResult {
        let (a, b) = (node(1), node(2));
        let order = toposort_constraints(&[(a, b), (b, a)]);

        assert_eq!(order.len(), 2, "neither constraint is dropped");
        success!()
    }

    /// The loader-only marker must not survive into the scene asset, and the
    /// resolved constraint must point at an entity of the same world.
    #[test]
    fn pending_constraints_are_resolved_and_removed() -> TestResult {
        let mut world = World::new();
        let root = world.spawn_empty().id();
        let source = world.spawn_empty().id();
        let destination = world.spawn_empty().id();
        world.entity_mut(destination).insert(PendingNodeConstraint {
            source_node: 7,
            weight: 0.5,
            kind: VrmConstraintKind::Aim {
                aim_axis: Dir3::NEG_Z,
            },
        });
        let state = VrmLoadState {
            node_entities: [(7, source)].into_iter().collect(),
            ..Default::default()
        };

        resolve_constraints(&state, &mut world, root);

        assert!(
            world.get::<PendingNodeConstraint>(destination).is_none(),
            "the pending marker would ship inside the scene asset"
        );
        let constraint = world
            .get::<VrmNodeConstraint>(destination)
            .expect("the constraint is resolved");
        assert_eq!(constraint.source, source);
        assert_eq!(constraint.weight, 0.5);
        assert_eq!(
            world
                .get::<ConstraintExecutionOrder>(root)
                .map(|order| order.destinations()),
            Some([destination].as_slice())
        );

        // A constraint whose source node was never spawned is dropped, and the
        // pending marker still goes away.
        let orphan = world.spawn_empty().id();
        world.entity_mut(orphan).insert(PendingNodeConstraint {
            source_node: 999,
            weight: 1.0,
            kind: VrmConstraintKind::Rotation,
        });
        resolve_constraints(&state, &mut world, root);
        assert!(world.get::<PendingNodeConstraint>(orphan).is_none());
        assert!(world.get::<VrmNodeConstraint>(orphan).is_none());

        success!()
    }

    /// Preset *and* custom expressions reach the root, and a bind is only kept
    /// when its node can actually carry `MorphWeights`.
    #[test]
    fn expressions_include_custom_and_require_morph_weights() -> TestResult {
        let mut world = World::new();
        let root = world.spawn_empty().id();
        let preset_target = world.spawn(MorphWeights::new(vec![0.0; 3], None)?).id();
        let custom_target = world.spawn(MorphWeights::new(vec![0.0; 2], None)?).id();
        // A node without `MorphWeights`: its bind must be skipped.
        let weightless = world.spawn_empty().id();
        let state = state_with(vrm(r#"{
                "specVersion": "1.0",
                "humanoid": {"humanBones": {}},
                "expressions": {
                    "preset": {"blink": {"isBinary": true, "morphTargetBinds": [
                        {"node": 1, "index": 0, "weight": 1.0},
                        {"node": 3, "index": 0, "weight": 1.0}
                    ]}},
                    "custom": {"Wave": {"morphTargetBinds": [
                        {"node": 2, "index": 1, "weight": 0.5}
                    ]}}
                }
            }"#));
        let state = VrmLoadState {
            node_entities: [(1, preset_target), (2, custom_target), (3, weightless)]
                .into_iter()
                .collect(),
            ..state
        };

        build_expressions(&state, &mut world, root);

        let weights = world
            .get::<VrmExpressionWeights>(root)
            .expect("the root carries the expression weights");
        assert_eq!(weights.0.get("blink"), Some(&0.0));
        assert_eq!(
            weights.0.get("Wave"),
            Some(&0.0),
            "`Expressions::custom` is part of the specification and is not dropped"
        );

        let binds = world
            .get::<ExpressionMorphBinds>(root)
            .expect("the root carries the morph binds");
        assert_eq!(binds.0["blink"].len(), 1, "node 3 has no `MorphWeights`");
        assert_eq!(binds.0["blink"][0].target, preset_target);
        assert_eq!(binds.0["Wave"][0].target, custom_target);
        assert_eq!(binds.0["Wave"][0].weight, 0.5);

        let settings = world
            .get::<ExpressionSettings>(root)
            .expect("the root carries the per-expression settings");
        assert!(settings.0["blink"].is_binary);
        assert_eq!(
            settings.0["blink"].category,
            ExpressionCategory::Blink,
            "a preset name is classified"
        );
        assert_eq!(
            settings.0["Wave"].category,
            ExpressionCategory::Other,
            "a custom expression is not a preset category"
        );

        success!()
    }

    /// `overrideBlink` / `overrideLookAt` / `overrideMouth` are spec strings and
    /// are parsed by the schema already.
    #[test]
    fn expression_overrides_are_typed() -> TestResult {
        let mut world = World::new();
        let root = world.spawn_empty().id();
        let state = state_with(vrm(r#"{
                "specVersion": "1.0",
                "humanoid": {"humanBones": {}},
                "expressions": {"preset": {"happy": {"overrideMouth": "block"}}}
            }"#));

        build_expressions(&state, &mut world, root);

        let settings = world.get::<ExpressionSettings>(root).unwrap();
        assert_eq!(
            settings.0["happy"].override_mouth,
            ExpressionOverrideType::Block
        );
        assert_eq!(
            settings.0["happy"].override_blink,
            ExpressionOverrideType::None
        );
        success!()
    }

    /// The scene root is an animation target under the synthetic name, and the
    /// `hips` bone under its VRM bone name.
    #[test]
    fn animation_targets_are_attached_to_the_root_and_the_bones() -> TestResult {
        let mut world = World::new();
        let root = world.spawn_empty().id();
        let hips = world.spawn(ChildOf(root)).id();
        let state = VrmLoadState {
            bone_nodes: [("hips".to_owned(), 0)].into_iter().collect(),
            node_entities: [(0, hips)].into_iter().collect(),
            ..Default::default()
        };

        setup_animation(&state, &mut world, root);

        assert!(world.get::<AnimationPlayer>(root).is_some());
        assert_eq!(
            world.get::<AnimationTargetId>(root).copied(),
            Some(vrm_root_animation_target()),
            "root-level curves address the avatar through a synthetic target"
        );
        assert_eq!(
            world.get::<AnimationTargetId>(hips).copied(),
            Some(bone_target_id("hips")),
            "a bone is addressed by its VRM bone name, so one clip retargets to any avatar"
        );
        assert_eq!(
            world.get::<AnimatedBy>(hips).map(|animated| animated.0),
            Some(root),
            "both are driven by the scene root's player"
        );
        // `ChildSearcher::find_root_bone` and the VRMA retarget look the root
        // bone up by this name.
        assert_eq!(
            world.get::<Name>(root).map(|name| name.as_str()),
            Some(Vrm::ROOT_BONE)
        );
        assert!(world.get::<AnimationTransitions>(root).is_some());
        success!()
    }

    /// An `auto` annotation must not force "both" on the whole subtree: it has
    /// already been honoured per primitive.
    #[test]
    fn auto_annotations_do_not_override_the_per_primitive_classification() -> TestResult {
        let mut world = World::new();
        let root = world.spawn_empty().id();
        let annotated = world.spawn(ChildOf(root)).id();
        let head_only = world
            .spawn((Mesh3d(Handle::default()), ChildOf(annotated)))
            .id();
        let mut first_person = state_with(vrm(r#"{
                "specVersion": "1.0",
                "humanoid": {"humanBones": {}},
                "firstPerson": {"meshAnnotations": [{"node": 1, "firstPersonFlag": "auto"}]}
            }"#));
        first_person.node_entities = [(1, annotated)].into_iter().collect();
        first_person
            .first_person
            .classes
            .insert((0, 0), AutoClass::ThirdPersonOnly);
        first_person.primitive_entities = [((0, 0), head_only)].into_iter().collect();

        apply_first_person_layers(&first_person, &mut world);

        assert_eq!(
            world.get::<RenderLayers>(head_only),
            Some(&third_person_only_mesh_layers()),
            "the primitive classification survives an `auto` annotation"
        );
        success!()
    }
}
