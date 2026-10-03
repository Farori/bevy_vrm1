use crate::vrm::gltf::extensions::VrmNode;
use bevy::platform::collections::HashMap;
use bevy::prelude::*;
use serde::{Deserialize, Serialize};

/// `VRMC_vrm`, parsed from either `VRMC_vrm` or the `VRMC_vrm_animation` object
/// a VRM 0.0 file carries.
///
/// The schema requires `specVersion`, `meta` and `humanoid`. `specVersion` and
/// `humanoid` are enforced. `meta` is the one required property kept tolerant:
/// the licence metadata is not what makes an avatar usable, and rejecting a
/// whole VRM over it costs the model its humanoid and its expressions.
#[derive(Serialize, Deserialize)]
pub struct VrmcVrm {
    pub expressions: Option<Expressions>,
    #[serde(rename = "firstPerson", default)]
    pub first_person: Option<FirstPerson>,
    pub humanoid: Humanoid,
    #[serde(rename = "lookAt")]
    pub look_at: Option<LookAtProperties>,
    pub meta: Option<Meta>,
    #[serde(rename = "specVersion")]
    pub spec_version: String,
}

#[derive(Serialize, Deserialize)]
pub struct Expressions {
    #[serde(default)]
    pub preset: HashMap<String, VrmPreset>,
    /// The custom expressions of `VRMC_vrm.expressions`.
    ///
    /// The schema declares `custom` as an open map of expressions with the
    /// same shape as a preset one. Nothing consumes it yet; it is modelled so
    /// a custom expression does not disappear from the parsed model.
    #[serde(default)]
    pub custom: HashMap<String, VrmPreset>,
}

#[derive(Serialize, Deserialize)]
pub struct VrmPreset {
    /// If this value is `true`, `weight` value greater than 0.5 is 1.0, otherwise 0.0.
    #[serde(rename = "isBinary", default)]
    pub is_binary: bool,
    #[serde(
        rename = "materialColorBinds",
        default,
        deserialize_with = "lenient_vec"
    )]
    pub material_color_binds: Vec<MaterialColorBind>,
    #[serde(rename = "morphTargetBinds")]
    pub morph_target_binds: Option<Vec<MorphTargetBind>>,
    #[serde(rename = "overrideBlink", default = "default_override_type")]
    pub override_blink: String,
    #[serde(rename = "overrideLookAt", default = "default_override_type")]
    pub override_look_at: String,
    #[serde(rename = "overrideMouth", default = "default_override_type")]
    pub override_mouth: String,
    #[serde(
        rename = "textureTransformBinds",
        default,
        deserialize_with = "lenient_vec"
    )]
    pub texture_transform_binds: Vec<TextureTransformBind>,
}

/// `VRMC_vrm.expressions.preset.*.override*` defaults to `none`.
fn default_override_type() -> String {
    "none".to_string()
}

/// `VRMC_vrm.expressions.expression.morphTargetBind`.
#[derive(Serialize, Deserialize)]
pub struct MorphTargetBind {
    pub index: usize,
    pub node: usize,
    pub weight: f32,
}

/// `VRMC_vrm.expressions.expression.materialColorBind`.
///
/// Modelled for completeness; nothing consumes it yet.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct MaterialColorBind {
    /// The index of the target material.
    pub material: usize,
    /// `color`, `emissionColor`, `shadeColor`, `matcapColor`, `rimColor` or
    /// `outlineColor`.
    #[serde(rename = "type")]
    pub r#type: String,
    /// The target color, as four components.
    #[serde(rename = "targetValue")]
    pub target_value: [f32; 4],
}

/// `VRMC_vrm.expressions.expression.textureTransformBind`.
///
/// Modelled for completeness; nothing consumes it yet.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq)]
pub struct TextureTransformBind {
    /// The index of the target material.
    pub material: usize,
    /// The UV offset for `TEXCOORD_0`. The specification default is
    /// `[0.0, 0.0]`.
    #[serde(default)]
    pub offset: [f32; 2],
    /// The UV scale for `TEXCOORD_0`. The specification default is
    /// `[1.0, 1.0]`.
    #[serde(default = "default_uv_scale")]
    pub scale: [f32; 2],
}

fn default_uv_scale() -> [f32; 2] {
    [1.0, 1.0]
}

