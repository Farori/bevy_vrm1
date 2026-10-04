//! Facial expressions: the shape a `.vrm` carries, and the three user-facing
//! triggers that write into it.
//!
//! # The shape
//!
//! The load-time glTF pipeline writes the expression data onto the VRM root
//! itself, while the file loads: [`VrmExpressionWeights`] holds the live
//! weights, [`ExpressionSettings`] holds one entry per expression the file
//! declares (`isBinary`, category, the three `override*` types), and
//! `ExpressionMorphBinds` maps each expression name onto the morph slots it
//! drives. There are no per-expression entities.
//!
//! [`apply_expression_morph_binds`] — scheduled by
//! [`VrmGltfPlugin`](crate::prelude::VrmGltfPlugin) — reads those three
//! components and distributes the weights into `MorphWeights`. A trigger
//! therefore only has to write the weight; thresholding `isBinary`, the
//! `overrideMouth`/`overrideBlink`/`overrideLookAt` suppressors and the
//! bind-weight multiplication all happen in that pass.
//!
//! A trigger that names an expression the model does not declare is skipped,
//! validated against [`ExpressionSettings`].

use crate::prelude::{ExpressionSetting, ExpressionSettings, VrmExpressionWeights};
use crate::vrm::{Vrm, VrmExpression};
use bevy::app::Plugin;
use bevy::platform::collections::HashMap;
use bevy::prelude::*;

#[derive(Reflect, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ExpressionCategory {
    Mouth,
    Blink,
    LookAt,
    Other,
}

impl ExpressionCategory {
    pub fn from_preset_name(name: &str) -> Self {
        match name {
            "aa" | "ih" | "ou" | "ee" | "oh" => Self::Mouth,
            "blink" | "blinkLeft" | "blinkRight" => Self::Blink,
            "lookUp" | "lookDown" | "lookLeft" | "lookRight" => Self::LookAt,
            _ => Self::Other,
        }
    }
}

#[derive(Reflect, Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExpressionOverrideType {
    None,
    Block,
    Blend,
}

impl ExpressionOverrideType {
    pub fn rate(
        &self,
        weight: f32,
    ) -> f32 {
        match self {
            Self::None => 0.0,
            Self::Block => {
                if weight > 0.0 {
                    1.0
                } else {
                    0.0
                }
            }
            Self::Blend => weight,
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "block" => Self::Block,
            "blend" => Self::Blend,
            _ => Self::None,
        }
    }
}

/// Sets expression weights on a VRM model, **replacing all previous overrides**.
///
/// Trigger this event to directly control facial expressions.
/// Expression weights are clamped to `0.0..=1.0`.
/// Expressions not included in this call will return to VRMA animation control.
///
/// For partial updates that preserve existing overrides, see [`ModifyExpressions`].
///
/// The trigger writes the [`VrmExpressionWeights`] of the target root — the
/// VRM root the load-time pipeline created — replacing the whole map. An
/// expression the map no longer names is a neutral face, which is the same
/// observable outcome as leaving it to whatever drives it: a `.vrma` expression
/// curve re-inserts its own entry.
///
/// **Note**: Triggering both `SetExpressions` and [`ModifyExpressions`]
/// on the same entity in the same frame produces undefined results.
///
/// ```no_run
/// use bevy::prelude::*;
/// use bevy_vrm1::prelude::*;
///
/// fn set_happy(mut commands: Commands, vrms: Query<Entity, With<Vrm>>) {
///     for vrm in vrms.iter() {
///         commands.trigger(SetExpressions::single(vrm, "happy", 1.0));
///     }
/// }
/// ```
#[derive(EntityEvent, Debug)]
pub struct SetExpressions {
    #[event_target]
    pub entity: Entity,
    pub weights: HashMap<VrmExpression, f32>,
}

impl SetExpressions {
    /// Creates a [`SetExpressions`] event for a single expression.
    pub fn single(
        entity: Entity,
        expression: impl Into<VrmExpression>,
        weight: f32,
    ) -> Self {
        Self {
            entity,
            weights: [(expression.into(), weight)].into_iter().collect(),
        }
    }

