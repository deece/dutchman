//! Assemblies, as SolidCraft components and occurrences.
//!
//! Each part becomes a component holding the part's feature history, when it maps and checks
//! out as a single part's would, followed by its geometry as a suppressed Import unless the
//! history already is that geometry; else its geometry as a live Import. A part that repeats
//! one written earlier in the assembly (see [`crate::repeats`]) places that component. Each
//! sub-assembly
//! becomes a component whose own parts are components beneath it. Each placement becomes an
//! occurrence of the placed component inside the component of the assembly that places it
//! (the root for the top assembly), at the placement's transform.

use std::collections::{BTreeMap, BTreeSet};

/// Most hidden-occurrence paths written: a hidden occurrence deep inside components placed
/// many times can be reached many ways.
const MAX_HIDDEN_PATHS: usize = 4096;

use dutchman_core::placement::{compose, rigid, IDENTITY};
use dutchman_core::{Assembly, Joint, Motion};
use dutchman_core::{
    Check, Error, FeatureOutcome, FeatureStatus, Mode, Model, Outcome, PartHistory, Report,
    Severity, Stage,
};
use serde_json::{json, Value};

use crate::container::Container;
use crate::document::{column_major, component_name, feature_name, import_feature, Live, ORIGINAL};
use crate::repeats::{self, PartPath};
use crate::{history, step, verify, SolidCraftWriter, FORMAT, HISTORY};

/// Public callers may build an assembly without the core's read report. Account for
/// reverse bindings that slicing could not attach to any of its component definitions.
fn report_appearance_bindings(model: &Model, report: &mut Report) -> bool {
    fn kept(model: &Model, ids: &mut BTreeSet<String>) {
        if let Some(assembly) = &model.assembly {
            for part in &assembly.parts {
                kept(part, ids);
            }
        } else {
            ids.extend(
                model
                    .ir
                    .model
                    .appearance_bindings
                    .iter()
                    .map(|b| b.id.to_string()),
            );
        }
    }
    fn parents(model: &Model, ids: &BTreeSet<String>, report: &mut Report) {
        let Some(assembly) = &model.assembly else {
            return;
        };
        for binding in &model.ir.model.appearance_bindings {
            if ids.contains(binding.id.as_str()) {
                continue;
            }
            let detail = model.unplaced_appearance_binding(binding);
            if !report
                .losses()
                .iter()
                .any(|loss| loss.subject == "appearance" && loss.detail.ends_with(&detail))
            {
                report.push_appearance(model.unplaced_appearance_cause(binding), detail);
            }
        }
        for part in &assembly.parts {
            parents(part, ids, report);
        }
    }
    let mut ids = BTreeSet::new();
    kept(model, &mut ids);
    parents(model, &ids, report);
    !ids.is_empty()
}

fn unwritten_appearances(part: &Model, report: &mut Report) {
    for binding in &part.ir.model.appearance_bindings {
        let cause = match part.unplaced_appearance_cause(binding) {
            dutchman_core::AppearanceLoss::Other
                if matches!(
                    binding.target,
                    cadmpeg_ir::appearance::AppearanceTarget::Body(_)
                        | cadmpeg_ir::appearance::AppearanceTarget::Face(_)
                ) =>
            {
                dutchman_core::AppearanceLoss::UnwritableBody
            }
            cause => cause,
        };
        report.push_appearance(
            cause,
            format!(
                "Appearance binding {} is left out: its component has no written geometry",
                binding.id
            ),
        );
    }
}

/// Writes `assembly`, the assembly `model` places, as a SolidCraft design container, and
/// records what became of its parts' histories.
pub(crate) fn write(
    writer: &SolidCraftWriter,
    model: &Model,
    assembly: &Assembly,
    report: &mut Report,
    progress: Option<&dutchman_core::Progress<'_>>,
) -> Result<Vec<u8>, Error> {
    // Use the source catalog before mirrored definitions are added or repeats shared.
    for part in dutchman_core::parts::catalog(model) {
        if !part.assembly && part.bodies == 0 {
            report.push(
                Severity::Degraded,
                format!("part {}", part.number),
                "placed as an empty component: it has no bodies; check the source part and convert again",
            );
        }
    }
    let admitted = crate::placements::prepare(assembly, writer.joints, report)?;
    let assembly = &admitted;
    report.set_completeness(assembly.completeness());
    if writer.joints {
        joint_notes(assembly, report);
    } else {
        report.push(
            Severity::Info,
            "joints",
            "assembly joints left out by choice; selected placements retained",
        );
    }
    let has_appearances = report_appearance_bindings(model, report);
    let repeats = repeats::find(assembly);
    if !repeats.is_empty() {
        report.push(
            Severity::Info,
            "assembly",
            format!(
                "{} repeated component(s) written once: each places the definition it repeats",
                repeats.len()
            ),
        );
    }
    let prepared = prepare_parts(writer, assembly, &repeats, report, progress).into_iter();
    let mut builder = Builder {
        progress,
        writer,
        prepared,
        container: Container::empty(),
        report,
        next_id: 1,
        components: Vec::new(),
        occurrences: Vec::new(),
        features: Vec::new(),
        params: Vec::new(),
        sheet_rules: Vec::new(),
        taken: BTreeSet::new(),
        looks: BTreeMap::new(),
        face_looks: Vec::new(),
        has_appearances,
        bodies: history::BodyNames::default(),
        names: Vec::new(),
        placed: BTreeMap::new(),
        repeats,
        written: BTreeMap::new(),
        path: Vec::new(),
        histories: Vec::new(),
        outcomes: Vec::new(),
        inner: BTreeMap::new(),
        placements: BTreeMap::new(),
        groups: Vec::new(),
        own_group: None,
        hidden: BTreeSet::new(),
        partial_hidden: BTreeSet::new(),
        reference_sketches: Vec::new(),
        slice_sketches: BTreeMap::new(),
        definition_components: BTreeMap::new(),
        written_sketches: BTreeSet::new(),
        unused_sketches: Vec::new(),
    };
    let top = builder.assembly(assembly, 0)?;
    builder.hold(&assembly.held, &top);
    let joints = if writer.joints {
        builder.joints(&assembly.joints, &top)
    } else {
        Vec::new()
    };
    if builder.features.is_empty() {
        return Err(Error::Encode(
            "none of the assembly's parts has geometry SolidCraft can import".to_owned(),
        ));
    }
    let start = builder.features.len();
    let mut hidden = builder.sketches(model);
    hidden.append(&mut builder.reference_sketches);
    hidden.append(&mut builder.unused_sketches);
    if builder.features.len() > start {
        let own = builder
            .own_group
            .and_then(|index| builder.groups.get_mut(index));
        match own {
            Some((_, range)) if range.end == start => range.end = builder.features.len(),
            _ => {
                let base = builder
                    .own_group
                    .and_then(|index| builder.groups.get(index))
                    .map(|(name, _)| name.clone())
                    .unwrap_or_else(|| component_name(&model.name));
                let name = builder.unique(base);
                builder.groups.push((name, start..builder.features.len()));
            }
        }
    }
    let groups = builder.timeline_groups();
    history::base::store(&mut builder.features, &mut builder.container)?;
    let mut doc = json!({
        "format": FORMAT,
        "name": model.name,
        "units": "mm",
        "params": builder.params,
        "next_id": builder.next_id,
        "components": builder.components,
        "occurrences": builder.occurrences,
        "features": builder.features,
    });
    let mut appearances = serde_json::Map::new();
    if let (false, Some(fields)) = (groups.is_empty(), doc.as_object_mut()) {
        fields.insert("timeline_groups".to_owned(), json!(groups));
    }
    if !builder.looks.is_empty() {
        appearances.insert("bodies".to_owned(), json!(builder.looks));
    }
    if !builder.face_looks.is_empty() {
        appearances.insert("faces".to_owned(), json!(builder.face_looks));
    }
    if let (false, Some(fields)) = (appearances.is_empty(), doc.as_object_mut()) {
        fields.insert("appearances".to_owned(), Value::Object(appearances));
    }
    if let (Some(first), Some(fields)) = (builder.sheet_rules.first(), doc.as_object_mut()) {
        fields.insert(
            "sheet".to_owned(),
            json!({"active": first.get("name"), "rules": builder.sheet_rules}),
        );
    }
    if let (false, Some(fields)) = (joints.is_empty(), doc.as_object_mut()) {
        fields.insert("assembly".to_owned(), json!({"joints": joints}));
    }
    let (hidden_occurrences, left_shown) = hidden_paths(
        &builder.occurrences,
        &builder.placements,
        &builder.hidden,
        MAX_HIDDEN_PATHS,
    );
    if left_shown != 0 {
        let kind = if builder.partial_hidden.is_empty() {
            "placement(s) the source hides"
        } else {
            "source-hidden or editable-history placement(s)"
        };
        builder.report.push(
            Severity::Degraded,
            "visibility",
            format!(
                "{left_shown} or more {kind} are left shown: hidden \
                 placements are written up to {MAX_HIDDEN_PATHS}"
            ),
        );
    }
    let partial_count = hidden_occurrences
        .iter()
        .filter(|path| {
            path.last()
                .is_some_and(|id| builder.partial_hidden.contains(id))
        })
        .count();
    let source_count = hidden_occurrences.len().saturating_sub(partial_count);
    if source_count != 0 {
        builder.report.push(
            Severity::Info,
            "visibility",
            format!("{source_count} component placement(s) the source hides are placed hidden"),
        );
    }
    if partial_count != 0 {
        builder.report.push(
            Severity::Info,
            "visibility",
            format!("{partial_count} editable-history component placement(s) are placed hidden"),
        );
    }
    let mut visibility = serde_json::Map::new();
    if !hidden.is_empty() {
        visibility.insert("hidden_sketches".to_owned(), json!(hidden));
    }
    if !hidden_occurrences.is_empty() {
        visibility.insert("hidden_occurrences".to_owned(), json!(hidden_occurrences));
    }
    if let (false, Some(fields)) = (visibility.is_empty(), doc.as_object_mut()) {
        fields.insert("visibility".to_owned(), Value::Object(visibility));
    }
    builder
        .report
        .set_outcome(outcome(&builder.histories, builder.outcomes));
    builder.container.finish(&doc)
}

