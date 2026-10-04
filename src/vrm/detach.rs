use crate::vrm::body_tracking::{BodyTracking, SmoothedGaze};
use crate::vrm::components::{
    ConstraintExecutionOrder, PendingNodeConstraint, VrmHeadOnly, VrmNodeConstraint, VrmNodeIndex,
};
use crate::vrm::gltf::extensions::vrmc_vrm::LookAtProperties;
use crate::vrm::look_at::LookAt;
use crate::vrm::spring_bone::{SpringJointProps, SpringJointState, SpringRoot};
use crate::vrm::{
    Initialized, RestGlobalTransform, RestTransform, RestWorldTransform, Vrm, VrmBone, VrmPath,
};
use crate::vrma::animation::prelude::{
    ExpressionMorphBinds, ExpressionSettings, VrmExpressionWeights,
};
use bevy::prelude::*;
use bevy::world_serialization::{WorldAsset, WorldAssetRoot};

/// Triggers VRM detachment on the target entity.
///
/// Removes all VRM-related components and despawns the child hierarchy,
/// leaving the root entity itself alive.
///
/// # Which entity is "the VRM root"
///
/// Trigger this on the scene root the load-time pipeline created — the entity
/// carrying [`Vrm`] and [`Initialized`], which is instantiated as a *child* of
/// the [`WorldAssetRoot`] entity the application spawned
/// (`bevy_world_serialization`'s `set_instance_parent_sync`).
/// `handler::scene::mark_initialized` is what wrote both markers on it.
/// Triggering this on the [`WorldAssetRoot`] entity itself finds no [`Vrm`]
/// there and is a no-op; [`spawn_vrm`](crate::prelude::spawn_vrm) hands you
/// the right entity in its `configure` closure.
///
/// # Usage
///
/// ```no_run
/// # use bevy::prelude::*;
/// # use bevy_vrm1::vrm::detach::RequestDetachVrm;
/// fn detach(mut commands: Commands, vrm_entity: Entity) {
///     commands.entity(vrm_entity).trigger(RequestDetachVrm);
/// }
/// ```
#[derive(EntityEvent)]
pub struct RequestDetachVrm(pub Entity);

pub(crate) struct VrmDetachPlugin;

impl Plugin for VrmDetachPlugin {
    fn build(
        &self,
        app: &mut App,
    ) {
        app.add_observer(apply_detach_vrm);
    }
}

fn apply_detach_vrm(
    trigger: On<RequestDetachVrm>,
    mut commands: Commands,
    children_query: Query<&Children>,
    vrm_check: Query<(), With<Vrm>>,
) {
    let entity = trigger.event_target();

    // `Vrm` is what a pipeline scene root carries (`handler::scene`'s
    // `mark_initialized`), and it is the only marker that says "this entity
    // owns a VRM mesh hierarchy".
    if vrm_check.get(entity).is_err() {
        return;
    }

    remove_vrm_components(&mut commands, entity);
    despawn_children(&mut commands, entity, &children_query);
}