    /// Creates a [`SetExpressions`] event from an iterator of expression-weight pairs.
    pub fn from_iter(
        entity: Entity,
        iter: impl IntoIterator<Item = (impl Into<VrmExpression>, f32)>,
    ) -> Self {
        Self {
            entity,
            weights: iter.into_iter().map(|(e, w)| (e.into(), w)).collect(),
        }
    }
}

/// Modifies specific expression weights without affecting others (partial update).
///
/// Unlike [`SetExpressions`] which replaces all overrides,
/// this only inserts/updates the specified expressions.
/// Existing overrides not mentioned in this call remain unchanged.
///
/// This is the equivalent of `UniVRM`'s `SetWeight()` and three-vrm's `setValue()`.
/// Ideal for lip-sync where mouth expressions are updated every frame
/// while other expression overrides (e.g. emotions) remain active.
///
/// The trigger merges over the current [`VrmExpressionWeights`] of the target
/// root, and the category rules (`mouth` zeroing the other vowels) are baked
/// into the event by [`ModifyExpressions::mouth`] / `mouth_weights`, so
/// nothing here has to classify an expression.
///
/// **Note**: Triggering both [`SetExpressions`] and `ModifyExpressions`
/// on the same entity in the same frame produces undefined results.
///
/// ```no_run
/// use bevy::prelude::*;
/// use bevy_vrm1::prelude::*;
///
/// fn add_blink(mut commands: Commands, vrms: Query<Entity, With<Vrm>>) {
///     for vrm in vrms.iter() {
///         // Only modifies "blink", leaves other overrides (e.g. "happy") intact
///         commands.trigger(ModifyExpressions::single(vrm, "blink", 1.0));
///     }
/// }
/// ```
#[derive(EntityEvent, Debug)]
pub struct ModifyExpressions {
    #[event_target]
    pub entity: Entity,
    pub weights: HashMap<VrmExpression, f32>,
}

/// The five VRM preset mouth expressions used for lip-sync.
const MOUTH_EXPRESSIONS: [&str; 5] = ["aa", "ih", "ou", "ee", "oh"];

impl ModifyExpressions {
    /// Creates a [`ModifyExpressions`] event for a single expression.
    pub fn single(
        entity: Entity,
        expression: impl Into<VrmExpression>,
        weight: f32,
    ) -> Self {
        Self {
            entity,
            weights: [(expression.into(), weight)].into_iter().collect(),
        }
    }

    /// Creates a [`ModifyExpressions`] event from an iterator of expression-weight pairs.
    pub fn from_iter(
        entity: Entity,
        iter: impl IntoIterator<Item = (impl Into<VrmExpression>, f32)>,
    ) -> Self {
        Self {
            entity,
            weights: iter.into_iter().map(|(e, w)| (e.into(), w)).collect(),
        }
    }

    /// Sets a single mouth expression for lip-sync, resetting all other mouth
    /// expressions to 0.0.
    ///
    /// This is a convenience method that sets all five VRM preset mouth
    /// expressions (aa, ih, ou, ee, oh) with the specified one active and
    /// the rest at 0.0. Non-mouth expression overrides are preserved.
    ///
    /// Inserts a zero weight for inactive mouth expressions, which overrides
    /// any VRMA animation value — a `0.0` entry in [`VrmExpressionWeights`].
    /// Use [`ClearExpressions`] to return all expressions to VRMA control.
    ///
    /// ```no_run
    /// use bevy::prelude::*;
    /// use bevy_vrm1::prelude::*;
    ///
    /// fn lip_sync(mut commands: Commands, vrms: Query<Entity, With<Vrm>>) {
    ///     for vrm in vrms.iter() {
    ///         commands.trigger(ModifyExpressions::mouth(vrm, "aa", 0.8));
    ///     }
    /// }
    /// ```
    pub fn mouth(
        entity: Entity,
        expression: impl Into<VrmExpression>,
        weight: f32,
    ) -> Self {
        let active = expression.into();
        let mut weights: HashMap<VrmExpression, f32> = MOUTH_EXPRESSIONS
            .iter()
            .map(|&name| (VrmExpression::from(name), 0.0))
            .collect();
        weights.insert(active, weight);
        Self { entity, weights }
    }

