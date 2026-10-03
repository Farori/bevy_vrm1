use crate::error::vrm_warn;
use bevy::color::LinearRgba;
use bevy::prelude::Reflect;
use bevy::render::render_resource::ShaderType;
use serde::{Deserialize, Deserializer, Serialize};

/// The name of the glTF material extension that scales the emissive factor.
///
/// [VRMC_materials_hdr_emissiveMultiplier-1.0](https://github.com/vrm-c/vrm-specification/blob/master/specification/VRMC_materials_hdr_emissiveMultiplier-1.0/README.md)
pub const VRMC_MATERIALS_HDR_EMISSIVE_MULTIPLIER: &str = "VRMC_materials_hdr_emissiveMultiplier";

/// [VRMC_materials_mtoon-1.0](https://github.com/vrm-c/vrm-specification/blob/master/specification/VRMC_materials_mtoon-1.0/README.md)
///
/// `specVersion` is the only required property. Every other property is
/// optional in the specification and carries the documented default here,
/// because exporters routinely omit them and a rejected material silently
/// loses its `MToon` shading.
#[derive(Serialize, Deserialize, Reflect, Debug, Clone)]
pub struct VrmcMaterialsExtensitions {
    /// Indicates the version number of `VRMC_materials_mtoon` extension.
    ///
    /// The value is fixed to "1.0".
    #[serde(rename = "specVersion")]
    pub spec_version: String,
    /// The color the `matcapTexture` is multiplied with.
    ///
    /// The specification default is `[1.0, 1.0, 1.0]`.
    #[serde(rename = "matcapFactor", default = "default_white_rgb")]
    pub matcap_factor: [f32; 3],
    #[serde(rename = "matcapTexture")]
    pub matcap_texture: Option<MatcapTexture>,
    /// The fresnel power of the parametric rim lighting.
    ///
    /// The specification default is `5.0`.
    #[serde(
        rename = "parametricRimFresnelPowerFactor",
        default = "default_parametric_rim_fresnel_power"
    )]
    pub parametric_rim_fresnel_power: f32,
    #[serde(rename = "rimMultiplyTexture")]
    pub rim_multiply_texture: Option<RimMultiplyTexture>,
    /// The outline color.
    ///
    /// The specification default is `[0.0, 0.0, 0.0]`.
    #[serde(rename = "outlineColorFactor", default = "default_black_rgb")]
    pub outline_color_factor: [f32; 3],
    /// The factor for the outline lighting mix.
    ///
    /// The specification default is `1.0`.
    #[serde(rename = "outlineLightingMixFactor", default = "default_one")]
    pub outline_lighting_mix_factor: f32,
    /// The outline width, in meters.
    ///
    /// The specification default is `0.0`, which the consumer maps from an
    /// absent property.
    #[serde(rename = "outlineWidthFactor")]
    pub outline_width_factor: Option<f32>,
    #[serde(rename = "outlineWidthMultiplyTexture")]
    pub outline_width_multiply_texture: Option<OutlineWidthMultiplyTexture>,
    /// `"none"`, `"worldCoordinates"` or `"screenCoordinates"`.
    ///
    /// The specification default is `"none"`, which draws no outline.
    #[serde(rename = "outlineWidthMode", default = "default_outline_width_mode")]
    pub outline_width_mode: String,
    /// The color of the parametric rim lighting.
    ///
    /// The specification default is `[0.0, 0.0, 0.0]`.
    #[serde(rename = "parametricRimColorFactor", default = "default_black_rgb")]
    pub parametric_rim_color_factor: [f32; 3],
    /// The lift factor of the parametric rim lighting.
    ///
    /// The specification default is `0.0`.
    #[serde(rename = "parametricRimLiftFactor", default = "default_zero")]
    pub parametric_rim_lift_factor: f32,
    /// The mix factor of the rim lighting.
    ///
    /// The specification default is `1.0`.
    #[serde(rename = "rimLightingMixFactor", default = "default_one")]
    pub rim_lighting_mix_factor: f32,
    /// The shade color.
    /// The value is evaluated in linear color space.
    ///
    /// The specification default is `[1.0, 1.0, 1.0]`, i.e. no shading at all.
    #[serde(rename = "shadeColorFactor", default = "default_white_rgb")]
    pub shade_color_factor: [f32; 3],
    #[serde(rename = "shadeMultiplyTexture")]
    pub shade_multiply_texture: Option<VrmTexture>,
    /// The offset added to the render queue.
    ///
    /// The specification default is `0.0`.
    #[serde(rename = "renderQueueOffsetNumber", default = "default_zero")]
    pub render_queue_offset_number: f32,
    /// The value that shifts the shading boundary.
    ///
    /// The specification default is `0.0`.
    #[serde(rename = "shadingShiftFactor", default = "default_zero")]
    pub shading_shift_factor: f32,
    #[serde(rename = "shadingShiftTexture")]
    pub shading_shift_texture: Option<ShadingShiftTexture>,
    /// The value that specifies the smoothness of the shading boundary.
    ///
    /// The specification default is `0.9`.
    #[serde(rename = "shadingToonyFactor", default = "default_toony_factor")]
    pub shading_toony_factor: f32,
    /// Whether a transparent material writes to the depth buffer.
    ///
    /// The specification default is `false`.
    #[serde(rename = "transparentWithZWrite", default)]
    pub transparent_with_z_write: bool,
    #[serde(rename = "uvAnimationMaskTexture")]
    pub uv_animation_mask_texture: Option<UVAnimationMaskTexture>,
    /// The rotation speed of the UV, in radians per second.
    ///
    /// The specification default is `0.0`.
    #[serde(rename = "uvAnimationRotationSpeedFactor", default = "default_zero")]
    pub uv_animation_rotation_speed_factor: f32,
    /// The scrolling speed of the UV in the X direction.
    ///
    /// The specification default is `0.0`.
    #[serde(rename = "uvAnimationScrollXSpeedFactor", default = "default_zero")]
    pub uv_animation_scroll_x_speed_factor: f32,
    /// The scrolling speed of the UV in the Y direction.
    ///
    /// The specification default is `0.0`.
    #[serde(rename = "uvAnimationScrollYSpeedFactor", default = "default_zero")]
    pub uv_animation_scroll_y_speed_factor: f32,
    /// The factor for the indirect diffuse light.
    ///
    /// The specification default is `0.9`.
    #[serde(
        rename = "giEqualizationFactor",
        default = "default_gi_equalization_factor"
    )]
    pub gi_equalization_factor: f32,
}

