//! [`spawn_vrm`]: the supported way to put a `.vrm` into an app.
//!
//! # Why a helper is needed at all
//!
//! Under the load-time glTF pipeline the entity that carries an avatar's
//! components is **not** the entity a spawn creates.
//! `bevy_world_serialization` instantiates the scene
//! [`WorldAsset`](bevy::world_serialization::WorldAsset)
//! *asynchronously*: [`WorldInstanceSpawner::spawn_queued_instances`] only runs
//! once the asset has been added, and only then writes the scene into the world
//! and parents its roots under the spawned entity
//! (`set_instance_parent_sync`, `bevy_world_serialization/src/world_asset_spawner.rs:501-520`).
//! [`Initialized`], [`LookAtProperties`], [`VrmExpressionWeights`],
//! [`ConstraintExecutionOrder`], the `AnimationPlayer` and every
//! [`VrmBone`](crate::prelude::VrmBone) descendant therefore land on an entity
//! that does not exist yet at spawn time.
//!
//! A raw `commands.spawn(WorldAssetRoot(asset_server.load(..)))` consequently
//! forces every app into the same readiness dance: a system on
//! `Added<Initialized>` plus a lookup, only to put [`LookAt`] or [`BodyTracking`]
//! on the right entity. [`spawn_vrm`] collapses that dance into one call and
//! hands the closure the root itself.
//!
//! # How the instance is resolved
//!
//! An observer on [`WorldInstanceReady`], registered **on the spawned entity
//! itself**. That event is an
//! [`EntityEvent`](bevy::ecs::event::EntityEvent) whose `entity` field is the
//! `WorldAssetRoot`'s own entity
//! (`bevy_world_serialization/src/world_asset_spawner.rs:31-38`), and bevy runs
//! entity-scoped observers on `event.event_target()`
//! (`bevy_ecs/src/event/trigger.rs:146`). Registering the observer per entity is
//! what makes this match *one* instance: a global observer, or a system on
//! `Added<Initialized>`, would also fire for a second avatar in the same app, and
//! a `.vrma` source rig keys on `Initialized` too
//! (`src/vrma/initialize.rs:118-121`).
//!
//! `set_instance_parent_sync` has already run when the event is delivered
//! (`world_asset_spawner.rs:633-642`), so the VRM root is a child of the spawned
//! entity by then. `bevy_gltf` builds each scene around exactly one root entity
//! (`bevy_gltf/src/loader/mod.rs:1029-1067`), and that is the entity
//! `handler::scene::mark_initialized` writes [`Vrm`] and [`Initialized`] onto
//! (`src/vrm/gltf/handler/scene.rs:73,89-94`). The root is therefore the only
//! child of the spawned entity that carries [`Initialized`].
//!
//! # What is *not* guaranteed, and what happens instead
//!
//! `bevy_world_serialization` re-instantiates a `WorldAssetRoot` when the asset
//! is hot-reloaded (`WorldInstanceSpawner::update_spawned_instances`,
//! `world_asset_spawner.rs:358-382`) and again when the component itself changes
//! (`world_instance_spawner`, `:688-708`). The replacement is a *different*
//! instance with *different* entity ids, so `configure` — an `FnOnce` — cannot
//! run on it a second time. That is reported rather than swallowed:
//! [`SpawnAction::Reconfigured`] names the new root and says `configure` has
//! already run. A duplicated `configure` is therefore impossible, not merely
//! unlikely, and a skipped one is never silent.
//!
//! An instance that never arrives at all (a file that fails to load, or a
//! spawned entity that is despawned first) is reported once by
//! [`warn_never_ready`] and its pending closure is dropped — never a panic and
//! never a leaked closure.
//!
//! # Requirement
//!
//! [`VrmGltfPlugin`](crate::prelude::VrmGltfPlugin) has to be in the app: it is
//! the only plugin that claims the `vrm` extension, and it is what writes
//! [`Initialized`] into the scene asset this call loads.

use bevy::asset::AssetServer;
use bevy::ecs::world::World;
use bevy::gltf::GltfAssetLabel;
use bevy::platform::collections::HashMap;
use bevy::prelude::*;
use bevy::world_serialization::{WorldAssetRoot, WorldInstanceReady};

use crate::error::vrm_warn;
use crate::vrm::Initialized;

