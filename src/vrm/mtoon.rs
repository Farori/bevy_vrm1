mod material;
mod outline_pass;
mod setup;

use crate::error::vrm_error;
use crate::prelude::*;
use crate::vrm::gltf::materials::{VrmcMaterialsExtensitions, VrmcMaterialsHdrEmissiveMultiplier};
use crate::vrm::mtoon::outline_pass::MToonOutlinePlugin;
use crate::vrm::mtoon::setup::MToonMaterialSetupPlugin;
use bevy::asset::{AssetId, load_internal_asset, uuid_handle};
use bevy::prelude::*;
use std::collections::HashMap;

pub mod prelude {
    pub use crate::vrm::mtoon::{MtoonMaterialPlugin, VrmcMaterialRegistry, material::prelude::*};
}

const MTOON_FRAGMENT_SHADER_HANDLE: Handle<Shader> =
    uuid_handle!("9a96eff2-1676-1dc0-9abc-2fd5e7134443");
const MTOON_VERTEX_SHADER_HANDLE: Handle<Shader> =
    uuid_handle!("f4041db8-c464-b84c-e3c9-e618527945a1");
const MTOON_TYPES_SHADER_HANDLE: Handle<Shader> =
    uuid_handle!("5d9302a3-6498-9d2a-fadb-842d01c87697");

pub struct MtoonMaterialPlugin;

impl Plugin for MtoonMaterialPlugin {
    fn build(
        &self,
        app: &mut App,
    ) {
        app.register_type::<MToonMaterial>()
            .register_type::<MToonOutline>()
            .register_type::<VrmcMaterialRegistry>()
            .register_type::<RimLighting>()
            .register_type::<UVAnimation>()
            .register_type::<Shade>()
            .add_plugins(MaterialPlugin::<MToonMaterial>::default())
            .add_plugins((MToonMaterialSetupPlugin, MToonOutlinePlugin));
        load_internal_asset!(
            app,
            MTOON_FRAGMENT_SHADER_HANDLE,
            "mtoon_fragment.wgsl",
            Shader::from_wgsl
        );
        load_internal_asset!(
            app,
            MTOON_TYPES_SHADER_HANDLE,
            "mtoon_types.wgsl",
            Shader::from_wgsl
        );
        load_internal_asset!(
            app,
            MTOON_VERTEX_SHADER_HANDLE,
            "mtoon_vertex.wgsl",
            Shader::from_wgsl
        );
    }
}

#[derive(Component, Default, Debug, Reflect)]
#[reflect(Component)]
pub struct VrmcMaterialRegistry {
    pub images: Vec<Handle<Image>>,
    pub materials: HashMap<AssetId<StandardMaterial>, VrmcMaterialsExtensitions>,
    /// The `VRMC_materials_hdr_emissiveMultiplier` of each material that
    /// declares the extension.
    ///
    /// Materials without the extension are absent, which means a multiplier
    /// of `1.0`. See [`VrmcMaterialRegistry::hdr_emissive_multiplier`].
    pub hdr_emissive_multipliers: HashMap<AssetId<StandardMaterial>, f32>,
}

impl VrmcMaterialRegistry {
    pub fn new(
        gltf: &Gltf,
        images: Vec<Handle<Image>>,
        asset_server: &AssetServer,
    ) -> Self {
        Self::try_new(gltf, images, asset_server).unwrap_or_default()
    }

    /// The multiplier for the emissive factor of the material.
    ///
    /// Returns `1.0` for a material without the
    /// `VRMC_materials_hdr_emissiveMultiplier` extension, or one that declares
    /// it malformed.
    pub fn hdr_emissive_multiplier(
        &self,
        id: AssetId<StandardMaterial>,
    ) -> f32 {
        self.hdr_emissive_multipliers
            .get(&id)
            .copied()
            .unwrap_or(1.0)
    }

    fn try_new(
        gltf: &Gltf,
        images: Vec<Handle<Image>>,
        asset_server: &AssetServer,
    ) -> Option<Self> {
        // Match glTF materials to Bevy `StandardMaterial` handles by index,
        // not by name. The glTF spec does not require material names to be
        // unique, and some exporters (e.g. VRoid) produce multiple materials
        // that share a name. `Gltf::named_materials` is a `HashMap` keyed by
        // name, so duplicates collapse to a single entry and any meshes bound
        // to the overwritten materials skip the MToon conversion entirely,
        // rendering with the default `StandardMaterial` instead.
        let mut materials = HashMap::new();
        let mut hdr_emissive_multipliers = HashMap::new();
        for material in gltf.source.as_ref()?.materials() {
            let Some(index) = material.index() else {
                continue;
            };
            let Some(gltf_material_path) = gltf.materials.get(index).and_then(|m| m.path()) else {
                continue;
            };
            let Some(std_label) = gltf_material_path.label() else {
                continue;
            };
            let std_path = gltf_material_path
                .clone()
                .with_label(format!("{std_label}/std"));
            let asset_id = asset_server.load::<StandardMaterial>(std_path).id();
            let Some(extensions) = material.extensions() else {
                continue;
            };
            let Some(mtoon) = extensions.get("VRMC_materials_mtoon") else {
                continue;
            };
            match serde_json::from_value(mtoon.clone()) {
                Ok(properties) => {
                    materials.insert(asset_id, properties);
                }
                Err(e) => {
                    vrm_error!("Failed to parse VRMC_materials_mtoon", e);
                }
            }
            // The emissive multiplier is a separate extension from the MToon
            // one, so it applies even when `VRMC_materials_mtoon` is malformed.
            if let Some(multiplier) =
                VrmcMaterialsHdrEmissiveMultiplier::from_material_extensions(extensions)
            {
                hdr_emissive_multipliers.insert(asset_id, multiplier.emissive_strength);
            }
        }
        Some(Self {
            materials,
            images,
            hdr_emissive_multipliers,
        })
    }
}