/// A color that does not modify the texture it is multiplied with.
fn default_white_rgb() -> [f32; 3] {
    [1.0, 1.0, 1.0]
}

/// A color that contributes no light.
fn default_black_rgb() -> [f32; 3] {
    [0.0, 0.0, 0.0]
}

fn default_zero() -> f32 {
    0.0
}

fn default_one() -> f32 {
    1.0
}

fn default_parametric_rim_fresnel_power() -> f32 {
    5.0
}

fn default_toony_factor() -> f32 {
    0.9
}

fn default_gi_equalization_factor() -> f32 {
    0.9
}

fn default_outline_width_mode() -> String {
    String::from("none")
}

impl VrmcMaterialsExtensitions {
    pub fn shade_color(&self) -> LinearRgba {
        let c = self.shade_color_factor;
        LinearRgba::rgb(c[0], c[1], c[2])
    }

    pub fn parametric_rim_color(&self) -> LinearRgba {
        let c = self.parametric_rim_color_factor;
        LinearRgba::rgb(c[0], c[1], c[2])
    }

    pub fn matcap_color(&self) -> LinearRgba {
        let c = self.matcap_factor;
        LinearRgba::rgb(c[0], c[1], c[2])
    }
}

/// The `VRMC_materials_hdr_emissiveMultiplier` glTF material extension.
///
/// It multiplies the glTF `emissiveFactor` of the material, which is what
/// lets an `MToon` material emit light stronger than white.
///
/// [VRMC_materials_hdr_emissiveMultiplier-1.0](https://github.com/vrm-c/vrm-specification/blob/master/specification/VRMC_materials_hdr_emissiveMultiplier-1.0/README.md)
#[derive(Serialize, Deserialize, Reflect, Debug, Clone, Copy, PartialEq)]
pub struct VrmcMaterialsHdrEmissiveMultiplier {
    /// The multiplier for the emissive factor.
    ///
    /// The property is named `emissiveMultiplier`, not `emissiveStrength`:
    /// > | emissiveMultiplier | number | A multiplier for emissiveFactor | ? |
    ///
    /// `emissiveStrength` belongs to `KHR_materials_emissive_strength`, the
    /// Khronos extension that supersedes this one and that `bevy_gltf` already
    /// applies to the `StandardMaterial`. Reading it here would double-count it
    /// and, because this extension is absent from most avatars, would in
    /// practice never fire at all.
    ///
    /// The specification default is `1.0`.
    #[serde(rename = "emissiveMultiplier", default = "default_one")]
    pub emissive_multiplier: f32,
}

