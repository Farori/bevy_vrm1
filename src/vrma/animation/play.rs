use crate::prelude::ChildSearcher;
use crate::vrm::spring_bone::{SpringJointState, SpringRoot};
use crate::vrma::VrmAnimationNodeIndex;
use crate::vrma::animation::expressions::{VrmaExpressionRegistry, reset_expression_weights};
use crate::vrma::animation::properties::VrmExpressionWeights;
use bevy::animation::{AnimationPlayer, RepeatAnimation};
use bevy::app::{App, Plugin};
use bevy::prelude::*;
use std::time::Duration;

/// The trigger event to play the Vrma's animation.
///
/// You need to emit this via [`On`] with the target entity of the VRMA you want to play the animation on.
///
/// If there are multiple VRMA entities, the animation of all other VRMAs will be stopped except for the one specified in the trigger.
#[derive(EntityEvent, Debug, Reflect)]
pub struct PlayVrma {
    #[event_target]
    pub vrma: Entity,

    /// Repetition behavior of an animation.
    pub repeat: RepeatAnimation,

    /// A time until the existing animation fades out.
    pub transition_duration: Duration,

    /// If true, resets all `SpringBone` velocities on the parent VRM entity
    /// to prevent bouncing caused by sudden bone movements during animation transitions.
    pub reset_spring_bones: bool,
}

impl PlayVrma {
    /// Creates a new `PlayVrma` event with default settings.
    ///
    /// Default repeat is [`RepeatAnimation::Never`] and transition duration is 300 milliseconds.
    pub fn new(entity: Entity) -> Self {
        Self {
            vrma: entity,
            repeat: RepeatAnimation::Never,
            transition_duration: Duration::from_millis(300),
            reset_spring_bones: false,
        }
    }
}

/// The trigger event to stop the Vrma's animation.
///You need to emit this via [`On`] with the target entity of the VRMA you want to stop the animation on.
#[derive(EntityEvent, Debug)]
pub struct StopVrma {
    pub entity: Entity,
}

pub(super) struct VrmaAnimationPlayPlugin;

impl Plugin for VrmaAnimationPlayPlugin {
    fn build(
        &self,
        app: &mut App,
    ) {
        app.register_type::<PlayVrma>()
            .add_observer(apply_play_vrma)
            .add_observer(apply_stop_vrma);
    }
}

fn apply_play_vrma(
    trigger: On<PlayVrma>,
    mut players: Query<(
        &mut Transform,
        &mut AnimationPlayer,
        Option<&mut AnimationTransitions>,
    )>,
    searcher: ChildSearcher,
    parents: Query<&ChildOf>,
    childrens: Query<&Children>,
    vrmas: Query<&VrmAnimationNodeIndex>,
    expression_registries: Query<&VrmaExpressionRegistry>,
    mut roots: Query<&mut VrmExpressionWeights>,
    spring_roots: Query<&SpringRoot>,
    mut joint_states: Query<&mut SpringJointState>,
) {
    let vrma_entity = trigger.event_target();
    let Ok(ChildOf(vrm_entity)) = parents.get(vrma_entity) else {
        return;
    };
    let Ok(node_index) = vrmas.get(vrma_entity) else {
        return;
    };
    play_humanoid_bone_animation(
        *vrm_entity,
        node_index.0,
        trigger.repeat,
        trigger.transition_duration,
        &searcher,
        &mut players,
    );
    // The expression tracks are nodes of the same clip, so the same player and
    // the same `repeat` drive them. What still needs doing is the neutral face
    // the transition blends out of: without it a clip that fades in over 300 ms
    // fades out of whatever the previous one left behind.
    reset_expression_weights(vrma_entity, &expression_registries, &mut roots, &parents);
    if trigger.reset_spring_bones {
        reset_spring_bone_velocities(*vrm_entity, &spring_roots, &mut joint_states, &childrens);
    }
}

/// Recursively traverses descendants of `entity` to find all [`SpringRoot`] components
/// and resets the velocity of their [`SpringJointState`]s.
fn reset_spring_bone_velocities(
    entity: Entity,
    spring_roots: &Query<&SpringRoot>,
    joint_states: &mut Query<&mut SpringJointState>,
    children: &Query<&Children>,
) {
    let Ok(entity_children) = children.get(entity) else {
        return;
    };
    for child in entity_children.into_iter().copied() {
        if let Ok(root) = spring_roots.get(child) {
            for &joint in root.joints.iter() {
                if let Ok(mut state) = joint_states.get_mut(joint) {
                    state.reset_velocity();
                }
            }
        }
        reset_spring_bone_velocities(child, spring_roots, joint_states, children);
    }
}

