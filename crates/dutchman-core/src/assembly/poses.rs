//! Select Fusion's decoded displayed and current poses before layouts and joints are built.
//! Native records retain both poses; no source bytes are interpreted here.

use super::*;

/// The one source-precedence seam: displayed, current, then first placement.
/// Smart admits the current branch only after joint, bounds, and instance checks.
pub(super) fn select(
    model: &Model,
    document: &mut Value,
    mode: crate::PartPositions,
    report: &mut Report,
) -> Result<BTreeSet<String>, String> {
    use crate::{PartPositionCounts, PartPositions};
    let references: BTreeMap<&str, &Value> = document
        .pointer("/native/f3d/xref_references")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|r| Some((r.get("id")?.as_str()?, r)))
        .collect();
    let records: BTreeMap<&str, &Value> = ["design_component_occurrences", "xref_references"]
        .into_iter()
        .flat_map(|name| {
            document
                .pointer(&format!("/native/f3d/{name}"))
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
        })
        .filter_map(|r| Some((r.get("id")?.as_str()?, r)))
        .collect();
    let rows = document
        .pointer("/model/occurrences")
        .and_then(Value::as_array)
        .map_or(&[][..], Vec::as_slice);
    if rows.len() > MAX_PLACEMENTS {
        return Err(format!("it places over {MAX_PLACEMENTS} components"));
    }
    let by_id: BTreeMap<&str, &Value> = rows
        .iter()
        .filter_map(|r| Some((r.get("id")?.as_str()?, r)))
        .collect();
    // CADIR retains the decoded placements used by the previous default. Native
    // current poses remain an independent source, so first-placed restores that
    // behavior exactly, including existing path and component-operation placement.
    let stored = Selected::new();
    let mut current = Selected::new();
    let mut display = BTreeSet::<String>::new();
    let mut uses = BTreeMap::<(Option<String>, &str), Vec<&str>>::new();
    let mut counts = PartPositionCounts::default();
    let mut displayed = if mode == PartPositions::FirstPlaced {
        BTreeMap::new()
    } else {
        displayed_poses(document)?
    };
    displayed.retain(|id, _| by_id.contains_key(id));
    let mut children = BTreeMap::<&str, Vec<&str>>::new();
    for (id, row) in &by_id {
        if let Some(parent) = row.pointer("/parent/occurrence").and_then(Value::as_str) {
            children.entry(parent).or_default().push(id);
        }
    }
    let mut derived = BTreeSet::<&str>::new();
    // Holders whose displayed children disagree keep their first placement.
    let mut conflicted = BTreeSet::<&str>::new();
    if !displayed.is_empty() {
        let (found, conflicts) = derive_holders(&by_id, &mut displayed)?;
        derived = found;
        counts.holder_conflicts = conflicts.len() as u64;
        counts.derived_holders = derived.len() as u64;
        conflicted = conflicts.clone();
        if !conflicts.is_empty() {
            report.push(Severity::Info, "part positions", format!(
                "{} holders keep their first placement because displayed children do not agree within 0.001 mm and 1e-6 rotation entries",
                conflicts.len()
            ));
        }
    }
    for (id, row) in &by_id {
        let native = row.get("native_ref").and_then(Value::as_str);
        let record = native.and_then(|native| records.get(native)).copied();
        let mut member = None;
        let mut up = row.pointer("/parent/occurrence").and_then(Value::as_str);
        for _ in 0..MAX_DEPTH {
            let Some(outer) = up.and_then(|id| by_id.get(id)) else {
                break;
            };
            if outer
                .get("native_ref")
                .and_then(Value::as_str)
                .is_some_and(|native| references.contains_key(native))
            {
                member = up.map(str::to_owned);
                break;
            }
            up = outer.pointer("/parent/occurrence").and_then(Value::as_str);
        }
        if let Some(native) = native
            .filter(|_| record.is_some_and(|r| r.get("current_pose").is_some_and(|p| !p.is_null())))
        {
            uses.entry((member.clone(), native)).or_default().push(id);
        }
        if mode == PartPositions::FirstPlaced {
            continue;
        }
        if let Some(pose) = displayed.get(id) {
            // Scene rows are already millimetres in the root design's world frame,
            // including bound occurrences inside inserted member documents.
            current.insert(*id, Pose::Document(None, *pose));
            display.insert((*id).to_owned());
            continue;
        }
        let pose = record
            .and_then(|r| r.get("current_pose"))
            .filter(|p| !p.is_null());
        let Some(pose) = pose else {
            counts.no_current_pose += 1;
            continue;
        };
        if !pose.is_object() {
            counts.unsupported += 1;
            continue;
        }
        match native_transform(pose.get("transform")) {
            Ok(pose) => {
                current.insert(*id, Pose::Document(member, pose));
            }
            Err(_) => counts.unsupported += 1,
        }
    }
    for ids in uses.values().filter(|ids| ids.len() != 1) {
        for id in ids {
            if !display.contains(*id) {
                current.remove(id);
                counts.ambiguous += 1;
            }
        }
    }
    if mode == PartPositions::FirstPlaced || current.is_empty() {
        counts.first_placed = by_id.len() as u64;
        report.record_part_positions(mode, counts);
        note_assumptions(document, mode, &BTreeSet::new(), &BTreeSet::new(), report);
        return Ok(BTreeSet::new());
    }
    let mut baseline = BTreeMap::new();
    for id in by_id.keys() {
        let _ = world(id, &by_id, &stored, &mut baseline, 0)?;
    }
    let candidates: BTreeSet<String> = current
        .keys()
        .filter(|id| !display.contains(**id))
        .map(|id| (*id).to_owned())
        .collect();
    // A conflicted holder stays where it was first placed even inside a holder that moves;
    // the subtree it holds then keeps that placement too, except parts with a displayed pose.
    let mut pinned = BTreeSet::<String>::new();
    if !display.is_empty() {
        for holder in &conflicted {
            if !display.contains(*holder) {
                if let Some(world) = baseline.get(*holder) {
                    current.insert(*holder, Pose::Document(None, *world));
                    pinned.insert((*holder).to_owned());
                }
            }
        }
    }
    // Displayed holders move whole subtrees, so Smart checks the geometry below them too.
    let guarded: BTreeSet<String> = display
        .iter()
        .filter(|id| children.contains_key(id.as_str()))
        .cloned()
        .collect();
    let accepted = if mode == PartPositions::Smart && !(candidates.is_empty() && guarded.is_empty())
    {
        let proposals = current
            .iter()
            .map(|(id, Pose::Document(member, pose))| ((*id).to_owned(), (member.clone(), *pose)))
            .collect();
        let (accepted, kept) = super::position_smart::admit(
            model,
            document,
            &baseline,
            &proposals,
            &candidates,
            &display,
            &pinned,
            &guarded,
            &references,
            &mut counts,
        )?;
        display = kept.into_iter().filter(|id| !pinned.contains(id)).collect();
        accepted
    } else {
        candidates
    };
    counts.displayed = display.len() as u64;
    counts.derived_holders = derived.iter().filter(|id| display.contains(**id)).count() as u64;
    counts.current = accepted.len() as u64;
    counts.first_placed = (by_id.len() as u64)
        .saturating_sub(counts.displayed)
        .saturating_sub(counts.current);
    let mut selected = stored;
    for (id, pose) in current {
        if accepted.contains(id) || display.contains(id) || pinned.contains(id) {
            selected.insert(id, pose);
        }
    }
    let mut worlds = BTreeMap::new();
    for id in by_id.keys() {
        let _ = world(id, &by_id, &selected, &mut worlds, 0)?;
    }
    // A local placement that matches the stored one keeps it exactly, so a shared
    // component whose instances keep the same relative placement stays shared.
    let mut settled = BTreeMap::new();
    let mut updates = BTreeMap::new();
    for id in by_id.keys() {
        settle(id, &by_id, &worlds, &mut settled, &mut updates, 0)?;
    }
    if let Some(rows) = document
        .pointer_mut("/model/occurrences")
        .and_then(Value::as_array_mut)
    {
        for row in rows {
            if let Some(position) = row
                .get("id")
                .and_then(Value::as_str)
                .and_then(|id| updates.get(id))
            {
                row["transform"] = serde_json::to_value(position).map_err(|e| e.to_string())?;
            }
        }
    }
    report.record_part_positions(mode, counts);
    note_assumptions(document, mode, &accepted, &display, report);
    // Pinned holders are placed where they were first placed, so joints check them too.
    display.extend(pinned);
    Ok(display)
}