/// Every path from the design to a hidden occurrence, as SolidCraft hides occurrences: the
/// occurrence ids from the root down. A hidden occurrence inside a component placed more than
/// once is hidden in each placement of it. At most `cap` paths are made; the second value
/// counts the paths left out past it (at least that many: making them stops at the cap).
fn hidden_paths(
    occurrences: &[Value],
    placements: &BTreeMap<u64, (u64, [[f64; 4]; 3])>,
    hidden: &BTreeSet<u64>,
    cap: usize,
) -> (Vec<Vec<u64>>, usize) {
    let parent_of: BTreeMap<u64, u64> = occurrences
        .iter()
        .filter_map(|o| Some((o.get("id")?.as_u64()?, o.get("parent")?.as_u64()?)))
        .collect();
    let mut paths = Vec::new();
    let mut left = 0usize;
    for &id in hidden {
        // Paths from the root to `id`, built upwards: each placement of the component the
        // occurrence sits in leads one level out.
        let mut partial = vec![vec![id]];
        for _ in 0..=occurrences.len() {
            let mut next = Vec::new();
            for path in partial {
                let Some(&parent) = path.first().and_then(|first| parent_of.get(first)) else {
                    continue;
                };
                if parent == 0 {
                    if paths.len() < cap {
                        paths.push(path);
                    } else {
                        left += 1;
                    }
                    continue;
                }
                for (&outer, &(component, _)) in placements {
                    if component == parent && !path.contains(&outer) {
                        let mut longer = vec![outer];
                        longer.extend(&path);
                        next.push(longer);
                    }
                }
            }
            if next.is_empty() {
                break;
            }
            // Past the cap, the paths still being made are left out too.
            if paths.len() + next.len() > cap.saturating_mul(2) {
                left += next.len();
                break;
            }
            partial = next;
        }
    }
    paths.sort();
    paths.dedup();
    (paths, left)
}

/// What writing an assembly would do with its parts' histories, without restating geometry,
/// writing anything or running SolidCraft.
/// Reports each joint the assembly leaves out or changes: SolidCraft gets the joints Dutchman
/// converted and no other.
fn joint_notes(assembly: &Assembly, report: &mut Report) {
    for note in &assembly.joint_notes {
        report.push(Severity::Dropped, "joints", note.text.clone());
    }
}

pub(crate) fn plan(writer: &SolidCraftWriter, assembly: &Assembly, report: &mut Report) -> Outcome {
    if writer.joints {
        joint_notes(assembly, report);
    } else {
        report.push(
            Severity::Info,
            "joints",
            "assembly joints left out by choice; selected placements retained",
        );
    }
    let mut checks = Vec::new();
    let mut features = Vec::new();
    let repeats = repeats::find(assembly);
    let mut parts = Vec::new();
    repeats::leaves(assembly, &repeats, &mut Vec::new(), &mut parts);
    plan_parts(writer, &parts, report, &mut checks, &mut features);
    outcome(&checks, features)
}

fn plan_parts(
    writer: &SolidCraftWriter,
    parts: &[&Model],
    report: &mut Report,
    checks: &mut Vec<Check>,
    features: &mut Vec<FeatureOutcome>,
) {
    for part in parts {
        if writer.import_only {
            continue;
        }
        let file = part.source_file.as_deref().unwrap_or(&part.name);
        let mut part_report = Report::default();
        let mut mapping = history::map_with_sketches(
            part,
            &mut part_report,
            writer.resolved_version(),
            writer.sketches,
        );
        writer.plan_blends(&mut mapping);
        features.extend(mapping.features.iter().cloned().map(|mut feature| {
            feature.name = Some(format!("{file}: {}", feature.name.unwrap_or_default()));
            feature
        }));
        let check = match &mapping.result {
            Ok(history) if history.stop.is_some() => {
                Check::Partial(crate::partial_summary(history))
            }
            Ok(_) => Check::Unverified("not checked: inspect writes nothing".to_owned()),
            Err(reason) => unwritten(&mapping.features, reason.clone()),
        };
        report.push_part(part_history(part, &mapping.features, check));
        match mapping.result {
            Ok(ref history) if history.stop.is_some() => {
                report.append_prefixed(part_report, file);
                report.push(
                    Severity::Degraded,
                    HISTORY,
                    format!("{file}: {}", crate::partial_summary(history)),
                );
                checks.push(Check::Partial(format!(
                    "{file}: {}; prefix unverified: inspect writes nothing",
                    crate::partial_summary(history)
                )));
            }
            Ok(_) => {
                report.append_prefixed(part_report, file);
                checks.push(Check::Unverified(
                    "not checked: inspect writes nothing".to_owned(),
                ));
            }
            Err(reason) if !mapping.features.is_empty() => {
                report.append_prefixed(part_report, file);
                report.push(
                    Severity::Info,
                    HISTORY,
                    format!("{file}: flattened: {reason}"),
                );
                checks.push(Check::Flattened(reason));
            }
            Err(_) => report.append_prefixed(part_report, file),
        }
    }
}