/// Reads a list, dropping the entries that do not deserialize.
///
/// `VRMC_vrm.expressions` carries binds for animation features this crate does
/// not act on. A malformed bind is not a reason to reject the whole `VRMC_vrm`
/// extension, which would cost the avatar its humanoid and its expressions —
/// the same rule `VrmExtensions::new` applies to `VRMC_springBone` as a whole.
fn lenient_vec<'de, D, T>(deserializer: D) -> Result<Vec<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::de::DeserializeOwned,
{
    let values = Vec::<serde_json::Value>::deserialize(deserializer)?;
    Ok(values
        .into_iter()
        .filter_map(|value| serde_json::from_value(value).ok())
        .collect())
}

/// `VRMC_vrm.humanoid`.
///
/// `humanBones` is required by the schema and is enforced as a whole, but the
/// fifteen bones the schema names individually (`hips`, `spine`, `head`, the
/// six leg bones, and the six arm bones) are deliberately not: a VRM 0.0 file
/// read through the `VRMC_vrm_animation` fallback has the same requirement, and
/// a partially mapped humanoid is still worth loading.
#[derive(Serialize, Deserialize, Debug)]
pub struct Humanoid {
    #[serde(rename = "humanBones")]
    pub human_bones: HashMap<String, VrmNode>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default, Reflect)]
#[serde(rename_all = "camelCase")]
pub enum FirstPersonFlag {
    #[default]
    Auto,
    Both,
    ThirdPersonOnly,
    FirstPersonOnly,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy)]
pub struct MeshAnnotation {
    pub node: usize,
    /// How the camera interprets the mesh.
    ///
    /// The specification documents this property as `firstPersonFlag`
    /// (`VRMC_vrm.firstPerson.meshAnnotations[*]`), while
    /// `VRMC_vrm.firstPerson.meshAnnotation.schema.json` names it `type` and
    /// makes it required. Exporters follow the document, so `firstPersonFlag`
    /// is the primary name and `type` is accepted as an alias.
    ///
    /// The document states that "when the `firstPerson` property itself does
    /// not exist […] you must assume all meshes are annotated as `auto`", so
    /// an absent property falls back to [`FirstPersonFlag::Auto`] instead of
    /// being rejected.
    #[serde(rename = "firstPersonFlag", alias = "type", default)]
    pub first_person_flag: FirstPersonFlag,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct FirstPerson {
    #[serde(rename = "meshAnnotations", default)]
    pub mesh_annotations: Vec<MeshAnnotation>,
}

#[derive(Serialize, Deserialize)]
pub struct Meta {
    #[serde(rename = "allowAntisocialOrHateUsage", default)]
    pub allow_antisocial_or_hate_usage: bool,
    #[serde(rename = "allowExcessivelySexualUsage", default)]
    pub allow_excessively_sexual_usage: bool,
    #[serde(rename = "allowExcessivelyViolentUsage", default)]
    pub allow_excessively_violent_usage: bool,
    #[serde(rename = "allowPoliticalOrReligiousUsage", default)]
    pub allow_political_or_religious_usage: bool,
    #[serde(rename = "allowRedistribution", default)]
    pub allow_redistribution: bool,
    #[serde(default)]
    pub authors: Vec<String>,
    #[serde(rename = "avatarPermission")]
    pub avatar_permission: Option<String>,
    #[serde(rename = "commercialUsage")]
    pub commercial_usage: Option<String>,
    #[serde(rename = "contactInformation")]
    pub contact_information: Option<String>,
    #[serde(rename = "copyrightInformation")]
    pub copyright_information: Option<String>,
    #[serde(rename = "creditNotation")]
    pub credit_notation: Option<String>,
    #[serde(rename = "licenseUrl")]
    pub license_url: Option<String>,
    pub modification: Option<String>,
    pub name: Option<String>,
    #[serde(rename = "otherLicenseUrl")]
    pub other_license_url: Option<String>,
    #[serde(default)]
    pub references: Vec<String>,
    #[serde(rename = "thirdPartyLicenses")]
    pub third_party_licenses: Option<String>,
    #[serde(rename = "thumbnailImage")]
    pub thumbnail_image: Option<i64>,
    pub version: Option<String>,
}

#[derive(Component, Debug, Clone, Reflect, Serialize, Deserialize, Default)]
#[reflect(Component, Default)]
#[cfg_attr(feature = "serde", reflect(Serialize, Deserialize))]
pub struct LookAtProperties {
    /// An offset from the head bone to the lookAt reference position (between both eyes).
    ///
    /// The schema declares no `default`, and the specification explicitly
    /// declines to name one: "If the model does not have `offsetFromHeadBone`,
    /// it is recommended to fall back to the appropriate value for each
    /// implementation." The neutral origin is used here, which puts the
    /// reference position at the head bone itself.
    #[serde(rename = "offsetFromHeadBone", default)]
    pub offset_from_head_bone: [f32; 3],
    /// A range map for the horizontal inner eye movement.
    #[serde(rename = "rangeMapHorizontalInner", default)]
    pub range_map_horizontal_inner: RangeMap,
    /// A range map for the horizontal outer eye movement (used by Expression's `LookLeft` and `LookRight`).
    #[serde(rename = "rangeMapHorizontalOuter", default)]
    pub range_map_horizontal_outer: RangeMap,
    /// A range map for the vertical down eye movement.
    #[serde(rename = "rangeMapVerticalDown", default)]
    pub range_map_vertical_down: RangeMap,
    /// A range map for the vertical up eye movement.
    #[serde(rename = "rangeMapVerticalUp", default)]
    pub range_map_vertical_up: RangeMap,
    /// `bone` or `expression` to look at.
    #[serde(rename = "type", default)]
    pub r#type: LookAtType,
}

#[derive(Serialize, Deserialize, Debug, Copy, Clone, PartialEq, Reflect)]
#[cfg_attr(feature = "serde", reflect(Serialize, Deserialize))]
pub struct RangeMap {
    /// The schema declares no `default` for `inputMaxValue`, so the value
    /// below is the one every reference implementation and the specification's
    /// own example use.
    #[serde(rename = "inputMaxValue", default = "default_input_max_value")]
    pub input_max_value: f32,
    /// The schema declares no `default` for `outputScale`, so the value below
    /// is the one every reference implementation and the specification's own
    /// example use.
    #[serde(rename = "outputScale", default = "default_output_scale")]
    pub output_scale: f32,
}

impl Default for RangeMap {
    fn default() -> Self {
        Self {
            input_max_value: default_input_max_value(),
            output_scale: default_output_scale(),
        }
    }
}

/// `RangeMap.inputMaxValue` defaults to `90.0`.
fn default_input_max_value() -> f32 {
    90.0
}

/// `RangeMap.outputScale` defaults to `10.0`.
fn default_output_scale() -> f32 {
    10.0
}

#[derive(Serialize, Deserialize, Debug, Copy, Clone, PartialEq, Eq, Reflect, Default)]
#[cfg_attr(feature = "serde", reflect(Serialize, Deserialize))]
#[serde(rename_all = "snake_case")]
pub enum LookAtType {
    #[default]
    Bone,
    Expression,
}

#[cfg(test)]
mod tests {
    use crate::success;
    use crate::tests::TestResult;
    use crate::vrm::gltf::extensions::vrmc_vrm::{
        Expressions, FirstPersonFlag, LookAtProperties, LookAtType, MeshAnnotation, Meta, RangeMap,
        TextureTransformBind, VrmPreset, VrmcVrm,
    };

