//! Stage roles: what each group of lights is *for*, so one playbook can
//! play on any stage.
//!
//! A role is set per group. Every role has a fallback chain that ends in
//! "every fixture", so a look written for a role always lights something,
//! even on a stage with no roles at all.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum StageRole {
    /// The shape's edge: lines that draw the outline.
    Outline,
    /// Focal points: a small matched pair, or a single bright feature.
    Eyes,
    /// A chain of strips that an effect travels along as one line.
    Path,
    /// One half of a left/right pair.
    Left,
    /// The other half.
    Right,
    /// The middle of the stage.
    Centre,
    /// Panels and matrices that fill the space.
    Fill,
}

impl StageRole {
    pub const ALL: [StageRole; 7] = [StageRole::Outline, StageRole::Eyes, StageRole::Path, StageRole::Left, StageRole::Right, StageRole::Centre, StageRole::Fill];

    pub fn name(self) -> &'static str {
        match self {
            StageRole::Outline => "Outline",
            StageRole::Eyes => "Eyes",
            StageRole::Path => "Path",
            StageRole::Left => "Left",
            StageRole::Right => "Right",
            StageRole::Centre => "Centre",
            StageRole::Fill => "Fill",
        }
    }

    pub fn note(self) -> &'static str {
        match self {
            StageRole::Outline => "Lines that draw the edge of a shape.",
            StageRole::Eyes => "Focal points: a small matched pair.",
            StageRole::Path => "A chain of strips; effects run along it end to end.",
            StageRole::Left => "The left half of a pair; mirrors the right.",
            StageRole::Right => "The right half of a pair; mirrors the left.",
            StageRole::Centre => "The middle of the stage.",
            StageRole::Fill => "Panels and matrices that fill the space.",
        }
    }

    /// What stands in for this role when no group has it, best first. After
    /// the last, every fixture plays.
    pub fn fallbacks(self) -> &'static [StageRole] {
        match self {
            StageRole::Outline => &[StageRole::Path, StageRole::Fill],
            StageRole::Eyes => &[StageRole::Centre, StageRole::Fill],
            StageRole::Path => &[StageRole::Outline, StageRole::Fill],
            StageRole::Left => &[StageRole::Fill],
            StageRole::Right => &[StageRole::Fill],
            StageRole::Centre => &[StageRole::Fill],
            StageRole::Fill => &[],
        }
    }

    /// The other half of a pair.
    pub fn mirror(self) -> Option<StageRole> {
        match self {
            StageRole::Left => Some(StageRole::Right),
            StageRole::Right => Some(StageRole::Left),
            _ => None,
        }
    }
}

/// Which role each group has. Groups without an entry have no role.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RoleMap(pub Vec<(String, StageRole)>);

impl RoleMap {
    pub fn get(&self, group: &str) -> Option<StageRole> {
        self.0.iter().find(|(g, _)| g == group).map(|(_, r)| *r)
    }

    pub fn set(&mut self, group: &str, role: Option<StageRole>) {
        self.0.retain(|(g, _)| g != group);
        if let Some(r) = role {
            self.0.push((group.to_string(), r));
        }
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Groups that have exactly this role (among `groups`), in `groups` order.
    pub fn direct(&self, role: StageRole, groups: &[String]) -> Vec<String> {
        groups.iter().filter(|g| self.get(g) == Some(role)).cloned().collect()
    }

    /// The groups that play `role`: those that have it, else the first
    /// fallback anyone has. Empty means "every fixture".
    pub fn groups_for(&self, role: StageRole, groups: &[String]) -> Vec<String> {
        let d = self.direct(role, groups);
        if !d.is_empty() {
            return d;
        }
        for f in role.fallbacks() {
            let d = self.direct(*f, groups);
            if !d.is_empty() {
                return d;
            }
        }
        Vec::new()
    }

    /// True if `role` is played by a group that really has it.
    pub fn has(&self, role: StageRole, groups: &[String]) -> bool {
        !self.direct(role, groups).is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn g(names: &[&str]) -> Vec<String> {
        names.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn a_group_with_the_role_plays_it() {
        let mut r = RoleMap::default();
        r.set("Mouth", Some(StageRole::Outline));
        r.set("Eye L", Some(StageRole::Eyes));
        assert_eq!(r.groups_for(StageRole::Eyes, &g(&["Mouth", "Eye L"])), g(&["Eye L"]));
    }

    #[test]
    fn a_missing_role_falls_back_in_order() {
        let mut r = RoleMap::default();
        r.set("Panels", Some(StageRole::Fill));
        r.set("Core", Some(StageRole::Centre));
        // Eyes -> Centre before Fill.
        assert_eq!(r.groups_for(StageRole::Eyes, &g(&["Panels", "Core"])), g(&["Core"]));
        // Left -> Fill.
        assert_eq!(r.groups_for(StageRole::Left, &g(&["Panels", "Core"])), g(&["Panels"]));
    }

    #[test]
    fn no_roles_at_all_means_every_fixture() {
        let r = RoleMap::default();
        for role in StageRole::ALL {
            assert!(r.groups_for(role, &g(&["A", "B"])).is_empty(), "{role:?} should fall through to everything");
        }
    }

    #[test]
    fn a_role_on_a_group_that_no_longer_exists_is_ignored() {
        let mut r = RoleMap::default();
        r.set("Gone", Some(StageRole::Path));
        assert!(r.groups_for(StageRole::Path, &g(&["Here"])).is_empty());
    }

    #[test]
    fn setting_a_role_replaces_and_clearing_removes() {
        let mut r = RoleMap::default();
        r.set("A", Some(StageRole::Left));
        r.set("A", Some(StageRole::Right));
        assert_eq!(r.get("A"), Some(StageRole::Right));
        r.set("A", None);
        assert!(r.is_empty());
    }

    #[test]
    fn left_and_right_mirror_each_other() {
        assert_eq!(StageRole::Left.mirror(), Some(StageRole::Right));
        assert_eq!(StageRole::Right.mirror(), Some(StageRole::Left));
        assert_eq!(StageRole::Fill.mirror(), None);
    }

    #[test]
    fn roles_survive_a_save_and_old_shows_have_none() {
        let mut r = RoleMap::default();
        r.set("A", Some(StageRole::Eyes));
        let j = serde_json::to_string(&r).unwrap();
        assert_eq!(serde_json::from_str::<RoleMap>(&j).unwrap(), r);
        assert_eq!(serde_json::from_str::<RoleMap>("[]").unwrap(), RoleMap::default());
    }
}
