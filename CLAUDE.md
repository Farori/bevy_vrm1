# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

`bevy_vrm1` is a Bevy plugin for loading and animating VRM 1.0 models and VRMA animations. It supports Spring Bone physics, LookAt gaze control, Node Constraints, and Expression systems following the official VRM specification update order.

**Important**: Only VRM 1.0 is supported. This crate is in early development and may undergo breaking changes.

## Development Commands

### Build and Check
```bash
# Check compilation
cargo check

# Build the project
cargo build

# Build with features
cargo build --features serde,log
```

### Testing
```bash
# Run all tests
cargo test

# Run a specific test
cargo test test_name

# Run tests with logging
cargo test --features log
```

### Running Examples
Every example adds `(DefaultPlugins, VrmPlugin, VrmGltfPlugin)` and spawns its
avatar with `spawn_vrm`. They all need a window; there is no headless example.
```bash
# Basic VRM loading
cargo run --example simple

# Spring bone physics demo
cargo run --example spring_bone

# LookAt demos
cargo run --example look_at_cursor
cargo run --example look_at_target

# Upper-body gaze
cargo run --example body_tracking

# Expression triggers from the keyboard
cargo run --example expressions

# First/third-person view switching
cargo run --example first_person

# VRMA animation playback
cargo run --example vrma
cargo run --example vrma_transition

# MToon multiple directional lights
cargo run --example multiple_lights
```

### Linting
The project uses Clippy with custom lints defined in `Cargo.toml`:
```bash
cargo clippy
```

## Architecture Overview

### Plugin Structure

The `VrmPlugin` is the **runtime** entry point that orchestrates sub-plugins:

```
VrmPlugin
├── VrmDetachPlugin       (RequestDetachVrm observer)
├── VrmSpringBonePlugin   (type registration + SpringBoneUpdatePlugin)
├── VrmHumanoidBonePlugin (bone markers / holder type registration)
├── VrmExpressionPlugin   (Set/Modify/Clear expression trigger observers)
├── MtoonMaterialPlugin   (shader & material rendering, + MToonOutlinePlugin)
├── LookAtPlugin          (gaze control systems)
└── BodyTrackingPlugin    (upper-body gaze)
```

`VrmPlugin` also adds the two manual transform-propagation passes
(`PropagateAfterConstraints`, `PropagateAfterExpressions`) and the
`spawn_vrm` watchdog (`spawn::warn_never_ready`).

Loading a `.vrm` is `VrmGltfPlugin` (`src/vrm/gltf.rs`), which is the only
plugin that claims the `vrm` extension. It registers the extension handler,
preregisters `VrmLoader`, adds `MtoonMaterialPlugin` and
`VrmGltfRuntimePlugin` (guarded), registers every type the scene asset
carries, and schedules `apply_expression_morph_binds`. VRMA (animation) is a
separate plugin (`VrmaPlugin`) that works alongside both and keeps its own
loader, because two loaders claiming one extension makes `AssetServer` refuse
the app at registration time.

### VRM Asset Loading Pipeline

Everything happens **while the file loads**, inside `bevy_gltf`'s
`GltfExtensionHandler` hooks (`src/vrm/gltf/handler*`), writing into the scene
`WorldAsset` before it is frozen (`WorldAsset::new(world)`):

1. **Load**: `spawn_vrm` (or a hand-written `WorldAssetRoot`) loads
   `GltfAssetLabel::Scene(0)` of the path through `VrmLoader`
2. **`on_root`** (`handler::root`): `VRMC_vrm` / `VRMC_springBone` are parsed
   into `VrmLoadState`, and the avatar's `VrmForwardPolicy` is resolved from
   bevy's `GltfConvertCoordinates` flags
3. **`on_material`** (`handler::materials`): `VRMC_materials_mtoon` (and
   `VRMC_materials_hdr_emissiveMultiplier`) become labeled `MToonMaterial`
   assets
