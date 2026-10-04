use crate::prelude::ChildSearcher;
use bevy::ecs::system::SystemParam;
use bevy::prelude::{AnimationPlayer, Entity, Query, Reflect};

/// Progress of the `.vrma` animations of one avatar.
///
/// # One player, everything on it
///
/// A load-time avatar has exactly **one** `AnimationPlayer`: the one on
/// [`Vrm::ROOT_BONE`](crate::prelude::Vrm), holding the graph
/// `AnimationGraph::from_clips` built from every `.vrma` the avatar owns. A
/// `.vrma`'s humanoid bone tracks *and* its expression tracks are nodes of that
/// same graph — the expression curves live on the avatar root's synthetic
/// animation target, played by that player — so there is no second timeline to
/// consult and no per-expression entity to walk.
///
/// That is what makes the two predicates below mean something for a load-time
/// avatar. `finished_expressions` is not a separate probe with its own answer:
/// a graph node's animation is either running or it is not, whatever property
/// that node drives.
#[derive(SystemParam)]
pub struct VrmAnimation<'w, 's> {
    searcher: ChildSearcher<'w, 's>,
    players: Query<'w, 's, &'static AnimationPlayer>,
}

impl VrmAnimation<'_, '_> {
    /// Whether every animation of `vrm` — bones and expressions alike — has
    /// finished.
    pub fn all_finished(
        &self,
        vrm: Entity,
    ) -> bool {
        self.finished_humanoid_bones(vrm)
    }

    /// Whether the avatar's root-bone player has stopped running.
    pub fn finished_humanoid_bones(
        &self,
        vrm: Entity,
    ) -> bool {
        if let Some(root_bone) = self.searcher.find_root_bone(vrm)
            && let Ok(animation_player) = self.players.get(root_bone)
            && !animation_player.all_finished()
        {
            false
        } else {
            true
        }
    }

    /// Whether the avatar's expression animation has finished.
    ///
    /// Equal to [`Self::finished_humanoid_bones`] by construction: the expression
    /// curves of every `.vrma` are nodes of the same graph, evaluated by the same
    /// player. A `.vrma` that animates expressions therefore reports itself here,
    /// and one that does not is covered by the same answer.
    pub fn finished_expressions(
        &self,
        vrm: Entity,
    ) -> bool {
        self.finished_humanoid_bones(vrm)
    }
}
