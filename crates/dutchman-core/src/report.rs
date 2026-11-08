use std::fmt;

use cadmpeg_ir::report::loss::LossNote;
use cadmpeg_ir::report::Severity as CadmpegSeverity;

/// How much a [`Loss`] matters to the person reading the converted design.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    /// Detail that has no effect on the geometry (a colour, a note).
    Info,
    /// Something carried across in a degraded form (history flattened to a body).
    Degraded,
    /// Something dropped entirely (a body or feature that is missing from the output).
    Dropped,
}

impl From<CadmpegSeverity> for Severity {
    /// How a cadmpeg finding reads to someone looking at the converted design.
    fn from(severity: CadmpegSeverity) -> Self {
        match severity {
            CadmpegSeverity::Info => Severity::Info,
            CadmpegSeverity::Warning => Severity::Degraded,
            CadmpegSeverity::Error | CadmpegSeverity::Blocking => Severity::Dropped,
        }
    }
}

/// Where in a conversion a loss or a failure arose, so a report of it reaches the code that
/// can fix it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Stage {
    /// The file itself, or how it was asked for: missing, too large, the wrong kind.
    Input,
    /// cadmpeg reading the file's bytes.
    Decode,
    /// cadmpeg turning what it read into its neutral model.
    Project,
    /// Dutchman mapping the neutral model onto SolidCraft's.
    Map,
    /// Writing the design: Dutchman's writer, and cadmpeg's STEP restatement within it.
    Write,
    /// SolidCraft opening or checking the written design.
    SolidCraft,
}

impl Stage {
    /// Its name in the reports.
    pub fn id(self) -> &'static str {
        match self {
            Stage::Input => "input",
            Stage::Decode => "decode",
            Stage::Project => "project",
            Stage::Map => "map",
            Stage::Write => "write",
            Stage::SolidCraft => "solidcraft",
        }
    }

    /// Every stage, in conversion order.
    pub const ALL: [Stage; 6] = [
        Stage::Input,
        Stage::Decode,
        Stage::Project,
        Stage::Map,
        Stage::Write,
        Stage::SolidCraft,
    ];
}

/// The cadmpeg loss-code words that say the source's bytes could not be read (a record
/// undecoded, unparsed, untyped or badly framed; a stream missing or unreadable), rather than
/// read and not carried into the neutral model (unprojected, unresolved, unbound, kept native,
/// capped).
const DECODE_WORDS: [&str; 11] = [
    "undecoded",
    "not-decoded",
    "unparsed",
    "untyped",
    "framing",
    "unreadable",
    "container-missing",
    "missing-geometry-stream",
    "not-transferred",
    "truncated",
    "malformed",
];

/// The stage of a cadmpeg loss reported while reading a file, from its code.
pub fn read_stage(code: &str) -> Stage {
    if DECODE_WORDS.iter().any(|word| code.contains(word)) {
        Stage::Decode
    } else {
        Stage::Project
    }
}

/// Stable, name-free categories for appearance assignments left out of the output.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AppearanceLoss {
    /// A redundant face colour (retained assignments do not produce a loss).
    RedundantFace,
    /// A wire body or saved free edge.
    Wire,
    /// Face geometry could not supply an output pick.
    FacePickMissed,
    /// The assignment's target is absent from the decoded model.
    TargetAbsent,
    /// The target belongs to a body with no writable geometry.
    UnwritableBody,
    /// Any other appearance mapping gap.
    Other,
}

impl AppearanceLoss {
    /// Its stable report code, never derived from design contents.
    pub fn code(self) -> &'static str {
        match self {
            Self::RedundantFace => "dutchman/appearance.redundant-face",
            Self::Wire => "dutchman/appearance.wire",
            Self::FacePickMissed => "dutchman/appearance.face-pick-missed",
            Self::TargetAbsent => "dutchman/appearance.target-absent",
            Self::UnwritableBody => "dutchman/appearance.unwritable-body",
            Self::Other => "dutchman/appearance.other",
        }
    }
}