    /// Sets multiple mouth expressions for blended lip-sync, resetting
    /// unspecified mouth expressions to 0.0.
    ///
    /// Useful for blend-based lip-sync where multiple vowels are active
    /// simultaneously (e.g. aa=0.3, ih=0.5). Non-mouth expression overrides
    /// are preserved.
    ///
    /// ```no_run
    /// use bevy::prelude::*;
    /// use bevy_vrm1::prelude::*;
    ///
    /// fn blended_lip_sync(mut commands: Commands, vrms: Query<Entity, With<Vrm>>) {
    ///     for vrm in vrms.iter() {
    ///         commands.trigger(ModifyExpressions::mouth_weights(
    ///             vrm,
    ///             [("aa", 0.3), ("ih", 0.5)],
    ///         ));
    ///     }
    /// }
    /// ```
    pub fn mouth_weights(
        entity: Entity,
        iter: impl IntoIterator<Item = (impl Into<VrmExpression>, f32)>,
    ) -> Self {
        let mut weights: HashMap<VrmExpression, f32> = MOUTH_EXPRESSIONS
            .iter()
            .map(|&name| (VrmExpression::from(name), 0.0))
            .collect();
        for (expr, weight) in iter {
            weights.insert(expr.into(), weight);
        }
        Self { entity, weights }
    }
}

/// Clears all expression overrides, returning control to VRMA animation.
///
/// After triggering this event, expressions previously set by [`SetExpressions`]
/// or [`ModifyExpressions`] will be controlled by VRMA animation again.
///
/// Empties the [`VrmExpressionWeights`] of the target root. The component
/// itself stays: `apply_expression_morph_binds` requires it, and it is what a
/// `.vrma` expression curve writes its own entry into.
#[derive(EntityEvent, Debug)]
pub struct ClearExpressions {
    #[event_target]
    pub entity: Entity,
}

pub(crate) struct VrmExpressionPlugin;

impl Plugin for VrmExpressionPlugin {
    fn build(
        &self,
        app: &mut App,
    ) {
        app.add_observer(apply_set_expressions)
            .add_observer(apply_modify_expressions)
            .add_observer(apply_clear_expressions);
    }
}

/// Clamps a raw weight to `0.0..=1.0` before writing it.
///
/// Both an animation curve and a trigger end up in the same place:
/// [`apply_expression_morph_binds`] thresholds `isBinary` at `0.5` and clamps
/// every other weight to `0.0..=1.0`, so clamping here keeps a value a trigger
/// wrote on the same side of the threshold and inside the `1.0 - clamp(rate)`
/// suppressors as a value an animation curve wrote, and keeps
/// [`SetExpressions`]' promise that weights arrive clamped.
fn clamped_weight(weight: f32) -> f32 {
    weight.clamp(0.0, 1.0)
}

/// Converts a trigger's weights into the shape's key type, dropping the names
/// the model does not declare.
///
/// `known` is the root's [`ExpressionSettings`] table when the loader wrote it.
/// That table holds one entry per expression the file declares, so a name the
/// model does not declare is reported and skipped here instead of being
/// inserted into a map no bind can consume. `None` means there is no table to
/// validate against — a root with weights but no settings is not a shape
/// `apply_expression_morph_binds` can distribute at all (it requires all three
/// components), so the names are taken as given, which is how that pass reads a
/// weight without a setting: non-binary, unsuppressed.
///
/// The values are written **raw**: `isBinary` thresholding, the
/// `overrideMouth`/`overrideBlink`/`overrideLookAt` suppressors and the
/// bind-weight multiplication all happen later, in
/// `apply_expression_morph_binds`. Applying them here instead would apply them
/// twice for any expression an animation also drives.
fn raw_expression_weights(
    weights: &HashMap<VrmExpression, f32>,
    known: Option<&HashMap<String, ExpressionSetting>>,
    trigger: &str,
) -> HashMap<String, f32> {
    weights
        .iter()
        .filter_map(|(expression, weight)| {
            let name = expression.as_str();
            if let Some(known) = known
                && !known.contains_key(name)
            {
                #[cfg(feature = "log")]
                warn!("{trigger}: expression '{name}' not found");
                return None;
            }
            Some((name.to_owned(), clamped_weight(*weight)))
        })
        .collect()
}