impl Default for VrmcMaterialsHdrEmissiveMultiplier {
    fn default() -> Self {
        Self {
            emissive_multiplier: default_one(),
        }
    }
}

impl VrmcMaterialsHdrEmissiveMultiplier {
    /// Reads the multiplier out of the `extensions` of a glTF material.
    ///
    /// Returns `None` when the material does not declare the extension, which
    /// leaves the emissive factor untouched. A malformed extension is reported
    /// with `vrm_warn!` and treated as if it were absent.
    pub fn from_material_extensions(
        extensions: &serde_json::map::Map<String, serde_json::Value>
    ) -> Option<Self> {
        let value = extensions.get(VRMC_MATERIALS_HDR_EMISSIVE_MULTIPLIER)?;
        match serde_json::from_value(value.clone()) {
            Ok(multiplier) => Some(multiplier),
            Err(e) => {
                vrm_warn!("Failed to parse VRMC_materials_hdr_emissiveMultiplier", e);
                None
            }
        }
    }
}

#[derive(Serialize, Deserialize, Reflect, Debug, Clone, Copy)]
pub struct MatcapTexture {
    pub index: usize,
}

#[derive(Serialize, Deserialize, Reflect, Debug, Clone, Copy)]
pub struct RimMultiplyTexture {
    pub index: usize,
}

#[derive(Serialize, Deserialize, Reflect, Debug, Clone, Copy)]
pub struct OutlineWidthMultiplyTexture {
    pub index: usize,
}

#[derive(Serialize, Deserialize, Reflect, Debug, Clone, Copy)]
pub struct UVAnimationMaskTexture {
    pub index: usize,
}

#[derive(Serialize, Deserialize, Reflect, Debug, Clone, Copy)]
pub struct ShadingShiftTexture {
    pub index: usize,
    /// The UV set index of the glTF `textureInfo`.
    ///
    /// The specification default is `0`, and exporters write it as an integer,
    /// so both an absent property and an integer literal are accepted.
    #[serde(
        rename = "texCoord",
        default = "default_zero",
        deserialize_with = "deserialize_lenient_f32"
    )]
    pub tex_coord: f32,
    /// The scale of the shading shift texture.
    ///
    /// The specification default is `1.0`.
    #[serde(default = "default_one")]
    pub scale: f32,
}

/// A texture reference of the `VRMC_materials_mtoon` extension.
#[derive(Serialize, Deserialize, Reflect, Debug, Clone, Copy)]
pub struct VrmTexture {
    /// The glTF texture extensions of the reference.
    ///
    /// `KHR_texture_transform` is optional, so this is `None` for a texture
    /// without any extension.
    #[serde(default)]
    pub extensions: Option<VrmTextureExtensions>,
    pub index: usize,
}

impl VrmTexture {
    /// The `KHR_texture_transform` of the texture.
    ///
    /// Falls back to the identity transform of [`KhrTextureTransform`] when the
    /// texture does not declare one.
    pub fn texture_transform(&self) -> KhrTextureTransform {
        self.extensions
            .and_then(|extensions| extensions.khr_texture_transform)
            .unwrap_or_default()
    }
}

#[derive(Serialize, Deserialize, Reflect, Debug, Clone, Copy)]
pub struct VrmTextureExtensions {
    #[serde(rename = "KHR_texture_transform", default)]
    pub khr_texture_transform: Option<KhrTextureTransform>,
}

