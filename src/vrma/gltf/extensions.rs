//! This module defines the data structures for VRMA extensions in the GLTF format.

use crate::error::AppResult;
use crate::vrm::gltf::extensions::{VrmNode, obtain_extensions, obtain_vrmc_vrm};
use bevy::gltf::Gltf;
use bevy::platform::collections::HashMap;
use serde::{Deserialize, Serialize};

/// `VRMC_vrm_animation.expressions`.
///
/// Both families are read, because the spec defines both and the shipped
/// `.vrma` files only happen to leave `custom` empty: a file is free to animate
/// an arbitrary custom expression, and dropping the map would silently discard
/// its tracks. `custom` has a `Default`, so a file that omits the key (or the
/// whole `expressions` object) still deserializes.
#[derive(Serialize, Deserialize, Debug, Clone, Default)]
#[serde(default)]
pub(crate) struct VrmaExpressions {
    /// `VRMC_vrm_animation.expressions.preset`: `VRMC_vrm` preset names that
    /// must not be one of `lookUp` / `lookDown` / `lookLeft` / `lookRight`.
    pub preset: HashMap<String, VrmNode>,
    /// `VRMC_vrm_animation.expressions.custom`: any name that is not a preset
    /// name.
    pub custom: HashMap<String, VrmNode>,
}

#[derive(Serialize, Deserialize, Debug)]
pub(crate) struct VrmaHumanoid {
    #[serde(rename = "humanBones")]
    pub human_bones: HashMap<String, VrmNode>,
}

#[derive(Serialize, Deserialize)]
pub(crate) struct VRMCVrmAnimation {
    pub expressions: Option<VrmaExpressions>,
    pub humanoid: VrmaHumanoid,
    #[serde(rename = "specVersion")]
    pub spec_version: Option<String>,
}

#[derive(Serialize, Deserialize)]
pub(crate) struct VrmaExtensions {
    #[serde(rename = "VRMC_vrm_animation")]
    pub vrmc_vrm_animation: VRMCVrmAnimation,
}

impl VrmaExtensions {
    pub fn new(json: &serde_json::map::Map<String, serde_json::Value>) -> AppResult<Self> {
        Ok(Self {
            vrmc_vrm_animation: serde_json::from_value(obtain_vrmc_vrm(json)?)?,
        })
    }

    pub fn from_gltf(gltf: &Gltf) -> AppResult<Self> {
        Self::new(obtain_extensions(gltf)?)
    }
}