4. **`on_gltf_primitive`** (`handler::first_person`): the `firstPerson`
   classification and the `auto` head split, which has to replace the `Mesh`
   asset here because this is the only hook that runs before bevy reads the
   vertices
5. **`on_gltf_node`** (`handler::nodes`): `VrmNodeIndex`, `VrmBone` and a
   `PendingNodeConstraint` per constrained destination — *parsed*, not resolved,
   because a node may not have been visited yet
6. **`on_spawn_mesh_and_material`** (`handler::materials`): the per-primitive
   entity map, and the `StandardMaterial` → `MToonMaterial` swap
7. **`on_scene_completed`** (`handler::scene::finalize`): resolves the pending
   constraints into `VrmNodeConstraint` + the root's `ConstraintExecutionOrder`,
   builds the spring chains, builds the expression data, snapshots rest
   transforms, sets up the `AnimationPlayer` on the root bone and the
   `AnimationTargetId`s the VRMA retarget binds to, spawns the `VrmHeadOnly`
   copies and their render layers, inserts the `<Bone>BoneEntity` holders,
   records `VrmPath`, and writes `Vrm` + `Initialized` on the scene root
8. **Instantiation**: the scene spawns as a *child* of the spawned entity, with
   every `Entity` field remapped through `Component::map_entities` — so any
   component written here holding an `Entity` **must** mark that field
   `#[entities]`

### Critical System Execution Order (VrmSystemSets)