    #[test]
    fn vrm_preset_tolerates_omitted_optional_fields() -> TestResult {
        let preset: VrmPreset = serde_json::from_str(r#"{}"#)?;

        assert!(!preset.is_binary);
        assert_eq!(preset.override_blink, "none");
        assert_eq!(preset.override_look_at, "none");
        assert_eq!(preset.override_mouth, "none");
        success!()
    }

    #[test]
    fn look_at_properties_tolerate_omitted_optional_fields() -> TestResult {
        let look_at: LookAtProperties = serde_json::from_str(r#"{}"#)?;

        assert_eq!(look_at.offset_from_head_bone, [0.0; 3]);
        assert_eq!(look_at.r#type, LookAtType::Bone);
        assert_eq!(look_at.range_map_horizontal_inner.input_max_value, 90.0);
        assert_eq!(look_at.range_map_horizontal_inner.output_scale, 10.0);
        assert_eq!(look_at.range_map_vertical_up, RangeMap::default());
        success!()
    }

    #[test]
    fn meta_tolerates_omitted_optional_fields() -> TestResult {
        let meta: Meta = serde_json::from_str(r#"{}"#)?;

        assert!(!meta.allow_redistribution);
        assert!(meta.authors.is_empty());
        assert!(meta.references.is_empty());
        assert!(meta.contact_information.is_none());
        assert!(meta.third_party_licenses.is_none());
        success!()
    }

    #[test]
    fn vrmc_vrm_requires_spec_version_and_humanoid_human_bones() {
        // `required: ["specVersion", "meta", "humanoid"]`, and `Humanoid`
        // additionally requires `humanBones`. `meta` is the one required
        // property this crate keeps tolerant, because a VRM without it is
        // still a loadable avatar.
        for invalid in [
            r#"{}"#,
            r#"{"humanoid":{"humanBones":{}}}"#,
            r#"{"specVersion":"1.0"}"#,
            r#"{"specVersion":"1.0","humanoid":{}}"#,
            r#"{"specVersion":"1.0","humanoid":{"humanBones":null}}"#,
            r#"{"specVersion":"1.0","humanoid":{"humanBones":{"head":{}}}}"#,
        ] {
            assert!(
                serde_json::from_str::<VrmcVrm>(invalid).is_err(),
                "`{invalid}` should be rejected"
            );
        }
    }

    #[test]
    fn vrmc_vrm_tolerates_an_absent_meta() -> TestResult {
        let vrm: VrmcVrm =
            serde_json::from_str(r#"{"specVersion":"1.0","humanoid":{"humanBones":{}}}"#)?;

        assert_eq!(vrm.spec_version, "1.0");
        assert!(vrm.meta.is_none());
        assert!(vrm.look_at.is_none());
        assert!(vrm.expressions.is_none());
        assert!(vrm.first_person.is_none());

        success!()
    }

    #[test]
    fn expressions_model_the_color_and_texture_transform_binds() -> TestResult {
        let expressions: Expressions = serde_json::from_str(
            r#"{
                "preset": {
                    "happy": {
                        "isBinary": true,
                        "materialColorBinds": [
                            {"material": 1, "type": "emissionColor", "targetValue": [1, 0, 0, 1]}
                        ],
                        "morphTargetBinds": [{"index": 0, "node": 2, "weight": 1}],
                        "textureTransformBinds": [{"material": 1, "offset": [0.5, 0.5]}]
                    }
                },
                "custom": {
                    "Wave": {
                        "materialColorBinds": [
                            {"material": 3, "type": "outlineColor", "targetValue": [0, 0, 1, 1]}
                        ]
                    }
                }
            }"#,
        )?;

        let happy = &expressions.preset["happy"];
        assert!(happy.is_binary);
        assert_eq!(happy.material_color_binds.len(), 1);
        assert_eq!(happy.material_color_binds[0].material, 1);
        assert_eq!(happy.material_color_binds[0].r#type, "emissionColor");
        assert_eq!(
            happy.material_color_binds[0].target_value,
            [1.0, 0.0, 0.0, 1.0]
        );
        assert_eq!(
            happy.texture_transform_binds,
            [TextureTransformBind {
                material: 1,
                offset: [0.5, 0.5],
                // The specification default of `scale`.
                scale: [1.0, 1.0],
            }]
        );

        let wave = &expressions.custom["Wave"];
        assert_eq!(wave.material_color_binds.len(), 1);
        assert_eq!(wave.material_color_binds[0].r#type, "outlineColor");

        success!()
    }

    #[test]
    fn malformed_bind_does_not_reject_the_whole_expression() -> TestResult {
        // `materialColorBind` requires `material`, `type` and `targetValue`;
        // dropping the offending entry keeps the rest of the expression, and
        // with it the whole `VRMC_vrm` extension.
        let expressions: Expressions = serde_json::from_str(
            r#"{
                "preset": {
                    "happy": {
                        "materialColorBinds": [{"material": 1, "type": "color"}],
                        "morphTargetBinds": [{"index": 0, "node": 2, "weight": 1}]
                    }
                }
            }"#,
        )?;

        let happy = &expressions.preset["happy"];
        assert!(happy.material_color_binds.is_empty());
        assert_eq!(happy.morph_target_binds.as_ref().map(Vec::len), Some(1));

        success!()
    }

    #[test]
    fn mesh_annotation_accepts_the_schema_and_the_document_property_name() -> TestResult {
        for json in [
            r#"{"node":1,"firstPersonFlag":"thirdPersonOnly"}"#,
            r#"{"node":1,"type":"thirdPersonOnly"}"#,
        ] {
            let annotation: MeshAnnotation = serde_json::from_str(json)?;
            assert_eq!(annotation.node, 1);
            assert_eq!(
                annotation.first_person_flag,
                FirstPersonFlag::ThirdPersonOnly
            );
        }

        // The document requires an `auto` fallback rather than a rejection.
        let annotation: MeshAnnotation = serde_json::from_str(r#"{"node":1}"#)?;
        assert_eq!(annotation.first_person_flag, FirstPersonFlag::Auto);

        assert!(
            serde_json::from_str::<MeshAnnotation>(r#"{"firstPersonFlag":"both"}"#).is_err(),
            "`node` is required"
        );

        success!()
    }
}
