//! VRM glTF support: extension schemas, `MToon` material conversion, and the
//! glTF-pipeline loader ([`VrmGltfPlugin`]) that hooks VRM handling into the
//! stock Bevy glTF loader through
//! [`GltfExtensionHandler`](bevy::gltf::extensions::GltfExtensionHandler).
//!
//! The point of the extension handler is that every VRM-specific component is
//! written into the scene [`WorldAsset`](bevy::world_serialization::WorldAsset)
//! *while the file loads*, so a `SceneRoot` of a `.vrm` is born initialized
//! instead of being patched over several frames. `bevy_gltf` calls
//! `on_scene_completed` at `crates/bevy_gltf/src/loader/mod.rs:1112-1120` and
//! only then freezes the world with `WorldAsset::new(world)` at `:1122`, which
//! is what makes writing into `scene_world` legal.
//!
//! # Two loaders claim `.vrm`
//!
//! [`VrmLoader`] below and the crate's legacy `VrmLoaderPlugin` both register
//! the `vrm` extension. Bevy's `AssetServer` resolves a loader by the asset's
//! [`TypeId`] first and only then by extension, so a `Handle<VrmAsset>` still
//! resolves to the legacy loader while a `Handle<Gltf>` resolves to
//! [`VrmLoader`]; the two never claim each other's assets. This is deliberate
//! until the legacy path is deleted by a follow-up commit — see
//! [`VrmLoaderPlugin`](crate::vrm::loader::VrmLoaderPlugin).
//!
//! # `.vrma` is *not* claimed
//!
//! [`VrmLoader::extensions`] returns only `vrm`. `.vrma` belongs to the legacy
//! [`VrmaLoaderPlugin`](crate::vrm::loader::VrmLoaderPlugin) until the VRMA
//! commit adds its own loader, so that two loaders never both claim the
//! extension.
//!
//! # How this is tested
//!
//! There is deliberately **no** end-to-end test that loads `assets/vrm/Elmer.vrm`
//! through `AssetServer` and inspects the resulting `WorldAsset`. `AssetServer`
//! loads asynchronously on bevy's IO task pool, and that task never completes
//! inside a `cargo test` binary in this crate's headless configuration: the
//! handle stays in [`LoadState::Loading`](bevy::asset::LoadState::Loading), no
//! failure event is emitted, and instrumentation placed in [`VrmLoader::load`]
//! and in `on_root` shows neither is ever entered - the wait ends in the
//! `bevy_asset` file reader, before any VRM code runs. The coverage is therefore
//! split:
//!
//! * [`handler::root`] parses the real `Elmer.vrm` and `AliciaSolid.vrm` bytes
//!   with the `gltf` crate and drives the same `on_root` entry point;
//! * [`handler::scene`], [`handler::first_person`], [`handler::materials`] and
//!   [`handler::nodes`] each test their own step on hand-built worlds.
//!
//! `examples/simple.rs` is the end-to-end case, and it needs a window.

pub mod extensions;
pub mod handler;
pub mod materials;

pub mod prelude {
    pub use crate::vrm::gltf::{
        VrmGltfPlugin, VrmLoader,
        extensions::{VrmExtensions, VrmNode, vrmc_spring_bone::*, vrmc_vrm::*},
        materials::*,
    };
}

use bevy::asset::io::Reader;
use bevy::asset::{AssetApp, AssetLoader, LoadContext};
use bevy::gltf::extensions::GltfExtensionHandlers;
use bevy::gltf::{
    DefaultGltfImageSampler, Gltf, GltfError, GltfLoader, GltfLoaderSettings, GltfPlugin,
};
use bevy::image::{CompressedImageFormatSupport, CompressedImageFormats, ImageSamplerDescriptor};
use bevy::prelude::*;
use bevy::reflect::TypePath;
use std::sync::{Arc, Mutex};

use crate::prelude::MtoonMaterialPlugin;
use crate::prelude::{
    ColliderShape, ConstraintExecutionOrder, ExpressionMorphBinds, ExpressionSettings, Initialized,
    LookAtProperties, LookAtType, MorphBind, PendingNodeConstraint, RestGlobalTransform,
    RestTransform, RestWorldTransform, VrmBone, VrmConstraintKind, VrmExpressionWeights,
    VrmHeadOnly, VrmNodeConstraint, VrmNodeIndex,
};
use crate::vrm::gltf::handler::VrmExtensionHandler;

/// Registers the VRM extension handler and the `.vrm` asset loader.
///
/// Must be added **after** `DefaultPlugins`:
///
/// * the handler must be registered after `GltfPlugin` and `PbrPlugin`, because
///   `on_spawn_mesh_and_material` swaps the `StandardMaterial` that
///   `GltfExtensionHandlerPbr` has already put on the entity, and
/// * `finish` reuses resources inserted by `GltfPlugin::finish`
///   ([`DefaultGltfImageSampler`], [`GltfExtensionHandlers`]).
///
/// It also adds [`MtoonMaterialPlugin`], because the loader creates labeled
/// `MToonMaterial` assets, and
/// [`VrmGltfRuntimePlugin`](crate::vrm::runtime::VrmGltfRuntimePlugin), which
/// evaluates the node constraints this loader attaches.
#[derive(Default)]
pub struct VrmGltfPlugin;