fn apply_set_expressions(
    trigger: On<SetExpressions>,
    mut load_time: Query<(&mut VrmExpressionWeights, Option<&ExpressionSettings>)>,
) {
    let vrm_entity = trigger.event_target();
    // The weights *are* the override state, so replacing the map is the
    // replace-all. A name the map no longer holds is a neutral face
    // (`apply_expression_morph_binds` reads a missing weight as `0.0`), which
    // leaves the expression to whatever drives it — here a `.vrma` expression
    // curve, which re-inserts its own entry.
    let Ok((mut weights, settings)) = load_time.get_mut(vrm_entity) else {
        #[cfg(feature = "log")]
        warn!(
            "SetExpressions: no VrmExpressionWeights on entity {:?}. Is it a VRM root?",
            vrm_entity
        );
        return;
    };
    weights.0 = raw_expression_weights(
        &trigger.weights,
        settings.map(|settings| &settings.0),
        "SetExpressions",
    );
}

fn apply_modify_expressions(
    trigger: On<ModifyExpressions>,
    mut load_time: Query<(&mut VrmExpressionWeights, Option<&ExpressionSettings>)>,
) {
    let vrm_entity = trigger.event_target();
    // Merging the written weights over the current ones is the documented
    // partial update; see [`ModifyExpressions`].
    let Ok((mut weights, settings)) = load_time.get_mut(vrm_entity) else {
        #[cfg(feature = "log")]
        warn!(
            "ModifyExpressions: no VrmExpressionWeights on entity {:?}. Is it a VRM root?",
            vrm_entity
        );
        return;
    };
    weights.0.extend(raw_expression_weights(
        &trigger.weights,
        settings.map(|settings| &settings.0),
        "ModifyExpressions",
    ));
}

fn apply_clear_expressions(
    trigger: On<ClearExpressions>,
    mut load_time: Query<&mut VrmExpressionWeights>,
) {
    let vrm_entity = trigger.event_target();
    // Emptying the map is the clear. The component itself stays — it is what
    // `apply_expression_morph_binds` requires, and removing it would make the
    // next trigger fail to find a shape to write.
    let Ok(mut weights) = load_time.get_mut(vrm_entity) else {
        return;
    };
    weights.0.clear();
}

#[cfg(test)]
mod tests {
    use crate::prelude::*;
    use crate::success;
    use crate::tests::{TestResult, test_app};
    use crate::vrm::expressions::{
        ClearExpressions, ExpressionCategory, ExpressionOverrideType, ModifyExpressions,
        SetExpressions, VrmExpressionPlugin,
    };
    use bevy::ecs::system::RunSystemOnce;
    use bevy::mesh::morph::MorphWeights;
    use bevy::platform::collections::HashMap;
    use bevy::prelude::*;

    /// The per-expression settings of a model declaring `names`: what the loader
    /// writes, and the only place a trigger can check that a name exists.
    fn load_time_settings(names: &[&str]) -> ExpressionSettings {
        ExpressionSettings(
            names
                .iter()
                .map(|name| {
                    (
                        (*name).to_owned(),
                        ExpressionSetting {
                            is_binary: false,
                            category: ExpressionCategory::from_preset_name(name),
                            override_blink: ExpressionOverrideType::None,
                            override_look_at: ExpressionOverrideType::None,
                            override_mouth: ExpressionOverrideType::None,
                        },
                    )
                })
                .collect(),
        )
    }

    /// The weights of a model declaring `names`, seeded the way the loader's
    /// `build_expressions` seeds them: one entry per expression, all at `0.0`.
    fn load_time_weights(names: &[&str]) -> VrmExpressionWeights {
        VrmExpressionWeights(names.iter().map(|name| ((*name).to_owned(), 0.0)).collect())
    }

    fn weights_of(
        app: &App,
        vrm_entity: Entity,
    ) -> HashMap<String, f32> {
        app.world()
            .get::<VrmExpressionWeights>(vrm_entity)
            .expect("the root carries the expression weights")
            .0
            .clone()
    }

