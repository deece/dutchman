//! Profile crossings: inferred construction lines, split regions and loop nesting.
//!
//! A sketch's profile table lists the closed regions its curves bound. A line from one point of
//! a region's boundary to another, through its inside, splits that region in two, so a region
//! the table lists whole cannot be crossed by one, unless the line is construction geometry. A
//! decoder that does not know which curves are construction (cadmpeg's Inventor decoder, for
//! now) leaves such a line solid; SolidCraft then finds the region split and no profile it can
//! match. Such lines are construction, and are written so.
//! When every consumer of a sketch selects regions (whole loops or trimmed), a curve that bounds
//! none of the selected regions is construction too, provided no consumer names it.
//!
//! When a source loop itself follows crossing curves, keep those curves solid and split
//! their straight-line arrangement for region selection. Sampling is used only for
//! membership and conservative nesting checks, never to replace curved geometry.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;

use super::sketch::{number, P2};

/// Points sampled along a full circle.
const CIRCLE_SAMPLES: usize = 64;

/// Points sampled along an arc.
const ARC_SAMPLES: usize = 32;

/// The ids of the solid lines among `entities` (a sketch's) that cross a region its profile
/// table, `sketch["profiles"]`, takes whole.
pub(super) fn crossing(sketch: &Value, entities: &[&Value]) -> BTreeSet<String> {
    let by_id: BTreeMap<&str, &Value> = entities
        .iter()
        .filter_map(|entity| Some((entity.get("id")?.as_str()?, *entity)))
        .collect();
    let mut bounding: BTreeSet<&str> = BTreeSet::new();
    let mut polygons: Vec<Vec<P2>> = Vec::new();
    for members in loops(sketch) {
        let mut polygon = Vec::new();
        for member in members {
            let Some(id) = member.get("entity").and_then(Value::as_str) else {
                continue;
            };
            bounding.insert(id);
            let Some(mut samples) = by_id.get(id).and_then(|e| samples(e)) else {
                // A loop of a curve that cannot be followed is not tested.
                polygon.clear();
                break;
            };
            if member.get("reversed").and_then(Value::as_bool) == Some(true) {
                samples.reverse();
            }
            polygon.extend(samples);
        }
        if polygon.len() >= 3 {
            polygons.push(polygon);
        }
    }
    entities
        .iter()
        .filter(|entity| entity.get("construction").and_then(Value::as_bool) != Some(true))
        .filter_map(|entity| {
            let id = entity.get("id")?.as_str()?;
            if bounding.contains(id) {
                return None;
            }
            let geometry = entity.get("geometry")?;
            if geometry.get("kind").and_then(Value::as_str) != Some("line") {
                return None;
            }
            let start = super::sketch::p2(geometry.get("start"))?;
            let end = super::sketch::p2(geometry.get("end"))?;
            let middle = [(start[0] + end[0]) / 2.0, (start[1] + end[1]) / 2.0];
            polygons
                .iter()
                .any(|polygon| {
                    let tolerance = tolerance(polygon);
                    on_boundary(start, polygon, tolerance)
                        && on_boundary(end, polygon, tolerance)
                        && !on_boundary(middle, polygon, tolerance)
                        && inside(middle, polygon)
                })
                .then(|| id.to_owned())
        })
        .collect()
}

/// The solid source curves of a sketch that are written as construction, by reason.
pub(super) struct Demoted {
    /// Lines across a region the sketch's profile table takes whole, and unused lines across
    /// selected circles.
    pub crossing: BTreeSet<String>,
    /// Curves that bound no region a feature selects ([`stray_curves`]).
    pub stray: BTreeSet<String>,
}

/// Which of the sketch's solid curves are written as construction. A trimmed region is
/// bounded by pieces of its curves: SolidCraft finds it only when they stay solid and split
/// the sketch there, so those never are.
pub(super) fn demoted(sketch: &Value, entities: &[&Value], features: &[Value]) -> Demoted {
    let mut crossing = crossing(sketch, entities);
    crossing.extend(region_lines(sketch, entities, features));
    let mut stray = stray_curves(sketch, entities, features);
    let trimmed = trimmed_region_curves(sketch, features);
    crossing.retain(|id| !trimmed.contains(id));
    stray.retain(|id| !trimmed.contains(id));
    Demoted { crossing, stray }
}

