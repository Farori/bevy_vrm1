//! Bone mask groups for composing VRMA animation layers.
//!
//! An `AnimationGraph` node carries a 64-bit mask (`AnimationMask`, a `u64`).
//! Bit `N` of that bitfield **masks out** mask group `N`: during evaluation a
//! clip node is skipped for a target whose own group mask intersects the
//! node's computed mask (`target_mask & node_mask != 0`). Consequently `0`
//! means "nothing is masked out", i.e. the node animates every target, and a
//! layer that must drive only part of the body masks out every group it must
//! *not* touch.
//!
//! Targets are attached to groups with `AnimationGraph::add_target_to_mask_group`;
//! [`mask_group_for_bone`] maps a VRM humanoid bone name onto its group.

use bevy::animation::graph::AnimationMask;

/// A group of VRM humanoid bones, used as a bit index in an
/// [`AnimationMask`].
///
/// Every bone is registered in exactly one group via
/// `AnimationGraph::add_target_to_mask_group`, and a layer masks out every
/// group it must not drive.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum VrmMaskGroup {
    /// The hips bone, and with it the root translation of the model.
    Hips = 0,
    /// Legs, feet and toes.
    LowerBody = 1,
    /// Spine, chest, shoulders and arms.
    UpperBody = 2,
    /// Neck, head, eyes and jaw.
    Head = 3,
    /// Thumb, index, middle, ring and little finger bones.
    Fingers = 4,
}

/// Ready-made [`AnimationMask`] values for VRMA layers.
///
/// These are *mask-out* bitfields: a set bit hides the matching group, so a
/// layer lists the groups it must leave alone. See the module documentation.
pub struct VrmaMask;

impl VrmaMask {
    /// Animates every group; nothing is masked out.
    pub const ALL: AnimationMask = 0;
    /// Animates the upper body only: hips and legs are masked out.
    pub const UPPER_BODY: AnimationMask =
        Self::bit(VrmMaskGroup::Hips) | Self::bit(VrmMaskGroup::LowerBody);
    /// Animates the hips and legs only; the upper body, head and fingers are
    /// masked out.
    pub const LOWER_BODY: AnimationMask = Self::bit(VrmMaskGroup::UpperBody)
        | Self::bit(VrmMaskGroup::Head)
        | Self::bit(VrmMaskGroup::Fingers);
    /// Animates everything except the hips, for in-place playback that keeps
    /// the model's own root motion.
    pub const NO_HIPS: AnimationMask = Self::bit(VrmMaskGroup::Hips);

    /// The single bit that masks out `group`.
    pub const fn bit(group: VrmMaskGroup) -> AnimationMask {
        1 << (group as u32)
    }
}

/// Maps a VRM humanoid bone name onto its [`VrmMaskGroup`].
///
/// Unrecognized names fall into [`VrmMaskGroup::UpperBody`], which is the
/// group that holds the spine, chest, shoulders and arms.
pub fn mask_group_for_bone(bone: &str) -> VrmMaskGroup {
    match bone {
        "hips" => VrmMaskGroup::Hips,
        // legs
        b if b.contains("UpperLeg")
            || b.contains("LowerLeg")
            || b.contains("Foot")
            || b.contains("Toes") =>
        {
            VrmMaskGroup::LowerBody
        }
        // head
        "neck" | "head" | "leftEye" | "rightEye" | "jaw" => VrmMaskGroup::Head,
        // fingers
        b if b.contains("Thumb")
            || b.contains("Index")
            || b.contains("Middle")
            || b.contains("Ring")
            || b.contains("Little") =>
        {
            VrmMaskGroup::Fingers
        }
        // everything else: spine, chest, shoulders, arms
        _ => VrmMaskGroup::UpperBody,
    }
}

#[cfg(test)]
mod tests {
    use bevy::animation::AnimationTargetId;
    use bevy::animation::graph::AnimationGraph;
    use bevy::prelude::*;

    use super::*;
    use crate::prelude::{VrmMaskGroup as ExportedGroup, VrmaMask as ExportedMask};

    /// The prelude must expose these for composing layers outside the crate.
    #[test]
    fn items_are_reachable_from_the_crate_prelude() {
        let _: AnimationMask = ExportedMask::NO_HIPS;
        assert_eq!(ExportedGroup::Head as u32, 3);
    }

    #[test]
    fn bones_fall_into_expected_groups() {
        assert_eq!(mask_group_for_bone("hips"), VrmMaskGroup::Hips);
        assert_eq!(mask_group_for_bone("leftUpperLeg"), VrmMaskGroup::LowerBody);
        assert_eq!(mask_group_for_bone("rightFoot"), VrmMaskGroup::LowerBody);
        assert_eq!(mask_group_for_bone("rightToes"), VrmMaskGroup::LowerBody);
        assert_eq!(mask_group_for_bone("head"), VrmMaskGroup::Head);
        assert_eq!(mask_group_for_bone("jaw"), VrmMaskGroup::Head);
        assert_eq!(
            mask_group_for_bone("rightIndexDistal"),
            VrmMaskGroup::Fingers
        );
        assert_eq!(mask_group_for_bone("chest"), VrmMaskGroup::UpperBody);
        assert_eq!(mask_group_for_bone("leftShoulder"), VrmMaskGroup::UpperBody);
        // Unknown / non-humanoid bones must not leak into a narrow group.
        assert_eq!(mask_group_for_bone("skirt"), VrmMaskGroup::UpperBody);
    }