/// What became of `part`'s history, from its features and its check.
fn part_history(part: &Model, features: &[FeatureOutcome], check: Check) -> PartHistory {
    let counted = features
        .iter()
        .filter(|f| f.status != FeatureStatus::LeftOut);
    PartHistory {
        name: part.name.clone(),
        check,
        mapped: counted
            .clone()
            .filter(|f| matches!(f.status, FeatureStatus::Mapped | FeatureStatus::NotNeeded))
            .count(),
        rebuilt: counted
            .clone()
            .filter(|f| f.status == FeatureStatus::Mapped && !f.reapplied)
            .count(),
        reapplied: counted
            .clone()
            .filter(|f| f.status == FeatureStatus::Mapped && f.reapplied)
            .count(),
        needing: counted
            .clone()
            .filter(|f| f.status != FeatureStatus::NotNeeded)
            .count(),
        total: counted.count(),
    }
}

/// The check of a part whose history was not written for `reason`: flattened when it has
/// source features, else none applies.
fn unwritten(features: &[FeatureOutcome], reason: String) -> Check {
    if features.is_empty() {
        Check::NotApplicable(reason)
    } else {
        Check::Flattened(reason)
    }
}

/// What became of the parts' histories, as one outcome for the design.
fn outcome(histories: &[Check], features: Vec<FeatureOutcome>) -> Outcome {
    let written: Vec<&Check> = histories
        .iter()
        .filter(|check| matches!(check, Check::Verified | Check::Unverified(_)))
        .collect();
    let partial: Vec<&str> = histories
        .iter()
        .filter_map(|check| match check {
            Check::Partial(why) => Some(why.as_str()),
            Check::Flattened(why) if !written.is_empty() => Some(why.as_str()),
            _ => None,
        })
        .collect();
    let check = if !partial.is_empty() {
        Check::Partial(partial.join("; "))
    } else if written.is_empty() {
        Check::NotApplicable(
            "no part's history was written; the parts are placed as geometry".to_owned(),
        )
    } else if written.iter().all(|check| **check == Check::Verified) {
        Check::Verified
    } else {
        Check::Unverified("not every part's history was checked in SolidCraft".to_owned())
    };
    Outcome {
        mode: if !partial.is_empty() {
            Mode::PartialHistory
        } else if written.is_empty() {
            Mode::Import
        } else {
            Mode::History
        },
        check,
        features,
    }
}

/// What became of a part's history in its component.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Replay {
    /// No history is live: the part's geometry is its live Import.
    None,
    /// The history is live, and the original geometry follows it, suppressed.
    BesideOriginal,
    /// The history is live and is the original geometry itself.
    IsOriginal,
}

/// Owned work for one leaf part. IDs, names, and all shared state belong to Builder.
struct PreparedPart {
    step: Option<String>,
    report: Report,
    history: Option<PreparedHistory>,
    references: Option<(history::History, Report)>,
}

struct PreparedHistory {
    mapping: history::Mapping,
    report: Report,
    checked: Option<verify::Outcome>,
    partial_check: Option<Check>,
}

fn prepare_history(
    writer: &SolidCraftWriter,
    part: &Model,
    step: &str,
    progress: Option<&dutchman_core::Progress<'_>>,
) -> Result<PreparedHistory, Error> {
    let mut part_report = Report::default();
    let mut mapping = history::map_with_sketches(
        part,
        &mut part_report,
        writer.resolved_version(),
        writer.sketches,
    );
    if let Ok(history) = &mut mapping.result {
        if let Err(why) = history::base::restate(history, part) {
            mapping.result = Err(why);
        }
    }
    writer.prove_blends(part, &mut mapping, step, progress)?;
    let checked = if let Ok(history) = &mapping.result {
        if history.stop.is_some() {
            None
        } else {
            let written = writer.history_design(part, history, step, Live::History)?;
            Some(writer.check(part, history, step, &written, progress)?)
        }
    } else {
        None
    };
    let partial_check = if let Some(verify::Outcome::Differs(why)) = &checked {
        Some(writer.partial(
            part,
            &mut mapping,
            step,
            Some(format!("replay differs ({why})")),
            &mut part_report,
            progress,
        )?)
    } else if mapping.result.as_ref().is_ok_and(|h| h.stop.is_some()) {
        Some(writer.partial(part, &mut mapping, step, None, &mut part_report, progress)?)
    } else {
        None
    };

    crate::report_sketches(&mapping.features, &mut part_report);
    if let Ok(history) = &mapping.result {
        if history.stop.is_some() {
            let original = writer.history_design(part, history, step, Live::Original)?;
            writer.check_import(part, &original, &mut part_report, progress);
        }
    }
    Ok(PreparedHistory {
        mapping,
        report: part_report,
        checked,
        partial_check,
    })
}

fn prepare_part(
    writer: &SolidCraftWriter,
    part: &Model,
    progress: Option<&dutchman_core::Progress<'_>>,
) -> Result<PreparedPart, Error> {
    let _preparing = progress.map(|p| {
        p.step(
            dutchman_core::StepKind::Preparing,
            Some(&part.name),
            None,
            None,
        )
    });
    #[cfg(all(debug_assertions, not(target_family = "wasm")))]
    crate::slow_part(&part.name);
    let mut report = Report::default();
    let step = if part.ir.model.bodies.is_empty() {
        None
    } else {
        Some(step::restate(part, &mut report)?)
    };
    let history = match &step {
        Some(step) if !writer.import_only => Some(prepare_history(writer, part, step, progress)?),
        _ => None,
    };
    let references = if step.is_some() && writer.import_only && writer.sketches {
        let mut notes = Report::default();
        let (references, _) = history::sketches(part, &mut notes);
        Some((references, notes))
    } else {
        None
    };
    Ok(PreparedPart {
        step,
        report,
        history,
        references,
    })
}

