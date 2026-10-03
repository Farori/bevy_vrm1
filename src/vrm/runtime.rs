//! The runtime half of the glTF-handler pipeline.
//!
//! The load-time handler writes node constraints into the scene `WorldAsset` as
//! [`ConstraintExecutionOrder`] (on the scene root) plus [`VrmNodeConstraint`]
//! (on each destination), so the evaluation *order* is data rather than schedule
//! configuration. [`apply_node_constraints`] is the whole runtime surface today:
//! one ordered pass over that list, driven by no registry and no
//! `Changed<Transform>` trigger.
//!
//! # Why a pipeline scene is never evaluated twice
//!
//! The legacy `VrmNodeConstraintPlugin` path stays in the tree until it is
//! removed, and its three `bind_*` systems live in the same
//! [`VrmSystemSets::Constraints`] set as [`apply_node_constraints`]. They cannot
//! see a pipeline scene, and the reason is a component, not archetype luck:
//!
//! * `bind_rotation_constraints` / `bind_roll_constraints` /
//!   `bind_aim_constraints` iterate sources carrying
//!   `RotationConstraintDestinations` / `RollConstraintDestinations` /
//!   `AimConstraintDestinations` (`src/vrm/node_constraint/bind/rotation.rs:26`,
//!   `roll.rs:24`, `aim.rs:24`). Those components are inserted from exactly one
//!   place: `register_rotation_constraint` and friends in
//!   `src/vrm/node_constraint/initialize.rs:105`, `:133`, `:159`, which are
//!   reached only from the `RequestInitializeNodeConstraints` observer.
//! * That observer bails out at `src/vrm/node_constraint/initialize.rs:33`
//!   unless the root carries `NodeConstraintRegistry`.
//! * `NodeConstraintRegistry` is inserted in exactly one place:
//!   `src/vrm/initialize.rs:82`, inside `spawn_vrm`, whose only trigger is the
//!   `VrmHandle` component. A pipeline scene is spawned from a
//!   `WorldAssetRoot` and never has a `VrmHandle`, so the registry never
//!   exists and the observer never inserts the three destination lists.
//!
//! The converse also holds and is what this system relies on: it reads nothing
//! but `ConstraintExecutionOrder`, `VrmNodeConstraint`, `RestTransform`,
//! `ChildOf` and `Transform`, none of which the legacy initializer writes. The
//! two paths are disjoint in both directions.
//!
//! One residue until the legacy path is deleted: the scheduler sees
//! `apply_node_constraints` as a `Transform` *writer* and the legacy `bind_*`
//! systems as `Transform` *readers*, both unordered inside the same set, so
//! enabling `ScheduleBuildSettings::ambiguity_detection` would report the pair.
//! Nothing is written twice — the legacy source queries match no entity on a
//! pipeline scene — and bevy's default is `LogLevel::Ignore`
//! (`bevy_ecs/src/schedule/schedule.rs:1627`). The private `fn`s in
//! `src/vrm/node_constraint/bind/` cannot be named from here to declare the
//! ambiguity, so it goes away with the legacy path rather than being papered
//! over.

use crate::system_set::VrmSystemSets;
use crate::vrm::RestTransform;
use crate::vrm::components::{ConstraintExecutionOrder, VrmConstraintKind, VrmNodeConstraint};
use bevy::app::{AnimationSystems, App, Plugin};
use bevy::prelude::*;
use bevy::transform::TransformSystems;

/// How far [`compute_global`] walks up a parent chain before giving up.
///
/// `ChildOf` cycles are impossible through bevy's own hierarchy API, and
/// bevy's parallel propagation uses the same bound as "larger than any
/// reasonable tree depth" (`bevy_transform/src/systems.rs:650`). Exceeding it
/// only skips one constraint, which is the right failure mode for a
/// malformed scene.
const MAX_ANCESTOR_WALK: usize = 10_000;

/// Adds the runtime systems for scenes built by the glTF extension handler.
pub struct VrmGltfRuntimePlugin;