    #[test]
    fn bit_is_the_group_index() {
        for (group, index) in [
            (VrmMaskGroup::Hips, 0u32),
            (VrmMaskGroup::LowerBody, 1),
            (VrmMaskGroup::UpperBody, 2),
            (VrmMaskGroup::Head, 3),
            (VrmMaskGroup::Fingers, 4),
        ] {
            assert_eq!(group as u32, index);
            assert_eq!(VrmaMask::bit(group), 1u64 << index);
        }
    }

    /// A `1` in bit `N` masks group `N` out, so a layer lists the groups it
    /// must *not* drive. `ALL = 0` therefore animates everything.
    #[test]
    fn masks_name_exactly_the_groups_they_hide() {
        assert_eq!(VrmaMask::ALL, 0);
        assert_eq!(VrmaMask::UPPER_BODY, 0b11);
        assert_eq!(VrmaMask::LOWER_BODY, 0b1_1100);
        assert_eq!(VrmaMask::NO_HIPS, 0b1);

        // `UPPER_BODY` hides hips and legs only.
        assert_ne!(VrmaMask::UPPER_BODY & VrmaMask::bit(VrmMaskGroup::Hips), 0);
        assert_ne!(
            VrmaMask::UPPER_BODY & VrmaMask::bit(VrmMaskGroup::LowerBody),
            0
        );
        for group in [
            VrmMaskGroup::UpperBody,
            VrmMaskGroup::Head,
            VrmMaskGroup::Fingers,
        ] {
            assert_eq!(VrmaMask::UPPER_BODY & VrmaMask::bit(group), 0);
        }

        // `LOWER_BODY` leaves hips and legs alone.
        assert_eq!(VrmaMask::LOWER_BODY & VrmaMask::bit(VrmMaskGroup::Hips), 0);
        assert_eq!(
            VrmaMask::LOWER_BODY & VrmaMask::bit(VrmMaskGroup::LowerBody),
            0
        );
        assert_ne!(
            VrmaMask::LOWER_BODY & VrmaMask::bit(VrmMaskGroup::UpperBody),
            0
        );
        assert_ne!(VrmaMask::LOWER_BODY & VrmaMask::bit(VrmMaskGroup::Head), 0);
        assert_ne!(
            VrmaMask::LOWER_BODY & VrmaMask::bit(VrmMaskGroup::Fingers),
            0
        );

        // `NO_HIPS` hides the hips and nothing else.
        assert_ne!(VrmaMask::NO_HIPS & VrmaMask::bit(VrmMaskGroup::Hips), 0);
        for group in [
            VrmMaskGroup::LowerBody,
            VrmMaskGroup::UpperBody,
            VrmMaskGroup::Head,
            VrmMaskGroup::Fingers,
        ] {
            assert_eq!(VrmaMask::NO_HIPS & VrmaMask::bit(group), 0);
        }

        // `ALL` never masks anything out.
        for group in [
            VrmMaskGroup::Hips,
            VrmMaskGroup::LowerBody,
            VrmMaskGroup::UpperBody,
            VrmMaskGroup::Head,
            VrmMaskGroup::Fingers,
        ] {
            assert_eq!(VrmaMask::ALL & VrmaMask::bit(group), 0);
        }
    }

    /// Pins the bit convention against bevy itself: the evaluator skips a clip
    /// when `target_mask & node_mask != 0`, so `mask_group_for_bone` +
    /// [`VrmaMask`] must agree with `AnimationGraph::add_target_to_mask_group`.
    #[test]
    fn bevy_target_mask_agrees_with_the_constants() {
        let mut graph = AnimationGraph::new();
        let hips = AnimationTargetId::from_name(&Name::new("hips"));
        let leg = AnimationTargetId::from_name(&Name::new("leftUpperLeg"));
        graph.add_target_to_mask_group(hips, VrmMaskGroup::Hips as u32);
        graph.add_target_to_mask_group(leg, mask_group_for_bone("leftUpperLeg") as u32);

        let hips_mask = graph.mask_groups[&hips];
        let leg_mask = graph.mask_groups[&leg];
        assert_eq!(hips_mask, VrmaMask::bit(VrmMaskGroup::Hips));
        assert_eq!(leg_mask, VrmaMask::bit(VrmMaskGroup::LowerBody));

        // A non-zero intersection is what makes bevy skip the node, so a mask
        // listing a group must hide that group and nothing else.
        assert_ne!(VrmaMask::NO_HIPS & hips_mask, 0);
        assert_eq!(VrmaMask::NO_HIPS & leg_mask, 0);
        // `UPPER_BODY` hides both hips and legs.
        assert_ne!(VrmaMask::UPPER_BODY & hips_mask, 0);
        assert_ne!(VrmaMask::UPPER_BODY & leg_mask, 0);
        // `LOWER_BODY` animates both hips and legs.
        assert_eq!(VrmaMask::LOWER_BODY & hips_mask, 0);
        assert_eq!(VrmaMask::LOWER_BODY & leg_mask, 0);
        // `ALL` never hides anything.
        assert_eq!(VrmaMask::ALL & hips_mask, 0);
        assert_eq!(VrmaMask::ALL & leg_mask, 0);
    }
}