fn prepare_parts(
    writer: &SolidCraftWriter,
    assembly: &Assembly,
    repeats: &BTreeMap<PartPath, PartPath>,
    report: &mut Report,
    progress: Option<&dutchman_core::Progress<'_>>,
) -> Vec<Option<PreparedPart>> {
    if writer.threads == 1 {
        report.set_threads(1, 1);
        return Vec::new();
    }
    let mut parts = Vec::new();
    repeats::leaves(assembly, repeats, &mut Vec::new(), &mut parts);
    // Include geometry density, not just bodies: preparing one detailed body can keep
    // STEP, history and a replay child alive together. Prepared results stay resident
    // until the coordinator writes them, so also budget the whole retained queue.
    let peaks: Vec<_> = parts
        .iter()
        .map(|part| {
            dutchman_core::parallel::part_peak_bytes(
                dutchman_core::parts::entity_count(part),
                part.ir.model.bodies.len(),
            )
        })
        .collect();
    let estimate = peaks.iter().copied().max().unwrap_or(256 << 20).max(
        if writer.history_check_skipped().is_none() {
            writer.memory_budget.unwrap_or(0)
        } else {
            0
        },
    );
    // The process floor belongs to a live worker, not to every queued result.
    let retained = parts.iter().fold(0u64, |total, part| {
        total.saturating_add(dutchman_core::parts::entity_count(part).saturating_mul(4 << 10))
    });
    // Small imported parts do not repay thread overhead. In a 256-cube benchmark,
    // 16 workers used about 14 MiB more than serial preparation without saving time.
    let light_imports = parts.iter().zip(&peaks).all(|(part, peak)| {
        *peak == (256 << 20)
            && part.ir.model.bodies.len() <= 1
            && (writer.import_only || part.ir.model.features.is_empty())
            && (!writer.sketches || part.ir.model.sketches.is_empty())
    });
    let available = writer.preparation_memory();
    let workers = if (writer.threads == 0 && light_imports)
        || available.is_some_and(|free| retained > free / 2)
    {
        1
    } else {
        dutchman_core::parallel::workers_with_memory(
            writer.threads,
            parts.len(),
            estimate,
            available,
        )
    };
    report.set_threads(writer.threads, workers);
    if workers == 1 {
        return Vec::new();
    }
    if !writer.import_only {
        // Initialise version detection on the coordinator; workers only read the cache.
        writer.resolved_version();
    }
    dutchman_core::parallel::prepare_ordered(&parts, workers, |part| {
        #[cfg(test)]
        if let Some(hook) = writer.prepare_hook {
            hook(part);
        }
        prepare_part(writer, part, progress)
    })
}

/// The design's components, occurrences and Import features as they are built.
struct Builder<'r> {
    progress: Option<&'r dutchman_core::Progress<'r>>,
    prepared: std::vec::IntoIter<Option<PreparedPart>>,
    writer: &'r SolidCraftWriter,
    container: Container,
    report: &'r mut Report,
    next_id: u64,
    components: Vec<Value>,
    occurrences: Vec<Value>,
    features: Vec<Value>,
    /// The design's parameters, from every part whose history is written.
    params: Vec<Value>,
    /// The sheet rules the parts' histories use, each once by name.
    sheet_rules: Vec<Value>,
    /// Parameter names given so far.
    taken: BTreeSet<String>,
    /// Looks of the bodies the parts' histories make, by body name.
    looks: BTreeMap<String, Value>,
    /// Looks of those bodies' faces, each by a point in its part's own frame, which SolidCraft
    /// applies to every occurrence of the part.
    face_looks: Vec<Value>,
    /// Name every imported body when looks are shared across the design.
    has_appearances: bool,
    /// Body names the parts' histories use so far, kept unique across parts.
    bodies: history::BodyNames,
    /// What became of each part's history.
    histories: Vec<Check>,
    /// Every part's features, for the outcome.
    outcomes: Vec<FeatureOutcome>,
    /// Component names given so far, kept unique.
    names: Vec<String>,
    /// Occurrences made so far under each component name, for naming the next.
    placed: BTreeMap<String, usize>,
    /// The parts that repeat a part before them, with where that one is.
    repeats: BTreeMap<PartPath, PartPath>,
    /// The component each part became (`None` for a part left out), by where the part is.
    written: BTreeMap<PartPath, Option<(u64, String)>>,
    /// Where the part being written is.
    path: PartPath,
    /// The occurrence each placement of a sub-assembly's became, by the sub-assembly's
    /// component (`None` for a part left out).
    inner: BTreeMap<u64, Vec<Option<u64>>>,
    /// Each occurrence's component and placement in its parent.
    placements: BTreeMap<u64, (u64, [[f64; 4]; 3])>,
    /// Each root design's contiguous feature range, including its nested components.
    groups: Vec<(String, std::ops::Range<usize>)>,
    own_group: Option<usize>,
    /// The occurrences to hide, including retained partial history.
    hidden: BTreeSet<u64>,
    /// Retained-history occurrences, distinguished from source-hidden placements in notes.
    partial_hidden: BTreeSet<u64>,
    reference_sketches: Vec<Value>,
    /// The component of the first part whose slice of the source holds each sketch (by its
    /// CADIR id): where a sketch no part wrote itself belongs.
    slice_sketches: BTreeMap<String, u64>,
    /// The component written for each component definition (CADIR product definition id), the
    /// outermost one where a sub-assembly and its own bodies share a definition.
    definition_components: BTreeMap<String, u64>,
    /// The CADIR ids of the sketches some part wrote under its own component.
    written_sketches: BTreeSet<String>,
    /// Sketches of written histories that no feature is built on or the source hides: they
    /// start hidden.
    unused_sketches: Vec<Value>,
}