/// Placements agree when rotation entries are within 1e-6 and origins within 0.001 mm.
pub(super) fn close(a: &Placement3x4, b: &Placement3x4) -> bool {
    let mut origin = 0.0;
    for (x, y) in a.iter().zip(b) {
        for (column, (p, q)) in x.iter().zip(y).enumerate() {
            if column == 3 {
                origin += (p - q).powi(2);
            } else if (p - q).abs() > 1e-6 {
                return false;
            }
        }
    }
    origin.sqrt() <= 1e-3
}

/// The local placement written for `id`, given the already settled world of its parent.
fn settle(
    id: &str,
    rows: &BTreeMap<&str, &Value>,
    placed: &BTreeMap<String, Placement3x4>,
    settled: &mut BTreeMap<String, Placement3x4>,
    updates: &mut BTreeMap<String, Placement3x4>,
    depth: usize,
) -> Result<Placement3x4, String> {
    if depth > MAX_DEPTH {
        return Err("current poses nest too deeply or name a component inside itself".to_owned());
    }
    if let Some(world) = settled.get(id) {
        return Ok(*world);
    }
    let row = rows.get(id).ok_or("a posed component names no parent")?;
    let outer = match row.pointer("/parent/occurrence").and_then(Value::as_str) {
        Some(parent) => settle(parent, rows, placed, settled, updates, depth + 1)?,
        None => IDENTITY,
    };
    let want = placed.get(id).ok_or("a placement is missing")?;
    let stored = transform(row)?;
    let local = compose(&inverse(&outer), want);
    let local = if close(&compose(&outer, &stored), want) {
        stored
    } else {
        local
    };
    let world = compose(&outer, &local);
    updates.insert(id.to_owned(), local);
    settled.insert(id.to_owned(), world);
    Ok(world)
}