/// Removes all VRM-related components from the entity.
///
/// The list is the load-time inventory, in the two places the pipeline writes
/// it: `handler::scene::finalize` puts the first group on the scene *root*
/// inside the asset, and `handler::nodes::process_node`,
/// `resolve_constraints`, `build_spring_chains` and `spawn_head_copies` put the
/// second group on the individual nodes.
///
/// The one that actually leaks is [`ConstraintExecutionOrder`]: it lives on the
/// root, `apply_node_constraints` (`src/vrm/runtime.rs`) iterates every entity
/// that has one on every frame, and a root that kept it stayed in that query for
/// the rest of the app's life.
///
/// Every removal is a `try_remove`, so the one list covers both a root and a
/// node: a component a given entity never had — a mesh node has no `VrmPath`,
/// which only `handler::scene::insert_source_path` writes, and only on the
/// root — costs nothing and needs no `if let`.
///
/// # Rest transforms: removed from the target only, deliberately
///
/// `insert_rest_transforms` (`handler::scene`) puts [`RestTransform`] and
/// [`RestGlobalTransform`] on *every* entity of a pipeline scene and
/// [`RestWorldTransform`] on its root. This function removes all three from the
/// target only, and the rest of the skeleton is left to `despawn_children`,
/// which despawns those entities outright. Nothing survives for a later system
/// to act on, and the mesh hierarchy that is left behind is an ordinary skinned
/// scene with its bones — which is exactly what a "plain glTF scene" means here.
fn remove_vrm_components(
    commands: &mut Commands,
    entity: Entity,
) {
    commands
        .entity(entity)
        // Core
        .try_remove::<Vrm>()
        .try_remove::<VrmPath>()
        .try_remove::<Initialized>()
        .try_remove::<Name>()
        // Removing `WorldAssetRoot` runs bevy's `on_remove` hook, which
        // unregisters the instance from its `WorldInstanceSpawner`.
        .try_remove::<WorldAssetRoot>()
        // Rest transforms
        .try_remove::<RestTransform>()
        .try_remove::<RestGlobalTransform>()
        .try_remove::<RestWorldTransform>()
        // Gaze/Body
        .try_remove::<LookAtProperties>()
        .try_remove::<LookAt>()
        .try_remove::<BodyTracking>()
        .try_remove::<SmoothedGaze>()
        // Node identity. `VrmNodeIndex` and `VrmBone` come from
        // `handler::nodes::process_node`.
        .try_remove::<VrmNodeIndex>()
        .try_remove::<VrmBone>()
        // Node constraints. `PendingNodeConstraint` is normally resolved away by
        // `resolve_constraints` before the scene is frozen, but a component
        // carrying a glTF node index must never outlive the loader even so.
        .try_remove::<PendingNodeConstraint>()
        .try_remove::<VrmNodeConstraint>()
        // The scene's topological constraint order. Without this removal the
        // root stays in `apply_node_constraints`' query for the rest of the
        // app's life.
        .try_remove::<ConstraintExecutionOrder>()
        // Expressions.
        .try_remove::<VrmExpressionWeights>()
        .try_remove::<ExpressionMorphBinds>()
        .try_remove::<ExpressionSettings>()
        // Spring bones. `SpringJoints`, `SpringColliders` and
        // `SpringCenterNode` are not components of their own — they are the
        // fields of `SpringRoot`, so removing that one removes all three.
        .try_remove::<SpringRoot>()
        .try_remove::<SpringJointProps>()
        .try_remove::<SpringJointState>()
        // `firstPerson: auto` head-copy marker.
        .try_remove::<VrmHeadOnly>();

    remove_bone_entities(commands, entity);
}

macro_rules! remove_bone_entities {
    ($cmd:expr, $entity:expr, $($bone:ident),+ $(,)?) => {
        paste::paste! {
            $cmd.entity($entity)
                $(.try_remove::<crate::vrm::humanoid_bone::prelude::[<$bone BoneEntity>]>())+;
        }
    };
}

/// Removes all 55 bone entity holder components.
fn remove_bone_entities(
    commands: &mut Commands,
    entity: Entity,
) {
    remove_bone_entities!(
        commands,
        entity,
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
        LeftRingProximal,
        LeftRingDistal,
        LeftThumbProximal,
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
        RightThumbProximal,
        Neck,
        Jaw,
        Head,
        LeftEye,
        RightEye,
        LeftMiddleIntermediate,
        RightRingDistal,
        RightIndexProximal,
        RightMiddleDistal,
    );
}