    /// There are no per-expression entities, so `SetExpressions` works from the
    /// root's weights alone and replaces every weight it did not name.
    #[test]
    fn test_set_expressions_writes_load_time_weights() -> TestResult {
        let mut app = test_app();
        app.add_plugins(VrmExpressionPlugin);

        let vrm_entity = app
            .world_mut()
            .spawn((
                load_time_weights(&["happy", "angry", "aa"]),
                load_time_settings(&["happy", "angry", "aa"]),
            ))
            .id();

        app.world_mut()
            .commands()
            .trigger(SetExpressions::single(vrm_entity, "happy", 0.8));
        app.update();

        let weights = weights_of(&app, vrm_entity);
        assert_eq!(weights.len(), 1, "SetExpressions replaces all weights");
        assert_eq!(weights["happy"], 0.8);
        success!()
    }

    /// A second `SetExpressions` drops the weight the first one wrote, leaving
    /// that expression to animation.
    #[test]
    fn test_set_expressions_replaces_previous_load_time_weights() -> TestResult {
        let mut app = test_app();
        app.add_plugins(VrmExpressionPlugin);

        let vrm_entity = app
            .world_mut()
            .spawn((
                load_time_weights(&["happy", "angry"]),
                load_time_settings(&["happy", "angry"]),
            ))
            .id();

        app.world_mut()
            .commands()
            .trigger(SetExpressions::single(vrm_entity, "happy", 1.0));
        app.update();
        assert_eq!(weights_of(&app, vrm_entity)["happy"], 1.0);

        app.world_mut()
            .commands()
            .trigger(SetExpressions::single(vrm_entity, "angry", 0.7));
        app.update();

        let weights = weights_of(&app, vrm_entity);
        assert!(
            !weights.contains_key("happy"),
            "the previous weight is dropped, leaving the expression to animation"
        );
        assert_eq!(weights["angry"], 0.7);
        success!()
    }

    /// Weights are clamped on write, as documented on [`SetExpressions`], and a
    /// name the model does not declare is skipped instead of being inserted.
    #[test]
    fn test_set_expressions_clamps_and_skips_unknown_load_time_weights() -> TestResult {
        let mut app = test_app();
        app.add_plugins(VrmExpressionPlugin);

        let vrm_entity = app
            .world_mut()
            .spawn((
                load_time_weights(&["happy"]),
                load_time_settings(&["happy"]),
            ))
            .id();

        app.world_mut()
            .commands()
            .trigger(SetExpressions::from_iter(
                vrm_entity,
                [("happy", 4.0), ("nope", 1.0)],
            ));
        app.update();

        let weights = weights_of(&app, vrm_entity);
        assert_eq!(weights["happy"], 1.0, "the weight is clamped to 1.0");
        assert!(
            !weights.contains_key("nope"),
            "an expression the model does not declare is not written"
        );
        success!()
    }

    /// `ModifyExpressions` is the partial update: it merges over the current
    /// weights instead of replacing them.
    #[test]
    fn test_modify_expressions_merges_load_time_weights() -> TestResult {
        let mut app = test_app();
        app.add_plugins(VrmExpressionPlugin);

        let vrm_entity = app
            .world_mut()
            .spawn((
                load_time_weights(&["happy", "angry"]),
                load_time_settings(&["happy", "angry"]),
            ))
            .id();

        app.world_mut()
            .commands()
            .trigger(SetExpressions::single(vrm_entity, "happy", 1.0));
        app.update();

        app.world_mut()
            .commands()
            .trigger(ModifyExpressions::single(vrm_entity, "angry", 0.7));
        app.update();

        let weights = weights_of(&app, vrm_entity);
        assert_eq!(weights["happy"], 1.0, "the existing weight is preserved");
        assert_eq!(weights["angry"], 0.7, "the named weight is merged in");
        success!()
    }

    /// `ModifyExpressions::mouth` zeroes the four other vowels on write, which
    /// is what makes lip-sync work, and leaves the non-mouth weights alone.
    #[test]
    fn test_modify_expressions_mouth_resets_the_other_vowels_on_load_time_weights() -> TestResult {
        let mut app = test_app();
        app.add_plugins(VrmExpressionPlugin);

        const NAMES: [&str; 6] = ["happy", "aa", "ih", "ou", "ee", "oh"];
        let vrm_entity = app
            .world_mut()
            .spawn((load_time_weights(&NAMES), load_time_settings(&NAMES)))
            .id();

        app.world_mut()
            .commands()
            .trigger(SetExpressions::single(vrm_entity, "happy", 0.6));
        app.update();

        app.world_mut()
            .commands()
            .trigger(ModifyExpressions::mouth(vrm_entity, "aa", 0.8));
        app.update();

        let weights = weights_of(&app, vrm_entity);
        assert_eq!(weights["aa"], 0.8, "the active vowel");
        for vowel in ["ih", "ou", "ee", "oh"] {
            assert_eq!(
                weights[vowel], 0.0,
                "`ModifyExpressions::mouth` resets {vowel}"
            );
        }
        assert_eq!(weights["happy"], 0.6, "a non-mouth weight is preserved");
        success!()
    }