/// `KHR_texture_transform`.
///
/// Every property of the extension is optional, so a partial object such as
/// `{"offset": [0.5, 0.5]}` fills the rest from [`Default`].
#[allow(dead_code)]
#[derive(Serialize, Deserialize, Reflect, Debug, Clone, PartialEq, Copy, ShaderType)]
#[serde(default)]
pub struct KhrTextureTransform {
    /// The specification default is `[0.0, 0.0]`.
    pub offset: [f32; 2],
    /// The specification default is `[1.0, 1.0]`.
    pub scale: [f32; 2],
}

impl Default for KhrTextureTransform {
    fn default() -> Self {
        Self {
            offset: [0.0, 0.0],
            scale: [1.0, 1.0],
        }
    }
}

/// Reads a number that glTF may write as either an integer or a float.
fn deserialize_lenient_f32<'de, D>(deserializer: D) -> Result<f32, D::Error>
where
    D: Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    value
        .as_f64()
        .map(|number| number as f32)
        .ok_or_else(|| serde::de::Error::custom(format!("expected a number, got `{value}`")))
}

#[cfg(test)]
mod tests {
    use crate::success;
    use crate::tests::TestResult;
    use crate::vrm::gltf::materials::{
        KhrTextureTransform, VrmTexture, VrmcMaterialsExtensitions,
        VrmcMaterialsHdrEmissiveMultiplier,
    };

    #[test]
    fn deserialize_minimal_vrmc_materials_mtoon() -> TestResult {
        let mtoon: VrmcMaterialsExtensitions = serde_json::from_str(r#"{"specVersion":"1.0"}"#)?;

        assert_eq!(mtoon.spec_version, "1.0");

        // Shade. The default shade color leaves the toon shading untouched.
        assert_eq!(mtoon.shade_color_factor, [1.0, 1.0, 1.0]);
        assert_eq!(mtoon.shading_shift_factor, 0.0);
        assert_eq!(mtoon.shading_toony_factor, 0.9);

        // Rim lighting.
        assert_eq!(mtoon.parametric_rim_color_factor, [0.0, 0.0, 0.0]);
        assert_eq!(mtoon.parametric_rim_fresnel_power, 5.0);
        assert_eq!(mtoon.parametric_rim_lift_factor, 0.0);
        assert_eq!(mtoon.rim_lighting_mix_factor, 1.0);
        assert_eq!(mtoon.matcap_factor, [1.0, 1.0, 1.0]);

        // Outline. The default mode draws no outline.
        assert_eq!(mtoon.outline_width_mode, "none");
        assert_eq!(mtoon.outline_color_factor, [0.0, 0.0, 0.0]);
        assert_eq!(mtoon.outline_lighting_mix_factor, 1.0);
        assert_eq!(mtoon.outline_width_factor, None);

        // UV animation.
        assert_eq!(mtoon.uv_animation_rotation_speed_factor, 0.0);
        assert_eq!(mtoon.uv_animation_scroll_x_speed_factor, 0.0);
        assert_eq!(mtoon.uv_animation_scroll_y_speed_factor, 0.0);

        // Rendering.
        assert_eq!(mtoon.render_queue_offset_number, 0.0);
        assert!(!mtoon.transparent_with_z_write);
        assert_eq!(mtoon.gi_equalization_factor, 0.9);

        success!()
    }

    #[test]
    fn deserialize_vrmc_materials_mtoon_without_optional_textures() -> TestResult {
        let mtoon: VrmcMaterialsExtensitions = serde_json::from_str(
            r#"{
                "specVersion": "1.0",
                "shadeColorFactor": [0.0, 0.0, 1.0],
                "giEqualizationFactor": 0.0,
                "shadingToonyFactor": 0.1
            }"#,
        )?;

        assert_eq!(mtoon.shade_color_factor, [0.0, 0.0, 1.0]);
        assert_eq!(mtoon.gi_equalization_factor, 0.0);
        assert_eq!(mtoon.shading_toony_factor, 0.1);

        assert!(mtoon.matcap_texture.is_none());
        assert!(mtoon.rim_multiply_texture.is_none());
        assert!(mtoon.shade_multiply_texture.is_none());
        assert!(mtoon.shading_shift_texture.is_none());
        assert!(mtoon.outline_width_multiply_texture.is_none());
        assert!(mtoon.uv_animation_mask_texture.is_none());

        success!()
    }