/// Despawns all direct children of the entity.
///
/// Bevy 0.18's `despawn()` is recursive, so this also despawns all descendants.
fn despawn_children(
    commands: &mut Commands,
    entity: Entity,
    children_query: &Query<&Children>,
) {
    let Ok(children) = children_query.get(entity) else {
        return;
    };
    for child in children.iter() {
        commands.entity(child).despawn();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::success;
    use crate::tests::{TestResult, test_app};
    use crate::vrm::components::VrmConstraintKind;
    use crate::vrm::expressions::{ExpressionCategory, ExpressionOverrideType};
    use crate::vrm::humanoid_bone::prelude::HipsBoneEntity;
    use crate::vrm::runtime::apply_node_constraints;
    use crate::vrm::spring_bone::{SpringCenterNode, SpringColliders, SpringJoints};
    use crate::vrma::animation::prelude::{ExpressionSetting, MorphBindTable};
    use bevy::ecs::system::RunSystemOnce;
    use bevy::platform::collections::HashMap;
    use std::f32::consts::FRAC_PI_2;

    /// Asserts that `$entity` carries none of the listed components, naming the
    /// one that survived instead of only a line number.
    macro_rules! assert_all_removed {
        ($entity:expr, $($component:ty),+ $(,)?) => {
            $(
                assert!(
                    !$entity.contains::<$component>(),
                    "`{}` survived the detach",
                    stringify!($component)
                );
            )+
        };
    }

    fn setup_app() -> App {
        let mut app = test_app();
        app.add_plugins(VrmDetachPlugin);
        app
    }

    fn trigger_detach(
        app: &mut App,
        entity: Entity,
    ) {
        app.world_mut()
            .commands()
            .entity(entity)
            .trigger(RequestDetachVrm);
        app.update();
    }

    fn run_constraints(app: &mut App) {
        app.world_mut()
            .run_system_once(apply_node_constraints)
            .expect("Failed to run apply_node_constraints");
        app.world_mut().flush();
    }

    fn set_rotation(
        app: &mut App,
        entity: Entity,
        rotation: Quat,
    ) {
        app.world_mut()
            .entity_mut(entity)
            .get_mut::<Transform>()
            .expect("the entity has a Transform")
            .rotation = rotation;
    }

    fn rotation(
        app: &App,
        entity: Entity,
    ) -> Quat {
        app.world().get::<Transform>(entity).unwrap().rotation
    }

    /// `q` and `-q` are the same rotation, so compare up to sign.
    fn rotation_is(
        actual: Quat,
        expected: Quat,
    ) -> bool {
        actual.abs_diff_eq(expected, 1e-5) || actual.abs_diff_eq(-expected, 1e-5)
    }

    #[test]
    fn test_detach_removes_vrm_components() {
        let mut app = setup_app();

        let vrm_entity = app
            .world_mut()
            .spawn((Vrm, Initialized, RestWorldTransform::default()))
            .id();

        app.world_mut()
            .commands()
            .entity(vrm_entity)
            .trigger(RequestDetachVrm);
        app.update();

        let world = app.world();
        assert!(!world.entity(vrm_entity).contains::<Vrm>());
        assert!(!world.entity(vrm_entity).contains::<Initialized>());
        assert!(!world.entity(vrm_entity).contains::<RestWorldTransform>());
        // Entity itself survives
        assert!(world.get_entity(vrm_entity).is_ok());
    }

    #[test]
    fn test_detach_despawns_children() {
        let mut app = setup_app();

        let child = app.world_mut().spawn_empty().id();
        let vrm_entity = app.world_mut().spawn(Vrm).id();
        app.world_mut()
            .commands()
            .entity(vrm_entity)
            .add_child(child);
        app.update();

        app.world_mut()
            .commands()
            .entity(vrm_entity)
            .trigger(RequestDetachVrm);
        app.update();

        // Root survives
        assert!(app.world().get_entity(vrm_entity).is_ok());
        // Child is despawned
        assert!(app.world().get_entity(child).is_err());
    }

    #[test]
    fn test_detach_on_non_vrm_entity() {
        let mut app = setup_app();

        let child = app.world_mut().spawn_empty().id();
        let entity = app.world_mut().spawn(Name::new("not-a-vrm")).id();
        app.world_mut().commands().entity(entity).add_child(child);
        app.update();

        app.world_mut()
            .commands()
            .entity(entity)
            .trigger(RequestDetachVrm);
        app.update();

        // Name and children should remain since this isn't a VRM entity
        assert!(app.world().entity(entity).contains::<Name>());
        assert!(app.world().get_entity(child).is_ok());
    }

    #[test]
    fn test_detach_idempotent() {
        let mut app = setup_app();

        let vrm_entity = app.world_mut().spawn(Vrm).id();

        // First detach
        app.world_mut()
            .commands()
            .entity(vrm_entity)
            .trigger(RequestDetachVrm);
        app.update();

        // Second detach — should not panic
        app.world_mut()
            .commands()
            .entity(vrm_entity)
            .trigger(RequestDetachVrm);
        app.update();

        assert!(app.world().get_entity(vrm_entity).is_ok());
    }

    /// The complete load-time inventory, on one entity, in the two places the
    /// pipeline writes it.
    ///
    /// `handler::scene::finalize` puts the first group on the scene *root*
    /// inside the asset; `handler::nodes::process_node`,
    /// `resolve_constraints`, `build_spring_chains` and `spawn_head_copies` put
    /// the second group on the individual nodes. They are asserted together
    /// because the target of a detach is whatever entity the caller named, so
    /// the removal cannot assume where in the scene a component sits — and
    /// because a component that ships inside a `WorldAsset` outlives every load
    /// that produced it, so the list is the only place it can be caught.
    ///
    /// `ConstraintExecutionOrder` is the one that actually leaked: it lives on
    /// the root, and `apply_node_constraints` iterates every entity carrying one
    /// on every frame.
    #[test]
    fn the_load_time_component_set_is_removed() -> TestResult {
        let mut app = setup_app();
        let source = app.world_mut().spawn_empty().id();
        let root = app.world_mut().spawn_empty().id();

        // Inserted in the same grouping as `handler::scene::finalize`, because
        // that is where each of them comes from.
        app.world_mut().entity_mut(root).insert((
            // `mark_initialized`
            Vrm,
            Initialized,
            Name::new("Scene0"),
            // `insert_rest_transforms`
            RestTransform(Transform::IDENTITY),
            RestGlobalTransform(GlobalTransform::IDENTITY),
            RestWorldTransform(GlobalTransform::IDENTITY),
            // `resolve_constraints`
            ConstraintExecutionOrder(vec![source]),
            VrmNodeConstraint {
                source,
                weight: 1.0,
                kind: VrmConstraintKind::Rotation,
            },
            // `build_expressions`
            VrmExpressionWeights(HashMap::from([("happy".to_owned(), 0.5)])),
            ExpressionMorphBinds(MorphBindTable(HashMap::from([(
                "happy".to_owned(),
                Vec::new(),
            )]))),
            // `build_look_at`
            LookAtProperties::default(),
        ));
        app.world_mut().entity_mut(root).insert((
            // `handler::nodes::process_node`
            VrmNodeIndex(7),
            VrmBone::from("hips"),
            PendingNodeConstraint {
                source_node: 7,
                weight: 1.0,
                kind: VrmConstraintKind::Rotation,
            },
            // `build_spring_chains`
            SpringJointProps::default(),
            SpringJointState::default(),
            // `spawn_head_copies`
            VrmHeadOnly,
        ));
        app.world_mut().entity_mut(root).insert(SpringRoot {
            joints: SpringJoints(vec![source]),
            colliders: SpringColliders(Vec::new()),
            center_node: SpringCenterNode(Some(source)),
        });
        app.world_mut()
            .entity_mut(root)
            .insert(ExpressionSettings(HashMap::from([(
                "happy".to_owned(),
                ExpressionSetting {
                    is_binary: false,
                    category: ExpressionCategory::Other,
                    override_blink: ExpressionOverrideType::None,
                    override_look_at: ExpressionOverrideType::None,
                    override_mouth: ExpressionOverrideType::None,
                },
            )])));

        trigger_detach(&mut app, root);

        let world = app.world();
        let entity = world.entity(root);
        assert_all_removed!(
            entity,
            Vrm,
            Initialized,
            Name,
            ConstraintExecutionOrder,
            VrmNodeConstraint,
            VrmNodeIndex,
            VrmBone,
            PendingNodeConstraint,
            VrmExpressionWeights,
            ExpressionMorphBinds,
            ExpressionSettings,
            SpringRoot,
            SpringJointProps,
            SpringJointState,
            VrmHeadOnly,
            LookAtProperties,
            RestTransform,
            RestGlobalTransform,
            RestWorldTransform,
        );
        // The entity itself survives: the result is a plain scene, not nothing.
        assert!(world.get_entity(root).is_ok());
        success!()
    }

    /// The root-side set, so the node-side list above did not quietly narrow it:
    /// `handler::scene::insert_source_path` writes `VrmPath` on the scene root,
    /// `insert_humanoid_bone_holders` puts a bone holder on it, and `LookAt` /
    /// `BodyTracking` are what [`spawn_vrm`]'s `configure` closure inserts.
    ///
    /// `VrmPath` is the asymmetry worth pinning: only `insert_source_path`
    /// writes it, and only on the root, so a mesh node is not guaranteed to have
    /// one. The removal is a `try_remove` precisely so the absent case needs no
    /// branch of its own.
    #[test]
    fn the_root_side_component_set_is_removed() -> TestResult {
        let mut app = setup_app();
        let hips = app.world_mut().spawn_empty().id();
        let root = app.world_mut().spawn_empty().id();

        app.world_mut().entity_mut(root).insert((
            Vrm,
            Initialized,
            Name::new("Elmer"),
            WorldAssetRoot(Handle::default()),
            VrmPath::new("vrm/Elmer.vrm"),
            RestWorldTransform::default(),
            LookAtProperties::default(),
            // User-inserted through `spawn_vrm`'s `configure` closure.
            LookAt::Cursor,
            BodyTracking::default(),
            SmoothedGaze::default(),
            HipsBoneEntity(hips),
        ));

        trigger_detach(&mut app, root);

        let world = app.world();
        let entity = world.entity(root);
        assert_all_removed!(
            entity,
            Vrm,
            Initialized,
            Name,
            WorldAssetRoot,
            VrmPath,
            RestWorldTransform,
            LookAtProperties,
            LookAt,
            BodyTracking,
            SmoothedGaze,
            HipsBoneEntity,
        );
        // The bone entity was never the target, and only VRM components are
        // removed, so it survives as an ordinary entity.
        assert!(world.get_entity(hips).is_ok());
        success!()
    }

    /// The regression the whole load-time set exists for: a pipeline root that
    /// kept `ConstraintExecutionOrder` stayed in `apply_node_constraints`'s
    /// query for the rest of the app's life, driving bones of a scene that was
    /// supposed to be inert.
    ///
    /// The destination is deliberately *not* a descendant of the root, because
    /// that is the case the removal has to survive: `ConstraintExecutionOrder`
    /// is load-time data that lists entities by id, so it keeps answering for
    /// destinations that outlive the root they were read from — exactly what
    /// `apply_node_constraints` documents when it skips a stale entry.
    #[test]
    fn a_detached_pipeline_scene_stops_running_node_constraints() -> TestResult {
        let mut app = setup_app();
        let source = app
            .world_mut()
            .spawn((Transform::IDENTITY, RestTransform(Transform::IDENTITY)))
            .id();
        let destination = app
            .world_mut()
            .spawn((
                Transform::IDENTITY,
                RestTransform(Transform::IDENTITY),
                VrmNodeConstraint {
                    source,
                    weight: 1.0,
                    kind: VrmConstraintKind::Rotation,
                },
            ))
            .id();
        let root = app
            .world_mut()
            .spawn((
                Vrm,
                Initialized,
                ConstraintExecutionOrder(vec![destination]),
            ))
            .id();

        let driven = Quat::from_rotation_x(FRAC_PI_2);
        set_rotation(&mut app, source, driven);
        run_constraints(&mut app);
        assert!(
            rotation_is(rotation(&app, destination), driven),
            "before the detach the pipeline constraint pass drives the destination: got {:?}",
            rotation(&app, destination)
        );

        trigger_detach(&mut app, root);

        assert!(
            app.world().get::<ConstraintExecutionOrder>(root).is_none(),
            "the execution order is what kept the root in the pass's query"
        );
        // Reset and drive again: with the order gone, the destination must stay
        // exactly where it is put.
        set_rotation(&mut app, destination, Quat::IDENTITY);
        set_rotation(&mut app, source, driven);
        run_constraints(&mut app);
        assert_eq!(
            rotation(&app, destination),
            Quat::IDENTITY,
            "a detached scene must not keep writing node constraints"
        );
        success!()
    }

    /// `VrmPath` is only on the scene *root* that
    /// `handler::scene::insert_source_path` wrote it on, so it is not a component
    /// every VRM entity is guaranteed to carry. Detaching one without it must not
    /// panic — the removal is a `try_remove` precisely for that.
    #[test]
    fn detaching_a_scene_without_a_vrm_path_is_fine() -> TestResult {
        let mut app = setup_app();
        let root = app
            .world_mut()
            .spawn((Vrm, Initialized, ConstraintExecutionOrder(Vec::new())))
            .id();

        trigger_detach(&mut app, root);

        assert!(app.world().get_entity(root).is_ok());
        success!()
    }

    /// The `Vrm` guard is what scopes a detach: an entity that does not own a
    /// VRM mesh hierarchy is left completely alone, even when it carries
    /// components that appear on the removal list.
    #[test]
    fn detaching_an_entity_without_vrm_is_a_no_op() -> TestResult {
        let mut app = setup_app();
        let node = app
            .world_mut()
            .spawn((
                VrmNodeIndex(3),
                VrmBone::from("hips"),
                RestTransform::default(),
            ))
            .id();

        trigger_detach(&mut app, node);

        assert!(
            app.world().get::<VrmBone>(node).is_some(),
            "a node without `Vrm` is not a VRM root and must not be stripped"
        );
        success!()
    }
}