impl Builder<'_> {
    fn id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);
        id
    }

    /// Adds `assembly`'s parts as components beneath component `parent` (0: the root), and its
    /// placements as occurrences inside it; returns the occurrence each placement became.
    fn assembly(&mut self, assembly: &Assembly, parent: u64) -> Result<Vec<Option<u64>>, Error> {
        let mut components = Vec::with_capacity(assembly.parts.len());
        let mut level = BTreeSet::new();
        for (index, part) in assembly.parts.iter().enumerate() {
            let start = self.features.len();
            self.path.push(index);
            let component = match self.repeats.get(&self.path) {
                Some(first) => {
                    let first = self.written.get(first).cloned().flatten();
                    first.map(|(id, _)| (id, self.unique(component_name(&part.name))))
                }
                None => {
                    let component = self.part(part, parent)?;
                    self.written.insert(self.path.clone(), component.clone());
                    component
                }
            };
            self.path.pop();
            if let (Some((id, _)), Some(Some(definition))) =
                (&component, assembly.definitions.get(index))
            {
                if level.insert(definition.clone()) {
                    self.definition_components.insert(definition.clone(), *id);
                }
            }
            if parent == 0 {
                if let Some((_, name)) = &component {
                    if assembly.own_part == Some(index) {
                        self.own_group = Some(self.groups.len());
                    }
                    self.groups.push((name.clone(), start..self.features.len()));
                }
            }
            components.push(component);
        }
        let mut occurrences = Vec::with_capacity(assembly.placements.len());
        for (index, placement) in assembly.placements.iter().enumerate() {
            let transform = if self.writer.joints {
                &placement.transform
            } else {
                assembly
                    .stored_placements
                    .get(index)
                    .unwrap_or(&placement.transform)
            };
            let transform = match rigid(transform) {
                Ok((rotation, false)) => rotation,
                result => {
                    let kind = result.err().unwrap_or("unresolved reflected placement");
                    self.report.push_at(Stage::Write, Severity::Dropped, "placements", format!(
                        "Occurrence index {index} ({}) is left out: {kind}; check its source placement and convert again",
                        placement.name.as_deref().unwrap_or("unnamed")
                    ));
                    occurrences.push(None);
                    continue;
                }
            };
            let Some(Some((component, name))) = components.get(placement.part).cloned() else {
                occurrences.push(None);
                continue;
            };
            let count = self.placed.entry(name.clone()).or_insert(0);
            *count += 1;
            let label = placement
                .name
                .clone()
                .unwrap_or_else(|| format!("{name}:{count}"));
            let id = self.id();
            self.occurrences.push(json!({
                "id": id,
                "name": label,
                "component": component,
                "parent": parent,
                "grounded": false,
                "transform": column_major(&transform),
            }));
            self.placements.insert(id, (component, transform));
            if placement.hidden {
                self.hidden.insert(id);
            }
            occurrences.push(Some(id));
        }
        Ok(occurrences)
    }

    fn timeline_groups(&mut self) -> Vec<Value> {
        if self.groups.len() < 2 {
            return Vec::new();
        }
        let mut groups = Vec::new();
        for (name, range) in std::mem::take(&mut self.groups) {
            let features: Vec<Value> = self
                .features
                .get(range)
                .unwrap_or_default()
                .iter()
                .filter_map(|feature| feature.get("id").cloned())
                .collect();
            if features.is_empty() {
                continue;
            }
            let id = self.id();
            self.report.push(
                Severity::Info,
                "timeline folders",
                format!("\"{name}\": {} feature(s) grouped", features.len()),
            );
            groups.push(json!({"id": id, "name": name, "features": features, "parent": null}));
        }
        groups
    }

    /// Joints in the frames the core places their parts by. Source offsets are already
    /// folded into those frames; write both snaps and the frame sense unchanged so the
    /// SolidCraft solver closes A * motion * facing = B at those placements.
    fn joints(&mut self, joints: &[Joint], top: &[Option<u64>]) -> Vec<Value> {
        let mut out = Vec::new();
        let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
        for joint in joints {
            let (Some((a, _)), Some((b, _))) =
                (self.resolve(&joint.a, top), self.resolve(&joint.b, top))
            else {
                self.report.push(
                    Severity::Dropped,
                    "joints",
                    "a joint is left out: a part it joins was left out".to_owned(),
                );
                continue;
            };
            // Each frame is in its own part, and the parts are placed where the joint puts
            // them (B's frame facing A's, moved by the joint's value), so solving moves nothing.
            let (kind, values, limits) = match &joint.motion {
                Motion::Rigid => ("rigid", json!([]), json!([])),
                Motion::Revolute { angle, limits } => (
                    "revolute",
                    json!([angle]),
                    json!([limits.map(|(min, max)| [min, max])]),
                ),
                Motion::Slider { distance } => ("slider", json!([distance]), json!([null])),
            };
            let (a_frame, b_frame) = (joint.frame_a, joint.frame_b);
            let id = self.id();
            *counts.entry(kind).or_insert(0) += 1;
            let mut row = json!({
                "id": id,
                "name": format!("{} {}", title(kind), counts.get(kind).copied().unwrap_or(1)),
                "kind": kind,
                "a": {"occurrence": a, "snap": snap(&a_frame)},
                "b": {"occurrence": b, "snap": snap(&b_frame)},
                "values": values,
                "limits": limits,
                "flip": joint.flip,
                "offset": 0.0,
                "angle": 0.0,
            });
            if let (true, Some(fields)) = (joint.suppressed, row.as_object_mut()) {
                fields.insert("suppressed".to_owned(), json!(true));
            }
            out.push(row);
        }
        self.report.set_joints(
            counts
                .iter()
                .map(|(kind, n)| ((*kind).to_owned(), u64::try_from(*n).unwrap_or(u64::MAX)))
                .collect(),
        );
        self.ground(&out);
        if !out.is_empty() {
            let kinds: Vec<String> = counts
                .iter()
                .map(|(kind, n)| format!("{n} {kind}"))
                .collect();
            self.report.push(
                Severity::Info,
                "joints",
                format!(
                    "{} joint(s) written as built ({}), holding their parts as placed",
                    out.len(),
                    kinds.join(", ")
                ),
            );
        }
        out
    }

    /// Grounds each held placement: held where it is in what holds it, as a design's root
    /// component is held by the document, or by the occurrence that inserts the design.
    fn hold(&mut self, held: &[Vec<usize>], top: &[Option<u64>]) {
        for path in held {
            let Some((id, _)) = self.resolve(path, top) else {
                continue;
            };
            if let Some(occurrence) = self
                .occurrences
                .iter_mut()
                .find(|occurrence| occurrence.get("id").and_then(Value::as_u64) == Some(id))
            {
                occurrence["grounded"] = json!(true);
            }
        }
    }

    /// Grounds one occurrence in each set of occurrences the joints tie together but none
    /// ties to the design: the second side of the set's first joint, the side a joint moves
    /// the other towards. A source that grounds nothing leaves SolidCraft's solver free to
    /// move the whole set; holding that side keeps every part where the source places it.
    fn ground(&mut self, joints: &[Value]) {
        let side = |row: &Value, key: &str| row.pointer(&format!("/{key}/occurrence"))?.as_u64();
        // Union-find over the occurrences the joints name, 0 being the design.
        let mut parent: BTreeMap<u64, u64> = BTreeMap::new();
        fn root(parent: &BTreeMap<u64, u64>, mut at: u64) -> u64 {
            let mut steps = 0;
            while let Some(&up) = parent.get(&at) {
                if up == at || steps > parent.len() {
                    break;
                }
                at = up;
                steps += 1;
            }
            at
        }
        for row in joints {
            let (Some(a), Some(b)) = (side(row, "a"), side(row, "b")) else {
                continue;
            };
            parent.entry(a).or_insert(a);
            parent.entry(b).or_insert(b);
            let (ra, rb) = (root(&parent, a), root(&parent, b));
            if ra != rb {
                // The design's own set keeps 0 as its root, so it is never grounded again.
                let (keep, join) = if rb == 0 { (rb, ra) } else { (ra, rb) };
                parent.insert(join, keep);
            }
        }
        let mut grounded = BTreeSet::new();
        for row in joints {
            let (Some(a), Some(b)) = (side(row, "a"), side(row, "b")) else {
                continue;
            };
            let set = root(&parent, a);
            if set == 0 || root(&parent, 0) == set || !grounded.insert(set) {
                continue;
            }
            let joint = row.get("name").and_then(Value::as_str).unwrap_or("a joint");
            if let Some(occurrence) = self
                .occurrences
                .iter_mut()
                .find(|occurrence| occurrence.get("id").and_then(Value::as_u64) == Some(b))
            {
                occurrence["grounded"] = json!(true);
                let name = occurrence
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or("an occurrence")
                    .to_owned();
                self.report.push(
                    Severity::Info,
                    "joints",
                    format!(
                        "\"{name}\" is grounded, inferred: the source grounds none of the parts \
                         its joints tie together, so the fixed side of \"{joint}\" holds them"
                    ),
                );
            }
        }
    }

    /// The occurrence a placement path leads to (0, the design itself, for an empty path) and
    /// its placement in the world.
    fn resolve(&self, path: &[usize], top: &[Option<u64>]) -> Option<(u64, [[f64; 4]; 3])> {
        let mut occurrence = 0;
        let mut world = IDENTITY;
        let mut level = top;
        for (depth, index) in path.iter().enumerate() {
            occurrence = (*level.get(*index)?)?;
            let (component, placement) = self.placements.get(&occurrence)?;
            world = compose(&world, placement);
            if depth + 1 < path.len() {
                level = self.inner.get(component)?;
            }
        }
        Some((occurrence, world))
    }

    /// The component a part or sub-assembly becomes, with its name; `None` when it has
    /// geometry SolidCraft cannot show and is left out. A part with no bodies at all, as a
    /// component empty in its source, becomes an empty component, so joints to it still hold.
    fn part(&mut self, part: &Model, parent: u64) -> Result<Option<(u64, String)>, Error> {
        let _preparing = self.progress.map(|p| {
            p.step(
                dutchman_core::StepKind::Preparing,
                Some(&part.name),
                None,
                None,
            )
        });
        let prepared = if part.assembly.is_none() {
            self.prepared.next().flatten()
        } else {
            None
        };
        #[cfg(all(debug_assertions, not(target_family = "wasm")))]
        if prepared.is_none() && part.assembly.is_none() {
            crate::slow_part(&part.name);
        }
        let mut prepared_history = None;
        let mut prepared_references = None;
        let step = match prepared {
            Some(prepared) => {
                self.report.append(prepared.report);
                prepared_history = prepared.history;
                prepared_references = prepared.references;
                prepared.step
            }
            None => match &part.assembly {
                Some(_) => None,
                None if part.ir.model.bodies.is_empty() => None,
                None => match step::restate(part, self.report) {
                    Ok(step) => Some(step),
                    Err(err) => {
                        let file = part.source_file.as_deref().unwrap_or(&part.name);
                        self.report
                            .push(Severity::Dropped, file, format!("left out: {err}"));
                        unwritten_appearances(part, self.report);
                        return Ok(None);
                    }
                },
            },
        };
        let component = self.id();
        let name = self.unique(component_name(&part.name));
        self.components
            .push(json!({"id": component, "name": name, "parent": parent}));
        match (&part.assembly, step) {
            (Some(sub), _) => {
                let occurrences = self.assembly(sub, component)?;
                self.inner.insert(component, occurrences);
            }
            (None, Some(step)) => {
                let replayed = self.history(
                    part,
                    &step,
                    component,
                    &name,
                    prepared_history,
                    prepared_references,
                )?;
                if replayed == Replay::IsOriginal {
                    return Ok(Some((component, name)));
                }
                let blob = self.container.add_step(step.as_bytes())?;
                let id = self.id();
                let (label, suppressed) = if replayed == Replay::BesideOriginal {
                    (ORIGINAL.to_owned(), true)
                } else {
                    (feature_name(part), false)
                };
                let mut imported =
                    import_feature(part, id, &label, Some(component), suppressed, &blob);
                if replayed == Replay::None {
                    let mut looks =
                        history::imported(part, &step, self.report, self.has_appearances)?;
                    looks.rename_bodies(&name, &mut self.bodies);
                    if let (Some(names), Some(fields)) = (
                        looks
                            .features
                            .first()
                            .and_then(|feature| feature.get("body_names")),
                        imported.as_object_mut(),
                    ) {
                        fields.insert("body_names".to_owned(), names.clone());
                    }
                    self.looks.append(&mut looks.looks);
                    self.face_looks.append(&mut looks.face_looks);
                }
                self.features.push(imported);
            }
            (None, None) => unwritten_appearances(part, self.report),
        }
        Ok(Some((component, name)))
    }

    /// Adds `part`'s feature history to component `component` when it maps and its replay
    /// checks out (or cannot be checked), as for a single part; says whether it did.
    fn history(
        &mut self,
        part: &Model,
        step: &str,
        component: u64,
        name: &str,
        prepared: Option<PreparedHistory>,
        references: Option<(history::History, Report)>,
    ) -> Result<Replay, Error> {
        let file = part.source_file.as_deref().unwrap_or(&part.name).to_owned();
        for sketch in &part.ir.model.sketches {
            self.slice_sketches
                .entry(sketch.id.to_string())
                .or_insert(component);
        }
        if self.writer.import_only {
            if self.writer.sketches {
                self.mark_written(part);
                let mut references = match references {
                    Some((references, report)) => {
                        self.report.append(report);
                        references
                    }
                    None => history::sketches(part, self.report).0,
                };
                references.relocate(self.next_id - 1, component, &prefix(name), &mut self.taken);
                self.next_id = references.next_id;
                self.reference_sketches.extend(
                    references
                        .features
                        .iter()
                        .filter_map(|f| f.get("id").cloned()),
                );
                self.features.append(&mut references.features);
                self.params.append(&mut references.params);
            }
            return Ok(Replay::None);
        }
        let PreparedHistory {
            mapping,
            report: mut part_report,
            checked,
            partial_check,
        } = match prepared {
            Some(prepared) => prepared,
            None => prepare_history(self.writer, part, step, self.progress)?,
        };
        self.outcomes
            .extend(mapping.features.iter().cloned().map(|mut feature| {
                feature.name = Some(format!("{file}: {}", feature.name.unwrap_or_default()));
                feature
            }));
        let mut history = match mapping.result {
            Ok(history) => history,
            Err(reason) => {
                if !mapping.features.is_empty() {
                    self.report.push(
                        Severity::Info,
                        HISTORY,
                        format!("{file}: flattened: {reason}"),
                    );
                    self.histories.push(Check::Flattened(reason.clone()));
                }
                let mut references = mapping.references;
                if !self.writer.sketches {
                    references.features.clear();
                }
                let kept = references.features.len();
                let left_out = part.ir.model.sketches.len().saturating_sub(kept);
                if kept > 0 {
                    self.mark_written(part);
                    references.relocate(
                        self.next_id - 1,
                        component,
                        &prefix(name),
                        &mut self.taken,
                    );
                    self.next_id = references.next_id;
                    self.reference_sketches.extend(
                        references
                            .features
                            .iter()
                            .filter_map(|f| f.get("id").cloned()),
                    );
                    self.features.append(&mut references.features);
                    self.params.append(&mut references.params);
                }
                if self.writer.sketches && (kept > 0 || left_out > 0) {
                    let noun = if kept == 1 { "sketch" } else { "sketches" };
                    let detail = if left_out > 0 {
                        format!("kept {kept} {noun} for reference; {left_out} left out")
                    } else {
                        format!("kept {kept} {noun} for reference")
                    };
                    part_report.push(Severity::Info, "sketches", detail);
                }
                self.report.append_prefixed(part_report, &file);
                self.report.push_part(part_history(
                    part,
                    &mapping.features,
                    unwritten(&mapping.features, reason),
                ));
                return Ok(Replay::None);
            }
        };
        let summary = self.writer.history_summary(&history);
        // A history that could not be checked, written for a writer that keeps such a history
        // hidden: the original stays the part's visible model, as for a single part.
        let mut unverified_hidden = false;
        let check = if let Some(check) = partial_check {
            if let Check::Partial(why) = &check {
                self.report
                    .push(Severity::Degraded, HISTORY, format!("{file}: {why}"));
            }
            check
        } else {
            match checked.unwrap_or(verify::Outcome::Unverified("not checked".to_owned())) {
                verify::Outcome::Matches => {
                    self.report.push(
                        Severity::Info,
                        HISTORY,
                        format!("{file}: {summary}; verified against the original in SolidCraft"),
                    );
                    Check::Verified
                }
                verify::Outcome::Unverified(why) if self.writer.keep_unverified_hidden() => {
                    self.report.push_at(
                        Stage::SolidCraft,
                        Severity::Degraded,
                        HISTORY,
                        format!(
                            "{file}: {summary}; unverified: {why}; kept hidden beside the original, which stays the visible model"
                        ),
                    );
                    unverified_hidden = true;
                    Check::Unverified(why)
                }
                verify::Outcome::Unverified(why) => {
                    self.report.push_at(
                        Stage::SolidCraft,
                        Severity::Info,
                        HISTORY,
                        format!("{file}: {summary}; unverified: {why}"),
                    );
                    Check::Unverified(why)
                }
                verify::Outcome::Differs(why) => {
                    self.report
                        .append_prefixed(history::helper_notes(&part_report), &file);
                    let reason = format!("replay differs ({why})");
                    self.report.push(
                        Severity::Info,
                        HISTORY,
                        format!("{file}: flattened: {reason}"),
                    );
                    self.report.push_part(part_history(
                        part,
                        &mapping.features,
                        Check::Flattened(reason.clone()),
                    ));
                    self.histories.push(Check::Flattened(reason));
                    return Ok(Replay::None);
                }
            }
        };
        let partial = history.stop.is_some();
        self.report.append_prefixed(part_report, &file);
        if partial && history::base::only_stored(&history) {
            // Nothing in it could be edited: the visible Import already is the part.
            self.report.push(
                Severity::Info,
                HISTORY,
                format!("{file}: the kept history is only stored bodies; no hidden copy written"),
            );
            self.report
                .push_part(part_history(part, &mapping.features, check.clone()));
            self.histories.push(check);
            return Ok(Replay::None);
        }
        let hidden = partial || unverified_hidden;
        let whole = !hidden && history::base::whole(&history, part);
        let component = if hidden {
            let child = self.id();
            let occurrence = self.id();
            let label = if partial {
                "Editable history (partial)"
            } else {
                "Editable history (unverified)"
            };
            self.components
                .push(crate::document::editable_history_component(
                    child,
                    label,
                    component,
                    self.writer.resolved_version(),
                ));
            self.occurrences.push(json!({"id": occurrence, "name": format!("{label}:1"), "component": child, "parent": component, "grounded": true, "transform": column_major(&IDENTITY)}));
            // The prefix belongs to the part definition; each placement inherits its hide.
            self.placements.insert(occurrence, (child, IDENTITY));
            self.hidden.insert(occurrence);
            self.partial_hidden.insert(occurrence);
            child
        } else {
            component
        };
        history.relocate(self.next_id - 1, component, &prefix(name), &mut self.taken);
        history.rename_bodies(name, &mut self.bodies);
        self.mark_written(part);
        self.unused_sketches.extend(history.hidden_sketches());
        self.next_id = history.next_id;
        self.features.append(&mut history.features);
        self.params.append(&mut history.params);
        for rule in history.sheet_rules.drain(..) {
            if !self
                .sheet_rules
                .iter()
                .any(|known| known.get("name") == rule.get("name"))
            {
                self.sheet_rules.push(rule);
            }
        }
        self.looks.append(&mut history.looks);
        self.face_looks.append(&mut history.face_looks);
        self.report
            .push_part(part_history(part, &mapping.features, check.clone()));
        self.histories.push(check);
        Ok(match (hidden, whole) {
            (true, _) => Replay::None,
            (false, true) => Replay::IsOriginal,
            (false, false) => Replay::BesideOriginal,
        })
    }

    /// Records that `part` wrote its slice's sketches under its own component.
    fn mark_written(&mut self, part: &Model) {
        self.written_sketches
            .extend(part.ir.model.sketches.iter().map(|s| s.id.to_string()));
    }

    /// Adds the sketches of the assembly's own file that no part wrote, those that map, hidden
    /// for reference as a flattened part's are, and returns their ids. A sketch in a part's
    /// slice of the source (the sketches its component owns) goes under that part's
    /// component, so a component's sketches sit with it; one that no part holds, such as a
    /// sketch of the design's own root component, goes to the design itself (SolidCraft's root
    /// component). The sketch is written once for the component, whatever number of times the
    /// component is placed.
    fn sketches(&mut self, model: &Model) -> Vec<Value> {
        if !self.writer.sketches {
            return Vec::new();
        }
        let owners = sketch_owners(model);
        let mut groups: BTreeMap<u64, BTreeSet<String>> = BTreeMap::new();
        let mut unplaced_owner = 0;
        for sketch in &model.ir.model.sketches {
            let id = sketch.id.to_string();
            if self.written_sketches.contains(&id) {
                continue;
            }
            let owner = owners.get(&id);
            let component = match owner.and_then(|d| self.definition_components.get(d)) {
                Some(component) => *component,
                None => {
                    let home = self.slice_sketches.get(&id).copied().unwrap_or(0);
                    unplaced_owner += usize::from(owner.is_some() && home == 0);
                    home
                }
            };
            groups.entry(component).or_default().insert(id);
        }
        let mut hidden = Vec::new();
        let (mut homed, mut homed_left_out) = (0, 0);
        let wire = history::SketchWire::new();
        for (component, ids) in groups {
            let (mut kept, left_out) =
                history::sketches_where(model, &wire, self.report, &|id| ids.contains(id));
            let count = kept.features.len();
            if component == 0 {
                if count > 0 || left_out > 0 {
                    let noun = if count == 1 { "sketch" } else { "sketches" };
                    let mut detail =
                        format!("kept {count} {noun} in the design itself, for reference");
                    if unplaced_owner > 0 {
                        detail.push_str(&format!(
                            "; {unplaced_owner} owned by a component the design does not write"
                        ));
                    }
                    if left_out > 0 {
                        detail.push_str(&format!("; {left_out} left out"));
                    }
                    self.report.push(Severity::Info, "sketches", detail);
                }
            } else {
                homed += count;
                homed_left_out += left_out;
            }
            if count == 0 {
                continue;
            }
            let offset = self.next_id.saturating_sub(1);
            kept.relocate(offset, component, &prefix(&model.name), &mut self.taken);
            self.next_id = kept.next_id;
            hidden.extend(
                kept.features
                    .iter()
                    .filter_map(|feature| feature.get("id").cloned()),
            );
            self.features.append(&mut kept.features);
            self.params.append(&mut kept.params);
        }
        if homed > 0 || homed_left_out > 0 {
            let noun = if homed == 1 { "sketch" } else { "sketches" };
            let detail = if homed_left_out > 0 {
                format!(
                    "kept {homed} {noun} under the component that holds them, for reference; \
                     {homed_left_out} left out"
                )
            } else {
                format!("kept {homed} {noun} under the component that holds them, for reference")
            };
            self.report.push(Severity::Info, "sketches", detail);
        }
        hidden
    }

    fn unique(&mut self, base: String) -> String {
        let mut name = base.clone();
        let mut n = 2;
        while self.names.contains(&name) {
            name = format!("{base} ({n})");
            n += 1;
        }
        self.names.push(name.clone());
        name
    }
}