/// Whether a line of `members` (a profile loop) runs along part of another solid line of the
/// sketch, one that is not construction in the source nor in `demoted`. SolidCraft splits
/// both where they overlap, so the loop's own curves no longer bound one of its profiles.
pub(super) fn shares_an_edge(
    members: &[Value],
    entities: &[&Value],
    demoted: &BTreeSet<String>,
) -> bool {
    let ends = |entity: &Value| -> Option<(P2, P2)> {
        let geometry = entity.get("geometry")?;
        if geometry.get("kind").and_then(Value::as_str) != Some("line") {
            return None;
        }
        Some((
            super::sketch::p2(geometry.get("start"))?,
            super::sketch::p2(geometry.get("end"))?,
        ))
    };
    let own: BTreeSet<&str> = members
        .iter()
        .filter_map(|member| member.get("entity").and_then(Value::as_str))
        .collect();
    let others: Vec<(P2, P2)> = entities
        .iter()
        .filter(|entity| entity.get("construction").and_then(Value::as_bool) != Some(true))
        .filter(|entity| {
            entity
                .get("id")
                .and_then(Value::as_str)
                .is_some_and(|id| !own.contains(id) && !demoted.contains(id))
        })
        .filter_map(|entity| ends(entity))
        .collect();
    own.iter()
        .filter_map(|id| entity_named(entities, id))
        .filter_map(ends)
        .any(|(a, b)| {
            let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
            let length = dx.hypot(dy);
            if length <= EPS || !length.is_finite() {
                return false;
            }
            let tol = (length * 1e-6).max(1e-9);
            let off = |p: P2| ((p[0] - a[0]) * dy - (p[1] - a[1]) * dx).abs() / length;
            let along = |p: P2| ((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / length;
            others.iter().any(|&(c, d)| {
                if off(c) > tol || off(d) > tol {
                    return false;
                }
                let (low, high) = (along(c).min(along(d)), along(c).max(along(d)));
                high.min(length) - low.max(0.0) > tol
            })
        })
}

/// The source curves that bound a trimmed profile region some feature selects from `sketch`.
pub(super) fn trimmed_region_curves(sketch: &Value, features: &[Value]) -> BTreeSet<String> {
    fn walk(value: &Value, sketch: &str, found: &mut BTreeSet<String>, depth: usize) {
        if depth > 32 {
            return;
        }
        match value {
            Value::Object(fields) => {
                if fields.get("kind").and_then(Value::as_str) == Some("sketch_regions")
                    && value.pointer("/value/sketch").and_then(Value::as_str) == Some(sketch)
                {
                    for region in value
                        .pointer("/value/regions")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                        .filter(|region| {
                            region.get("region").and_then(Value::as_str) == Some("trimmed")
                        })
                    {
                        let rings = region
                            .get("outer_boundary")
                            .into_iter()
                            .chain(
                                region
                                    .get("hole_boundaries")
                                    .and_then(Value::as_array)
                                    .into_iter()
                                    .flatten(),
                            )
                            .filter_map(Value::as_array);
                        for ring in rings {
                            found.extend(
                                ring.iter()
                                    .filter_map(|use_| use_.get("entity")?.as_str())
                                    .map(str::to_owned),
                            );
                        }
                    }
                    return;
                }
                for child in fields.values() {
                    walk(child, sketch, found, depth + 1);
                }
            }
            Value::Array(items) => {
                for item in items {
                    walk(item, sketch, found, depth + 1);
                }
            }
            _ => {}
        }
    }
    let mut found = BTreeSet::new();
    if let Some(id) = sketch.get("id").and_then(Value::as_str) {
        for feature in features {
            walk(super::operation(feature), id, &mut found, 0);
        }
    }
    found
}

/// The curves that the sketch's consumers (every feature naming it) select whole-loop regions
/// from, when every consumer is such a selection: the source entities of the selected loops.
/// `None` when some consumer needs the sketch's curves as geometry, or nothing selects a region.
fn whole_loop_uses(
    sketch: &Value,
    entities: &[&Value],
    features: &[Value],
) -> Option<BTreeSet<String>> {
    let id = sketch.get("id").and_then(Value::as_str)?;
    let mut identities = vec![id];
    identities.extend(features.iter().filter_map(|feature| {
        let definition = super::operation(feature);
        if definition.get("definition").and_then(Value::as_str) != Some("sketch")
            || definition.pointer("/sketch/sketch").and_then(Value::as_str) != Some(id)
        {
            return None;
        }
        feature.get("id").and_then(Value::as_str)
    }));
    let mut used = BTreeSet::new();
    let mut selected = false;
    for feature in features {
        if feature.get("suppressed").and_then(Value::as_bool) == Some(true) {
            continue;
        }
        let definition = super::operation(feature);
        if definition.get("definition").and_then(Value::as_str) == Some("sketch") {
            continue;
        }
        let found = region_uses(definition, id, &identities, sketch, entities, &mut used, 0)?;
        if !found
            && feature
                .get("dependencies")
                .and_then(Value::as_array)
                .is_some_and(|dependencies| {
                    dependencies
                        .iter()
                        .filter_map(Value::as_str)
                        .any(|id| identities.contains(&id))
                })
        {
            return None;
        }
        selected |= found;
    }
    selected.then_some(used)
}

/// Curves that bound no region a feature selects from the sketch, when every consumer of the
/// sketch selects regions (whole loops or trimmed) and none of them names the curve. Solid,
/// they only divide the regions that are selected: a loop no feature takes, sharing part of an
/// edge with one that is taken, or a ring around a hole the region keeps, leaves SolidCraft no
/// profile with the source's boundary. They are construction.
pub(super) fn stray_curves(
    sketch: &Value,
    entities: &[&Value],
    features: &[Value],
) -> BTreeSet<String> {
    let Some(used) = whole_loop_uses(sketch, entities, features).filter(|used| !used.is_empty())
    else {
        return BTreeSet::new();
    };
    let strays: BTreeSet<String> = entities
        .iter()
        .filter(|entity| entity.get("construction").and_then(Value::as_bool) != Some(true))
        .filter(|entity| {
            !matches!(
                entity.pointer("/geometry/kind").and_then(Value::as_str),
                None | Some("point")
            )
        })
        .filter_map(|entity| entity.get("id").and_then(Value::as_str))
        .filter(|id| !used.contains(*id))
        .map(str::to_owned)
        .collect();
    if strays.is_empty() {
        return strays;
    }
    // A consumer that names a curve itself (a path, a rail, a reference) keeps it real.
    let mut named = BTreeSet::new();
    for feature in features {
        if super::operation(feature)
            .get("definition")
            .and_then(Value::as_str)
            != Some("sketch")
        {
            named_in(feature, &strays, &mut named, 0);
        }
    }
    strays.difference(&named).cloned().collect()
}

/// Adds to `found` each of `ids` that a string inside `value` names.
fn named_in(value: &Value, ids: &BTreeSet<String>, found: &mut BTreeSet<String>, depth: usize) {
    if depth > MAX_SELECTION_DEPTH {
        found.extend(ids.iter().cloned());
        return;
    }
    match value {
        Value::String(s) if ids.contains(s.as_str()) => {
            found.insert(s.clone());
        }
        Value::Array(values) => values
            .iter()
            .for_each(|v| named_in(v, ids, found, depth + 1)),
        Value::Object(fields) => fields
            .values()
            .for_each(|v| named_in(v, ids, found, depth + 1)),
        _ => {}
    }
}

/// Unused lines crossing selected circles are helpers when all consumers of the sketch
/// select whole-loop regions. Keeping them solid would split those circles.
pub(super) fn region_lines(
    sketch: &Value,
    entities: &[&Value],
    features: &[Value],
) -> BTreeSet<String> {
    let Some(used) = whole_loop_uses(sketch, entities, features) else {
        return BTreeSet::new();
    };
    if entities.len() > MAX_EDGES {
        return BTreeSet::new();
    }
    let circles: Vec<&Value> = entities
        .iter()
        .copied()
        .filter(|entity| {
            entity
                .get("id")
                .and_then(Value::as_str)
                .is_some_and(|id| used.contains(id))
                && entity.pointer("/geometry/kind").and_then(Value::as_str) == Some("circle")
        })
        .collect();
    if circles.len() > MAX_LINES {
        return BTreeSet::new();
    }
    entities
        .iter()
        .filter(|entity| entity.get("construction").and_then(Value::as_bool) != Some(true))
        .filter(|entity| {
            entity
                .get("geometry")
                .and_then(|g| g.get("kind"))
                .and_then(Value::as_str)
                == Some("line")
        })
        .filter(|entity| circles.iter().any(|circle| cuts_circle(entity, circle)))
        .filter_map(|entity| entity.get("id").and_then(Value::as_str))
        .filter(|id| !used.contains(*id))
        .map(str::to_owned)
        .collect()
}

/// A proper secant through a whole circle; tangencies and lines ending inside it are left
/// alone. The margin avoids treating round-off at a tangent as evidence of a crossing.
fn cuts_circle(line: &Value, circle: &Value) -> bool {
    let positions = || {
        Some((
            super::sketch::p2(line.pointer("/geometry/start"))?,
            super::sketch::p2(line.pointer("/geometry/end"))?,
            super::sketch::p2(circle.pointer("/geometry/center"))?,
            number(circle.pointer("/geometry/radius"))?,
        ))
    };
    let Some(([ax, ay], [bx, by], [cx, cy], radius)) = positions() else {
        return false;
    };
    let (dx, dy) = (bx - ax, by - ay);
    let length2 = dx * dx + dy * dy;
    let radius2 = radius * radius;
    if length2 <= 0.0 || !length2.is_finite() || radius <= 0.0 || !radius2.is_finite() {
        return false;
    }
    let along = ((cx - ax) * dx + (cy - ay) * dy) / length2;
    let distance2 = (ax + along * dx - cx).powi(2) + (ay + along * dy - cy).powi(2);
    along > 0.0
        && along < 1.0
        && distance2 < radius2 * (1.0 - 1e-6)
        && (ax - cx).powi(2) + (ay - cy).powi(2) > radius2 * (1.0 + 1e-6)
        && (bx - cx).powi(2) + (by - cy).powi(2) > radius2 * (1.0 + 1e-6)
}

/// Any other reference to the sketch prevents changing its profile geometry globally.
fn region_uses(
    value: &Value,
    id: &str,
    identities: &[&str],
    sketch: &Value,
    entities: &[&Value],
    used: &mut BTreeSet<String>,
    depth: usize,
) -> Option<bool> {
    if depth > MAX_SELECTION_DEPTH {
        return None;
    }
    // A list of the sketch's profile loops: the curves of those loops are used. A feature that
    // takes the whole sketch takes every region its curves bound, which the profile table may
    // not list in full, so it says nothing about which curves are unused (it is left to the
    // identity check below, which keeps every curve).
    let kind = value.get("kind").and_then(Value::as_str);
    let listed: Option<Vec<usize>> = match kind {
        Some("sketch_profiles")
            if value.pointer("/value/sketch").and_then(Value::as_str) == Some(id) =>
        {
            Some(
                value
                    .pointer("/value/profiles")?
                    .as_array()?
                    .iter()
                    .map(|index| index.as_u64().and_then(|i| usize::try_from(i).ok()))
                    .collect::<Option<Vec<usize>>>()?,
            )
        }
        _ => None,
    };
    if let Some(indices) = listed {
        let profiles = sketch.get("profiles")?.as_array()?;
        if indices.is_empty() || indices.len() > MAX_LINES {
            return None;
        }
        let mut members = 0usize;
        for index in indices {
            let profile = profiles.get(index)?;
            // A profile is its loop, or (in some files) an object listing several loops.
            let boundaries: Vec<&Vec<Value>> = match profile {
                Value::Array(members) => vec![members],
                Value::Object(fields) => fields
                    .get("loops")?
                    .as_array()?
                    .iter()
                    .map(Value::as_array)
                    .collect::<Option<Vec<_>>>()?,
                _ => return None,
            };
            for boundary in boundaries {
                if boundary.is_empty() {
                    return None;
                }
                members = members.checked_add(boundary.len())?;
                if members > MAX_EDGES {
                    return None;
                }
                for member in boundary {
                    used.insert(member.get("entity")?.as_str()?.to_owned());
                    if used.len() > MAX_EDGES {
                        return None;
                    }
                }
            }
        }
        return Some(true);
    }
    if kind == Some("sketch_regions")
        && value.get("value")?.get("sketch").and_then(Value::as_str) == Some(id)
    {
        let regions = value.get("value")?.get("regions")?.as_array()?;
        if regions.is_empty() || regions.len() > MAX_LINES {
            return None;
        }
        let profiles = sketch.get("profiles")?.as_array()?;
        let mut members = 0usize;
        for region in regions {
            match region.get("region").and_then(Value::as_str) {
                Some("loops") => {}
                // A trimmed region is bounded by pieces of its curves: each is used.
                Some("trimmed") => {
                    let holes = match region.get("hole_boundaries") {
                        Some(holes) => holes.as_array()?.as_slice(),
                        None => &[],
                    };
                    let rings = std::iter::once(region.get("outer_boundary")?).chain(holes);
                    for ring in rings {
                        let ring = ring.as_array()?;
                        // A boundary that doesn't close leaves the curves finishing it unknown.
                        if ring.is_empty() || trimmed_ring(ring, entities).is_none() {
                            return None;
                        }
                        members = members.checked_add(ring.len())?;
                        if members > MAX_EDGES {
                            return None;
                        }
                        for piece in ring {
                            used.insert(piece.get("entity")?.as_str()?.to_owned());
                            if used.len() > MAX_EDGES {
                                return None;
                            }
                        }
                    }
                    continue;
                }
                _ => return None,
            }
            let holes = match region.get("holes") {
                Some(holes) => Some(holes.as_array()?),
                None => None,
            };
            if holes.is_some_and(|holes| holes.len() > MAX_LINES) {
                return None;
            }
            for index in std::iter::once(region.get("outer")?).chain(holes.into_iter().flatten()) {
                let index = usize::try_from(index.as_u64()?).ok()?;
                let boundary = profiles.get(index)?.as_array()?;
                if boundary.is_empty() {
                    return None;
                }
                members = members.checked_add(boundary.len())?;
                if members > MAX_EDGES {
                    return None;
                }
                for member in boundary {
                    used.insert(member.get("entity")?.as_str()?.to_owned());
                    if used.len() > MAX_EDGES {
                        return None;
                    }
                }
            }
        }
        return Some(true);
    }
    match value {
        Value::String(s) if identities.contains(&s.as_str()) => None,
        Value::Array(values) => values.iter().try_fold(false, |found, value| {
            Some(region_uses(value, id, identities, sketch, entities, used, depth + 1)? || found)
        }),
        Value::Object(fields) => fields.values().try_fold(false, |found, value| {
            Some(region_uses(value, id, identities, sketch, entities, used, depth + 1)? || found)
        }),
        _ => Some(false),
    }
}

/// The loops of the sketch's profile table, each its members (`{entity, reversed}`) in order.
fn loops(sketch: &Value) -> Vec<&[Value]> {
    let mut out = Vec::new();
    for profile in sketch
        .get("profiles")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        match profile {
            Value::Array(members) => out.push(members.as_slice()),
            Value::Object(fields) => {
                for members in fields
                    .get("loops")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                {
                    if let Some(members) = members.as_array() {
                        out.push(members.as_slice());
                    }
                }
            }
            _ => {}
        }
    }
    out
}

/// Points along a sketch curve from its start to its end; `None` for a curve not followed.
fn samples(entity: &Value) -> Option<Vec<P2>> {
    let geometry = entity.get("geometry")?;
    match geometry.get("kind")?.as_str()? {
        "line" => Some(vec![
            super::sketch::p2(geometry.get("start"))?,
            super::sketch::p2(geometry.get("end"))?,
        ]),
        kind @ ("arc" | "circle") => {
            let center = super::sketch::p2(geometry.get("center"))?;
            let radius = number(geometry.get("radius"))?;
            let (start, sweep, count) = if kind == "arc" {
                let start = number(geometry.get("start_angle"))?;
                let end = number(geometry.get("end_angle"))?;
                let sweep = (end - start).rem_euclid(std::f64::consts::TAU);
                (start, sweep, ARC_SAMPLES)
            } else {
                (0.0, std::f64::consts::TAU, CIRCLE_SAMPLES)
            };
            Some(
                (0..=count)
                    .map(|i| {
                        let angle = start + sweep * i as f64 / count as f64;
                        [
                            center[0] + radius * angle.cos(),
                            center[1] + radius * angle.sin(),
                        ]
                    })
                    .collect(),
            )
        }
        _ => None,
    }
}

/// How close the ends of two boundary uses must be for a ring to close.
const RING_CLOSES: f64 = 1e-3;

/// A point must be at least this far inside a region to name it.
const CLEARANCE: f64 = 1e-3;

/// How a boundary use's parameter can run on a circle or arc against the sketch's own angle: the
/// carrier may be mirrored or turned against the sketch. Only a reading whose uses meet end to
/// end can be the one the source means.
const READINGS: [fn(f64) -> f64; 4] = [
    |t| t,
    |t| -t,
    |t| std::f64::consts::PI - t,
    |t| std::f64::consts::PI + t,
];

/// Whether a region uses part of a whole circle. Several regions of two crossing closed curves
/// have the same curves, so naming the curves can't tell them apart.
pub(super) fn trims_a_closed_curve(uses: &[Value], entities: &[&Value]) -> bool {
    uses.iter().any(|use_| {
        let entity = use_
            .get("entity")
            .and_then(Value::as_str)
            .and_then(|id| entity_named(entities, id));
        let is_circle = entity
            .and_then(|e| e.pointer("/geometry/kind"))
            .and_then(Value::as_str)
            == Some("circle");
        let span = range(use_).map(|(from, to)| (to - from).abs());
        is_circle
            && span.is_some_and(|span| {
                span < std::f64::consts::TAU - 1e-6
                    && !complete_circle(uses, use_.get("entity").and_then(Value::as_str))
            })
    })
}

/// Whether the uses of one circle together cover all of it.
fn complete_circle(uses: &[Value], entity: Option<&str>) -> bool {
    let total: f64 = uses
        .iter()
        .filter(|use_| use_.get("entity").and_then(Value::as_str) == entity)
        .filter_map(range)
        .map(|(from, to)| (to - from).abs())
        .sum();
    (total - std::f64::consts::TAU).abs() < 1e-6
}

fn entity_named<'a>(entities: &[&'a Value], id: &str) -> Option<&'a Value> {
    entities
        .iter()
        .find(|entity| entity.get("id").and_then(Value::as_str) == Some(id))
        .copied()
}