impl Plugin for VrmGltfPlugin {
    fn build(
        &self,
        app: &mut App,
    ) {
        assert!(
            app.is_plugin_added::<GltfPlugin>(),
            "VrmGltfPlugin must be added after DefaultPlugins (GltfPlugin/PbrPlugin)",
        );

        // The loader creates labeled `MToonMaterial` assets. `VrmPlugin` also
        // adds this plugin, hence the guard.
        if !app.is_plugin_added::<MtoonMaterialPlugin>() {
            app.add_plugins(MtoonMaterialPlugin);
        }

        // Guarded so `VrmPlugin + VrmGltfPlugin` apps do not add it twice.
        if !app.is_plugin_added::<crate::vrm::runtime::VrmGltfRuntimePlugin>() {
            app.add_plugins(crate::vrm::runtime::VrmGltfRuntimePlugin);
        }

        // Everything written into the scene `WorldAsset` must be registered:
        // the scene spawn pipeline copies components out of the asset through
        // reflection, and drops the ones whose type is unknown
        // (`ReflectComponent::apply_or_insert_mapped`).
        app.register_type::<Initialized>()
            .register_type::<VrmBone>()
            .register_type::<RestTransform>()
            .register_type::<RestGlobalTransform>()
            .register_type::<RestWorldTransform>()
            .register_type::<VrmNodeIndex>()
            .register_type::<VrmHeadOnly>()
            .register_type::<PendingNodeConstraint>()
            .register_type::<VrmNodeConstraint>()
            .register_type::<VrmConstraintKind>()
            .register_type::<ConstraintExecutionOrder>()
            // Written on the root bone and every humanoid bone so the VRMA
            // retarget can recognise a model as a pose source. `VrmaPlugin`
            // registers it too; double registration is a no-op.
            .register_type::<crate::vrma::RetargetSource>()
            .register_type::<LookAtProperties>()
            .register_type::<LookAtType>()
            // Spring bones. Double registration with `VrmSpringBonePlugin` is a
            // no-op.
            .register_type::<crate::vrm::spring_bone::SpringRoot>()
            .register_type::<crate::vrm::spring_bone::SpringJoints>()
            .register_type::<crate::vrm::spring_bone::SpringColliders>()
            .register_type::<crate::vrm::spring_bone::SpringCenterNode>()
            .register_type::<crate::vrm::spring_bone::SpringJointProps>()
            .register_type::<crate::vrm::spring_bone::SpringJointState>()
            .register_type::<ColliderShape>()
            // The expression property types of `vrma::animation::properties`:
            // `apply_expression_morph_binds` reads the first three off the root.
            .register_type::<VrmExpressionWeights>()
            .register_type::<ExpressionMorphBinds>()
            .register_type::<ExpressionSettings>()
            .register_type::<MorphBind>();

        // `GltfConvertCoordinates` is a public type with public bool fields, so
        // only the two flags this crate cares about are read.
        let (default_rotate_scene_entity, default_rotate_meshes) = app
            .get_added_plugins::<GltfPlugin>()
            .first()
            .map(|gltf_plugin| {
                (
                    gltf_plugin.convert_coordinates.rotate_scene_entity,
                    gltf_plugin.convert_coordinates.rotate_meshes,
                )
            })
            .unwrap_or((false, false));

        let handler = VrmExtensionHandler {
            default_rotate_scene_entity,
            default_rotate_meshes,
            ..Default::default()
        };

        // The registry is shared by every glTF loader in the app; the handler
        // no-ops for non-VRM files.
        #[cfg(target_family = "wasm")]
        bevy::tasks::block_on(async {
            app.world_mut()
                .resource_mut::<GltfExtensionHandlers>()
                .0
                .write()
                .await
                .push(Box::new(handler));
        });
        #[cfg(not(target_family = "wasm"))]
        app.world_mut()
            .resource_mut::<GltfExtensionHandlers>()
            .0
            .write_blocking()
            .push(Box::new(handler));

        app.preregister_asset_loader::<VrmLoader>(&["vrm"]);
    }