/// The caller's `configure` closure, boxed so a pending spawn fits in a
/// [`Resource`] and can be handed to an observer.
type Configure = Box<dyn FnOnce(&mut EntityCommands) + Send + Sync>;

/// How many frames a [`spawn_vrm`] call waits for its instance before it gives
/// up. Generous on purpose: a large `.vrm` on a cold cache has to be read, parsed
/// and written into the world first, and waiting longer only costs a closure
/// held for a few seconds more.
const NEVER_READY_AFTER_FRAMES: u32 = 1200;

/// One [`spawn_vrm`] call whose instance has not been configured yet.
///
/// The entry outlives the instance on purpose: a tombstone (`configure: None`)
/// is what lets a *replaced* instance warn instead of silently skipping
/// `configure`, and it is also what stops the watchdog from timing out a spawn
/// that has already been resolved.
struct PendingVrmSpawn {
    /// The asset path, quoted in every warning this spawn can emit.
    path: String,
    /// Frames since [`spawn_vrm`] returned. Only the watchdog advances it.
    frames_waited: u32,
    /// The caller's closure, dropped as soon as it has run — so `None` is both
    /// "already configured" and "already given up on".
    configure: Option<Configure>,
}

impl PendingVrmSpawn {
    /// Whether the watchdog should still be waiting for this spawn.
    fn is_waiting(&self) -> bool {
        self.configure.is_some()
    }
}

/// Every [`spawn_vrm`] call whose instance is not resolved yet, keyed by the
/// entity that carries its [`WorldAssetRoot`].
#[derive(Resource, Default)]
pub(crate) struct PendingVrmSpawns(HashMap<Entity, PendingVrmSpawn>);

/// Spawns a `.vrm` through the load-time glTF pipeline and runs `configure` on
/// the instantiated VRM root once it is ready.
///
/// The file is loaded as [`GltfAssetLabel::Scene(0)`] of `path`, exactly like a
/// hand-written [`WorldAssetRoot`]; [`VrmGltfPlugin`] has to be in the app for the
/// loader to claim the `vrm` extension, and [`VrmPlugin`](crate::vrm::VrmPlugin)
/// for the runtime systems and for the watchdog that reports an instance which
/// never arrives.
///
/// `configure` runs **once**, on the entity that carries [`Initialized`] — the
/// VRM root the scene asset instantiates — and may insert components on it,
/// trigger events on it, or both. See the [module docs](self) for how the
/// instance is resolved, and what happens when it is replaced or never arrives.
///
/// The return value is the *parent* entity: the one that positions the avatar,
/// and the root ancestor of every mesh it owns. Give it a `Transform`, or parent
/// it to whatever the app needs.
///
/// ```no_run
/// use bevy::prelude::*;
/// use bevy_vrm1::prelude::*;
///
/// fn spawn_avatar(mut commands: Commands) {
///     spawn_vrm(&mut commands, "vrm/Elmer.vrm", |root| {
///         // `root` is the VRM root: components go on it …
///         root.insert(LookAt::Cursor);
///         // … and so do events.
///         root.trigger(|root| SetExpressions::single(root, "happy", 1.0));
///     })
///     // The spawned entity is the avatar's parent, which is what an app moves.
///     .insert(Transform::from_xyz(0.0, 0.0, 1.0));
/// }
/// ```
pub fn spawn_vrm<'a>(
    commands: &'a mut Commands,
    path: impl Into<String>,
    configure: impl FnOnce(&mut EntityCommands) + Send + Sync + 'static,
) -> EntityCommands<'a> {
    let path = path.into();
    // The observer below reads this resource, so it has to exist before the
    // observer can possibly fire. Idempotent, which is why it is not guarded
    // against `VrmPlugin`, which initialises it too.
    commands.init_resource::<PendingVrmSpawns>();
    let parent = commands.spawn_empty().id();
    // `AssetServer` is not reachable from `&mut Commands`, so the load is
    // deferred into the same queue: by the time this runs, the entity exists and
    // the resource is already inserted.
    commands.queue(move |world: &mut World| {
        // Scoped to this entity, which is what `WorldInstanceReady` is triggered
        // on. Registering it per entity is what matches *one* instance.
        world.entity_mut(parent).observe(configure_ready_vrm);
        let scene = world
            .resource::<AssetServer>()
            .load(GltfAssetLabel::Scene(0).from_asset(path.clone()));
        world.entity_mut(parent).insert(WorldAssetRoot(scene));
        world.resource_mut::<PendingVrmSpawns>().0.insert(
            parent,
            PendingVrmSpawn {
                path,
                frames_waited: 0,
                configure: Some(Box::new(configure)),
            },
        );
    });
    commands.entity(parent)
}