impl Plugin for VrmGltfRuntimePlugin {
    fn build(
        &self,
        app: &mut App,
    ) {
        // `VrmSystemSets::Constraints` is the VRM spec's constraint step: it is
        // what `VrmSystemSets::PropagateAfterConstraints` is chained after
        // (`src/vrm.rs:172`) and what `VrmSystemSets::GazeControl` looks
        // downstream of, so putting the pass here gets the constraints'
        // `GlobalTransform` published before gaze control reads the head and
        // eyes — the established manual-propagation order, unchanged.
        //
        // `.after(AnimationSystems)` because the sources are humanoid bones
        // driven by `AnimationPlayer`, matching the legacy bind systems.
        // `.before(TransformSystems::Propagate)` because this system writes
        // `Transform` and bevy's propagation reads it; the constraint step
        // must be done before the engine publishes globals, and bevy already
        // orders `AnimationSystems` the same way
        // (`bevy_animation/src/lib.rs:1306`).
        app.add_systems(
            PostUpdate,
            apply_node_constraints
                .in_set(VrmSystemSets::Constraints)
                .after(AnimationSystems)
                .before(TransformSystems::Propagate),
        );

        // `VrmPlugin`'s manual propagation pass (`src/vrm.rs:172`) is only
        // constrained to run after `VrmSystemSets::Constraints`, and bevy's own
        // `TransformSystems::Propagate` is unordered against it, so which of
        // the two publishes a constrained bone first was up to the executor.
        //
        // That matters because the manual pass is what makes the constraints
        // visible to `VrmSystemSets::GazeControl`, which reads `GlobalTransform`
        // (`src/vrm/look_at.rs:81`), and because bevy's parallel propagation
        // skips any root whose `TransformTreeChanged` is not set
        // (`bevy_transform/src/systems.rs:525`); only bevy's `mark_dirty_trees`
        // sets it, and it runs inside `TransformSystems::Propagate`
        // (`bevy_transform/src/plugins.rs:40`). Ordering the manual pass after
        // it therefore guarantees both that the tree is marked and that the
        // globals are published, every frame.
        //
        // The edge is additive: the pass is idempotent, and it also resolves the
        // pre-existing `GlobalTransform` ambiguity between the two propagations.
        app.configure_sets(
            PostUpdate,
            VrmSystemSets::PropagateAfterConstraints.after(TransformSystems::Propagate),
        );
    }
}

/// Evaluates one scene's node constraints in the topological order the handler
/// computed at load time.
///
/// Every destination is recomputed from its rest pose on every frame, so a
/// constraint never feeds its own previous output back into itself, and a
/// source that is itself a destination is fresh for its dependents: the write
/// happens through `&mut Transform` inside this loop, before the next entry is
/// read. That is the entire reason the order is a precomputed list.
pub fn apply_node_constraints(
    orders: Query<&ConstraintExecutionOrder>,
    constraints: Query<&VrmNodeConstraint>,
    rests: Query<&RestTransform>,
    parents: Query<&ChildOf>,
    mut transforms: Query<&mut Transform>,
) {
    for order in orders.iter() {
        for &destination in order.destinations() {
            // A destination that has been despawned (or not spawned yet) is not
            // an error: the list is load-time data that outlives the entities
            // it names, so a stale entry is skipped and the walk continues.
            let Some(rotation) =
                constraint_rotation(destination, &constraints, &rests, &parents, &transforms)
            else {
                continue;
            };
            if let Ok(mut transform) = transforms.get_mut(destination) {
                transform.rotation = rotation;
            }
        }
    }
}

/// The destination's new local rotation, or [`None`] to leave it as it is.
fn constraint_rotation(
    destination: Entity,
    constraints: &Query<&VrmNodeConstraint>,
    rests: &Query<&RestTransform>,
    parents: &Query<&ChildOf>,
    transforms: &Query<&mut Transform>,
) -> Option<Quat> {
    let constraint = constraints.get(destination).ok()?;
    let weight = constraint.weight;
    // `VrmNodeConstraint::weight` documents `0.0` as "skipped"
    // (`src/vrm/components.rs:171`). Skipping is also what the spec's slerp
    // means in practice: `slerp(rest, .., 0.0)` is the rest rotation, and
    // writing that would wipe out whatever else moved the bone this frame
    // (another constraint, spring bones, the user).
    if weight == 0.0 {
        return None;
    }

    let destination_rest = rests.get(destination).ok()?.0;
    match constraint.kind {
        VrmConstraintKind::Rotation => {
            let delta = source_pose(constraint.source, rests, transforms)?.1;
            Some(
                destination_rest
                    .rotation
                    .slerp(destination_rest.rotation * delta, weight),
            )
        }
        VrmConstraintKind::Roll { roll_axis } => {
            let (source_rest, delta) = source_pose(constraint.source, rests, transforms)?;
            roll_rotation(
                destination_rest.rotation,
                source_rest,
                delta,
                roll_axis,
                weight,
            )
        }
        VrmConstraintKind::Aim { aim_axis } => aim_rotation(
            destination,
            destination_rest.rotation,
            aim_axis,
            constraint.source,
            weight,
            parents,
            transforms,
        ),
    }
}