    fn finish(
        &self,
        app: &mut App,
    ) {
        // Assemble the inner `GltfLoader` from the same resources as the stock
        // loader, sharing the handler registry `Arc`.
        let supported_compressed_formats = app
            .world()
            .get_resource::<CompressedImageFormatSupport>()
            .map(|support| support.0)
            .unwrap_or(CompressedImageFormats::NONE);

        let default_sampler = app
            .world()
            .get_resource::<DefaultGltfImageSampler>()
            .map(|sampler| sampler.get_internal())
            // `GltfPlugin::finish` has already run for any correctly ordered
            // app; this fallback only exists so a hand-built app does not panic.
            .unwrap_or_else(|| Arc::new(Mutex::new(ImageSamplerDescriptor::default())));

        let extensions = app
            .world()
            .get_resource::<GltfExtensionHandlers>()
            .map(|handlers| handlers.0.clone())
            .unwrap_or_default();

        let default_convert_coordinates = app
            .get_added_plugins::<GltfPlugin>()
            .first()
            .map(|gltf_plugin| gltf_plugin.convert_coordinates)
            .unwrap_or_default();

        app.register_asset_loader(VrmLoader(GltfLoader {
            supported_compressed_formats,
            custom_vertex_attributes: Default::default(),
            default_sampler,
            default_convert_coordinates,
            extensions,
            // Left at its `Default`: `GltfSkinnedMeshBoundsPolicy::default()`
            // is already `Dynamic` (`bevy_gltf/src/lib.rs:213-214`), which is
            // what a skinned avatar needs — rest-pose bounds would cull a mesh
            // once spring bones and animation move it.
            default_skinned_mesh_bounds_policy: Default::default(),
        }));
    }
}

/// A thin wrapper around the stock [`GltfLoader`] that registers the `vrm` file
/// extension. All glTF parsing is done by Bevy; everything VRM-specific lives
/// in [`VrmExtensionHandler`].
///
/// Load a scene out of it with
/// [`GltfAssetLabel::Scene`](bevy::gltf::GltfAssetLabel::Scene):
///
/// ```no_run
/// use bevy::prelude::*;
/// use bevy_vrm1::prelude::*;
///
/// fn spawn(mut commands: Commands, asset_server: Res<AssetServer>) {
///     commands.spawn(WorldAssetRoot(
///         asset_server.load(GltfAssetLabel::Scene(0).from_asset("vrm/Elmer.vrm")),
///     ));
/// }
/// ```
#[derive(TypePath)]
pub struct VrmLoader(pub GltfLoader);

impl AssetLoader for VrmLoader {
    type Asset = Gltf;
    type Settings = GltfLoaderSettings;
    type Error = GltfError;

    async fn load(
        &self,
        reader: &mut dyn Reader,
        settings: &GltfLoaderSettings,
        load_context: &mut LoadContext<'_>,
    ) -> Result<Gltf, GltfError> {
        // `GltfLoaderSettings` does not implement `Clone`, so copy the fields.
        // Every field is passed through untouched:
        //
        // * `validate` keeps `GltfLoaderSettings`'s own default of `true`.
        //   VRM is a glTF profile, not a glTF dialect, so a `.vrm` is expected
        //   to validate; forcing it off would trade a precise error message for
        //   silently mis-parsed files.
        // * `include_source` is the caller's choice. The handler does **not**
        //   need it: every hook receives the parsed `gltf::Gltf` (or
        //   `gltf::Node` / `gltf::Material`) directly and reads
        //   `document.extensions()` off it, so the `Gltf::source` copy of the
        //   JSON is dead weight for this loader.
        let settings = GltfLoaderSettings {
            load_meshes: settings.load_meshes,
            load_materials: settings.load_materials,
            load_cameras: settings.load_cameras,
            load_lights: settings.load_lights,
            load_animations: settings.load_animations,
            include_source: settings.include_source,
            default_sampler: settings.default_sampler.clone(),
            override_sampler: settings.override_sampler,
            validate: settings.validate,
            convert_coordinates: settings.convert_coordinates,
            skinned_mesh_bounds_policy: settings.skinned_mesh_bounds_policy,
        };
        self.0.load(reader, &settings, load_context).await
    }

    fn extensions(&self) -> &[&str] {
        // `vrma` is deliberately absent: the legacy `VrmaLoaderPlugin` owns it
        // until the VRMA commit introduces its own pipeline loader.
        &["vrm"]
    }
}

#[cfg(test)]
mod tests {
    use crate::success;
    use crate::tests::TestResult;
    use bevy::asset::AssetLoader;
    use bevy::gltf::GltfLoader;
    use bevy::image::{CompressedImageFormats, ImageSamplerDescriptor};
    use std::sync::{Arc, Mutex};

    use super::VrmLoader;

    /// A loader with the fields `VrmGltfPlugin::finish` fills; only
    /// `extensions()` is under test, so the rest is arbitrary.
    fn loader() -> VrmLoader {
        VrmLoader(GltfLoader {
            supported_compressed_formats: CompressedImageFormats::NONE,
            custom_vertex_attributes: Default::default(),
            default_sampler: Arc::new(Mutex::new(ImageSamplerDescriptor::default())),
            default_convert_coordinates: Default::default(),
            extensions: Default::default(),
            default_skinned_mesh_bounds_policy: Default::default(),
        })
    }

    /// The extension list is a *claim*: `vrma` belongs to the legacy loader
    /// until the VRMA commit adds its own, and two loaders claiming one
    /// extension makes `AssetServer` refuse the app at registration time.
    #[test]
    fn the_loader_claims_only_the_vrm_extension() -> TestResult {
        assert_eq!(loader().extensions(), &["vrm"]);
        success!()
    }
}