/// The observer [`spawn_vrm`] registers on the entity it spawns.
fn configure_ready_vrm(
    ready: On<WorldInstanceReady>,
    children: Query<&Children>,
    marked: Query<(), With<Initialized>>,
    mut commands: Commands,
    mut pending: ResMut<PendingVrmSpawns>,
) {
    let parent = ready.event_target();
    let Some(mut spawn) = pending.0.remove(&parent) else {
        // Only reachable when the watchdog already gave up on this spawn, which
        // it has warned about.
        return;
    };
    spawn.frames_waited = 0;
    // Scoped to this instance's own children, so another avatar's root and a
    // `.vrma` source rig cannot be picked up.
    let candidates: Vec<Entity> = children
        .get(parent)
        .into_iter()
        .flatten()
        .copied()
        .filter(|child| marked.contains(*child))
        .collect();

    match decide_ready(spawn.configure.is_some(), resolve_vrm_root(&candidates)) {
        SpawnAction::Configure(root) => {
            let configure = spawn
                .configure
                .take()
                .expect("`decide_ready` only returns `Configure` while the closure is armed");
            let mut root_commands = commands.entity(root);
            configure(&mut root_commands);
        }
        SpawnAction::Reconfigured(root) => {
            vrm_warn!(format!(
                "VRM: the instance of `{}` was replaced, so its new root {root:?} is \
                 unconfigured; `configure` has already run once",
                spawn.path,
            ));
        }
        SpawnAction::NotAVrm => {
            spawn.configure = None;
            vrm_warn!(format!(
                "VRM: the instance of `{}` spawned, but no `Initialized` child is under it, so \
                 it is not a VRM; `configure` is not called",
                spawn.path,
            ));
        }
        SpawnAction::Ambiguous(count) => {
            spawn.configure = None;
            vrm_warn!(format!(
                "VRM: the instance of `{}` has {count} `Initialized` children, so which one is \
                 the root is not decidable; `configure` is not called",
                spawn.path,
            ));
        }
    }
    // Re-inserted even after `configure` ran: see [`PendingVrmSpawn`].
    pending.0.insert(parent, spawn);
}

/// Reports every spawn whose instance never arrived, and forgets it.
///
/// Registered by [`VrmPlugin`](crate::vrm::VrmPlugin). One warning per spawn:
/// the entry is dropped, so a later ready event finds nothing and stays silent
/// rather than repeating it.
pub(crate) fn warn_never_ready(mut pending: ResMut<PendingVrmSpawns>) {
    pending.0.retain(|_, spawn| {
        if !spawn.is_waiting() {
            // Already resolved: kept as a tombstone, never timed out.
            return true;
        }
        spawn.frames_waited = spawn.frames_waited.saturating_add(1);
        if !has_timed_out(spawn.frames_waited) {
            return true;
        }
        vrm_warn!(format!(
            "VRM: the instance of `{}` never became ready within {NEVER_READY_AFTER_FRAMES} \
             frames (the asset may have failed to load, or the spawned entity may have been \
             despawned); `configure` is not called",
            spawn.path,
        ));
        false
    });
}

/// Whether a spawn that has waited `frames_waited` frames has given up.
fn has_timed_out(frames_waited: u32) -> bool {
    frames_waited >= NEVER_READY_AFTER_FRAMES
}

/// Which of an instance's children is the VRM root.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
enum RootResolution {
    /// Exactly one child carries [`Initialized`].
    Found(Entity),
    /// No child does, so the instance is not a pipeline VRM.
    NotAVrm,
    /// Several children do, which `bevy_gltf` should never produce.
    Ambiguous(usize),
}

/// The pure half of root resolution: exactly one candidate is the root.
fn resolve_vrm_root(candidates: &[Entity]) -> RootResolution {
    match candidates {
        [] => RootResolution::NotAVrm,
        [root] => RootResolution::Found(*root),
        many => RootResolution::Ambiguous(many.len()),
    }
}