    /// `ClearExpressions` empties the weights but keeps the component, which
    /// `apply_expression_morph_binds` requires.
    #[test]
    fn test_clear_expressions_clears_load_time_weights() -> TestResult {
        let mut app = test_app();
        app.add_plugins(VrmExpressionPlugin);

        let vrm_entity = app
            .world_mut()
            .spawn((
                load_time_weights(&["happy", "angry"]),
                load_time_settings(&["happy", "angry"]),
            ))
            .id();

        app.world_mut()
            .commands()
            .trigger(SetExpressions::from_iter(
                vrm_entity,
                [("happy", 0.8), ("angry", 0.5)],
            ));
        app.update();
        assert_eq!(weights_of(&app, vrm_entity).len(), 2);

        app.world_mut()
            .commands()
            .trigger(ClearExpressions { entity: vrm_entity });
        app.update();

        assert!(
            weights_of(&app, vrm_entity).is_empty(),
            "every weight is cleared"
        );
        assert!(
            app.world()
                .get::<VrmExpressionWeights>(vrm_entity)
                .is_some(),
            "the component stays: the bind pass needs it"
        );
        success!()
    }

    /// A trigger writes a *raw* weight: `isBinary` is thresholded by
    /// `apply_expression_morph_binds`, not by the trigger.
    #[test]
    fn test_triggered_load_time_weights_are_distributed_by_the_bind_pass() -> TestResult {
        let mut app = test_app();
        app.add_plugins(VrmExpressionPlugin);

        let mesh_entity = app
            .world_mut()
            .spawn(MorphWeights::new(vec![0.0], None)?)
            .id();
        let vrm_entity = app
            .world_mut()
            .spawn((
                load_time_weights(&["blink"]),
                ExpressionMorphBinds(MorphBindTable(HashMap::from([(
                    "blink".to_owned(),
                    vec![MorphBind {
                        target: mesh_entity,
                        index: 0,
                        weight: 1.0,
                    }],
                )]))),
                ExpressionSettings(HashMap::from([(
                    "blink".to_owned(),
                    ExpressionSetting {
                        is_binary: true,
                        category: ExpressionCategory::Blink,
                        override_blink: ExpressionOverrideType::None,
                        override_look_at: ExpressionOverrideType::None,
                        override_mouth: ExpressionOverrideType::None,
                    },
                )])),
            ))
            .id();

        app.world_mut()
            .commands()
            .trigger(SetExpressions::single(vrm_entity, "blink", 0.3));
        app.update();

        assert_eq!(
            weights_of(&app, vrm_entity)["blink"],
            0.3,
            "the trigger stores the weight verbatim"
        );

        app.world_mut()
            .run_system_once(apply_expression_morph_binds)
            .expect("the bind pass runs");

        let morph = app.world().get::<MorphWeights>(mesh_entity).unwrap();
        assert_eq!(
            morph.weights()[0],
            0.0,
            "`isBinary` thresholds 0.3 to 0.0, in the bind pass"
        );
        success!()
    }

    /// A trigger aimed at an entity that is not a VRM root is a no-op rather
    /// than a panic: the target has no weights to write.
    #[test]
    fn a_trigger_on_an_entity_without_weights_is_ignored() -> TestResult {
        let mut app = test_app();
        app.add_plugins(VrmExpressionPlugin);

        let entity = app.world_mut().spawn(Name::new("not-a-vrm")).id();
        app.world_mut()
            .commands()
            .trigger(SetExpressions::single(entity, "happy", 1.0));
        app.update();

        assert!(app.world().get::<Name>(entity).is_some());
        success!()
    }
}