fn range(use_: &Value) -> Option<(f64, f64)> {
    let range = use_.get("parameter_range")?.as_array()?;
    Some((range.first()?.as_f64()?, range.get(1)?.as_f64()?))
}

/// Points along one boundary use, under one reading of its parameter.
fn use_samples(use_: &Value, entities: &[&Value], reading: fn(f64) -> f64) -> Option<Vec<P2>> {
    let entity = entity_named(entities, use_.get("entity")?.as_str()?)?;
    let geometry = entity.get("geometry")?;
    let (from, to) = range(use_)?;
    match geometry.get("kind")?.as_str()? {
        // A line's parameter is the fraction of the way from its stored start to its end
        // (cadmpeg: start + t·(end − start)), not a distance.
        "line" => {
            let start = super::sketch::p2(geometry.get("start"))?;
            let end = super::sketch::p2(geometry.get("end"))?;
            let length = super::sketch::distance(start, end);
            let slack = RING_CLOSES / length;
            if length <= 0.0 || from < -slack || to > 1.0 + slack {
                return None;
            }
            Some(vec![along(start, end, from), along(start, end, to)])
        }
        "arc" | "circle" => {
            let center = super::sketch::p2(geometry.get("center"))?;
            let radius = number(geometry.get("radius"))?;
            let count = ((to - from).abs() / std::f64::consts::TAU * CIRCLE_SAMPLES as f64)
                .ceil()
                .max(2.0) as usize;
            Some(
                (0..=count)
                    .map(|i| {
                        let angle = reading(from + (to - from) * i as f64 / count as f64);
                        [
                            center[0] + radius * angle.cos(),
                            center[1] + radius * angle.sin(),
                        ]
                    })
                    .collect(),
            )
        }
        _ => None,
    }
}

/// One ring of boundary uses under one reading: each use continues from the end of the last,
/// and the last meets the first. `None` when they don't: this isn't how the source means them.
fn ring_under(uses: &[Value], entities: &[&Value], reading: fn(f64) -> f64) -> Option<Vec<P2>> {
    if uses.is_empty() || uses.len() > MAX_LINES {
        return None;
    }
    let mut pieces: Vec<Vec<P2>> = uses
        .iter()
        .map(|use_| use_samples(use_, entities, reading))
        .collect::<Option<_>>()?;
    // The first use's direction is settled by what it must meet next.
    if let [first, second, ..] = pieces.as_mut_slice() {
        let (start, end) = (*first.first()?, *first.last()?);
        let (a, b) = (*second.first()?, *second.last()?);
        let reach = |p: P2| super::sketch::distance(p, a).min(super::sketch::distance(p, b));
        if reach(start) < reach(end) {
            first.reverse();
        }
    }
    let mut out: Vec<P2> = Vec::new();
    for mut piece in pieces {
        if let Some(&last) = out.last() {
            if super::sketch::distance(last, *piece.last()?)
                < super::sketch::distance(last, *piece.first()?)
            {
                piece.reverse();
            }
            if super::sketch::distance(last, *piece.first()?) > RING_CLOSES {
                return None;
            }
        }
        out.extend(piece);
    }
    let (&first, &last) = (out.first()?, out.last()?);
    if super::sketch::distance(first, last) > RING_CLOSES {
        return None;
    }
    out.dedup_by(|a, b| super::sketch::distance(*a, *b) <= RING_CLOSES / 10.0);
    out.pop();
    (out.len() >= 3 && !self_crossing(&out)).then_some(out)
}

fn area_and_centre(ring: &[P2]) -> (f64, P2) {
    let (mut area, mut x, mut y) = (0.0, 0.0, 0.0);
    for (a, b) in edges(ring) {
        let cross = a[0] * b[1] - b[0] * a[1];
        area += cross;
        x += (a[0] + b[0]) * cross;
        y += (a[1] + b[1]) * cross;
    }
    if area.abs() < f64::EPSILON {
        return (0.0, [0.0, 0.0]);
    }
    ((area / 2.0).abs(), [x / (3.0 * area), y / (3.0 * area)])
}

/// The ring of a region's boundary uses. Every reading that closes must give the same shape;
/// readings that close on different shapes can't be told apart, so there is no ring.
pub(super) fn trimmed_ring(uses: &[Value], entities: &[&Value]) -> Option<Vec<P2>> {
    trimmed_rings(&[uses], entities)?.pop()
}

