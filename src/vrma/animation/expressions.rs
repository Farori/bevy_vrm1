//!  This module handles the retargeting of expressions from a VRM model to a mascot model.
//!
//! The destination expressions are the load-time shape: the `.vrma` retarget
//! writes [`VrmExpressionWeights`] on the VRM root through
//! [`ExpressionWeightProperty`](crate::prelude::ExpressionWeightProperty), and
//! [`apply_expression_morph_binds`](crate::prelude::apply_expression_morph_binds)
//! — scheduled by [`VrmGltfPlugin`](crate::prelude::VrmGltfPlugin) — distributes
//! them into the bound `MorphWeights`. There are no per-expression entities and
//! no separate runtime expression pass to register.

use crate::vrm::VrmExpression;
use crate::vrma::gltf::extensions::VrmaExtensions;
use bevy::app::App;
use bevy::prelude::*;

pub(in crate::vrma) struct VrmaRetargetExpressionsPlugin;

impl Plugin for VrmaRetargetExpressionsPlugin {
    fn build(
        &self,
        _app: &mut App,
    ) {
    }
}

#[derive(Component, Deref, Reflect)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", reflect(Serialize, Deserialize))]
pub(crate) struct VrmaExpressionNames(Vec<VrmExpression>);

impl VrmaExpressionNames {
    pub fn new(extensions: &VrmaExtensions) -> Self {
        let Some(expressions) = extensions.vrmc_vrm_animation.expressions.as_ref() else {
            return Self(Vec::default());
        };
        Self(
            expressions
                .preset
                .keys()
                .map(|expression| VrmExpression(expression.clone()))
                .collect(),
        )
    }
}

#[cfg(test)]
mod tests {
    use crate::success;
    use crate::tests::TestResult;
    use crate::vrm::expressions::{
        ExpressionCategory, ExpressionOverrideType, SetExpressions, VrmExpressionPlugin,
    };
    use crate::vrma::animation::prelude::{
        ExpressionMorphBinds, ExpressionSetting, ExpressionSettings, MorphBind, MorphBindTable,
        VrmExpressionWeights, apply_expression_morph_binds,
    };
    use bevy::ecs::system::RunSystemOnce;
    use bevy::mesh::morph::MorphWeights;
    use bevy::platform::collections::HashMap;
    use bevy::prelude::*;

    /// The pipeline shape, end to end for one expression: the `.vrma`
    /// destination curve writes [`VrmExpressionWeights`], and the bind pass
    /// moves it into the mesh's morph weights.
    ///
    /// This is the case `apply_regenerate_expression_clips` used to cover for the
    /// per-expression-entity shape: the clip's expression curves reach the
    /// avatar's morph targets without any intermediate entity.
    #[test]
    fn a_retargeted_expression_weight_reaches_the_morph_target() -> TestResult {
        let mut app = crate::tests::test_app();
        app.add_plugins(VrmExpressionPlugin);

        let mesh = app
            .world_mut()
            .spawn(MorphWeights::new(vec![0.0, 0.0], None)?)
            .id();
        let root = app
            .world_mut()
            .spawn((
                VrmExpressionWeights(HashMap::from([("happy".to_owned(), 0.0)])),
                ExpressionMorphBinds(MorphBindTable(HashMap::from([(
                    "happy".to_owned(),
                    vec![MorphBind {
                        target: mesh,
                        index: 1,
                        weight: 1.0,
                    }],
                )]))),
                ExpressionSettings(HashMap::from([(
                    "happy".to_owned(),
                    ExpressionSetting {
                        is_binary: false,
                        category: ExpressionCategory::Other,
                        override_blink: ExpressionOverrideType::None,
                        override_look_at: ExpressionOverrideType::None,
                        override_mouth: ExpressionOverrideType::None,
                    },
                )])),
            ))
            .id();

        // The retarget's weight, as an animation curve or an
        // `ExpressionWeightProperty` writes it.
        app.world_mut()
            .entity_mut(root)
            .get_mut::<VrmExpressionWeights>()
            .expect("the root carries the weights")
            .0
            .insert("happy".to_owned(), 0.6);
        app.world_mut()
            .run_system_once(apply_expression_morph_binds)
            .expect("the bind pass runs");

        assert_eq!(
            app.world().get::<MorphWeights>(mesh).unwrap().weights()[1],
            0.6
        );

        // A user trigger on the same root wins over the retargeted weight, which
        // is what makes an expression overridable at runtime.
        app.world_mut()
            .commands()
            .trigger(SetExpressions::single(root, "happy", 0.25));
        app.update();
        app.world_mut()
            .run_system_once(apply_expression_morph_binds)
            .expect("the bind pass runs");

        assert_eq!(
            app.world().get::<MorphWeights>(mesh).unwrap().weights()[1],
            0.25
        );
        success!()
    }
}