/// The source's rest rotation, and its rotation relative to that rest pose in
/// its own local frame — the spec's `deltaSrcQuat = srcRestQuat^-1 * srcQuat`.
fn source_pose(
    source: Entity,
    rests: &Query<&RestTransform>,
    transforms: &Query<&mut Transform>,
) -> Option<(Quat, Quat)> {
    let rest = rests.get(source).ok()?.0.rotation;
    let local = transforms.get(source).ok()?.rotation;
    Some((rest, rest.inverse() * local))
}

/// `VRMC_node_constraint-1.0`, roll constraint.
fn roll_rotation(
    destination_rest: Quat,
    source_rest: Quat,
    source_delta: Quat,
    roll_axis: Dir3,
    weight: f32,
) -> Option<Quat> {
    // Spec pseudocode:
    //   deltaSrcQuatInParent = srcRestQuat * deltaSrcQuat * srcRestQuat^-1
    //   deltaSrcQuatInDst    = dstRestQuat^-1 * deltaSrcQuatInParent * dstRestQuat
    // The first line conjugates by the *source's* rest rotation. That is the
    // fix the legacy `bind_roll_constraints` carries
    // (`src/vrm/node_constraint/bind/roll.rs:37`); conjugating by
    // `destination_rest` instead would rotate the delta through the wrong
    // basis.
    let delta_in_parent = source_rest * source_delta * source_rest.inverse();
    let delta_in_destination = destination_rest.inverse() * delta_in_parent * destination_rest;

    let axis = roll_axis.as_vec3();
    // `from_rotation_arc` needs unit inputs. The product is unit for unit
    // quaternions, but a `Transform` whose rotation was written as something
    // else must not turn into a NaN quaternion here.
    let to_vec = (delta_in_destination * axis).normalize_or_zero();
    if to_vec == Vec3::ZERO {
        return None;
    }
    let from_to = Quat::from_rotation_arc(axis, to_vec);

    Some(destination_rest.slerp(
        destination_rest * from_to.inverse() * delta_in_destination,
        weight,
    ))
}

/// `VRMC_node_constraint-1.0`, aim constraint.
fn aim_rotation(
    destination: Entity,
    destination_rest: Quat,
    aim_axis: Dir3,
    source: Entity,
    weight: f32,
    parents: &Query<&ChildOf>,
    transforms: &Query<&mut Transform>,
) -> Option<Quat> {
    // The spec's `dstParentWorldQuat`. A destination without a parent is a
    // root, and for a root that is the identity.
    let destination_parent = match parents.get(destination) {
        Ok(child_of) => compute_global(child_of.0, parents, transforms)?,
        Err(_) => Transform::IDENTITY,
    };
    let source_global = compute_global(source, parents, transforms)?;
    let destination_local = *transforms.get(destination).ok()?;
    let destination_global = destination_parent.mul_transform(destination_local);

    // `toVec = (srcWorldPos - dstWorldPos).normalized`. Upstream's legacy
    // `bind_aim_constraints` calls a bare `Vec3::normalize()` here
    // (`src/vrm/node_constraint/bind/aim.rs:37`), which is `NaN` when the
    // source and the destination coincide — a configuration a sleeve bone and
    // a hand bone reach whenever the arm is at the origin. There is no
    // direction to aim at, so the constraint is skipped instead.
    let to_vec = (source_global.translation - destination_global.translation).normalize_or_zero();
    if to_vec == Vec3::ZERO {
        return None;
    }

    // `fromVec = aimAxis.applyQuaternion(dstParentWorldQuat * dstRestQuat)`: the
    // rest pose's aim axis in world space. The destination's *current* rotation
    // is deliberately not used, or the constraint would chase its own output.
    let from_vec =
        (destination_parent.rotation * destination_rest * aim_axis.as_vec3()).normalize_or_zero();
    if from_vec == Vec3::ZERO {
        return None;
    }
    let from_to = Quat::from_rotation_arc(from_vec, to_vec);

    Some(destination_rest.slerp(
        destination_parent.rotation.inverse()
            * from_to
            * destination_parent.rotation
            * destination_rest,
        weight,
    ))
}