/// A joint kind's name for the joints' own names.
fn title(kind: &str) -> &'static str {
    match kind {
        "revolute" => "Revolute",
        "slider" => "Slider",
        _ => "Rigid",
    }
}

/// A SolidCraft joint origin at a placement: its origin, and its z and x axes.
fn snap(frame: &[[f64; 4]; 3]) -> Value {
    let column = |c: usize| -> [f64; 3] {
        [0, 1, 2].map(|r| {
            frame
                .get(r)
                .and_then(|row| row.get(c))
                .copied()
                .unwrap_or(0.0)
        })
    };
    json!({"type": "frame", "origin": column(3), "z": column(2), "x": column(0)})
}

/// The component definition that owns each sketch of `model`, by sketch id: the one listing
/// it, else the one that owns the feature building it.
fn sketch_owners(model: &Model) -> BTreeMap<String, String> {
    use cadmpeg_ir::features::{FeatureDefinition, FeatureOperation};

    let mut owners = BTreeMap::new();
    if model.ir.check_decode_depth().is_err() {
        return owners;
    }
    // Ownership does not require expanding geometry or native payloads. Match the
    // same direct sketch operation as the previous /definition/sketch/sketch lookup.
    for feature in &model.ir.model.features {
        let FeatureDefinition::Operation(FeatureOperation::Sketch { sketch }) =
            feature.evaluation.definition()
        else {
            continue;
        };
        if let (Some(sketch), Some(owner)) = (sketch.id(), &feature.product_definition) {
            owners.insert(sketch.to_string(), owner.to_string());
        }
    }
    // An explicit definition list takes precedence over its feature's owner.
    for definition in &model.ir.model.product_definitions {
        for sketch in &definition.sketches {
            owners.insert(sketch.to_string(), definition.id.to_string());
        }
    }
    owners
}