/// Resolve parameter readings together only for rings sharing a curved carrier. A diameter
/// alone cannot distinguish the two halves of a circle, but another closed boundary using
/// that same circle can settle its reading. Unrelated carriers supply no such evidence.
pub(super) fn trimmed_rings(uses: &[&[Value]], entities: &[&Value]) -> Option<Vec<Vec<P2>>> {
    if uses.is_empty()
        || uses.len() > MAX_LINES
        || uses
            .iter()
            .any(|ring| ring.is_empty() || ring.len() > MAX_LINES)
    {
        return None;
    }
    let carriers: Vec<BTreeSet<&str>> = uses
        .iter()
        .map(|ring| {
            ring.iter()
                .filter_map(|use_| use_.get("entity")?.as_str())
                .filter(|id| {
                    entity_named(entities, id)
                        .and_then(|entity| entity.pointer("/geometry/kind"))
                        .and_then(Value::as_str)
                        .is_some_and(|kind| matches!(kind, "arc" | "circle"))
                })
                .collect()
        })
        .collect();
    let mut out = vec![None; uses.len()];
    for start in 0..uses.len() {
        if out.get(start)?.is_some() {
            continue;
        }
        let mut component = BTreeSet::from([start]);
        let mut pending = vec![start];
        while let Some(index) = pending.pop() {
            for (other, curves) in carriers.iter().enumerate() {
                if !carriers.get(index)?.is_disjoint(curves) && component.insert(other) {
                    pending.push(other);
                }
            }
        }
        let boundaries: Vec<&[Value]> = component
            .iter()
            .filter_map(|index| uses.get(*index).copied())
            .collect();
        for (index, ring) in component
            .into_iter()
            .zip(rings_under_readings(&boundaries, entities)?)
        {
            *out.get_mut(index)? = Some(ring);
        }
    }
    out.into_iter().collect()
}

fn rings_under_readings(uses: &[&[Value]], entities: &[&Value]) -> Option<Vec<Vec<P2>>> {
    let mut found: Option<Vec<Vec<P2>>> = None;
    for reading in READINGS {
        let Some(rings) = uses
            .iter()
            .map(|uses| ring_under(uses, entities, reading))
            .collect::<Option<Vec<_>>>()
        else {
            continue;
        };
        if let Some(known) = &found {
            for (known, ring) in known.iter().zip(&rings) {
                let (a, c) = area_and_centre(known);
                let (b, d) = area_and_centre(ring);
                if (a - b).abs() > 1e-3 * a.max(b).max(1e-9)
                    || super::sketch::distance(c, d) > RING_CLOSES * 10.0
                {
                    return None;
                }
            }
        } else {
            found = Some(rings);
        }
    }
    found
}

/// The ring of a whole source loop, for the regions selected beside a trimmed one.
pub(super) fn loop_ring(members: &[Value], entities: &[&Value]) -> Option<Vec<P2>> {
    polygon(members, entities)
}

/// A point inside `ring`, outside each of `others`, as far from all their edges as the scan
/// finds. `None` when no such point stands clear of them.
pub(super) fn interior_point(ring: &[P2], others: &[&[P2]]) -> Option<P2> {
    const ROWS: usize = 96;
    let low = ring.iter().map(|p| p[1]).fold(f64::INFINITY, f64::min);
    let high = ring.iter().map(|p| p[1]).fold(f64::NEG_INFINITY, f64::max);
    let clearance = |p: P2| {
        std::iter::once(ring)
            .chain(others.iter().copied())
            .flat_map(edges)
            .map(|(a, b)| distance_to_edge(p, a, b))
            .fold(f64::INFINITY, f64::min)
    };
    let mut best: Option<(f64, P2)> = None;
    for row in 0..ROWS {
        // Off the rows' round fractions, so a scan doesn't run along a horizontal edge.
        let y = low + (high - low) * (row as f64 + 0.4871) / ROWS as f64;
        let mut hits: Vec<f64> = edges(ring)
            .filter(|(a, b)| (a[1] > y) != (b[1] > y))
            .map(|(a, b)| a[0] + (y - a[1]) / (b[1] - a[1]) * (b[0] - a[0]))
            .collect();
        hits.sort_by(f64::total_cmp);
        for (&left, &right) in hits.iter().step_by(2).zip(hits.iter().skip(1).step_by(2)) {
            for part in 1..8 {
                let p = [left + (right - left) * f64::from(part) / 8.0, y];
                if others.iter().any(|other| inside(p, other)) || !inside(p, ring) {
                    continue;
                }
                let room = clearance(p);
                if room >= CLEARANCE && best.is_none_or(|(known, _)| room > known) {
                    best = Some((room, p));
                }
            }
        }
    }
    best.map(|(_, p)| p)
}

fn distance_to_edge(p: P2, a: P2, b: P2) -> f64 {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let length = dx * dx + dy * dy;
    if length <= 0.0 {
        return super::sketch::distance(p, a);
    }
    let t = (((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / length).clamp(0.0, 1.0);
    super::sketch::distance(p, [a[0] + t * dx, a[1] + t * dy])
}

/// How near a point must be to a polygon's edge to be on it: a millionth of its size.
fn tolerance(polygon: &[P2]) -> f64 {
    let (mut low, mut high) = ([f64::MAX; 2], [f64::MIN; 2]);
    for &[x, y] in polygon {
        low = [low[0].min(x), low[1].min(y)];
        high = [high[0].max(x), high[1].max(y)];
    }
    ((high[0] - low[0]).hypot(high[1] - low[1]) * 1e-6).max(1e-9)
}

/// The polygon's edges, the last closing it.
fn edges(polygon: &[P2]) -> impl Iterator<Item = (P2, P2)> + '_ {
    polygon
        .iter()
        .zip(polygon.iter().cycle().skip(1))
        .map(|(a, b)| (*a, *b))
}

fn on_boundary(p: P2, polygon: &[P2], tolerance: f64) -> bool {
    edges(polygon).any(|(a, b)| {
        let along = [b[0] - a[0], b[1] - a[1]];
        let to = [p[0] - a[0], p[1] - a[1]];
        let length = along[0] * along[0] + along[1] * along[1];
        let t = if length > 0.0 {
            ((to[0] * along[0] + to[1] * along[1]) / length).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let near = [a[0] + along[0] * t - p[0], a[1] + along[1] * t - p[1]];
        near[0].hypot(near[1]) <= tolerance
    })
}

/// Whether `p` is inside the polygon, by the even-odd rule.
fn inside(p: P2, polygon: &[P2]) -> bool {
    edges(polygon)
        .filter(|(a, b)| {
            (a[1] > p[1]) != (b[1] > p[1])
                && p[0] < a[0] + (p[1] - a[1]) * (b[0] - a[0]) / (b[1] - a[1])
        })
        .count()
        % 2
        == 1
}

/// A line arrangement is deliberately bounded: intersections and directed edges grow
/// quadratically. Larger arrangements keep the existing refusal.
const MAX_LINES: usize = 128;
const MAX_SELECTION_DEPTH: usize = 64;
const MAX_EDGES: usize = 16_384;
const MAX_POINTS: usize = 2048;
const EPS: f64 = 1e-8;

struct Segment<'a> {
    id: &'a str,
    a: P2,
    b: P2,
    cuts: Vec<f64>,
}

fn determinant(a: P2, b: P2) -> f64 {
    a[0] * b[1] - a[1] * b[0]
}

fn subtract(a: P2, b: P2) -> P2 {
    [a[0] - b[0], a[1] - b[1]]
}

fn along(a: P2, b: P2, t: f64) -> P2 {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]
}

/// Parameters at an intersection, including endpoints. Collinear overlapping lines
/// cannot be matched uniquely by source ids and are refused.
fn intersection(a: P2, b: P2, c: P2, d: P2) -> Result<Option<(f64, f64)>, ()> {
    let u = subtract(b, a);
    let v = subtract(d, c);
    let offset = subtract(c, a);
    let denominator = determinant(u, v);
    let length = u[0].hypot(u[1]);
    let other_length = v[0].hypot(v[1]);
    if length <= EPS || other_length <= EPS {
        return Err(());
    }
    if denominator.abs() <= EPS * length * other_length {
        if determinant(offset, u).abs() <= EPS * length {
            let t = |p: P2| {
                let delta = subtract(p, a);
                (delta[0] * u[0] + delta[1] * u[1]) / (length * length)
            };
            if t(c).max(t(d)).min(1.0) - t(c).min(t(d)).max(0.0) > EPS {
                return Err(());
            }
        }
        return Ok(None);
    }
    let t = determinant(offset, v) / denominator;
    let s = determinant(offset, u) / denominator;
    Ok(
        ((-EPS..=1.0 + EPS).contains(&t) && (-EPS..=1.0 + EPS).contains(&s))
            .then(|| (t.clamp(0.0, 1.0), s.clamp(0.0, 1.0))),
    )
}