fn play_humanoid_bone_animation(
    vrm: Entity,
    node_index: AnimationNodeIndex,
    repeat: RepeatAnimation,
    transition_duration: Duration,
    searcher: &ChildSearcher,
    players: &mut Query<(
        &mut Transform,
        &mut AnimationPlayer,
        Option<&mut AnimationTransitions>,
    )>,
) {
    let Some(root_bone) = searcher.find_root_bone(vrm) else {
        return;
    };
    let Ok((_, mut player, Some(mut transitions))) = players.get_mut(root_bone) else {
        return;
    };
    transitions
        .play(&mut player, node_index, transition_duration)
        .set_repeat(repeat);
}

fn apply_stop_vrma(
    trigger: On<StopVrma>,
    mut players: Query<&mut AnimationPlayer>,
    vrmas: Query<&VrmAnimationNodeIndex>,
    parents: Query<&ChildOf>,
    expression_registries: Query<&VrmaExpressionRegistry>,
    mut roots: Query<&mut VrmExpressionWeights>,
    searcher: ChildSearcher,
) {
    let vrma_entity = trigger.event_target();
    let Ok(ChildOf(vrm)) = parents.get(vrma_entity) else {
        return;
    };
    let Ok(node_index) = vrmas.get(vrma_entity) else {
        return;
    };
    // The player is on the avatar's root bone — the same entity
    // `play_humanoid_bone_animation` starts it on — and the node it holds plays
    // the bone curves *and* the expression curves of this very `.vrma`, so one
    // `stop` covers the face as well as the body.
    if let Some(root_bone) = searcher.find_root_bone(*vrm)
        && let Ok(mut player) = players.get_mut(root_bone)
    {
        player.stop(node_index.0);
    }
    // A finished animation would otherwise leave the face frozen at its last
    // weight, since the weight lives in a map nothing else decays.
    reset_expression_weights(vrma_entity, &expression_registries, &mut roots, &parents);
}

#[cfg(test)]
mod tests {
    use crate::prelude::*;
    use crate::tests::test_app;
    use crate::vrma::VrmAnimationNodeIndex;
    use crate::vrma::animation::play::VrmaAnimationPlayPlugin;
    use bevy::ecs::system::RunSystemOnce;
    use bevy::prelude::*;
    use bevy_test_helper::system::SystemExt;

    #[test]
    fn test_play_vrma() {
        let mut app = test_app();
        app.add_plugins(VrmaAnimationPlayPlugin);

        let vrm = app.world_mut().spawn_empty().id();
        let vrma = app.world_mut().spawn(VrmAnimationNodeIndex::default()).id();
        app.world_mut().commands().entity(vrm).add_child(vrma);

        app.world_mut().commands().entity(vrm).with_child((
            Name::new(Vrm::ROOT_BONE),
            Transform::default(),
            AnimationPlayer::default(),
            AnimationTransitions::default(),
        ));

        app.world_mut()
            .commands()
            .entity(vrma)
            .trigger(PlayVrma::new);
        app.update();

        app.world_mut()
            .run_system_once(|player: Query<&AnimationPlayer>| {
                let player = player.single().expect("Failed to find AnimationPlayer");
                assert!(!player.all_finished());
            })
            .unwrap();
    }

    #[test]
    fn test_stop_vrma() {
        let mut app = test_app();
        app.add_plugins(VrmaAnimationPlayPlugin);

        let vrm = app.world_mut().spawn_empty().id();
        let vrma = app.world_mut().spawn(VrmAnimationNodeIndex::default()).id();
        app.world_mut().commands().entity(vrm).add_child(vrma);

        app.world_mut().commands().entity(vrm).with_child((
            Name::new(Vrm::ROOT_BONE),
            AnimationPlayer::default(),
            AnimationTransitions::default(),
        ));

        app.world_mut()
            .commands()
            .entity(vrma)
            .trigger(PlayVrma::new);
        app.update();

        app.world_mut()
            .commands()
            .entity(vrma)
            .trigger(|entity| StopVrma { entity });
        app.update();

        app.world_mut()
            .run_system_once(|player: Query<&AnimationPlayer>| {
                let player = player.single().expect("Failed to find AnimationPlayer");
                assert!(player.all_finished());
            })
            .unwrap();
    }
}