/// Only explicit decoder bindings establish a displayed pose. Older pins omit this arena.
pub(super) fn displayed_poses(document: &Value) -> Result<BTreeMap<&str, Placement3x4>, String> {
    let rows = document
        .pointer("/native/f3d/display_poses")
        .and_then(Value::as_array)
        .map_or(&[][..], Vec::as_slice);
    if rows.len() > MAX_PLACEMENTS {
        return Err(format!("it has over {MAX_PLACEMENTS} displayed poses"));
    }
    let mut selected = BTreeMap::<&str, Option<Placement3x4>>::new();
    for row in rows {
        if row.pointer("/binding/state").and_then(Value::as_str) != Some("bound") {
            continue;
        }
        let Some(id) = row.pointer("/binding/occurrence").and_then(Value::as_str) else {
            continue;
        };
        let Some(matrix) = row.get("transform") else {
            continue;
        };
        let Ok([a, b, c, last]) = serde_json::from_value::<[[f64; 4]; 4]>(matrix.clone()) else {
            continue;
        };
        let pose = [a, b, c];
        if last != [0.0, 0.0, 0.0, 1.0] {
            continue;
        }
        // Scene matrices originate as float32. Remove admitted axis-length drift
        // before rigid inversion and joint closure checks, retaining the displayed pose.
        let Ok((pose, false)) = crate::placement::rigid(&pose) else {
            continue;
        };
        // Conflicting rows must not make record order choose a pose. Identical
        // duplicates still identify one pose; conflicts fall through to current.
        selected
            .entry(id)
            .and_modify(|old| {
                if *old != Some(pose) {
                    *old = None;
                }
            })
            .or_insert(Some(pose));
    }
    Ok(selected
        .into_iter()
        .filter_map(|(id, pose)| Some((id, pose?)))
        .collect())
}

/// Infer missing parent worlds only from agreeing displayed children, bottom-up.
/// An explicit binding always wins; conflicting children never select a parent.
fn derive_holders<'a>(
    rows: &BTreeMap<&'a str, &'a Value>,
    displayed: &mut BTreeMap<&'a str, Placement3x4>,
) -> Result<(BTreeSet<&'a str>, BTreeSet<&'a str>), String> {
    let mut children = BTreeMap::<&str, Vec<&str>>::new();
    for (id, row) in rows {
        if let Some(parent) = row.pointer("/parent/occurrence").and_then(Value::as_str) {
            children.entry(parent).or_default().push(id);
        }
    }
    let mut derived = BTreeSet::new();
    let mut conflicts = BTreeSet::new();
    let mut visited = BTreeSet::new();
    let mut open = BTreeSet::new();
    #[allow(clippy::too_many_arguments)]
    fn visit<'a>(
        id: &'a str,
        rows: &BTreeMap<&'a str, &'a Value>,
        children: &BTreeMap<&'a str, Vec<&'a str>>,
        displayed: &mut BTreeMap<&'a str, Placement3x4>,
        derived: &mut BTreeSet<&'a str>,
        conflicts: &mut BTreeSet<&'a str>,
        visited: &mut BTreeSet<&'a str>,
        open: &mut BTreeSet<&'a str>,
        depth: usize,
    ) -> Result<(), String> {
        if visited.contains(id) {
            return Ok(());
        }
        if depth > MAX_DEPTH || !open.insert(id) {
            return Err("displayed-pose parents form a cycle or nest too deeply".to_owned());
        }
        let mut first = None;
        let mut shown = Vec::new();
        let mut low = [[f64::INFINITY; 4]; 3];
        let mut high = [[f64::NEG_INFINITY; 4]; 3];
        for child in children.get(id).into_iter().flatten() {
            visit(
                child,
                rows,
                children,
                displayed,
                derived,
                conflicts,
                visited,
                open,
                depth + 1,
            )?;
            if let Some(pose) = displayed.get(child) {
                let local = transform(
                    rows.get(child)
                        .ok_or("a displayed child has no placement")?,
                )?;
                let (local, reflected) = crate::placement::rigid(&local).map_err(str::to_owned)?;
                let local = if reflected {
                    compose(&local, &crate::placement::MIRROR_X)
                } else {
                    local
                };
                let candidate = compose(pose, &inverse(&local));
                shown.push((*pose, local));
                first.get_or_insert(candidate);
                for ((low, high), row) in low.iter_mut().zip(&mut high).zip(candidate) {
                    for ((lo, hi), value) in low.iter_mut().zip(high).zip(row) {
                        *lo = lo.min(value);
                        *hi = hi.max(value);
                    }
                }
            }
        }
        if !displayed.contains_key(id) {
            if let Some(first) = first {
                let mut translation_span = 0.0;
                let mut agrees = true;
                for (low, high) in low.into_iter().zip(high) {
                    for (column, (lo, hi)) in low.into_iter().zip(high).enumerate() {
                        if column == 3 {
                            translation_span += (hi - lo).powi(2);
                        } else {
                            agrees &= hi - lo <= 1e-6;
                        }
                    }
                }
                // The derived world must reproduce every displayed child.
                agrees &= shown
                    .iter()
                    .all(|(pose, local)| close(&compose(&first, local), pose));
                if agrees && translation_span.sqrt() <= 1e-3 {
                    displayed.insert(id, first);
                    derived.insert(id);
                } else {
                    conflicts.insert(id);
                }
            }
        }
        open.remove(id);
        visited.insert(id);
        Ok(())
    }
    for id in rows.keys() {
        visit(
            id,
            rows,
            &children,
            displayed,
            &mut derived,
            &mut conflicts,
            &mut visited,
            &mut open,
            0,
        )?;
    }
    Ok((derived, conflicts))
}