/// One thing that did not survive the conversion intact.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Loss {
    /// How much it matters.
    pub severity: Severity,
    /// What was affected, in the source's terms (for example `feature "Extrusion3"`), or the
    /// loss code when cadmpeg reported it.
    pub subject: String,
    /// A stable cadmpeg or Dutchman loss code, when available.
    pub code: Option<String>,
    /// What happened to it.
    pub detail: String,
    /// Where in the conversion it arose.
    pub stage: Stage,
}

impl Loss {
    /// The project whose code the loss arose in, when it is not Dutchman's own: `cadmpeg` for
    /// what cadmpeg reported, `solidcraft` for what SolidCraft found.
    pub fn upstream(&self) -> Option<&'static str> {
        match self.stage {
            Stage::Decode | Stage::Project => Some("cadmpeg"),
            Stage::Write if self.code.is_some() => Some("cadmpeg"),
            Stage::SolidCraft => Some("solidcraft"),
            _ => None,
        }
    }
}

impl fmt::Display for Loss {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let label = match self.severity {
            Severity::Info => "Info",
            Severity::Degraded => "Degraded",
            Severity::Dropped => "Dropped",
        };
        write!(f, "{label}: {}: {}", self.subject, self.detail)
    }
}

/// The most names one report records: enough for any real design, bounded for a hostile one.
const MAX_NAMES: usize = 100_000;

/// Everything a conversion could not carry across. A conversion that returns a report
/// succeeded; the report says how faithfully.
#[derive(Clone, Debug, Default)]
pub struct Report {
    threads: Option<(usize, usize)>,
    part_positions: Option<(crate::PartPositions, crate::PartPositionCounts)>,
    placement_assumptions: Vec<(String, crate::PlacementAssumption)>,
    history_check_skipped: Option<(u64, u64)>,
    losses: Vec<Loss>,
    outcome: Option<Outcome>,
    parts: Vec<PartHistory>,
    completeness: Option<Completeness>,
    joints: std::collections::BTreeMap<String, u64>,
    names: std::collections::BTreeSet<String>,
}

/// How much of an assembly was placed: every placement its file and its placed sub-assemblies'
/// files record, each sub-assembly counted as often as it is placed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Completeness {
    /// Placements with geometry, including assembly holders of child placements.
    pub placed: u64,
    /// Placements of leaf parts with no bodies. These retain their component and joints.
    pub placed_empty: u64,
    /// The placements left out, by the file each places and why, most first.
    pub left_out: Vec<LeftOutGroup>,
}

/// Placements of one file left out for one reason.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LeftOutGroup {
    /// The file they place, by name, when they name one.
    pub file: Option<String>,
    /// Why they are left out.
    pub reason: String,
    /// The stage they were lost at.
    pub stage: Stage,
    /// How many there are.
    pub count: u64,
}

impl Completeness {
    /// Every placement recorded, placed or not.
    pub fn placements(&self) -> u64 {
        self.left_out.iter().fold(
            self.placed.saturating_add(self.placed_empty),
            |total, group| total.saturating_add(group.count),
        )
    }

    /// How many placements were left out.
    pub fn left_out_count(&self) -> u64 {
        self.left_out
            .iter()
            .fold(0u64, |total, group| total.saturating_add(group.count))
    }

    /// Whether every placement has geometry or holds child placements.
    pub fn is_complete(&self) -> bool {
        self.placed_empty == 0 && self.left_out.is_empty()
    }
}

/// What a writer made of the design: which form it wrote and what became of the history.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Outcome {
    /// The form written.
    pub mode: Mode,
    /// Whether a written history was checked against the original geometry.
    pub check: Check,
    /// What became of each source feature, in timeline order.
    pub features: Vec<FeatureOutcome>,
}

/// The form a design was written in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// An editable prefix and sketches, hidden beside the visible original geometry.
    PartialHistory,
    /// Editable feature history, with the original geometry kept beside it.
    History,
    /// Imported geometry without body-building history, possibly with reference sketches.
    Import,
}

/// What became of the history's check, with the reason where there is one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Check {
    /// Only part of the history is retained; the reason states its replay check.
    Partial(String),
    /// The replayed history matched the original geometry.
    Verified,
    /// A rebuilt result differs from its source; original geometry is retained.
    Differs(String),
    /// The history was written without being checked.
    Unverified(String),
    /// No body-building prefix was retained: it did not map or could not replay.
    Flattened(String),
    /// There was no history to write, or it was not asked for.
    NotApplicable(String),
}