/// Accumulates an entity's world transform by walking up its parent chain.
///
/// [`GlobalTransform`] is deliberately not consulted. At this point in
/// `PostUpdate` bevy has not propagated yet — `TransformSystems::Propagate` runs
/// after `AnimationSystems` and `VrmSystemSets::Constraints` is unordered
/// against it — so the stored `GlobalTransform` is whatever
/// `VrmSystemSets::PropagateAfterConstraints` published at the end of the
/// *previous* frame. Reading it here would break two things:
///
/// * `VrmSystemSets::PropagateAfterConstraints` publishes once, after the whole
///   set, so a source that is itself a constrained destination evaluated earlier
///   in `ConstraintExecutionOrder` would still read its pre-constraint pose.
///   Honouring the topological order means the *source* has to be read at the
///   moment its dependent runs, which no `GlobalTransform` can express.
/// * Whether the value happened to be one frame stale or current would depend
///   on how the scheduler interleaved the two sets, so the same rig could
///   behave differently run to run.
fn compute_global(
    entity: Entity,
    parents: &Query<&ChildOf>,
    transforms: &Query<&mut Transform>,
) -> Option<Transform> {
    let mut chain = Vec::new();
    let mut current = entity;
    loop {
        chain.push(current);
        if chain.len() > MAX_ANCESTOR_WALK {
            return None;
        }
        match parents.get(current) {
            Ok(child_of) => current = child_of.0,
            Err(_) => break,
        }
    }

    let mut global = Transform::IDENTITY;
    for entity in chain.iter().rev() {
        global = global.mul_transform(*transforms.get(*entity).ok()?);
    }
    Some(global)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::success;
    use crate::tests::{TestResult, test_app};
    use crate::vrm::node_constraint::{RotationConstraintDest, RotationConstraintDestinations};
    use bevy::ecs::system::RunSystemOnce;
    use bevy::transform::TransformPlugin;
    use bevy::transform::systems::{propagate_parent_transforms, sync_simple_transforms};
    use std::f32::consts::{FRAC_PI_2, FRAC_PI_3};

    /// A destination entity for the schedule test's probes to find.
    #[derive(Resource)]
    struct Watched(Entity);

    /// What the probes saw.
    #[derive(Resource, Default)]
    struct Observed {
        after_propagate: Option<Quat>,
        at_gaze: Option<GlobalTransform>,
    }

    /// Stands in for the source bone a `AnimationPlugin` graph drives.
    #[derive(Component)]
    struct Animated;

    fn run_constraints(app: &mut App) {
        app.world_mut()
            .run_system_once(apply_node_constraints)
            .expect("Failed to run apply_node_constraints");
        app.world_mut().flush();
    }

    fn rotation(
        app: &App,
        entity: Entity,
    ) -> Quat {
        app.world().get::<Transform>(entity).unwrap().rotation
    }

    /// Rotates in place: the translation matters to the aim constraint, so it
    /// must survive.
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

    /// `q` and `-q` are the same rotation, so compare up to sign.
    fn rotation_is(
        actual: Quat,
        expected: Quat,
    ) -> bool {
        actual.abs_diff_eq(expected, 1e-5) || actual.abs_diff_eq(-expected, 1e-5)
    }

    fn rest(rotation: Quat) -> RestTransform {
        RestTransform(Transform::from_rotation(rotation))
    }

    /// The roll pseudocode from `VRMC_node_constraint-1.0`, transcribed so the
    /// test's oracle is independent of the implementation.
    fn spec_roll(
        source_rest: Quat,
        source_delta: Quat,
        destination_rest: Quat,
        axis: Dir3,
        weight: f32,
    ) -> Quat {
        let delta_in_parent = source_rest * source_delta * source_rest.inverse();
        let delta_in_destination = destination_rest.inverse() * delta_in_parent * destination_rest;
        let to_vec = delta_in_destination * axis.as_vec3();
        let from_to = Quat::from_rotation_arc(axis.as_vec3(), to_vec);
        destination_rest.slerp(
            destination_rest * from_to.inverse() * delta_in_destination,
            weight,
        )
    }

    #[test]
    fn rotation_constraint_blends_by_weight() -> TestResult {
        let mut app = test_app();
        let source = app
            .world_mut()
            .spawn((Transform::IDENTITY, rest(Quat::IDENTITY)))
            .id();
        let destination = app
            .world_mut()
            .spawn((
                Transform::IDENTITY,
                rest(Quat::IDENTITY),
                VrmNodeConstraint {
                    source,
                    weight: 0.5,
                    kind: VrmConstraintKind::Rotation,
                },
            ))
            .id();
        app.world_mut()
            .spawn(ConstraintExecutionOrder(vec![destination]));

        set_rotation(&mut app, source, Quat::from_rotation_x(FRAC_PI_2));
        run_constraints(&mut app);

        // `slerp(rest, rest * delta, 0.5)` is half of the source's 90 degrees.
        assert!(
            rotation_is(
                rotation(&app, destination),
                Quat::from_rotation_x(FRAC_PI_2 / 2.0)
            ),
            "got {:?}",
            rotation(&app, destination)
        );
        success!()
    }

    #[test]
    fn a_zero_weight_constraint_leaves_the_destination_alone() -> TestResult {
        let mut app = test_app();
        let source = app
            .world_mut()
            .spawn((Transform::IDENTITY, rest(Quat::IDENTITY)))
            .id();
        let destination = app
            .world_mut()
            .spawn((
                Transform::IDENTITY,
                rest(Quat::IDENTITY),
                VrmNodeConstraint {
                    source,
                    weight: 0.0,
                    kind: VrmConstraintKind::Rotation,
                },
            ))
            .id();
        app.world_mut()
            .spawn(ConstraintExecutionOrder(vec![destination]));

        // A pose the constraint must not disturb: with weight `0.0` the spec's
        // slerp would produce the *rest* rotation, which is not the same thing.
        let held = Quat::from_rotation_z(FRAC_PI_3);
        set_rotation(&mut app, destination, held);
        set_rotation(&mut app, source, Quat::from_rotation_x(FRAC_PI_2));

        run_constraints(&mut app);

        assert_eq!(rotation(&app, destination), held);
        success!()
    }

    /// `deltaSrcQuatInParent` conjugates by the *source's* rest rotation. The
    /// two rest rotations are deliberately different, and the assertion that
    /// the results differ is what makes this test able to fail if the bases are
    /// swapped.
    #[test]
    fn roll_constraint_conjugates_through_the_source_rest_pose() -> TestResult {
        let mut app = test_app();
        let source_rest = Quat::IDENTITY;
        let destination_rest = Quat::from_rotation_y(FRAC_PI_2);
        let source_delta = Quat::from_rotation_x(FRAC_PI_2);
        let roll_axis = Dir3::Z;

        let source = app
            .world_mut()
            .spawn((Transform::IDENTITY, rest(source_rest)))
            .id();
        let destination = app
            .world_mut()
            .spawn((
                Transform::IDENTITY,
                rest(destination_rest),
                VrmNodeConstraint {
                    source,
                    weight: 1.0,
                    kind: VrmConstraintKind::Roll { roll_axis },
                },
            ))
            .id();
        app.world_mut()
            .spawn(ConstraintExecutionOrder(vec![destination]));

        set_rotation(&mut app, source, source_delta);
        run_constraints(&mut app);

        let expected = spec_roll(source_rest, source_delta, destination_rest, roll_axis, 1.0);
        // The same formula with the destination's rest rotation where the
        // source's belongs: the bug the legacy `e45bb21` fix corrected.
        let wrong_basis = spec_roll(
            destination_rest,
            source_delta,
            destination_rest,
            roll_axis,
            1.0,
        );
        assert!(
            !rotation_is(expected, wrong_basis),
            "the two bases must give different results or this test proves nothing"
        );
        assert!(
            rotation_is(rotation(&app, destination), expected),
            "expected {:?}, got {:?}",
            expected,
            rotation(&app, destination)
        );
        success!()
    }

    /// The aim axis must end up pointing at the source **in world space**, so
    /// the destination's parent rotation has to be conjugated in. The parent is
    /// rotated 90 degrees about `Y` on purpose: treating
    /// `dstParentWorldQuat` as the identity would leave the axis at 90 degrees
    /// off and fail the alignment check.
    #[test]
    fn aim_constraint_points_its_axis_at_the_source_in_world_space() -> TestResult {
        let mut app = test_app();
        let parent = app
            .world_mut()
            .spawn(Transform::from_rotation(Quat::from_rotation_y(FRAC_PI_2)))
            .id();
        let aim_axis = Dir3::NEG_Z;

        let source = app
            .world_mut()
            .spawn((Transform::from_xyz(0.0, 0.0, -1.0), rest(Quat::IDENTITY)))
            .id();
        let destination = app
            .world_mut()
            .spawn((
                Transform::IDENTITY,
                rest(Quat::IDENTITY),
                ChildOf(parent),
                VrmNodeConstraint {
                    source,
                    weight: 1.0,
                    kind: VrmConstraintKind::Aim { aim_axis },
                },
            ))
            .id();
        app.world_mut()
            .spawn(ConstraintExecutionOrder(vec![destination]));

        run_constraints(&mut app);

        let world = app.world();
        let parent_rotation = world.get::<Transform>(parent).expect("parent").rotation;
        let world_rotation =
            parent_rotation * world.get::<Transform>(destination).expect("dst").rotation;
        // The parent is a root, so its local rotation is its global rotation.
        assert_eq!(parent_rotation, Quat::from_rotation_y(FRAC_PI_2));

        let aimed = world_rotation * aim_axis.as_vec3();
        let expected =
            (world.get::<Transform>(source).expect("src").translation).normalize_or_zero();
        assert!(
            aimed.abs_diff_eq(expected, 1e-5),
            "aim axis points at {aimed:?}, the source is at {expected:?}"
        );
        success!()
    }

    /// The legacy `bind_aim_constraints` normalizes a zero-length vector here
    /// (`src/vrm/node_constraint/bind/aim.rs:37`) and writes the resulting
    /// `NaN` quaternion into the scene.
    #[test]
    fn aim_constraint_skips_a_destination_coincident_with_its_source() -> TestResult {
        let mut app = test_app();
        let source = app
            .world_mut()
            .spawn((Transform::from_xyz(1.0, 0.0, 0.0), rest(Quat::IDENTITY)))
            .id();
        let destination = app
            .world_mut()
            .spawn((
                Transform::from_xyz(1.0, 0.0, 0.0),
                rest(Quat::IDENTITY),
                VrmNodeConstraint {
                    source,
                    weight: 1.0,
                    kind: VrmConstraintKind::Aim {
                        aim_axis: Dir3::NEG_Z,
                    },
                },
            ))
            .id();
        app.world_mut()
            .spawn(ConstraintExecutionOrder(vec![destination]));

        let held = Quat::from_rotation_x(FRAC_PI_3);
        set_rotation(&mut app, destination, held);

        run_constraints(&mut app);

        let rotation = rotation(&app, destination);
        assert!(
            rotation.is_finite(),
            "the aim constraint wrote {rotation:?}"
        );
        assert_eq!(rotation, held, "a zero-length to-vector must be skipped");
        success!()
    }

    /// The whole point of the load-time topological sort: `tail`'s source is
    /// `middle`, and `middle` is itself constrained. Reading `middle` before
    /// its own constraint ran gives a different answer than reading it after.
    #[test]
    fn the_execution_order_decides_whether_a_dependent_sees_a_fresh_source() -> TestResult {
        let mut app = test_app();
        let driver = app
            .world_mut()
            .spawn((
                Transform::from_rotation(Quat::from_rotation_x(FRAC_PI_2)),
                rest(Quat::IDENTITY),
            ))
            .id();
        let middle = app
            .world_mut()
            .spawn((
                Transform::IDENTITY,
                rest(Quat::IDENTITY),
                VrmNodeConstraint {
                    source: driver,
                    weight: 1.0,
                    kind: VrmConstraintKind::Rotation,
                },
            ))
            .id();
        let tail = app
            .world_mut()
            .spawn((
                Transform::IDENTITY,
                rest(Quat::IDENTITY),
                VrmNodeConstraint {
                    source: middle,
                    weight: 1.0,
                    kind: VrmConstraintKind::Rotation,
                },
            ))
            .id();
        let order = app.world_mut().spawn_empty().id();
        app.world_mut()
            .entity_mut(order)
            .insert(ConstraintExecutionOrder(vec![middle, tail]));

        run_constraints(&mut app);

        let driven = Quat::from_rotation_x(FRAC_PI_2);
        assert!(rotation_is(rotation(&app, middle), driven));
        // `middle` was written before `tail` read it, so `tail` inherits the
        // driver's rotation rather than `middle`'s rest pose.
        assert!(
            rotation_is(rotation(&app, tail), driven),
            "got {:?}",
            rotation(&app, tail)
        );

        // Reversed order, poses reset: `tail` now reads `middle` before its own
        // constraint, so it stays at its rest rotation.
        set_rotation(&mut app, middle, Quat::IDENTITY);
        set_rotation(&mut app, tail, Quat::IDENTITY);
        app.world_mut()
            .entity_mut(order)
            .insert(ConstraintExecutionOrder(vec![tail, middle]));

        run_constraints(&mut app);

        assert!(
            rotation_is(rotation(&app, tail), Quat::IDENTITY),
            "got {:?}",
            rotation(&app, tail)
        );
        assert!(rotation_is(rotation(&app, middle), driven));
        success!()
    }

    #[test]
    fn a_destination_that_is_no_longer_in_the_world_is_skipped() -> TestResult {
        let mut app = test_app();
        let source = app
            .world_mut()
            .spawn((Transform::IDENTITY, rest(Quat::IDENTITY)))
            .id();
        let gone = app
            .world_mut()
            .spawn((
                Transform::IDENTITY,
                rest(Quat::IDENTITY),
                VrmNodeConstraint {
                    source,
                    weight: 1.0,
                    kind: VrmConstraintKind::Rotation,
                },
            ))
            .id();
        let destination = app
            .world_mut()
            .spawn((
                Transform::IDENTITY,
                rest(Quat::IDENTITY),
                VrmNodeConstraint {
                    source,
                    weight: 1.0,
                    kind: VrmConstraintKind::Rotation,
                },
            ))
            .id();
        // The dead destination comes first: the walk has to continue, not bail.
        app.world_mut()
            .spawn(ConstraintExecutionOrder(vec![gone, destination]));

        app.world_mut().despawn(gone);

        set_rotation(&mut app, source, Quat::from_rotation_x(FRAC_PI_2));
        run_constraints(&mut app);

        assert!(rotation_is(
            rotation(&app, destination),
            Quat::from_rotation_x(FRAC_PI_2)
        ));
        success!()
    }

    /// The legacy bind systems key off `RotationConstraintDestinations`; this
    /// one keys off `ConstraintExecutionOrder`. With the legacy list present
    /// and no execution order, nothing must move — that is the disjointness the
    /// module docs argue from, seen from this side.
    #[test]
    fn the_legacy_destination_registries_do_not_drive_this_system() -> TestResult {
        let mut app = test_app();
        let source = app
            .world_mut()
            .spawn((Transform::IDENTITY, rest(Quat::IDENTITY)))
            .id();
        let destination = app
            .world_mut()
            .spawn((
                Transform::IDENTITY,
                rest(Quat::IDENTITY),
                VrmNodeConstraint {
                    source,
                    weight: 1.0,
                    kind: VrmConstraintKind::Rotation,
                },
            ))
            .id();
        // Exactly what `register_rotation_constraint` would have inserted.
        app.world_mut()
            .entity_mut(source)
            .insert(RotationConstraintDestinations(vec![
                RotationConstraintDest {
                    dest: destination,
                    weight: 1.0,
                },
            ]));
        set_rotation(&mut app, source, Quat::from_rotation_x(FRAC_PI_2));

        run_constraints(&mut app);
        assert_eq!(
            rotation(&app, destination),
            Quat::IDENTITY,
            "a legacy registry must not drive the pipeline system"
        );

        // The execution order is the only thing that starts it.
        app.world_mut()
            .spawn(ConstraintExecutionOrder(vec![destination]));
        run_constraints(&mut app);
        assert!(rotation_is(
            rotation(&app, destination),
            Quat::from_rotation_x(FRAC_PI_2)
        ));
        success!()
    }

    fn animate_source(mut sources: Query<&mut Transform, With<Animated>>) {
        for mut transform in &mut sources {
            transform.rotation = Quat::from_rotation_x(FRAC_PI_2);
        }
    }

    fn observe_after_propagate(
        watched: Res<Watched>,
        transforms: Query<&Transform>,
        mut observed: ResMut<Observed>,
    ) {
        observed.after_propagate = transforms.get(watched.0).ok().map(|tf| tf.rotation);
    }

    fn observe_at_gaze(
        watched: Res<Watched>,
        globals: Query<&GlobalTransform>,
        mut observed: ResMut<Observed>,
    ) {
        observed.at_gaze = globals.get(watched.0).ok().copied();
    }

    /// Pins the two edges the plugin adds, plus the crate's manual propagation:
    /// the constraint must see `AnimationSystems` output, and by the time
    /// `TransformSystems::Propagate` has run the rotation is already the
    /// constrained one. The second probe stands in for `GazeControl`, which
    /// `VrmPlugin` orders after `VrmSystemSets::PropagateAfterConstraints`, and
    /// reads `GlobalTransform`.
    #[test]
    fn constraints_run_after_animation_and_before_propagation() -> TestResult {
        fn register(app: &mut App) {
            app.add_plugins((TransformPlugin, VrmGltfRuntimePlugin));
            // Stands in for `AnimationPlugin`'s output.
            app.add_systems(PostUpdate, animate_source.in_set(AnimationSystems));
            app.add_systems(
                PostUpdate,
                observe_after_propagate.after(TransformSystems::Propagate),
            );
            // Installed by `VrmPlugin` exactly like this (`src/vrm.rs:172`).
            app.add_systems(
                PostUpdate,
                (sync_simple_transforms, propagate_parent_transforms)
                    .chain()
                    .in_set(VrmSystemSets::PropagateAfterConstraints)
                    .after(VrmSystemSets::Constraints)
                    .before(VrmSystemSets::GazeControl),
            );
            app.add_systems(
                PostUpdate,
                observe_at_gaze
                    .in_set(VrmSystemSets::GazeControl)
                    .after(VrmSystemSets::PropagateAfterConstraints),
            );
        }

        let mut app = test_app();
        register(&mut app);
        app.init_resource::<Observed>();

        let root = app
            .world_mut()
            .spawn(Transform::from_xyz(0.0, 1.0, 0.0))
            .id();
        let source = app
            .world_mut()
            .spawn((Transform::IDENTITY, rest(Quat::IDENTITY), Animated))
            .id();
        let destination = app
            .world_mut()
            .spawn((
                Transform::from_xyz(0.0, 0.0, 0.5),
                rest(Quat::IDENTITY),
                ChildOf(root),
                VrmNodeConstraint {
                    source,
                    weight: 1.0,
                    kind: VrmConstraintKind::Rotation,
                },
            ))
            .id();
        app.world_mut()
            .spawn(ConstraintExecutionOrder(vec![destination]));
        app.insert_resource(Watched(destination));

        app.update();

        let expected = Quat::from_rotation_x(FRAC_PI_2);
        let observed = app.world().resource::<Observed>();
        assert!(
            observed
                .after_propagate
                .is_some_and(|seen| rotation_is(seen, expected)),
            "the constraint had not run when TransformSystems::Propagate finished: {:?}",
            observed.after_propagate
        );

        let global = observed
            .at_gaze
            .expect("GazeControl must see the destination's GlobalTransform");
        assert!(
            rotation_is(global.rotation(), expected),
            "the manual propagation published {:?}",
            global.rotation()
        );
        assert!(
            global
                .translation()
                .abs_diff_eq(Vec3::new(0.0, 1.0, 0.5), 1e-5),
            "the manual propagation published {:?}",
            global.translation()
        );
        success!()
    }
}