#[cfg(test)]
#[test]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
fn sketch_owner_lists_override_feature_owners_and_unresolved_sketches_have_none() {
    let mut wire = serde_json::to_value(cadmpeg_ir::CadIr::empty()).unwrap();
    let owner_a = "test:model:component#a";
    let owner_b = "test:model:component#b";
    let listed = "test:model:sketch#listed";
    let feature_only = "test:model:sketch#feature";
    wire["model"]["product_definitions"] = json!([
        {"id": owner_a, "kind": "part", "bodies": [], "sketches": [listed]},
        {"id": owner_b, "kind": "part", "bodies": []}
    ]);
    wire["model"]["features"] = json!([
        {"id": "test:model:feature#listed", "ordinal": 0, "product_definition": owner_b,
            "suppressed": false, "outputs": [], "definition": {"definition": "sketch",
                "sketch": {"space": "planar", "sketch": listed}}},
        {"id": "test:model:feature#feature", "ordinal": 1, "product_definition": owner_b,
            "suppressed": false, "outputs": [], "definition": {"definition": "sketch",
                "sketch": {"space": "planar", "sketch": feature_only}}},
        {"id": "test:model:feature#unresolved", "ordinal": 2, "product_definition": owner_b,
            "suppressed": false, "outputs": [], "definition": {"definition": "sketch", "sketch": {"space": "unresolved"}}}
    ]);
    let model = Model {
        ir: cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap(),
        fidelity: None,
        name: "synthetic".to_owned(),
        source_file: None,
        assembly: None,
        source_step: None,
    };
    assert_eq!(
        sketch_owners(&model),
        BTreeMap::from([
            (listed.to_owned(), owner_a.to_owned()),
            (feature_only.to_owned(), owner_b.to_owned()),
        ])
    );
}