/// Follow one source loop, including retraced curves, for even-odd membership tests.
fn polygon(members: &[Value], entities: &[&Value]) -> Option<Vec<P2>> {
    if members.len() > MAX_LINES {
        return None;
    }
    let mut out = Vec::new();
    for member in members {
        let id = member.get("entity")?.as_str()?;
        let entity = entities
            .iter()
            .find(|entity| entity.get("id").and_then(Value::as_str) == Some(id))?;
        let mut points = samples(entity)?;
        if member.get("reversed").and_then(Value::as_bool) == Some(true) {
            points.reverse();
        }
        if let (Some(&last), Some(&first)) = (out.last(), points.first()) {
            if super::sketch::distance(last, first) > EPS {
                return None;
            }
        }
        out.extend(points);
        if out.len() > MAX_POINTS {
            return None;
        }
    }
    let (&first, &last) = (out.first()?, out.last()?);
    if out.len() < 3 || super::sketch::distance(first, last) > EPS {
        return None;
    }
    out.dedup_by(|a, b| super::sketch::distance(*a, *b) <= EPS);
    out.pop();
    (out.len() >= 3).then_some(out)
}

/// Whether a source loop has intersections beyond adjacent endpoints.
pub(super) fn crossed_loop(members: &[Value], entities: &[&Value]) -> bool {
    polygon(members, entities).is_some_and(|ring| self_crossing(&ring))
}

/// Whether two closed rings cross or run along each other: some edge of one meets an edge of
/// the other. Two holes of one region that do (a nut pocket drawn over a bolt hole) bound one
/// hole together, so neither ring is a boundary SolidCraft finds on its own.
pub(super) fn rings_cross(a: &[P2], b: &[P2]) -> bool {
    edges(a).any(|(p, q)| {
        edges(b).any(|(r, s)| !intersection(p, q, r, s).is_ok_and(|hit| hit.is_none()))
    })
}

fn self_crossing(ring: &[P2]) -> bool {
    for (i, (a, b)) in edges(ring).enumerate() {
        for (j, (c, d)) in edges(ring).enumerate().skip(i + 1) {
            if j == i + 1 || (i == 0 && j + 1 == ring.len()) {
                continue;
            }
            if !intersection(a, b, c, d).is_ok_and(|hit| hit.is_none()) {
                return true;
            }
        }
    }
    false
}

/// A line or arc of a source loop, for the exact self-crossing test.
#[derive(Clone, Copy)]
enum Piece {
    Line(P2, P2),
    /// Centre, radius, start angle, sweep (a full turn for a circle).
    Arc(P2, f64, f64, f64),
}

impl Piece {
    fn of(entity: &Value) -> Option<Piece> {
        let geometry = entity.get("geometry")?;
        match geometry.get("kind")?.as_str()? {
            "line" => Some(Piece::Line(
                super::sketch::p2(geometry.get("start"))?,
                super::sketch::p2(geometry.get("end"))?,
            )),
            "arc" => {
                let start = number(geometry.get("start_angle"))?;
                let end = number(geometry.get("end_angle"))?;
                Some(Piece::Arc(
                    super::sketch::p2(geometry.get("center"))?,
                    number(geometry.get("radius"))?,
                    start,
                    (end - start).rem_euclid(std::f64::consts::TAU),
                ))
            }
            "circle" => Some(Piece::Arc(
                super::sketch::p2(geometry.get("center"))?,
                number(geometry.get("radius"))?,
                0.0,
                std::f64::consts::TAU,
            )),
            _ => None,
        }
    }

    fn ends(self) -> [P2; 2] {
        match self {
            Piece::Line(a, b) => [a, b],
            Piece::Arc(c, r, start, sweep) => [
                [c[0] + r * start.cos(), c[1] + r * start.sin()],
                [
                    c[0] + r * (start + sweep).cos(),
                    c[1] + r * (start + sweep).sin(),
                ],
            ],
        }
    }

    /// Whether a point already known to lie on the curve's circle is within the arc.
    fn holds_angle_of(self, p: P2, tol: f64) -> bool {
        match self {
            Piece::Line(..) => true,
            Piece::Arc(c, r, start, sweep) => {
                if sweep >= std::f64::consts::TAU - 1e-12 {
                    return true;
                }
                let angle = (p[1] - c[1]).atan2(p[0] - c[0]);
                let offset = (angle - start).rem_euclid(std::f64::consts::TAU);
                let slack = tol / r.abs().max(tol);
                offset <= sweep + slack || offset >= std::f64::consts::TAU - slack
            }
        }
    }

    /// Whether a point on the curve's line is within the segment.
    fn holds_along(self, p: P2, tol: f64) -> bool {
        match self {
            Piece::Line(a, b) => {
                let u = subtract(b, a);
                let length = u[0].hypot(u[1]);
                if length <= tol {
                    return false;
                }
                let t = ((p[0] - a[0]) * u[0] + (p[1] - a[1]) * u[1]) / length;
                (-tol..=length + tol).contains(&t)
            }
            arc => arc.holds_angle_of(p, tol),
        }
    }
}

/// Points where two pieces meet, to within `tol`. Overlapping lines or arcs give the ends
/// of the shared stretch.
fn meetings(a: Piece, b: Piece, tol: f64) -> Vec<P2> {
    let mut out = Vec::new();
    match (a, b) {
        (Piece::Line(p, q), Piece::Line(r, s)) => {
            let u = subtract(q, p);
            let v = subtract(s, r);
            let offset = subtract(r, p);
            let denominator = determinant(u, v);
            let (lu, lv) = (u[0].hypot(u[1]), v[0].hypot(v[1]));
            if lu <= tol || lv <= tol {
                return out;
            }
            if denominator.abs() <= 1e-12 * lu * lv {
                // Parallel: they meet only when collinear.
                if determinant(offset, u).abs() / lu <= tol {
                    for point in [p, q, r, s] {
                        if a.holds_along(point, tol) && b.holds_along(point, tol) {
                            out.push(point);
                        }
                    }
                }
                return out;
            }
            let t = determinant(offset, v) / denominator;
            let point = along(p, q, t);
            if a.holds_along(point, tol) && b.holds_along(point, tol) {
                out.push(point);
            }
        }
        (line @ Piece::Line(p, q), Piece::Arc(c, r, ..))
        | (Piece::Arc(c, r, ..), line @ Piece::Line(p, q)) => {
            let arc = if matches!(a, Piece::Arc(..)) { a } else { b };
            let u = subtract(q, p);
            let length = u[0].hypot(u[1]);
            if length <= tol {
                return out;
            }
            let unit = [u[0] / length, u[1] / length];
            let to = subtract(p, c);
            let half = to[0] * unit[0] + to[1] * unit[1];
            let rest = to[0] * to[0] + to[1] * to[1] - r * r;
            let discriminant = half * half - rest;
            // A tangent line has a discriminant near zero; its root is repeated.
            let reach = r.abs() * tol;
            if discriminant < -(2.0 * reach.max(tol * tol)) {
                return out;
            }
            let root = discriminant.max(0.0).sqrt();
            let roots = if root <= tol {
                vec![-half]
            } else {
                vec![-half - root, -half + root]
            };
            for t in roots {
                let point = [p[0] + unit[0] * t, p[1] + unit[1] * t];
                if line.holds_along(point, tol) && arc.holds_angle_of(point, tol) {
                    out.push(point);
                }
            }
        }
        (Piece::Arc(c1, r1, ..), Piece::Arc(c2, r2, ..)) => {
            let d = subtract(c2, c1);
            let distance = d[0].hypot(d[1]);
            if distance <= tol {
                if (r1 - r2).abs() <= tol {
                    for point in a.ends().into_iter().chain(b.ends()) {
                        if a.holds_angle_of(point, tol) && b.holds_angle_of(point, tol) {
                            out.push(point);
                        }
                    }
                }
                return out;
            }
            if distance > r1.abs() + r2.abs() + tol || distance < (r1.abs() - r2.abs()).abs() - tol
            {
                return out;
            }
            let x = (distance * distance + r1 * r1 - r2 * r2) / (2.0 * distance);
            let height = (r1 * r1 - x * x).max(0.0).sqrt();
            let base = [c1[0] + d[0] * x / distance, c1[1] + d[1] * x / distance];
            let side = [-d[1] / distance * height, d[0] / distance * height];
            let candidates = if height <= tol {
                vec![base]
            } else {
                vec![
                    [base[0] + side[0], base[1] + side[1]],
                    [base[0] - side[0], base[1] - side[1]],
                ]
            };
            for point in candidates {
                if a.holds_angle_of(point, tol) && b.holds_angle_of(point, tol) {
                    out.push(point);
                }
            }
        }
    }
    out
}