/// What became of one assembly part's history.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PartHistory {
    /// The part's name.
    pub name: String,
    /// What became of its history: as for a whole design, with `NotApplicable` when it has
    /// no source features.
    pub check: Check,
    /// Its source features that mapped or need no feature, and all it has, suppressed ones
    /// not counted.
    pub mapped: usize,
    pub total: usize,
    /// Of those, the ones that became SolidCraft features, and the ones that need one: all but
    /// those that need none, such as a component insert or a joint.
    pub rebuilt: usize,
    pub needing: usize,
    /// Kept features that re-apply to a base body as the design ends, not counted as rebuilt.
    pub reapplied: usize,
}

/// What became of one source feature.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FeatureOutcome {
    /// Its name in the source, when it has one.
    pub name: Option<String>,
    /// Its kind in cadmpeg's model (`extrude`, `sketch`, …).
    pub kind: String,
    /// Whether it mapped.
    pub status: FeatureStatus,
    /// Why it did not map, or was left out.
    pub reason: Option<String>,
    /// Its ordinal in the design's timeline, as the source model numbers it.
    pub ordinal: Option<u64>,
    /// A short description of its form (`extrude, to a plane`), without names or sizes.
    pub variant: Option<String>,
    /// It mapped, but its profile region was not checked against the source.
    pub profile_unverified: bool,
    /// It mapped, but re-applies to a base body that already holds its result (the body as
    /// the design ends), so editing it may not change the shape: kept, not rebuilt.
    pub reapplied: bool,
    /// A base feature written with its body as the design ends, which later features re-apply
    /// to.
    pub final_body: bool,
}

/// Whether a source feature mapped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FeatureStatus {
    /// It became a feature of the target.
    Mapped,
    /// The target needs no feature for it (a principal datum plane).
    NotNeeded,
    /// It was suppressed in the source and left out.
    LeftOut,
    /// It stops body-building history, or follows the first blocking feature.
    Blocked,
}

impl Report {
    /// Memory needed and available, in bytes, when a writer skipped the history check.
    pub fn history_check_skipped(&self) -> Option<(u64, u64)> {
        self.history_check_skipped
    }

    /// Record a history check skipped for memory; the converted design is still usable.
    pub fn skip_history_check(&mut self, needed: u64, available: u64) {
        self.history_check_skipped = Some((needed, available));
    }

    /// Requested and bounded maximum Dutchman part workers, excluding child-process threads.
    pub fn threads(&self) -> Option<(usize, usize)> {
        self.threads
    }

    /// Records the bounded number of part workers used by this conversion.
    pub fn set_threads(&mut self, requested: usize, used: usize) {
        let used = self
            .threads
            .filter(|(previous, _)| *previous == requested)
            .map_or(used, |(_, previous)| previous.max(used));
        self.threads = Some((requested, used));
    }

    /// Records a loss in Dutchman's own mapping.
    pub fn push(
        &mut self,
        severity: Severity,
        subject: impl Into<String>,
        detail: impl Into<String>,
    ) {
        self.push_at(Stage::Map, severity, subject, detail);
    }

    /// Records a dropped appearance with a fixed cause and a human-readable explanation.
    pub fn push_appearance(&mut self, cause: AppearanceLoss, detail: impl Into<String>) {
        self.losses.push(Loss {
            severity: Severity::Dropped,
            subject: "appearance".to_owned(),
            code: Some(cause.code().to_owned()),
            detail: detail.into(),
            stage: Stage::Map,
        });
    }

    /// Records a loss in Dutchman's own mapping under a stable `code`.
    pub fn push_coded(
        &mut self,
        severity: Severity,
        subject: impl Into<String>,
        code: &str,
        detail: impl Into<String>,
    ) {
        self.losses.push(Loss {
            severity,
            subject: subject.into(),
            code: Some(code.to_owned()),
            detail: detail.into(),
            stage: Stage::Map,
        });
    }