    #[test]
    fn reject_vrmc_materials_mtoon_without_spec_version() {
        for invalid in [
            "{}",
            r#"{"shadeColorFactor":[1.0,1.0,1.0]}"#,
            r#"{"specVersion":"1.0","shadeColorFactor":null}"#,
            r#"{"specVersion":"1.0","outlineWidthMode":null}"#,
            r#"{"specVersion":"1.0","transparentWithZWrite":null}"#,
            r#"{"specVersion":"1.0","transparentWithZWrite":"true"}"#,
            r#"{"specVersion":"1.0","parametricRimColorFactor":[0.0,0.0]}"#,
            r#"{"specVersion":"1.0","matcapTexture":{}}"#,
        ] {
            assert!(
                serde_json::from_str::<VrmcMaterialsExtensitions>(invalid).is_err(),
                "`{invalid}` should be rejected"
            );
        }
    }

    #[test]
    fn deserialize_texture_without_khr_texture_transform() -> TestResult {
        let mtoon: VrmcMaterialsExtensitions =
            serde_json::from_str(r#"{"specVersion":"1.0","shadeMultiplyTexture":{"index":1}}"#)?;
        let texture = mtoon
            .shade_multiply_texture
            .expect("shadeMultiplyTexture should be parsed");

        assert_eq!(texture.index, 1);
        assert!(texture.extensions.is_none());
        assert_eq!(texture.texture_transform(), KhrTextureTransform::default());

        success!()
    }

    #[test]
    fn deserialize_texture_with_khr_texture_transform() -> TestResult {
        let texture: VrmTexture = serde_json::from_str(
            r#"{"index":0,"extensions":{"KHR_texture_transform":{"offset":[0.5,0.5],"scale":[2.0,3.0]}}}"#,
        )?;

        assert_eq!(
            texture.texture_transform(),
            KhrTextureTransform {
                offset: [0.5, 0.5],
                scale: [2.0, 3.0]
            }
        );

        success!()
    }

    #[test]
    fn deserialize_texture_with_partial_khr_texture_transform() -> TestResult {
        // Every property of `KHR_texture_transform` is optional.
        let texture: VrmTexture = serde_json::from_str(
            r#"{"index":0,"extensions":{"KHR_texture_transform":{"scale":[2.0,2.0]}}}"#,
        )?;

        assert_eq!(
            texture.texture_transform(),
            KhrTextureTransform {
                offset: [0.0, 0.0],
                scale: [2.0, 2.0]
            }
        );

        success!()
    }

    #[test]
    fn deserialize_texture_without_khr_texture_transform_in_extensions() -> TestResult {
        let texture: VrmTexture = serde_json::from_str(r#"{"index":0,"extensions":{}}"#)?;

        assert!(texture.extensions.is_some());
        assert_eq!(texture.texture_transform(), KhrTextureTransform::default());

        success!()
    }

    #[test]
    fn deserialize_shading_shift_texture_without_scale() -> TestResult {
        // `texCoord` is absent and `scale` is omitted, both of which the
        // specification defines.
        let mtoon: VrmcMaterialsExtensitions =
            serde_json::from_str(r#"{"specVersion":"1.0","shadingShiftTexture":{"index":3}}"#)?;
        let texture = mtoon
            .shading_shift_texture
            .expect("shadingShiftTexture should be parsed");

        assert_eq!(texture.index, 3);
        assert_eq!(texture.tex_coord, 0.0);
        assert_eq!(texture.scale, 1.0);

        success!()
    }

    #[test]
    fn deserialize_shading_shift_texture_with_integer_tex_coord() -> TestResult {
        let mtoon: VrmcMaterialsExtensitions = serde_json::from_str(
            r#"{"specVersion":"1.0","shadingShiftTexture":{"index":3,"texCoord":0,"scale":0.5}}"#,
        )?;
        let texture = mtoon
            .shading_shift_texture
            .expect("shadingShiftTexture should be parsed");

        assert_eq!(texture.tex_coord, 0.0);
        assert_eq!(texture.scale, 0.5);

        success!()
    }

    #[test]
    fn parse_hdr_emissive_multiplier() -> TestResult {
        let extensions = serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(
            r#"{"VRMC_materials_hdr_emissiveMultiplier":{"emissiveMultiplier":2.5}}"#,
        )?;
        let multiplier = VrmcMaterialsHdrEmissiveMultiplier::from_material_extensions(&extensions)
            .expect("VRMC_materials_hdr_emissiveMultiplier should be parsed");

        assert_eq!(multiplier.emissive_multiplier, 2.5);

        success!()
    }

    #[test]
    fn hdr_emissive_multiplier_is_read_from_the_material_extensions() -> TestResult {
        // The extension is a *material* extension, so it sits next to
        // `VRMC_materials_mtoon` and not inside it.
        let extensions = serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(
            r#"{
                "VRMC_materials_mtoon": {"specVersion": "1.0"},
                "VRMC_materials_hdr_emissiveMultiplier": {"emissiveMultiplier": 3.0}
            }"#,
        )?;
        let multiplier = VrmcMaterialsHdrEmissiveMultiplier::from_material_extensions(&extensions)
            .expect("a sibling of VRMC_materials_mtoon is still read");

        assert_eq!(multiplier.emissive_multiplier, 3.0);

        // Nested inside the MToon extension it is not an extension of the
        // material at all, so it is not read.
        let nested = serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(
            r#"{
                "VRMC_materials_mtoon": {
                    "specVersion": "1.0",
                    "VRMC_materials_hdr_emissiveMultiplier": {"emissiveMultiplier": 3.0}
                }
            }"#,
        )?;
        assert!(
            VrmcMaterialsHdrEmissiveMultiplier::from_material_extensions(&nested).is_none(),
            "the multiplier is not a property of VRMC_materials_mtoon"
        );

        success!()
    }

    #[test]
    fn hdr_emissive_multiplier_does_not_read_khr_emissive_strength() -> TestResult {
        // `emissiveStrength` is the property of `KHR_materials_emissive_strength`,
        // which `bevy_gltf` already folded into `StandardMaterial::emissive`.
        // Reading it here as well would square the strength.
        let extensions = serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(
            r#"{"VRMC_materials_hdr_emissiveMultiplier":{"emissiveStrength":4.0}}"#,
        )?;
        let multiplier = VrmcMaterialsHdrEmissiveMultiplier::from_material_extensions(&extensions)
            .expect("the extension is declared, its unknown property is just ignored");

        assert_eq!(
            multiplier.emissive_multiplier, 1.0,
            "`emissiveStrength` belongs to KHR_materials_emissive_strength, so it must not \
             be read as a VRM multiplier of 4.0"
        );

        success!()
    }

    #[test]
    fn parse_hdr_emissive_multiplier_without_emissive_multiplier() -> TestResult {
        let extensions = serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(
            r#"{"VRMC_materials_hdr_emissiveMultiplier":{}}"#,
        )?;
        let multiplier = VrmcMaterialsHdrEmissiveMultiplier::from_material_extensions(&extensions)
            .expect("an empty extension is tolerated");

        assert_eq!(multiplier.emissive_multiplier, 1.0);
        assert_eq!(
            multiplier,
            VrmcMaterialsHdrEmissiveMultiplier::default(),
            "the specification default is 1.0"
        );

        success!()
    }

    #[test]
    fn parse_absent_or_malformed_hdr_emissive_multiplier() -> TestResult {
        let absent = serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(
            r#"{"VRMC_materials_mtoon":{"specVersion":"1.0"}}"#,
        )?;
        assert!(
            VrmcMaterialsHdrEmissiveMultiplier::from_material_extensions(&absent).is_none(),
            "a material without the extension has a multiplier of 1.0"
        );

        for malformed in [
            r#"{"VRMC_materials_hdr_emissiveMultiplier":2.0}"#,
            r#"{"VRMC_materials_hdr_emissiveMultiplier":null}"#,
            r#"{"VRMC_materials_hdr_emissiveMultiplier":{"emissiveMultiplier":"2.0"}}"#,
            r#"{"VRMC_materials_hdr_emissiveMultiplier":{"emissiveMultiplier":null}}"#,
        ] {
            let extensions =
                serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(malformed)?;
            assert!(
                VrmcMaterialsHdrEmissiveMultiplier::from_material_extensions(&extensions).is_none(),
                "`{malformed}` should be treated as absent"
            );
        }

        success!()
    }
}