enum Pose {
    Document(Option<String>, Placement3x4),
}

type Selected<'a> = BTreeMap<&'a str, Pose>;

fn world(
    id: &str,
    rows: &BTreeMap<&str, &Value>,
    selected: &Selected<'_>,
    cache: &mut BTreeMap<String, Placement3x4>,
    depth: usize,
) -> Result<Placement3x4, String> {
    if depth > MAX_DEPTH {
        return Err("current poses nest too deeply or name a component inside itself".to_owned());
    }
    if let Some(placement) = cache.get(id) {
        return Ok(*placement);
    }
    let row = rows.get(id).ok_or("a posed component names no parent")?;
    let placement = if let Some(Pose::Document(member, pose)) = selected.get(id) {
        let insertion = match member {
            Some(member) => world(member, rows, selected, cache, depth + 1)?,
            None => IDENTITY,
        };
        compose(&insertion, pose)
    } else {
        let parent = match row.pointer("/parent/occurrence").and_then(Value::as_str) {
            Some(parent) => world(parent, rows, selected, cache, depth + 1)?,
            None => IDENTITY,
        };
        let local = transform(row)?;
        compose(&parent, &local)
    };
    crate::placement::rigid(&placement).map_err(str::to_owned)?;
    cache.insert(id.to_owned(), placement);
    Ok(placement)
}

/// Native pose rows are centimetres; absent transform is the identity form.
fn native_transform(value: Option<&Value>) -> Result<Placement3x4, String> {
    let Some(value) = value.filter(|value| !value.is_null()) else {
        return Ok(IDENTITY);
    };
    let rows = value.as_array().ok_or("current pose has no matrix rows")?;
    if !matches!(rows.len(), 3 | 4) {
        return Err("current pose needs three or four rows".to_owned());
    }
    let mut pose =
        transform(&serde_json::json!({"transform": rows.iter().take(3).collect::<Vec<_>>()}))?;
    if let Some(last) = rows.get(3) {
        if !last.as_array().is_some_and(|values| {
            values.len() == 4
                && values
                    .iter()
                    .zip([0.0, 0.0, 0.0, 1.0])
                    .all(|(value, expected)| value.as_f64() == Some(expected))
        }) {
            return Err("current pose is not affine".to_owned());
        }
    }
    for row in &mut pose {
        row[3] *= 10.0;
    }
    crate::placement::rigid(&pose).map_err(str::to_owned)?;
    Ok(pose)
}

