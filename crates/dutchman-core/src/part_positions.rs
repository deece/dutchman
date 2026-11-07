//! The conversion-wide part-position choice and its source-rule counts.

use std::{fmt, str::FromStr};

/// Where assembly occurrences are placed before the existing joint solver runs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PartPositions {
    /// Prefer displayed poses, then admit current poses only after the placement guards.
    #[default]
    Smart,
    /// Prefer displayed poses and current poses without the Smart guards.
    LastShown,
    /// Ignore later poses and retain the native occurrence placements.
    FirstPlaced,
}

impl PartPositions {
    /// Stable command-line and JSON spelling.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Smart => "smart",
            Self::LastShown => "last-shown",
            Self::FirstPlaced => "first-placed",
        }
    }
}
impl fmt::Display for PartPositions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}
impl FromStr for PartPositions {
    type Err = String;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "smart" => Ok(Self::Smart),
            "last-shown" => Ok(Self::LastShown),
            "first-placed" => Ok(Self::FirstPlaced),
            _ => Err("part positions must be smart, last-shown, or first-placed".to_owned()),
        }
    }
}

/// Why a part's position rests on an assumption rather than on something the file confirms.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum PlacementAssumption {
    /// The decoder could not read its placement and put it at identity.
    Identity,
    /// The file holds a later pose that was not used; the stored placement stayed.
    LaterPoseUnused,
    /// The stored placement is all the file holds, and no joint holds the part.
    OnlyStoredPose,
}

impl PlacementAssumption {
    /// Its name in the reports.
    pub fn id(self) -> &'static str {
        match self {
            Self::Identity => "identity",
            Self::LaterPoseUnused => "later_pose_unused",
            Self::OnlyStoredPose => "only_stored_pose",
        }
    }
}

/// Disjoint placement rules and additional reasons for falling back to first placements.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PartPositionCounts {
    /// A displayed pose, directly bound or derived from agreeing displayed children.
    pub displayed: u64,
    /// Subset of displayed: holder worlds derived from agreeing displayed children.
    pub derived_holders: u64,
    /// Holders whose displayed children do not agree on a world pose.
    pub holder_conflicts: u64,
    /// A native current pose, accepted by the guards in Smart mode.
    pub current: u64,
    /// A stored placement, including occurrences with no later pose.
    pub first_placed: u64,
    /// No current pose was available.
    pub no_current_pose: u64,
    /// A carrier did not identify one occurrence in its document.
    pub ambiguous: u64,
    /// A pose was not rigid or its source frame could not be established.
    pub unsupported: u64,
    /// A touching joint did not close, or could not be checked.
    pub joints: u64,
    /// Geometry bounds could not be established or were exceeded.
    pub bounds: u64,
    /// Copies of an inserted design did not keep their relative placement.
    pub instances: u64,
}
impl PartPositionCounts {
    /// Number of occurrences considered, counting each once.
    pub fn total(self) -> u64 {
        self.displayed
            .saturating_add(self.current)
            .saturating_add(self.first_placed)
    }
    pub(crate) fn add(&mut self, other: Self) {
        self.displayed = self.displayed.saturating_add(other.displayed);
        self.derived_holders = self.derived_holders.saturating_add(other.derived_holders);
        self.holder_conflicts = self.holder_conflicts.saturating_add(other.holder_conflicts);
        self.current = self.current.saturating_add(other.current);
        self.first_placed = self.first_placed.saturating_add(other.first_placed);
        self.no_current_pose = self.no_current_pose.saturating_add(other.no_current_pose);
        self.ambiguous = self.ambiguous.saturating_add(other.ambiguous);
        self.unsupported = self.unsupported.saturating_add(other.unsupported);
        self.joints = self.joints.saturating_add(other.joints);
        self.bounds = self.bounds.saturating_add(other.bounds);
        self.instances = self.instances.saturating_add(other.instances);
    }
}