/// What one ready instance means for one [`spawn_vrm`] call.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
enum SpawnAction {
    /// Run `configure` on this root.
    Configure(Entity),
    /// The instance is a VRM, but `configure` already ran for an earlier instance
    /// of the same spawn, so it cannot run again.
    Reconfigured(Entity),
    /// The instance spawned without a VRM root.
    NotAVrm,
    /// The instance has more than one VRM root.
    Ambiguous(usize),
}

/// The pure half of the readiness decision.
///
/// `armed` is whether `configure` is still waiting to run. It is the only thing
/// separating a first instance from a re-instantiated one, which is what makes a
/// duplicated `configure` impossible instead of merely unlikely.
fn decide_ready(
    armed: bool,
    resolution: RootResolution,
) -> SpawnAction {
    match (armed, resolution) {
        (_, RootResolution::NotAVrm) => SpawnAction::NotAVrm,
        (_, RootResolution::Ambiguous(count)) => SpawnAction::Ambiguous(count),
        (false, RootResolution::Found(root)) => SpawnAction::Reconfigured(root),
        (true, RootResolution::Found(root)) => SpawnAction::Configure(root),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::success;
    use crate::tests::{TestResult, test_app};
    use crate::vrm::{Vrm, VrmBone};
    use bevy::world_serialization::{
        DynamicWorld, WorldAsset, WorldInstanceSpawner, world_instance_spawner_system,
    };

    /// What the test `configure` closure inserts, so "did it run, and on what" is
    /// observable without a renderer.
    #[derive(Component)]
    struct Configured(Entity);

    /// The VRM root of a scene asset: one root carrying `Vrm` + `Initialized`
    /// (what `mark_initialized` writes), with a bone below it (what `VrmBone`
    /// looks like).
    fn vrm_scene_asset() -> WorldAsset {
        let mut asset_world = World::new();
        let root = asset_world.spawn((Vrm, Initialized)).id();
        asset_world.spawn((ChildOf(root), VrmBone::from("head")));
        WorldAsset::new(asset_world)
    }

    /// A pending spawn for `parent`, as [`spawn_vrm`] would have left it.
    fn pending(
        parent: Entity,
        path: &str,
    ) -> PendingVrmSpawn {
        PendingVrmSpawn {
            path: path.to_owned(),
            frames_waited: 0,
            configure: Some(Box::new(|root: &mut EntityCommands| {
                let id = root.id();
                root.insert(Configured(id));
            })),
        }
    }

    /// A headless app that drives `WorldInstanceSpawner` by hand, so the real
    /// instantiation path — parent the asset's roots, then trigger
    /// `WorldInstanceReady` on the parent — runs without a renderer.
    ///
    /// `WorldSerializationPlugin` is deliberately not used: its `Plugin` impl is
    /// `#[cfg(feature = "serialize")]`, and that feature is *not* stable across
    /// this crate's builds — `cargo check` of the lib sees only the reduced
    /// `bevy` feature set, while `cargo test` unifies in the dev-dependency's
    /// default features. The three things the plugin would do here are done
    /// explicitly instead.
    fn spawner_app() -> bevy::app::App {
        let mut app = test_app();
        app.init_asset::<WorldAsset>()
            // `world_instance_spawner_system` reads `AssetEvent<DynamicWorld>`
            // unconditionally, so the spawner needs both asset storages.
            .init_asset::<DynamicWorld>()
            .init_resource::<WorldInstanceSpawner>()
            .init_resource::<PendingVrmSpawns>()
            .register_type::<Initialized>()
            .register_type::<Vrm>()
            .register_type::<ChildOf>()
            .register_type::<Children>()
            .add_systems(Update, (world_instance_spawner_system, warn_never_ready));
        app
    }

    /// `bevy_gltf` gives every scene exactly one root entity
    /// (`bevy_gltf/src/loader/mod.rs:1029-1067`), and `mark_initialized` writes
    /// `Vrm` + `Initialized` on it, so exactly one candidate is the normal case.
    #[test]
    fn one_initialized_child_is_the_root() {
        assert_eq!(
            resolve_vrm_root(&[entity(7)]),
            RootResolution::Found(entity(7)),
        );
    }

    /// No candidate means the file was not a VRM: the handler never wrote
    /// `Initialized`, or its components were not registered and were dropped by
    /// `ReflectComponent::apply_or_insert_mapped`. Either way `configure` must not
    /// guess a root.
    #[test]
    fn no_initialized_child_is_not_a_vrm() {
        assert_eq!(resolve_vrm_root(&[]), RootResolution::NotAVrm);
    }

    /// Refuses to pick one of several roots rather than silently configuring the
    /// wrong entity.
    #[test]
    fn several_initialized_children_are_ambiguous() {
        assert_eq!(
            resolve_vrm_root(&[entity(7), entity(8), entity(9)]),
            RootResolution::Ambiguous(3),
        );
    }

    /// The first instance configures. A second instance — a hot reload, or a
    /// changed `WorldAssetRoot` — cannot, because `configure` is an `FnOnce`, so
    /// it is reported instead of silently dropped.
    #[test]
    fn a_replaced_instance_is_reported_not_reconfigured() {
        assert_eq!(
            decide_ready(true, RootResolution::Found(entity(7))),
            SpawnAction::Configure(entity(7)),
        );
        assert_eq!(
            decide_ready(false, RootResolution::Found(entity(8))),
            SpawnAction::Reconfigured(entity(8)),
        );
    }

    /// A file that is not a VRM is reported exactly once, whatever the state of
    /// the closure, so a re-instantiation of the same non-VRM scene cannot turn
    /// into a misleading "replaced instance" warning.
    #[test]
    fn a_non_vrm_is_reported_whatever_the_state() {
        for armed in [true, false] {
            assert_eq!(
                decide_ready(armed, RootResolution::NotAVrm),
                SpawnAction::NotAVrm,
            );
            assert_eq!(
                decide_ready(armed, RootResolution::Ambiguous(2)),
                SpawnAction::Ambiguous(2),
            );
        }
    }

    /// The watchdog gives up late enough for a slow load and early enough that a
    /// closure is never held for a whole session.
    #[test]
    fn the_watchdog_gives_up_exactly_once() {
        assert!(!has_timed_out(0));
        assert!(!has_timed_out(NEVER_READY_AFTER_FRAMES - 1));
        assert!(has_timed_out(NEVER_READY_AFTER_FRAMES));
        assert!(
            has_timed_out(NEVER_READY_AFTER_FRAMES + 1),
            "the predicate must not become false again once the deadline passed"
        );
    }

    /// The end-to-end case, headless: the observer finds the VRM root of *its
    /// own* instance and runs `configure` on it, while a second avatar of the
    /// same app in the same asset is instantiated but left alone.
    ///
    /// [`spawn_vrm`] itself needs an `AssetServer` load, which a headless
    /// `cargo test` binary cannot complete (see the note in `crate::vrm::gltf`),
    /// so the pending entry is installed directly and the scene is instanced
    /// through the real [`WorldInstanceSpawner`]. Everything from
    /// `set_instance_parent_sync` onwards is the production path.
    #[test]
    fn the_observer_configures_only_its_own_instance() -> TestResult {
        let mut app = spawner_app();
        let scene = app
            .world_mut()
            .resource_mut::<Assets<WorldAsset>>()
            .add(vrm_scene_asset());
        let parent = app.world_mut().spawn_empty().id();
        let other_parent = app.world_mut().spawn_empty().id();
        app.world_mut()
            .entity_mut(parent)
            .observe(configure_ready_vrm);
        app.world_mut()
            .resource_mut::<PendingVrmSpawns>()
            .0
            .insert(parent, pending(parent, "vrm/Elmer.vrm"));
        app.world_mut()
            .resource_scope(|_world, mut spawner: Mut<WorldInstanceSpawner>| {
                spawner.spawn_as_child(scene.clone(), parent);
                spawner.spawn_as_child(scene.clone(), other_parent);
            });

        // `WorldInstanceReady` is delivered through the world's command queue, so
        // give it a few frames rather than pinning a delivery frame.
        for _ in 0..4 {
            app.update();
        }

        // `configure` ran exactly once, on the root of `parent`'s own instance.
        let configured: Vec<(Entity, Entity)> = {
            let mut query = app.world_mut().query::<(Entity, &Configured)>();
            query
                .iter(app.world())
                .map(|(id, configured)| (id, configured.0))
                .collect()
        };
        assert_eq!(
            configured.len(),
            1,
            "exactly one of the two avatars may be configured: {configured:?}"
        );
        let (root, configured_root) = configured[0];
        assert_eq!(
            configured_root, root,
            "`configure` must be handed the entity it runs on"
        );
        assert!(
            app.world().get::<Initialized>(root).is_some(),
            "the configured entity is the one carrying `Initialized`"
        );
        let children = app
            .world()
            .entity(parent)
            .get::<Children>()
            .expect("the VRM root is parented under the spawn");
        assert_eq!(**children, [root], "the root is this spawn's own child");
        assert_eq!(
            app.world()
                .entity(other_parent)
                .get::<Children>()
                .map(|children| children.len()),
            Some(1),
            "the second avatar still instantiated, it just was not configured"
        );

        // The entry survives as a tombstone, so a replaced instance warns.
        let pending_spawns = app.world().resource::<PendingVrmSpawns>();
        assert_eq!(pending_spawns.0.len(), 1, "the tombstone is kept");
        let spawn = &pending_spawns.0[&parent];
        assert!(!spawn.is_waiting(), "`configure` has run");
        assert_eq!(
            spawn.frames_waited, 0,
            "the watchdog must not time out a resolved spawn"
        );
        success!()
    }

    /// A scene that is not a VRM still produces a ready instance, so the observer
    /// has to report it rather than leave `configure` pending forever.
    #[test]
    fn a_non_vrm_instance_reports_and_arms_nothing() -> TestResult {
        let mut app = spawner_app();
        let scene = {
            let mut asset_world = World::new();
            asset_world.spawn_empty();
            app.world_mut()
                .resource_mut::<Assets<WorldAsset>>()
                .add(WorldAsset::new(asset_world))
        };
        let parent = app.world_mut().spawn_empty().id();
        app.world_mut()
            .entity_mut(parent)
            .observe(configure_ready_vrm);
        app.world_mut()
            .resource_mut::<PendingVrmSpawns>()
            .0
            .insert(parent, pending(parent, "model.gltf"));

        app.world_mut()
            .resource_scope(|_world, mut spawner: Mut<WorldInstanceSpawner>| {
                spawner.spawn_as_child(scene.clone(), parent);
            });
        for _ in 0..4 {
            app.update();
        }

        let spawn = &app.world().resource::<PendingVrmSpawns>().0[&parent];
        assert!(!spawn.is_waiting(), "the closure is dropped, not held");
        let configured = {
            let mut query = app.world_mut().query_filtered::<Entity, With<Configured>>();
            query.iter(app.world()).count()
        };
        assert_eq!(
            configured, 0,
            "`configure` must not run on a non-VRM instance"
        );
        success!()
    }

    /// An instance that never arrives — a failed load, or a parent despawned
    /// first — is warned about once and its closure dropped, not leaked.
    #[test]
    fn a_spawn_that_never_becomes_ready_is_dropped_once() -> TestResult {
        let mut app = test_app();
        app.init_resource::<PendingVrmSpawns>()
            .add_systems(Update, warn_never_ready);

        let parent = app.world_mut().spawn_empty().id();
        app.world_mut().resource_mut::<PendingVrmSpawns>().0.insert(
            parent,
            PendingVrmSpawn {
                path: "vrm/Missing.vrm".to_owned(),
                frames_waited: 0,
                // A closure that would panic if it ever ran.
                configure: Some(Box::new(|_: &mut EntityCommands| panic!("never"))),
            },
        );

        for _ in 0..NEVER_READY_AFTER_FRAMES - 1 {
            app.update();
            assert_eq!(
                app.world().resource::<PendingVrmSpawns>().0.len(),
                1,
                "the spawn is still waiting"
            );
        }
        app.update();
        assert!(
            app.world().resource::<PendingVrmSpawns>().0.is_empty(),
            "the timeout drops the entry, which is what stops the warning repeating"
        );

        // Further frames change nothing, and the dropped spawn is not resurrected.
        app.update();
        assert!(app.world().resource::<PendingVrmSpawns>().0.is_empty());
        success!()
    }

    fn entity(index: u32) -> Entity {
        Entity::from_raw_u32(index).expect("a valid entity index")
    }
}