/// Names the parts whose placement nothing in the file confirms (#512): a part the decoder put
/// at identity, a part whose later pose was not used, and a part with only its stored pose that
/// no joint holds. Holders are left out: the parts inside them say the same thing by name.
fn note_assumptions(
    document: &Value,
    mode: crate::PartPositions,
    accepted: &BTreeSet<String>,
    display: &BTreeSet<String>,
    report: &mut Report,
) {
    use crate::PlacementAssumption;
    let Some(rows) = document
        .pointer("/model/occurrences")
        .and_then(Value::as_array)
    else {
        return;
    };
    let by_id: BTreeMap<&str, &Value> = rows
        .iter()
        .filter_map(|r| Some((r.get("id")?.as_str()?, r)))
        .collect();
    let mut holders = BTreeSet::<&str>::new();
    for row in rows {
        if let Some(parent) = row.pointer("/parent/occurrence").and_then(Value::as_str) {
            holders.insert(parent);
        }
    }
    let mut records = BTreeMap::<&str, (&Value, bool)>::new();
    for (name, external) in [
        ("design_component_occurrences", false),
        ("xref_references", true),
    ] {
        for row in document
            .pointer(&format!("/native/f3d/{name}"))
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if let Some(id) = row.get("id").and_then(Value::as_str) {
                records.insert(id, (row, external));
            }
        }
    }
    let mut jointed = BTreeSet::<&str>::new();
    for joint in document
        .pointer("/model/assembly_joints")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|joint| joint.get("suppressed").and_then(Value::as_bool) != Some(true))
    {
        for connector in joint
            .pointer("/operands/connectors")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if let Some(id) = connector
                .pointer("/operand/container/occurrence")
                .and_then(Value::as_str)
            {
                jointed.insert(id);
            }
        }
    }
    let held = |id: &str| {
        let mut at = Some(id);
        for _ in 0..=MAX_DEPTH {
            let Some(id) = at else { break };
            if jointed.contains(id) {
                return true;
            }
            at = by_id
                .get(id)
                .and_then(|row| row.pointer("/parent/occurrence"))
                .and_then(Value::as_str);
        }
        false
    };
    let definitions: BTreeMap<&str, &Value> = document
        .pointer("/model/product_definitions")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|row| Some((row.get("id")?.as_str()?, row)))
        .collect();
    fn label<'v>(
        document: &'v Value,
        definitions: &BTreeMap<&str, &'v Value>,
        row: &'v Value,
    ) -> &'v str {
        row.get("name")
            .and_then(Value::as_str)
            .or_else(|| inserted_design_name(document, row))
            .or_else(|| {
                row.pointer("/prototype/definition")
                    .and_then(Value::as_str)
                    .and_then(|id| definitions.get(id))
                    .and_then(|definition| name_of(definition))
            })
            .or_else(|| {
                row.pointer("/prototype/document/path")
                    .and_then(Value::as_str)
                    .map(file_name)
            })
            .unwrap_or("unnamed part")
    }
    let present = |row: &Value, key: &str| row.get(key).is_some_and(|value| !value.is_null());
    for (id, row) in &by_id {
        if holders.contains(id) || accepted.contains(*id) || display.contains(*id) {
            continue;
        }
        let record = row
            .get("native_ref")
            .and_then(Value::as_str)
            .and_then(|native| records.get(native));
        let later = record.is_some_and(|(record, _)| present(record, "current_pose"));
        let assumption = if record.is_some_and(|(record, external)| {
            *external
                && !later
                && !present(record, "transform")
                && !present(record, "placement_source")
        }) {
            PlacementAssumption::Identity
        } else if mode == crate::PartPositions::FirstPlaced {
            continue;
        } else if later {
            PlacementAssumption::LaterPoseUnused
        } else if held(id) {
            continue;
        } else {
            PlacementAssumption::OnlyStoredPose
        };
        // Outermost first, so copies of one part can be told apart by what holds them.
        let mut path = vec![label(document, &definitions, row)];
        let mut at = row.pointer("/parent/occurrence").and_then(Value::as_str);
        for _ in 0..MAX_DEPTH {
            let Some(outer) = at.and_then(|id| by_id.get(id)) else {
                break;
            };
            let name = label(document, &definitions, outer);
            if name != "unnamed part" {
                path.insert(0, name);
            }
            at = outer.pointer("/parent/occurrence").and_then(Value::as_str);
        }
        for name in &path {
            report.saw(name);
        }
        report.assume_placement(path.join(" / "), assumption);
    }
}

#[cfg(test)]
#[allow(clippy::indexing_slicing)]
mod tests {
    use super::*;
    use serde_json::json;

    fn empty_model() -> Model {
        Model {
            ir: cadmpeg_ir::CadIr::empty(),
            fidelity: None,
            name: String::new(),
            source_file: None,
            assembly: None,
            source_step: None,
        }
    }

    fn at(x: f64) -> Placement3x4 {
        let mut pose = IDENTITY;
        pose[0][3] = x;
        pose
    }

    fn assumed(document: &mut Value, mode: crate::PartPositions) -> Vec<(String, &'static str)> {
        let mut report = Report::default();
        select(&empty_model(), document, mode, &mut report).unwrap();
        report.note_part_positions(mode);
        report
            .placement_assumptions()
            .iter()
            .map(|(part, why)| (part.clone(), why.id()))
            .collect()
    }

    #[test]
    fn a_reference_with_no_placement_is_named_as_identity_in_every_mode() {
        for mode in [
            crate::PartPositions::Smart,
            crate::PartPositions::FirstPlaced,
        ] {
            let mut document = json!({"model":{"occurrences":[
                {"id":"a","native_ref":"r","transform":at(0.0),"parent":{"kind":"root"},
                 "prototype":{"scope":"external","document":{"path":"dir/Pin.f3d"}}}
            ]},"native":{"f3d":{"xref_references":[{"id":"r"}]}}});
            assert_eq!(
                assumed(&mut document, mode),
                [("Pin.f3d".to_owned(), "identity")]
            );
        }
    }