/// Whether a source loop made of lines and arcs crosses itself: two of its curves, other than
/// a curve and its own retrace, meet at a point that is not the endpoint they share where the
/// loop passes from one to the other. This is how several selected regions arrive when the
/// decoder merges them into one loop. A loop with any other kind of curve gives `false`.
pub(super) fn merged_regions(members: &[Value], entities: &[&Value]) -> bool {
    let mut pieces = Vec::with_capacity(members.len());
    for member in members {
        let Some(id) = member.get("entity").and_then(Value::as_str) else {
            return false;
        };
        let Some(entity) = entities
            .iter()
            .find(|entity| entity.get("id").and_then(Value::as_str) == Some(id))
        else {
            return false;
        };
        let Some(piece) = Piece::of(entity) else {
            return false;
        };
        pieces.push((id, piece));
    }
    // A curve the loop runs along twice (there and back) is a cut inside the loop, which the
    // region split handles; it does not mean regions were merged. Leave such curves out, so the
    // curves on either side of a cut still count as neighbours.
    let counts = pieces
        .iter()
        .fold(BTreeMap::<&str, usize>::new(), |mut counts, (id, _)| {
            *counts.entry(*id).or_default() += 1;
            counts
        });
    pieces.retain(|(id, _)| counts.get(id).copied() == Some(1));
    let mut low = [f64::MAX; 2];
    let mut high = [f64::MIN; 2];
    for (_, piece) in &pieces {
        let (centre, reach) = match *piece {
            Piece::Line(..) => ([0.0; 2], 0.0),
            Piece::Arc(c, r, ..) => (c, r.abs()),
        };
        let points: Vec<P2> = match *piece {
            Piece::Line(a, b) => vec![a, b],
            Piece::Arc(..) => vec![
                [centre[0] - reach, centre[1] - reach],
                [centre[0] + reach, centre[1] + reach],
            ],
        };
        for [x, y] in points {
            low = [low[0].min(x), low[1].min(y)];
            high = [high[0].max(x), high[1].max(y)];
        }
    }
    let tol = ((high[0] - low[0]).hypot(high[1] - low[1]) * 1e-9).max(1e-12);
    let count = pieces.len();
    for i in 0..count {
        for j in i + 1..count {
            let (Some(&(id_a, a)), Some(&(id_b, b))) = (pieces.get(i), pieces.get(j)) else {
                return false;
            };
            if id_a == id_b {
                continue;
            }
            let adjacent = j == i + 1 || (i == 0 && j + 1 == count);
            let near = |p: P2, q: P2| super::sketch::distance(p, q) <= tol.max(1e-9);
            for point in meetings(a, b, tol.max(1e-9)) {
                let shared = adjacent
                    && a.ends().iter().any(|e| near(*e, point))
                    && b.ends().iter().any(|e| near(*e, point));
                if !shared {
                    return true;
                }
            }
        }
    }
    false
}

/// Split straight sketch curves at their intersections, then walk the left side of
/// every directed edge. Each positive-area walk bounds an atomic region. The returned
/// ids still name the original curves: SolidCraft matches their pieces on recompute.
/// Curved arrangements and indistinguishable curve sets retain the refusal.
pub(super) fn split(members: &[Value], entities: &[&Value]) -> Option<Vec<Vec<String>>> {
    let selected = polygon(members, entities)?;
    let mut segments = Vec::new();
    for entity in entities {
        if entity.get("construction").and_then(Value::as_bool) == Some(true) {
            continue;
        }
        let geometry = entity.get("geometry")?;
        match geometry.get("kind")?.as_str()? {
            "point" | "reference_line" => continue,
            "line" => {
                if segments.len() >= MAX_LINES {
                    return None;
                }
                segments.push(Segment {
                    id: entity.get("id")?.as_str()?,
                    a: super::sketch::p2(geometry.get("start"))?,
                    b: super::sketch::p2(geometry.get("end"))?,
                    cuts: vec![0.0, 1.0],
                });
            }
            // A disjoint curved loop is harmless. Use its sampled bounding box with
            // generous padding; never approximate a curved intersection as a line.
            _ => {
                let points = samples(entity)?;
                if !disjoint(&selected, &points, tolerance(&points) * 10_000.0) {
                    return None;
                }
            }
        }
    }
    for i in 0..segments.len() {
        for j in i + 1..segments.len() {
            let a = segments.get(i)?;
            let b = segments.get(j)?;
            if let Some((t, s)) = intersection(a.a, a.b, b.a, b.b).ok()? {
                segments.get_mut(i)?.cuts.push(t);
                segments.get_mut(j)?.cuts.push(s);
            }
        }
    }
    let mut points: Vec<P2> = Vec::new();
    let mut links: Vec<(usize, usize, &str)> = Vec::new();
    for segment in &mut segments {
        segment.cuts.sort_by(f64::total_cmp);
        segment.cuts.dedup_by(|a, b| (*a - *b).abs() <= EPS);
        for pair in segment.cuts.windows(2) {
            let mut vertex = |t: f64| {
                let p = along(segment.a, segment.b, t);
                if let Some(index) = points
                    .iter()
                    .position(|q| super::sketch::distance(p, *q) <= EPS)
                {
                    index
                } else {
                    points.push(p);
                    points.len() - 1
                }
            };
            let a = vertex(*pair.first()?);
            let b = vertex(*pair.get(1)?);
            if a == b || links.len() >= MAX_EDGES {
                return None;
            }
            links.push((a, b, segment.id));
            links.push((b, a, segment.id));
        }
    }
    let mut outgoing = vec![Vec::new(); points.len()];
    for (index, &(a, _, _)) in links.iter().enumerate() {
        outgoing.get_mut(a)?.push(index);
    }
    for (vertex, edges) in outgoing.iter_mut().enumerate() {
        let a = *points.get(vertex)?;
        let angle = |index: &usize| {
            let b = links
                .get(*index)
                .and_then(|(_, b, _)| points.get(*b))
                .copied()
                .unwrap_or(a);
            (b[1] - a[1]).atan2(b[0] - a[0])
        };
        edges.sort_by(|a, b| angle(a).total_cmp(&angle(b)));
    }
    // Only components connected to the source boundary belong to its arrangement.
    // Disconnected nested loops remain holes, which SolidCraft finds from the sketch.
    let source_ids: BTreeSet<&str> = members
        .iter()
        .filter_map(|member| member.get("entity")?.as_str())
        .collect();
    let mut connected = BTreeSet::new();
    let mut pending: Vec<usize> = links
        .iter()
        .filter(|(_, _, id)| source_ids.contains(id))
        .map(|(a, _, _)| *a)
        .collect();
    while let Some(vertex) = pending.pop() {
        if connected.insert(vertex) {
            for &edge in outgoing.get(vertex)? {
                pending.push(links.get(edge)?.1);
            }
        }
    }
    let mut visited = vec![false; links.len()];
    let mut faces = Vec::new();
    for start in 0..links.len() {
        if *visited.get(start)? {
            continue;
        }
        let mut edge = start;
        let mut ring = Vec::new();
        let mut ids = BTreeSet::new();
        loop {
            if *visited.get(edge)? {
                if edge != start {
                    return None;
                }
                break;
            }
            *visited.get_mut(edge)? = true;
            let &(a, b, id) = links.get(edge)?;
            ring.push(*points.get(a)?);
            ids.insert(id.to_owned());
            let exits = outgoing.get(b)?;
            let reverse = edge ^ 1;
            let index = exits.iter().position(|candidate| *candidate == reverse)?;
            edge = *exits.get((index + exits.len() - 1) % exits.len())?;
        }
        let area = edges(&ring).map(|(a, b)| determinant(a, b)).sum::<f64>();
        if area > EPS && connected.contains(&links.get(start)?.0) {
            let a = *ring.first()?;
            let b = *ring.get(1)?;
            let delta = subtract(b, a);
            let size = delta[0].hypot(delta[1]);
            let middle = along(a, b, 0.5);
            let pick = [
                middle[0] - delta[1] / size * EPS,
                middle[1] + delta[0] / size * EPS,
            ];
            faces.push((ids.into_iter().collect::<Vec<_>>(), inside(pick, &selected)));
        }
    }
    // Curve-based selections must match exactly one atomic region, including regions
    // outside the chosen source loop. Two halves sharing the same ids are ambiguous.
    let mut out = Vec::new();
    for (ids, selected) in &faces {
        if *selected {
            if faces.iter().filter(|(other, _)| other == ids).count() != 1 {
                return None;
            }
            out.push(ids.clone());
        }
    }
    (!out.is_empty()).then_some(out)
}

fn disjoint(a: &[P2], b: &[P2], padding: f64) -> bool {
    (0..2).any(|axis| {
        let bounds = |points: &[P2]| {
            points
                .iter()
                .filter_map(|p| p.get(axis))
                .fold((f64::INFINITY, f64::NEG_INFINITY), |(low, high), &p| {
                    (low.min(p), high.max(p))
                })
        };
        let (a_low, a_high) = bounds(a);
        let (b_low, b_high) = bounds(b);
        a_high + padding < b_low || b_high + padding < a_low
    })
}