The system execution order follows the [VRM specification](https://vrm.dev/api/api_update/):

```
Animation (Bevy standard)
    ↓
VrmSystemSets::Constraints
    ↓
VrmSystemSets::PropagateAfterConstraints (manual transform propagation)
    ↓
VrmSystemSets::GazeControl (LookAt)
    ↓
VrmSystemSets::Expressions
    ↓
VrmSystemSets::PropagateAfterExpressions (manual transform propagation)
    ↓
VrmSystemSets::SpringBone
    ↓
VrmSystemSets::DetermineRedraw (triggers RequestRedraw if needed)
```

**Important**: Manual transform propagation is inserted at two points to ensure `GlobalTransform` is updated before downstream systems use it. This is critical for correct rendering and physics.

### Key Architectural Patterns

#### 1. Resolve at load time, not by name
References between glTF nodes (`VRMC_node_constraint`, `VRMC_springBone`,
`VRMC_vrm.firstPerson`, `morphTargetBinds`) are resolved inside the loader, where
every referenced entity already exists. The result is stored as data:
- `VrmNodeIndex` on every node, the lookup key for everything
- `ConstraintExecutionOrder` on the scene root, plus `VrmNodeConstraint` per destination
- `SpringRoot` / `SpringJointProps` / `SpringJointState` / `ColliderShape` per joint
- `VrmExpressionWeights` / `ExpressionSettings` / `ExpressionMorphBinds` on the root

`HumanoidBoneRegistry` (`VrmBone -> node Name`) is the one registry left, and it
exists only for the **VRMA source rig**: a `.vrma` carries node *references*, not
names, so its own rig is built at load time from its `vrmc_vrm_animation.humanoid`.

#### 2. RestTransform Baseline
Systems use stored `RestTransform`/`RestGlobalTransform`/`RestWorldTransform` (captured while the file loads, before any system runs) as a baseline to compute deltas. This enables multiple systems to read the same base state without conflicts.

#### 3. `#[entities]` is mandatory
The scene spawn pipeline remaps entity references through
`Component::map_entities`, which `#[derive(Component)]` generates from
`#[entities]` field attributes. A component written into the scene asset that
holds an `Entity` without `#[entities]` keeps the loader's scratch-world id: inert
at best, a wrong bone at worst. See `src/macros.rs` (`entity_component!`) and
`src/vrm/components.rs`.

#### 4. VRMA Retargeting
A `.vrma` is loaded by `VrmaLoaderPlugin` into a `VrmaAsset`, spawned as a
`VrmaHandle` **child** of an already-initialized avatar, and retargeted at
runtime — per instance, against that avatar's own bones. There are no custom
animation curve wrappers: `apply_retarget_animation_clips` /
`retarget_expression_curves` sample the source curve through the evaluator
pipeline (`bake.rs`) and write plain `AnimatableKeyframeCurve`s into the
avatar's own **cloned** clip, so two avatars playing one `.vrma` cannot corrupt
each other. The per-bone `Transformation` tables are keyed by the VRMA entity
(`RetargetRotationTable` / `RetargetTranslationTable`), which is stable across
graph rebuilds. `.vrma` expression tracks are retargeted into
`ExpressionWeightProperty` curves on the avatar root. `VrmaMask` /
`VrmMaskGroup` (`animation::mask`) map humanoid bones onto bevy's 64-bit
`AnimationMask` bitfields so a layer can drive only part of the body.

## Component Constraints

### Node Constraint System

Three constraint types, all evaluated by one ordered pass
(`apply_node_constraints`, `src/vrm/runtime.rs`) in
`VrmSystemSets::Constraints`. They do **not** run in parallel: `ConstraintExecutionOrder`
is the scene's topological order, computed while the file loaded, and the pass
walks it in sequence so a destination that is itself a source sees a fresh value.

- **RotationConstraint**: Transfers entire local rotation from source to destination (use case: sub-arms)
- **RollConstraint**: Transfers rotation around a specific axis only (use case: twist bones)
- **AimConstraint**: Rotates destination to face source (use case: clothing accessories)

All use spherical linear interpolation (slerp) based on weight parameter (0.0-1.0).

### Spring Bone Physics

- Runs **after** all pose changes in `VrmSystemSets::SpringBone`
- Uses Verlet integration for physics simulation
- Each `SpringRoot` contains a chain of joints with collision detection
- Center node defines reference frame for inertia calculations

### LookAt System

Two modes:
- **Cursor Mode**: Tracks mouse cursor position via camera ray casting
- **Target Mode**: Tracks a specific entity

Updates `Head`, `LeftEye`, `RightEye` bone rotations based on `LookAtProperties` ranges.

## Transform Propagation Strategy

Bevy's default `TransformPropagate` runs once in `PostUpdate`. This crate manually invokes transform propagation **twice** to comply with VRM spec:

1. **After Constraints**: Ensures constraint changes propagate to `GlobalTransform` before LookAt reads positions
2. **After Expressions**: Ensures expression changes propagate before SpringBone physics reads positions

This is implemented in `src/vrm.rs` using:
```rust
use bevy::transform::systems::{propagate_parent_transforms, sync_simple_transforms};

app.add_systems(
    PostUpdate,
    (sync_simple_transforms, propagate_parent_transforms)
        .chain()
        .in_set(VrmSystemSets::PropagateAfterConstraints)
);
```

## Working with VRM Specifications

When modifying update order or system timing, always reference:
- [VRM Update Order Specification](https://vrm.dev/api/api_update/)
- [Spring Bone Specification](https://github.com/vrm-c/vrm-specification/blob/master/specification/VRMC_springBone-1.0/README.md)
- [Node Constraint Specification](https://github.com/vrm-c/vrm-specification/blob/master/specification/VRMC_node_constraint-1.0/README.md)
- [LookAt Specification](https://github.com/vrm-c/vrm-specification/blob/master/specification/VRMC_vrm-1.0/lookAt.md)
- [VRMA Specification](https://github.com/vrm-c/vrm-specification/blob/master/specification/VRMC_vrm_animation-1.0/README.md)

## Version Compatibility

| bevy_vrm1 | bevy |
|-----------|------|
| 0.8.0 ~   | 0.19 |
| 0.5.0 ~   | 0.18 |
| 0.4.0 ~   | 0.17 |
| 0.1.0 ~   | 0.16 |

Rust edition: 2024

## Module Organization

```
src/
├── lib.rs                  (Main exports)
├── error.rs                (vrm_error! / vrm_warn! macros)
├── macros.rs               (new_type!, marker_component!, entity_component! with #[entities])
├── system_set.rs           (VrmSystemSets enum)
├── system_param.rs         (Helper system params: ChildSearcher, ParentSearcher, etc.)
├── system_param/           (cameras.rs, child_searcher.rs, parent_searcher.rs, vrm_animation.rs)
├── vrm/                    (VRM 1.0 implementation)
│   ├── spawn.rs            (spawn_vrm: the supported way to put a .vrm in an app)
│   ├── components.rs       (Components + render-layer helpers the handler writes)
│   ├── coords.rs           (VrmForwardPolicy: glTF +Z vs bevy -Z)
│   ├── detach.rs           (RequestDetachVrm observer)
│   ├── expressions.rs      (Set/Modify/Clear expression triggers)
│   ├── runtime.rs          (VrmGltfRuntimePlugin + apply_node_constraints)
│   ├── body_tracking.rs    (BodyTracking: upper-body gaze)
│   ├── humanoid_bone.rs    (Bone markers, holders, VRMA source registry)
│   ├── humanoid_bone/      (bones.rs: the bone marker/holder types)
│   ├── look_at.rs          (Gaze control)
│   ├── spring_bone.rs      (Spring physics)
│   ├── spring_bone/        (update.rs: the simulation)
│   ├── mtoon/              (Shader implementation)
│   └── gltf/               (VrmLoader + GltfExtensionHandler; glTF extension parsing)
│       ├── extensions.rs   (VrmExtensions: the VRMC_* entry points)
│       ├── extensions/     (vrmc_vrm.rs, vrmc_spring_bone.rs, vrmc_node_constraint.rs)
│       ├── materials.rs    (VRMC_materials_* schemas)
│       ├── handler.rs      (VrmExtensionHandler: the six hooks)
│       └── handler/        (root.rs, nodes.rs, materials.rs, first_person.rs, scene.rs)
└── vrma/                   (VRMA animation implementation)
    ├── loader.rs           (VrmaAsset / VrmaLoaderPlugin)
    ├── gltf.rs             (VRMC_vrm_animation schemas)
    ├── gltf/extensions.rs
    ├── initialize.rs       (VRMA scene setup, per-instance clip clone)
    └── animation/          (Retargeting system)
        ├── animation_graph.rs, bake.rs, bone_rotation.rs, bone_translation.rs
        ├── expressions.rs, mask.rs, play.rs, properties.rs
```

## Testing Notes

- Tests use `bevy_test_helper` for setting up minimal Bevy apps
- Test VRM models are in `assets/` (excluded from crate publication)
- Sample model credit: **AliciaSolid** by **© DWANGO Co., Ltd.**

## Common Pitfalls

1. **System Ordering**: When adding new VRM-related systems, always ensure they run in the correct `VrmSystemSets` and respect the specification order
2. **Transform Propagation**: If a system modifies `Transform` and another system needs to read `GlobalTransform` in the same frame, manual propagation may be needed
3. **Bone holders are load-time data**: a system that needs a bone addresses it through the `<Bone>BoneEntity` holder on the VRM root (`HeadBoneEntity`, `SpineBoneEntity`, …). Those are written into the scene asset by `handler::scene::insert_humanoid_bone_holders` and remapped on instantiation — do not resolve bones by name at runtime
4. **No `Changed<Transform>` filters**: the constraint pass reads the whole `ConstraintExecutionOrder` every frame, so the ordering edge to `AnimationSystems` (`src/vrm/runtime.rs`) is what makes it see this frame's animation output
5. **`AssetServer` loads do not complete in `cargo test`**: an end-to-end `asset_server.load(..)` test hangs in the IO pool. Test the handler steps directly instead — see the module docs in `src/vrm/gltf.rs`