    #[test]
    fn a_part_with_only_its_stored_pose_is_named_with_what_holds_it_unless_a_joint_holds_it() {
        let rows = json!([
            {"id":"holder","native_ref":"h","transform":at(0.0),"parent":{"kind":"root"},
             "prototype":{"definition":"d1"}},
            {"id":"loose","native_ref":"l","transform":at(5.0),
             "parent":{"kind":"occurrence","occurrence":"holder"},"prototype":{"definition":"d2"}},
            {"id":"joined","native_ref":"j","transform":at(9.0),"parent":{"kind":"root"},
             "prototype":{"definition":"d2"}}
        ]);
        let mut document = json!({"model":{"occurrences":rows,
            "product_definitions":[{"id":"d1","label":"Motor"},{"id":"d2","label":"Pulley"}],
            "assembly_joints":[{"operands":{"connectors":[
                {"operand":{"container":{"occurrence":"joined"}}}]}}]},
            "native":{"f3d":{}}});
        assert_eq!(
            assumed(&mut document, crate::PartPositions::Smart),
            [("Motor / Pulley".to_owned(), "only_stored_pose")]
        );
        assert!(assumed(&mut document, crate::PartPositions::FirstPlaced).is_empty());
    }

    #[test]
    fn a_later_pose_that_was_not_used_is_named_and_a_used_one_is_not() {
        let pose = |x: f64| json!({"transform": at(x)});
        let mut document = json!({"model":{"occurrences":[
            occurrence("near", None, "n", 0.0),
            occurrence("far", None, "f", 1.0)
        ]},"native":{"f3d":{"design_component_occurrences":[
            {"id":"n","current_pose":pose(0.5)},
            {"id":"f","current_pose":"not a pose"}
        ]}}});
        let got = assumed(&mut document, crate::PartPositions::LastShown);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].1, "later_pose_unused");
    }

    #[test]
    fn identity_parts_warn_by_name_and_stored_poses_stay_a_note() {
        let mut report = Report::default();
        report.assume_placement("Pin".to_owned(), crate::PlacementAssumption::Identity);
        for n in 0..25 {
            report.assume_placement(
                format!("Part {n:02}"),
                crate::PlacementAssumption::OnlyStoredPose,
            );
        }
        report.note_part_positions(crate::PartPositions::Smart);
        let lines: Vec<_> = report
            .losses()
            .iter()
            .filter(|l| l.subject == "part positions")
            .collect();
        let warning = lines
            .iter()
            .find(|l| l.severity == Severity::Degraded)
            .unwrap();
        assert!(warning.detail.contains("Pin"), "{}", warning.detail);
        assert!(warning.detail.contains("issues/512"));
        let note = lines
            .iter()
            .find(|l| l.severity == Severity::Info && l.detail.contains("Part 19"))
            .unwrap();
        assert!(note.detail.contains("and 5 more"), "{}", note.detail);
        assert!(!note.detail.contains("Part 20"));
    }

    fn occurrence(id: &str, parent: Option<&str>, native: &str, x: f64) -> Value {
        json!({"id": id, "native_ref": native, "transform": at(x),
            "parent": parent.map_or(json!({"kind": "root"}), |id| json!({"kind": "occurrence", "occurrence": id})),
            "prototype": {"scope": "local"}})
    }

    fn displayed(id: &str, x: f64) -> Value {
        json!({"path": [1, 2], "transform": [
            [1.0, 0.0, 0.0, x], [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0], [0.0, 0.0, 0.0, 1.0]],
            "binding": {"state": "bound", "occurrence": id}})
    }

    #[test]
    fn missing_holder_worlds_are_derived_bottom_up_independent_of_record_order() {
        let mut rows = vec![
            occurrence("outer", None, "", 10.0),
            occurrence("holder", Some("outer"), "", 5.0),
            occurrence("a", Some("holder"), "", 20.0),
            occurrence("b", Some("holder"), "", 30.0),
        ];
        for _ in 0..2 {
            let mut document = json!({"model":{"occurrences":rows}, "native":{"f3d":{
                "display_poses":[displayed("a",325.0),displayed("b",335.0)]}}});
            let mut report = Report::default();
            let held = select(
                &empty_model(),
                &mut document,
                crate::PartPositions::Smart,
                &mut report,
            )
            .unwrap();
            let selected: Vec<_> = document["model"]["occurrences"]
                .as_array()
                .unwrap()
                .iter()
                .collect();
            let worlds = world_placements(&selected).unwrap();
            assert_eq!(worlds["outer"], at(300.0));
            assert_eq!(worlds["holder"], at(305.0));
            assert_eq!(worlds["a"], at(325.0));
            assert_eq!(held.len(), 4);
            let counts = report.part_positions().unwrap().1;
            assert_eq!(counts.displayed, 4);
            assert_eq!(counts.derived_holders, 2);
            assert_eq!(counts.total(), 4);
            rows.reverse();
        }
    }

    #[test]
    fn conflicting_displayed_children_leave_the_holder_and_report_the_conflict() {
        let mut document = json!({"model":{"occurrences":[
            occurrence("holder",None,"",40.0),
            occurrence("a",Some("holder"),"",10.0),
            occurrence("b",Some("holder"),"",20.0)
        ]},"native":{"f3d":{"display_poses":[displayed("a",100.0),displayed("b",300.0)]}}});
        let mut report = Report::default();
        let held = select(
            &empty_model(),
            &mut document,
            crate::PartPositions::LastShown,
            &mut report,
        )
        .unwrap();
        let rows: Vec<_> = document["model"]["occurrences"]
            .as_array()
            .unwrap()
            .iter()
            .collect();
        assert_eq!(world_placements(&rows).unwrap()["holder"], at(40.0));
        // The holder is pinned where it was first placed, which joints check but do not solve.
        assert!(held.contains("holder"));
        let counts = report.part_positions().unwrap().1;
        assert_eq!(counts.derived_holders, 0);
        assert_eq!(counts.holder_conflicts, 1);
        assert_eq!(counts.first_placed, 1);
    }

    #[test]
    fn displayed_poses_override_current_and_bounds_in_root_world_mm() {
        let document = json!({"model": {"occurrences": [
            occurrence("insert", None, "xref", 100.0),
            occurrence("child", Some("insert"), "local", 20.0)
        ]}, "native": {"f3d": {
            "xref_references": [{"id": "xref"}],
            "design_component_occurrences": [{"id": "local", "current_pose": {"transform": at(3.0)}}],
            "display_poses": [displayed("insert", 200.0), displayed("child", 6000.0)]
        }}});
        for mode in [crate::PartPositions::Smart, crate::PartPositions::LastShown] {
            let mut selected = document.clone();
            let mut report = Report::default();
            select(&empty_model(), &mut selected, mode, &mut report).unwrap();
            let rows: Vec<_> = selected["model"]["occurrences"]
                .as_array()
                .unwrap()
                .iter()
                .collect();
            let worlds = world_placements(&rows).unwrap();
            assert_eq!(worlds["insert"], at(200.0));
            assert_eq!(worlds["child"], at(6000.0));
            let counts = report.part_positions().unwrap().1;
            assert_eq!(counts.displayed, 2);
            assert_eq!(counts.current, 0);
            assert_eq!(counts.bounds, 0);
        }
        let mut first = document.clone();
        select(
            &empty_model(),
            &mut first,
            crate::PartPositions::FirstPlaced,
            &mut Report::default(),
        )
        .unwrap();
        assert_eq!(first, document);
    }

    #[test]
    fn displayed_poses_require_bound_valid_rows_and_conflicts_are_order_independent() {
        let mut unbound = displayed("unbound", 10.0);
        unbound["binding"]["state"] = json!("unbound");
        let mut ambiguous = displayed("ambiguous", 20.0);
        ambiguous["binding"]["state"] = json!("ambiguous");
        let mut bad_matrix = displayed("bad", 30.0);
        bad_matrix["transform"][3][0] = json!(1.0);
        let mut rows = vec![
            unbound,
            ambiguous,
            bad_matrix,
            displayed("same", 50.0),
            displayed("same", 50.0),
            displayed("conflict", 60.0),
            displayed("conflict", 70.0),
        ];
        for _ in 0..2 {
            let document = json!({"native": {"f3d": {"display_poses": rows}}});
            assert_eq!(
                displayed_poses(&document).unwrap(),
                BTreeMap::from([("same", at(50.0))])
            );
            rows.reverse();
        }
        assert!(displayed_poses(&json!({})).unwrap().is_empty());
    }

    #[test]
    fn float32_displayed_axes_allow_rigid_inverse_without_moving_the_origin() {
        let c = f64::from(std::f32::consts::FRAC_1_SQRT_2);
        let mut row = displayed("part", 0.0);
        row["transform"] = json!([
            [c, -c, 0.0, 962.1],
            [c, c, 0.0, -104.9],
            [0.0, 0.0, 1.0, -1064.0],
            [0.0, 0.0, 0.0, 1.0]
        ]);
        let document = json!({"native":{"f3d":{"display_poses":[row]}}});
        let poses = displayed_poses(&document).unwrap();
        let pose = poses["part"];
        assert_eq!(
            [pose[0][3], pose[1][3], pose[2][3]],
            [962.1, -104.9, -1064.0]
        );
        assert!(same(&compose(&inverse(&pose), &pose), &IDENTITY));
    }

    #[test]
    fn first_placed_retains_the_decoder_component_operation_placement() {
        let mut document = json!({"model":{"occurrences":[
            occurrence("placed",None,"record",80.0)
        ]},"native":{"f3d":{"design_component_occurrences":[
            {"id":"record","transform":at(99.0),
             "operation_pose":{"transform":at(8.0)},
             "current_pose":{"transform":at(12.0)}}
        ]}}});
        let original = document.clone();
        let mut report = Report::default();
        select(
            &empty_model(),
            &mut document,
            crate::PartPositions::FirstPlaced,
            &mut report,
        )
        .unwrap();
        assert_eq!(document, original);
        assert_eq!(report.part_positions().unwrap().1.first_placed, 1);
    }

    #[test]
    fn current_world_pose_is_converted_to_local_mm_and_stored_is_available() {
        let document = json!({"model": {"occurrences": [
            occurrence("owner", None, "owner-record", 100.0),
            occurrence("child", Some("owner"), "child-record", 30.0),
            occurrence("unchanged", Some("child"), "no-pose", 5.0)
        ]}, "native": {"f3d": {"design_component_occurrences": [
            {"id": "owner-record", "transform": at(10.0), "current_pose": {"transform": at(20.0)}},
            {"id": "child-record", "transform": at(3.0), "current_pose": {"transform": at(25.0)}}
        ]}}});
        let mut current = document.clone();
        select(
            &Model {
                ir: cadmpeg_ir::CadIr::empty(),
                fidelity: None,
                name: String::new(),
                source_file: None,
                assembly: None,
                source_step: None,
            },
            &mut current,
            crate::PartPositions::LastShown,
            &mut Report::default(),
        )
        .unwrap();
        let rows = current["model"]["occurrences"].as_array().unwrap();
        assert_eq!(transform(&rows[0]).unwrap(), at(200.0));
        assert_eq!(transform(&rows[1]).unwrap(), at(50.0));
        assert_eq!(transform(&rows[2]).unwrap(), at(5.0));
        let mut stored = document;
        select(
            &Model {
                ir: cadmpeg_ir::CadIr::empty(),
                fidelity: None,
                name: String::new(),
                source_file: None,
                assembly: None,
                source_step: None,
            },
            &mut stored,
            crate::PartPositions::FirstPlaced,
            &mut Report::default(),
        )
        .unwrap();
        let rows = stored["model"]["occurrences"].as_array().unwrap();
        assert_eq!(transform(&rows[0]).unwrap(), at(100.0));
        assert_eq!(transform(&rows[1]).unwrap(), at(30.0));
    }

    #[test]
    fn member_pose_composes_with_each_insertion_and_not_its_local_owner() {
        let mut insertion = occurrence("insert", None, "reference", 100.0);
        // cadmpeg resolves an archive insertion to a local prototype.
        assert_eq!(insertion["prototype"]["scope"], "local");
        insertion["transform"] = json!([
            [0.0, -1.0, 0.0, 100.0],
            [1.0, 0.0, 0.0, 20.0],
            [0.0, 0.0, 1.0, 0.0]
        ]);
        let mut document = json!({"model": {"occurrences": [insertion,
            occurrence("owner", Some("insert"), "owner", 300.0),
            occurrence("child", Some("owner"), "record", 50.0)
        ]}, "native": {"f3d": {"xref_references": [{"id": "reference"}],
            "design_component_occurrences": [
            {"id": "record", "current_pose": {"transform": at(4.0)}}
        ]}}});
        select(
            &empty_model(),
            &mut document,
            crate::PartPositions::LastShown,
            &mut Report::default(),
        )
        .unwrap();
        let rows: Vec<_> = document["model"]["occurrences"]
            .as_array()
            .unwrap()
            .iter()
            .collect();
        let worlds = world_placements(&rows).unwrap();
        assert_eq!(
            worlds["child"],
            [
                [0.0, -1.0, 0.0, 100.0],
                [1.0, 0.0, 0.0, 60.0],
                [0.0, 0.0, 1.0, 0.0]
            ]
        );
    }

    #[test]
    fn identity_pose_and_absent_pose_are_distinct_and_operation_pose_is_ignored() {
        let mut document = json!({"model": {"occurrences": [
            occurrence("posed", None, "a", 70.0), occurrence("unposed", None, "b", 80.0)
        ]}, "native": {"f3d": {"xref_references": [
            {"id": "a", "current_pose": {"carrier_record_index": 3}},
            {"id": "b", "operation_pose": {"transform": at(99.0)}}
        ]}}});
        select(
            &empty_model(),
            &mut document,
            crate::PartPositions::LastShown,
            &mut Report::default(),
        )
        .unwrap();
        let rows = document["model"]["occurrences"].as_array().unwrap();
        assert_eq!(transform(&rows[0]).unwrap(), IDENTITY);
        assert_eq!(transform(&rows[1]).unwrap(), at(80.0));
        let four = json!([[1, 0, 0, 2], [0, 1, 0, 0], [0, 0, 1, 0], [0, 0, 0, 1]]);
        assert_eq!(native_transform(Some(&four)).unwrap(), at(20.0));
    }

    #[test]
    fn shared_world_carrier_keeps_placements_and_reports_count() {
        let mut document = json!({"model": {"occurrences": [
            occurrence("first", None, "record", 10.0), occurrence("second", None, "record", 20.0)
        ]}, "native": {"f3d": {"design_component_occurrences": [
            {"id": "record", "current_pose": {"transform": at(4.0)}}
        ]}}});
        let before = document.clone();
        let mut report = Report::default();
        select(
            &empty_model(),
            &mut document,
            crate::PartPositions::LastShown,
            &mut report,
        )
        .unwrap();
        assert_eq!(document, before);
        assert_eq!(report.part_positions().unwrap().1.ambiguous, 2);
        for bad in [
            json!([]),
            json!([[1, 0, 0, 0]]),
            json!([[2, 0, 0, 0], [0, 1, 0, 0], [0, 0, 1, 0]]),
        ] {
            assert!(native_transform(Some(&bad)).is_err());
        }
    }
}