/// An opaque native pick is unambiguous only when even-odd nesting leaves one
/// material region: one outer loop and its immediate holes, with no islands.
pub(super) fn single_region(sketch: &Value, entities: &[&Value]) -> Option<usize> {
    if sketch.get("profiles")?.as_array()?.len() > MAX_LINES {
        return None;
    }
    let loops = loops(sketch);
    if loops.is_empty() || loops.len() > MAX_LINES {
        return None;
    }
    let boundary: BTreeSet<&str> = loops
        .iter()
        .flat_map(|members| members.iter())
        .filter_map(|member| member.get("entity")?.as_str())
        .collect();
    let construction = crossing(sketch, entities);
    for entity in entities {
        let id = entity.get("id")?.as_str()?;
        let kind = entity.get("geometry")?.get("kind")?.as_str()?;
        if !boundary.contains(id)
            && !construction.contains(id)
            && entity.get("construction").and_then(Value::as_bool) != Some(true)
            && !matches!(kind, "point" | "reference_line")
        {
            return None;
        }
    }
    let polygons = loops
        .iter()
        .map(|members| polygon(members, entities))
        .collect::<Option<Vec<_>>>()?;
    if polygons.iter().map(Vec::len).sum::<usize>() > MAX_POINTS
        || polygons.iter().any(|ring| self_crossing(ring))
    {
        return None;
    }
    // Conservative clearance covers the chord error of the sampled circular curves.
    // Touching, crossing and coincident loops cannot supply a unique nesting tree.
    for (i, a) in polygons.iter().enumerate() {
        for b in polygons.iter().skip(i + 1) {
            let clearance = tolerance(a).max(tolerance(b)) * 10_000.0;
            if edges(a).any(|(p, q)| {
                on_boundary(p, b, clearance)
                    || edges(b)
                        .any(|(r, s)| !intersection(p, q, r, s).is_ok_and(|hit| hit.is_none()))
            }) || b.iter().any(|&p| on_boundary(p, a, clearance))
            {
                return None;
            }
        }
    }
    let mut outer = None;
    let mut regions = 0;
    for (i, ring) in polygons.iter().enumerate() {
        let p = *ring.first()?;
        let depth = polygons
            .iter()
            .enumerate()
            .filter(|(j, other)| *j != i && inside(p, other))
            .count();
        if depth % 2 == 0 {
            regions += 1;
        }
        if depth == 0 {
            outer = Some(i);
        }
    }
    (regions == 1).then_some(outer).flatten()
}

/// The area (mm squared) of the region the source loops `wanted` (indices into the sketch's
/// profile table) bound under even-odd nesting: material minus the holes inside it. Arcs and
/// circles are followed as fine polygons. `None` when a loop cannot be followed, crosses itself,
/// or touches or crosses another selected loop.
pub(super) fn selected_area(sketch: &Value, entities: &[&Value], wanted: &[usize]) -> Option<f64> {
    let loops = loops(sketch);
    if wanted.is_empty() || wanted.len() > MAX_LINES || loops.len() > MAX_LINES {
        return None;
    }
    let polygons = wanted
        .iter()
        .map(|&index| polygon(loops.get(index)?, entities))
        .collect::<Option<Vec<_>>>()?;
    if polygons.iter().map(Vec::len).sum::<usize>() > MAX_POINTS
        || polygons.iter().any(|ring| self_crossing(ring))
    {
        return None;
    }
    for (i, a) in polygons.iter().enumerate() {
        for b in polygons.iter().skip(i + 1) {
            let clearance = tolerance(a).max(tolerance(b)) * 10_000.0;
            if edges(a).any(|(p, q)| {
                on_boundary(p, b, clearance)
                    || edges(b)
                        .any(|(r, s)| !intersection(p, q, r, s).is_ok_and(|hit| hit.is_none()))
            }) || b.iter().any(|&p| on_boundary(p, a, clearance))
            {
                return None;
            }
        }
    }
    let mut total = 0.0;
    for (i, ring) in polygons.iter().enumerate() {
        let point = ring.first().copied()?;
        let depth = polygons
            .iter()
            .enumerate()
            .filter(|(j, other)| *j != i && inside(point, other))
            .count();
        let area = ring_area(ring);
        total += if depth % 2 == 0 { area } else { -area };
    }
    (total > 0.0).then_some(total)
}

/// The absolute area of a closed polygon.
pub(super) fn ring_area(ring: &[P2]) -> f64 {
    let sum: f64 = ring
        .iter()
        .zip(ring.iter().cycle().skip(1))
        .map(|(a, b)| a[0] * b[1] - b[0] * a[1])
        .sum();
    sum.abs() / 2.0
}