/// A component name as a parameter-name prefix: letters, digits and underscores.
fn prefix(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    if cleaned.starts_with(|c: char| c.is_ascii_alphabetic()) {
        cleaned
    } else {
        format!("c_{cleaned}")
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod hidden_tests {
    use super::*;

    #[test]
    fn hidden_paths_past_the_cap_are_counted_as_left_shown() {
        // Component 10 is placed by occurrences 1 and 2 at the root, and holds hidden 3.
        let occurrences = vec![
            json!({"id": 1, "parent": 0, "component": 10}),
            json!({"id": 2, "parent": 0, "component": 10}),
            json!({"id": 3, "parent": 10, "component": 20}),
        ];
        let identity = [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
        ];
        let placements: BTreeMap<u64, (u64, [[f64; 4]; 3])> = [
            (1, (10, identity)),
            (2, (10, identity)),
            (3, (20, identity)),
        ]
        .into();
        let hidden = BTreeSet::from([3]);
        let (paths, left) = hidden_paths(&occurrences, &placements, &hidden, 8);
        assert_eq!(paths, [vec![1, 3], vec![2, 3]]);
        assert_eq!(left, 0);
        let (paths, left) = hidden_paths(&occurrences, &placements, &hidden, 1);
        assert_eq!(paths.len(), 1);
        assert_eq!(left, 1);
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod prepare_tests {
    use super::*;
    use dutchman_core::{Format, Writer};

    #[test]
    fn a_worker_panic_retries_the_part_serially_without_changing_the_design() {
        let part = dutchman_core::read(
            Format::InventorPart,
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../dutchman-core/tests/fixtures/synthetic_cube.ipt"),
            &mut Report::default(),
        )
        .unwrap();
        let mut model = part.clone();
        let mut second = part.clone();
        second.name = "Forced panic".to_owned();
        model.assembly = Some(Assembly {
            parts: vec![part, second],
            placements: vec![
                dutchman_core::Placement {
                    hidden: false,
                    name: None,
                    part: 0,
                    transform: IDENTITY,
                },
                dutchman_core::Placement {
                    hidden: false,
                    name: None,
                    part: 1,
                    transform: IDENTITY,
                },
            ],
            own_part: None,
            joints: Vec::new(),
            left_out: Vec::new(),
            held: Vec::new(),
            joint_notes: Vec::new(),
            stored_placements: Vec::new(),
            identities: Vec::new(),
            definitions: Vec::new(),
        });
        let serial = SolidCraftWriter::new()
            .verification(crate::Verification::Off)
            .threads(1);
        let mut parallel = serial.clone().threads(4);
        // Exercise actual workers even when the surrounding test suite has little free memory.
        parallel.prepare_available = Some(8 << 30);
        parallel.prepare_hook = Some(|part| {
            if part.name == "Forced panic" {
                panic!("forced part preparation panic");
            }
        });
        let mut expected = Vec::new();
        let mut expected_report = Report::default();
        serial
            .write(&model, &mut expected, &mut expected_report)
            .unwrap();
        let mut actual = Vec::new();
        let mut actual_report = Report::default();
        let events = std::sync::Mutex::new(Vec::new());
        let callback = |event| events.lock().unwrap().push(event);
        let progress = dutchman_core::Progress::new(&callback);
        parallel
            .write_with_progress(&model, &mut actual, &mut actual_report, Some(&progress))
            .unwrap();
        let events = events.into_inner().unwrap();
        assert!(events
            .iter()
            .any(|event| event
                .running
                .as_ref()
                .is_some_and(|step| step.name.as_deref() == Some("Forced panic")
                    && step.kind == dutchman_core::StepKind::Preparing)));
        assert!(events
            .last()
            .unwrap()
            .running
            .as_ref()
            .unwrap()
            .name
            .is_none());
        assert_eq!(expected, actual);
        assert_eq!(actual_report.threads(), Some((4, 2)));
        let mut automatic = Vec::new();
        let mut automatic_report = Report::default();
        serial
            .clone()
            .threads(0)
            .write(&model, &mut automatic, &mut automatic_report)
            .unwrap();
        assert_eq!(automatic, expected);
        assert_eq!(automatic_report.threads(), Some((0, 1)));
        actual_report.set_threads(1, 1);
        assert_eq!(format!("{expected_report:?}"), format!("{actual_report:?}"));
    }
}