    /// Records a loss that arose at `stage`.
    pub fn push_at(
        &mut self,
        stage: Stage,
        severity: Severity,
        subject: impl Into<String>,
        detail: impl Into<String>,
    ) {
        self.losses.push(Loss {
            severity,
            subject: subject.into(),
            code: None,
            detail: detail.into(),
            stage,
        });
    }

    /// Records cadmpeg's loss notes from reading a file, each under its loss code: at the
    /// decode stage when the code says the source could not be read, else at projection.
    pub fn extend_from_cadmpeg<'a>(&mut self, notes: impl IntoIterator<Item = &'a LossNote>) {
        for note in notes {
            let code = note.code.to_string();
            self.push_cadmpeg(read_stage(&code), note, code);
        }
    }

    /// Records cadmpeg's loss notes from restating the model as STEP, at the write stage.
    pub fn extend_from_cadmpeg_export<'a>(
        &mut self,
        notes: impl IntoIterator<Item = &'a LossNote>,
    ) {
        for note in notes {
            self.push_cadmpeg(Stage::Write, note, note.code.to_string());
        }
    }

    fn push_cadmpeg(&mut self, stage: Stage, note: &LossNote, code: String) {
        self.losses.push(Loss {
            severity: note.severity.into(),
            subject: code.clone(),
            code: Some(code.clone()),
            detail: if code == "f3d/material.not-transferred" {
                "Materials and appearances were not carried over.".to_owned()
            } else {
                note.message.clone()
            },
            stage,
        });
    }

    // A child report can have a different local limit. Keep the conversion's request
    // and the largest part-worker count observed in any child or earlier phase.
    fn merge_threads(&mut self, other: Option<(usize, usize)>) {
        if let Some((requested, used)) = other {
            let (requested, used) = self
                .threads
                .map_or((requested, used), |(parent, previous)| {
                    (parent, previous.max(used))
                });
            self.set_threads(requested, used);
        }
    }

    fn merge_history_check(&mut self, other: Option<(u64, u64)>) {
        if let Some((needed, available)) = other {
            if self
                .history_check_skipped
                .is_none_or(|(previous, _)| needed > previous)
            {
                self.skip_history_check(needed, available);
            }
        }
    }

    /// Accumulate source-occurrence placement rules for this conversion.
    pub fn record_part_positions(
        &mut self,
        mode: crate::PartPositions,
        counts: crate::PartPositionCounts,
    ) {
        match &mut self.part_positions {
            Some((_, total)) => total.add(counts),
            None => self.part_positions = Some((mode, counts)),
        }
    }
    /// Records a part placed by an assumption, by the names the design gives it.
    pub(crate) fn assume_placement(&mut self, part: String, why: crate::PlacementAssumption) {
        self.placement_assumptions.push((part, why));
    }
    /// The parts placed by an assumption, most doubtful reason first, then by name.
    pub fn placement_assumptions(&self) -> &[(String, crate::PlacementAssumption)] {
        &self.placement_assumptions
    }
    /// Selected policy and disjoint placement counts, with fallback reasons.
    pub fn part_positions(&self) -> Option<(crate::PartPositions, crate::PartPositionCounts)> {
        self.part_positions
    }
    /// One user-facing placement summary after the whole source has been read.
    pub(crate) fn note_part_positions(&mut self, mode: crate::PartPositions) {
        let (mode, counts) = *self
            .part_positions
            .get_or_insert((mode, Default::default()));
        self.push(Severity::Info, "part positions", format!(
            "{} parts placed as last shown in the original program ({} directly bound, {} holders derived from displayed children), {} at current poses, {} as first placed ({mode}); first-placement reasons: {} without a current pose, {} ambiguous, {} unsupported, {} joint checks, {} bounds checks, {} inserted-design checks; {} holder conflicts",
            counts.displayed, counts.displayed.saturating_sub(counts.derived_holders), counts.derived_holders, counts.current, counts.first_placed, counts.no_current_pose, counts.ambiguous, counts.unsupported, counts.joints, counts.bounds, counts.instances, counts.holder_conflicts
        ));
        self.placement_assumptions
            .sort_by(|a, b| (a.1, &a.0).cmp(&(b.1, &b.0)));
        self.note_placement_assumptions();
    }

    /// Names the parts placed by an assumption. A part the decoder put at the origin is a
    /// warning; a stored pose is usually right, so it stays a note.
    fn note_placement_assumptions(&mut self) {
        const SHOWN: usize = 20;
        const ISSUE: &str = "https://github.com/dutchman-cad/dutchman/issues/512";
        let names = |parts: &[&str]| {
            let shown = parts
                .iter()
                .take(SHOWN)
                .copied()
                .collect::<Vec<_>>()
                .join("; ");
            match parts.len().saturating_sub(SHOWN) {
                0 => shown,
                more => format!("{shown} and {more} more (all in the JSON report)"),
            }
        };
        let of = |wanted: &dyn Fn(crate::PlacementAssumption) -> bool| {
            self.placement_assumptions
                .iter()
                .filter(|(_, why)| wanted(*why))
                .map(|(part, _)| part.clone())
                .collect::<Vec<_>>()
        };
        let identity = of(&|why| why == crate::PlacementAssumption::Identity);
        let stored = of(&|why| why != crate::PlacementAssumption::Identity);
        if !identity.is_empty() {
            let parts: Vec<&str> = identity.iter().map(String::as_str).collect();
            self.push(Severity::Degraded, "part positions", format!(
                "{} parts sit at their design's origin because their placement could not be read, so they may not be where the source has them: {}. See {ISSUE}",
                parts.len(), names(&parts)
            ));
        }
        if !stored.is_empty() {
            let parts: Vec<&str> = stored.iter().map(String::as_str).collect();
            self.push(Severity::Info, "part positions", format!(
                "{} parts rest on a stored pose that no joint, saved view or usable later pose confirms. Most are right; check these if one looks misplaced: {}. See {ISSUE}",
                parts.len(), names(&parts)
            ));
        }
    }

    /// Records every loss of `other`, after this report's own.
    pub fn append(&mut self, other: Report) {
        self.merge_threads(other.threads);
        if let Some((mode, counts)) = other.part_positions {
            self.record_part_positions(mode, counts);
        }
        self.placement_assumptions
            .extend(other.placement_assumptions);
        self.merge_history_check(other.history_check_skipped);
        self.losses.extend(other.losses);
        self.names.extend(other.names);
    }

    /// Records every loss of `other` under `prefix` (`part.ipt: …`), after this report's own.
    pub fn append_prefixed(&mut self, other: Report, prefix: &str) {
        self.merge_threads(other.threads);
        if let Some((mode, counts)) = other.part_positions {
            self.record_part_positions(mode, counts);
        }
        self.placement_assumptions
            .extend(other.placement_assumptions);
        self.merge_history_check(other.history_check_skipped);
        self.losses
            .extend(other.losses.into_iter().map(|loss| Loss {
                subject: format!("{prefix}: {}", loss.subject),
                ..loss
            }));
        self.names.extend(other.names);
    }

    /// Records a name the conversion met: a file's, folder's or path's, or one the design
    /// gives a part, component, body, feature, sketch or parameter. They are never reported;
    /// they are what an anonymous report must take out of messages.
    pub fn saw(&mut self, name: &str) {
        let name = name.trim();
        if !name.is_empty() && self.names.len() < MAX_NAMES {
            self.names.insert(name.to_owned());
        }
    }

    /// Every name the conversion met (see [`Report::saw`]).
    pub fn names(&self) -> &std::collections::BTreeSet<String> {
        &self.names
    }

    /// Records what the writer made of the design.
    pub fn set_outcome(&mut self, outcome: Outcome) {
        self.outcome = Some(outcome);
    }

    /// What the writer made of the design, when it said.
    pub fn outcome(&self) -> Option<&Outcome> {
        self.outcome.as_ref()
    }

    /// Update the writer's recorded outcome after a final check, without copying its features.
    pub fn outcome_mut(&mut self) -> Option<&mut Outcome> {
        self.outcome.as_mut()
    }

    /// Records what became of one assembly part's history.
    pub fn push_part(&mut self, part: PartHistory) {
        self.parts.push(part);
    }

    /// What became of each assembly part's history, in the order they were written.
    pub fn parts(&self) -> &[PartHistory] {
        &self.parts
    }

    /// Records how much of the assembly was placed.
    pub fn set_completeness(&mut self, completeness: Completeness) {
        self.completeness = Some(completeness);
    }

    /// How much of the assembly was placed, for an assembly.
    pub fn completeness(&self) -> Option<&Completeness> {
        self.completeness.as_ref()
    }

    /// Records the joints written, by kind (`revolute`, `slider`, `rigid`).
    pub fn set_joints(&mut self, joints: std::collections::BTreeMap<String, u64>) {
        self.joints = joints;
    }

    /// The joints written, by kind; empty when none were.
    pub fn joints(&self) -> &std::collections::BTreeMap<String, u64> {
        &self.joints
    }

    /// The recorded losses, in the order they occurred.
    pub fn losses(&self) -> &[Loss] {
        &self.losses
    }

    /// Whether the conversion was lossless.
    pub fn is_lossless(&self) -> bool {
        self.losses.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cadmpeg_losses_are_staged_by_their_code() {
        for code in [
            "f3d/geometry.not-transferred",
            "f3d/geometry.brep-blob-undecoded",
            "f3d/history.record-framing-failed",
            "f3d/parameter.record-unparsed",
            "f3d/source.archive-entry-unreadable",
            "f3d/container.missing-geometry-stream",
        ] {
            assert_eq!(read_stage(code), Stage::Decode, "{code}");
        }
        for code in [
            "f3d/feature.definition-incomplete",
            "f3d/feature.scope-payload-uninterpreted",
            "f3d/sketch.curve-unprojected",
            "f3d/feature.profile-selection-native",
            "f3d/assembly.components-external",
            "f3d/parameter.expression-unbound",
        ] {
            assert_eq!(read_stage(code), Stage::Project, "{code}");
        }
    }

    #[test]
    fn a_loss_names_the_project_it_arose_in() {
        let mut report = Report::default();
        report.push(Severity::Degraded, "history", "a fillet is not mapped yet");
        report.push_at(Stage::SolidCraft, Severity::Info, "geometry", "not checked");
        report.push_at(Stage::Project, Severity::Info, "validation", "a finding");
        let upstream: Vec<_> = report.losses().iter().map(Loss::upstream).collect();
        assert_eq!(upstream, [None, Some("solidcraft"), Some("cadmpeg")]);
    }
}