/// The outer loops and holes of the source loops `wanted` (indices into the sketch's profile
/// table) under even-odd nesting: a selected loop inside an odd number of other selected
/// loops is a hole of the nearest one around it, one inside an even number is material. `None`
/// when no selected loop lies inside another (nothing to nest), or when the loops touch,
/// cross or cannot be followed, so no unique nesting tree exists.
pub(super) fn nesting(
    sketch: &Value,
    entities: &[&Value],
    wanted: &[usize],
) -> Option<Vec<(usize, Vec<usize>)>> {
    let loops = loops(sketch);
    if wanted.len() < 2 || wanted.len() > MAX_LINES || loops.len() > MAX_LINES {
        return None;
    }
    let polygons = wanted
        .iter()
        .map(|&index| polygon(loops.get(index)?, entities))
        .collect::<Option<Vec<_>>>()?;
    if polygons.iter().map(Vec::len).sum::<usize>() > MAX_POINTS
        || polygons.iter().any(|ring| self_crossing(ring))
    {
        return None;
    }
    // The same conservative clearance as `single_region`: loops that touch, cross or share a
    // boundary supply no unique nesting tree.
    for (i, a) in polygons.iter().enumerate() {
        for b in polygons.iter().skip(i + 1) {
            let clearance = tolerance(a).max(tolerance(b)) * 10_000.0;
            if edges(a).any(|(p, q)| {
                on_boundary(p, b, clearance)
                    || edges(b)
                        .any(|(r, s)| !intersection(p, q, r, s).is_ok_and(|hit| hit.is_none()))
            }) || b.iter().any(|&p| on_boundary(p, a, clearance))
            {
                return None;
            }
        }
    }
    let around: Vec<Vec<usize>> = polygons
        .iter()
        .enumerate()
        .map(|(i, ring)| {
            let p = ring.first().copied();
            polygons
                .iter()
                .enumerate()
                .filter(|(j, other)| *j != i && p.is_some_and(|p| inside(p, other)))
                .map(|(j, _)| j)
                .collect()
        })
        .collect();
    if around.iter().all(Vec::is_empty) {
        return None;
    }
    let mut out = Vec::new();
    for (i, containing) in around.iter().enumerate() {
        if containing.len() % 2 == 0 {
            let holes: Vec<usize> = around
                .iter()
                .enumerate()
                .filter(|(_, other)| other.len() == containing.len() + 1 && other.contains(&i))
                .filter_map(|(j, _)| wanted.get(j).copied())
                .collect();
            out.push((*wanted.get(i)?, holes));
        }
    }
    Some(out)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn curves_bounding_a_selected_trimmed_region_are_named() {
        let sketch = json!({"id": "s"});
        let features = vec![
            json!({"id": "e", "definition": {"definition": "extrude", "profile": {
                "kind": "sketch_regions", "value": {"sketch": "s", "regions": [
                    {"region": "trimmed",
                     "outer_boundary": [{"entity": "top"}, {"entity": "divider"}],
                     "hole_boundaries": [[{"entity": "circle"}]]},
                    {"region": "loops", "outer": 0}]}}}}),
            json!({"id": "f", "definition": {"definition": "extrude", "profile": {
                "kind": "sketch_regions", "value": {"sketch": "other", "regions": [
                    {"region": "trimmed", "outer_boundary": [{"entity": "elsewhere"}]}]}}}}),
        ];
        let found = trimmed_region_curves(&sketch, &features);
        assert_eq!(
            found.into_iter().collect::<Vec<_>>(),
            ["circle", "divider", "top"]
        );
    }

    fn line(id: &str, a: P2, b: P2) -> Value {
        json!({"id": id, "geometry": {"kind": "line",
            "start": {"u": a[0], "v": a[1]}, "end": {"u": b[0], "v": b[1]}}})
    }

    fn member(id: &str) -> Value {
        json!({"entity": id, "reversed": false})
    }

    fn pieces_loop(ids: &[&str]) -> Vec<Value> {
        ids.iter().map(|id| member(id)).collect()
    }

    #[test]
    fn a_bow_tie_loop_is_several_regions_merged_and_a_square_is_not() {
        let bow = [
            line("a", [0.0, 0.0], [4.0, 4.0]),
            line("b", [4.0, 4.0], [4.0, 0.0]),
            line("c", [4.0, 0.0], [0.0, 4.0]),
            line("d", [0.0, 4.0], [0.0, 0.0]),
        ];
        let bow: Vec<&Value> = bow.iter().collect();
        assert!(merged_regions(&pieces_loop(&["a", "b", "c", "d"]), &bow));

        let square = [
            line("a", [0.0, 0.0], [4.0, 0.0]),
            line("b", [4.0, 0.0], [4.0, 4.0]),
            line("c", [4.0, 4.0], [0.0, 4.0]),
            line("d", [0.0, 4.0], [0.0, 0.0]),
            line("cross", [-1.0, 2.0], [5.0, 2.0]),
        ];
        let square: Vec<&Value> = square.iter().collect();
        // A separate line across a square is not the loop crossing itself.
        assert!(!merged_regions(
            &pieces_loop(&["a", "b", "c", "d"]),
            &square
        ));
    }

    #[test]
    fn retraced_cut_lines_are_not_merged_regions() {
        let lines = [
            line("a", [0.0, 0.0], [10.0, 0.0]),
            line("d1", [10.0, 0.0], [0.0, 10.0]),
            line("d2", [0.0, 0.0], [10.0, 10.0]),
        ];
        let lines: Vec<&Value> = lines.iter().collect();
        // Both diagonals run there and back: they cross, but as cuts, not merged regions.
        assert!(!merged_regions(
            &pieces_loop(&["a", "d1", "d1", "d2", "d2"]),
            &lines
        ));
    }

    #[test]
    fn arcs_that_cross_a_line_of_their_own_loop_are_found() {
        let arc = json!({"id": "arc", "geometry": {"kind": "arc",
            "center": {"u": 0.0, "v": 0.0}, "radius": 2.0,
            "start_angle": 0.0, "end_angle": std::f64::consts::PI}});
        // The chord runs past the arc on both sides, crossing it twice between ends.
        let chord = line("chord", [-3.0, 1.0], [3.0, 1.0]);
        let edge = line("edge", [-2.0, 0.0], [2.0, 0.0]);
        let all = [arc, chord, edge];
        let all: Vec<&Value> = all.iter().collect();
        assert!(merged_regions(
            &pieces_loop(&["arc", "edge", "chord"]),
            &all
        ));
        assert!(!merged_regions(&pieces_loop(&["arc", "edge"]), &all));
    }

    #[test]
    fn circle_crossings_require_a_proper_secant() {
        let circle = json!({"geometry": {"kind": "circle",
            "center": {"u": 0.0, "v": 0.0}, "radius": 2.0}});
        for (start, end, crosses) in [
            ([-3.0, 0.0], [3.0, 0.0], true),
            ([-3.0, -3.0], [3.0, 3.0], true),
            ([-3.0, 2.0], [3.0, 2.0], false),
            ([-3.0, 3.0], [3.0, 3.0], false),
            ([-3.0, 0.0], [0.0, 0.0], false),
            ([0.0, 0.0], [0.0, 0.0], false),
        ] {
            assert_eq!(cuts_circle(&line("line", start, end), &circle), crosses);
        }
    }

    #[test]
    fn diagonals_of_a_square_taken_whole_are_construction() {
        let s = 10.0;
        let entities = [
            line("e1", [s, -s], [-s, -s]),
            line("e2", [-s, -s], [-s, s]),
            line("e3", [-s, s], [s, s]),
            line("e4", [s, s], [s, -s]),
            line("d1", [s, -s], [-s, s]),
            line("d2", [-s, -s], [s, s]),
            // From the boundary to outside, and wholly inside: neither splits the square.
            line("out", [s, 0.0], [2.0 * s, 0.0]),
            line("in", [-1.0, 0.0], [1.0, 0.0]),
            // Along an edge: on the boundary all the way.
            line("edge", [s, -s], [s, s]),
        ];
        let refs: Vec<&Value> = entities.iter().collect();
        let sketch = json!({"profiles": [[member("e1"), member("e2"), member("e3"),
                                          member("e4")]]});
        let crossing = crossing(&sketch, &refs);
        assert_eq!(
            crossing.iter().map(String::as_str).collect::<Vec<_>>(),
            ["d1", "d2"]
        );
    }

    #[test]
    fn a_line_already_construction_or_bounding_a_profile_is_left_alone() {
        let mut d1 = line("d1", [10.0, 0.0], [0.0, 10.0]);
        d1["construction"] = json!(true);
        let entities = [
            line("e1", [0.0, 0.0], [10.0, 0.0]),
            line("e2", [10.0, 0.0], [0.0, 10.0]),
            line("e3", [0.0, 10.0], [0.0, 0.0]),
            d1,
        ];
        let refs: Vec<&Value> = entities.iter().collect();
        let sketch = json!({"profiles": [[member("e1"), member("e2"), member("e3")]]});
        assert!(crossing(&sketch, &refs).is_empty());
    }

    #[test]
    fn a_chord_across_a_circle_taken_whole_is_construction() {
        let circle = json!({"id": "c", "geometry": {"kind": "circle",
            "center": {"u": 0.0, "v": 0.0}, "radius": 5.0}});
        let entities = [circle, line("chord", [-5.0, 0.0], [5.0, 0.0])];
        let refs: Vec<&Value> = entities.iter().collect();
        let sketch = json!({"profiles": [[member("c")]]});
        assert_eq!(crossing(&sketch, &refs).len(), 1);
    }

    /// A square loop (a to d) and a tab loop beside it (t1 to t4), with a line (x) in no loop.
    fn square_and_tab() -> (Value, Vec<Value>) {
        let sketch = json!({"id": "s", "profiles": [
            pieces_loop(&["a", "b", "c", "d"]), pieces_loop(&["t1", "t2", "t3", "t4"])]});
        let entities = vec![
            line("a", [0.0, 0.0], [10.0, 0.0]),
            line("b", [10.0, 0.0], [10.0, 10.0]),
            line("c", [10.0, 10.0], [0.0, 10.0]),
            line("d", [0.0, 10.0], [0.0, 0.0]),
            line("t1", [10.0, 3.0], [14.0, 3.0]),
            line("t2", [14.0, 3.0], [14.0, 7.0]),
            line("t3", [14.0, 7.0], [10.0, 7.0]),
            line("t4", [10.0, 7.0], [10.0, 3.0]),
            line("x", [5.0, 5.0], [12.0, 5.0]),
        ];
        (sketch, entities)
    }

    fn extrude(regions: Value) -> Value {
        json!({"id": "e", "definition": {"definition": "extrude", "profile": {
            "kind": "sketch_regions", "value": {"sketch": "s", "regions": regions}}}})
    }

    fn strays(sketch: &Value, entities: &[Value], features: &[Value]) -> Vec<String> {
        let refs: Vec<&Value> = entities.iter().collect();
        stray_curves(sketch, &refs, features).into_iter().collect()
    }

    #[test]
    fn a_loop_no_feature_takes_is_construction_with_the_curves_in_no_loop() {
        let (sketch, entities) = square_and_tab();
        let features = [extrude(json!([{"region": "loops", "outer": 0}]))];
        assert_eq!(
            strays(&sketch, &entities, &features),
            ["t1", "t2", "t3", "t4", "x"]
        );
        // Both loops taken: only the line in no loop.
        let features = [extrude(
            json!([{"region": "loops", "outer": 0}, {"region": "loops", "outer": 1}]),
        )];
        assert_eq!(strays(&sketch, &entities, &features), ["x"]);
    }

    #[test]
    fn a_list_of_loops_counts_its_loops_and_the_whole_sketch_keeps_every_curve() {
        let (sketch, entities) = square_and_tab();
        let listed = json!({"id": "e", "definition": {"definition": "extrude", "profile": {
            "kind": "sketch_profiles", "value": {"sketch": "s", "profiles": [0]}}}});
        assert_eq!(
            strays(&sketch, &entities, &[listed]),
            ["t1", "t2", "t3", "t4", "x"]
        );
        // The whole sketch takes every region its curves bound, which the profile table may
        // not list in full: nothing is inferred.
        let whole = json!({"id": "e", "definition": {"definition": "extrude", "profile": {
            "kind": "sketch", "value": "s"}}});
        assert!(strays(&sketch, &entities, &[whole]).is_empty());
    }

    #[test]
    fn the_curves_of_a_trimmed_region_taken_stay_solid() {
        let (sketch, entities) = square_and_tab();
        // Line ranges are fractions of the line: the tab's three outer sides whole, and the
        // stretch of the square's side between them.
        let trimmed = json!({"region": "trimmed", "outer_boundary": [
            {"entity": "t1", "parameter_range": [0.0, 1.0]},
            {"entity": "t2", "parameter_range": [0.0, 1.0]},
            {"entity": "t3", "parameter_range": [0.0, 1.0]},
            {"entity": "b", "parameter_range": [0.3, 0.7]}]});
        let features = [
            extrude(json!([{"region": "loops", "outer": 0}])),
            extrude(json!([trimmed])),
        ];
        assert_eq!(strays(&sketch, &entities, &features), ["t4", "x"]);
        // A trimmed region with no boundary, or one that doesn't close (the curves finishing
        // it are not named), says nothing about which curves it needs.
        for boundary in [
            json!([]),
            json!([{"entity": "t1", "parameter_range": [0.0, 1.0]},
                   {"entity": "t2", "parameter_range": [0.0, 1.0]}]),
        ] {
            let features = [
                extrude(json!([{"region": "loops", "outer": 0}])),
                extrude(json!([{"region": "trimmed", "outer_boundary": boundary}])),
            ];
            assert!(strays(&sketch, &entities, &features).is_empty());
        }
    }

    #[test]
    fn a_line_range_is_a_fraction_of_the_line() {
        let (_, entities) = square_and_tab();
        let refs: Vec<&Value> = entities.iter().collect();
        let tab = |piece: [f64; 2]| {
            json!([{"entity": "t1", "parameter_range": [0.0, 1.0]},
                   {"entity": "t2", "parameter_range": [0.0, 1.0]},
                   {"entity": "t3", "parameter_range": [0.0, 1.0]},
                   {"entity": "b", "parameter_range": piece}])
        };
        // 30% to 70% of the square's 10 mm side closes the 4 mm tab.
        let ring = trimmed_ring(tab([0.3, 0.7]).as_array().unwrap(), &refs).unwrap();
        assert!((ring_area(&ring).abs() - 16.0).abs() < 1e-6, "{ring:?}");
        // Read as millimetres, the same stretch would run past the line's end.
        assert!(trimmed_ring(tab([3.0, 7.0]).as_array().unwrap(), &refs).is_none());
    }
}