#[cfg(test)]
mod worker_report_tests {
    use super::Report;

    #[test]
    fn fusion_material_notes_use_plain_language_and_keep_the_code() {
        use cadmpeg_ir::report::loss::{
            LossKind, LossNamespace, LossNote, LossTaxonomy, NamespacedLossKind,
        };
        let kind = LossKind::Namespaced(NamespacedLossKind::new(
            LossNamespace::new("f3d").unwrap(),
            "material.not-transferred",
            LossTaxonomy::MaterialNotTransferred,
        ));
        let note = LossNote::new(
            kind,
            "Materials/appearances (.protein assets, ACT/design assignments) were not transferred.",
        );
        let mut report = Report::default();
        report.extend_from_cadmpeg([&note]);
        assert_eq!(
            report.losses()[0].detail,
            "Materials and appearances were not carried over."
        );
        assert_eq!(
            report.losses()[0].code.as_deref(),
            Some("f3d/material.not-transferred")
        );
    }

    #[test]
    fn child_reports_keep_peak_workers_and_the_conversion_request() {
        let mut report = Report::default();
        report.set_threads(0, 4);
        let mut child = Report::default();
        child.set_threads(1, 1);
        report.append(child);
        assert_eq!(report.threads(), Some((0, 4)));
        let mut child = Report::default();
        child.set_threads(0, 6);
        report.append_prefixed(child, "part.f3d");
        report.set_threads(0, 1);
        assert_eq!(report.threads(), Some((0, 6)));
        let mut outer = Report::default();
        outer.append_prefixed(report, "assembly.f3z");
        assert_eq!(outer.threads(), Some((0, 6)));
    }
}
