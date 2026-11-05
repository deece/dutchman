//! Profile crossings: inferred construction lines, split regions and loop nesting.
//!
//! A sketch's profile table lists the closed regions its curves bound. A line from one point of
//! a region's boundary to another, through its inside, splits that region in two, so a region
//! the table lists whole cannot be crossed by one, unless the line is construction geometry. A
//! decoder that does not know which curves are construction (cadmpeg's Inventor decoder, for
//! now) leaves such a line solid; SolidCraft then finds the region split and no profile it can
//! match. Such lines are construction, and are written so.
//! Explicit whole-loop region consumers also identify unused layout lines, provided no other
//! consumer needs those lines as profile geometry.
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

/// The curves that the sketch's consumers (every feature naming it) select whole-loop regions
/// from, when every consumer is such a selection: the source entities of the selected loops.
/// `None` when some consumer needs the sketch's curves as geometry, or nothing selects a region.
fn whole_loop_uses(sketch: &Value, features: &[Value]) -> Option<BTreeSet<String>> {
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
        let found = region_uses(definition, id, &identities, sketch, &mut used, 0)?;
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

/// Curves no closed loop of the sketch's profile table uses, when every consumer of the sketch
/// selects whole-loop regions and none of them names the curve. They bound no region, so
/// solid they only divide the regions that are selected; they are construction.
pub(super) fn stray_curves(
    sketch: &Value,
    entities: &[&Value],
    features: &[Value],
) -> BTreeSet<String> {
    let in_loops: BTreeSet<&str> = loops(sketch)
        .into_iter()
        .flatten()
        .filter_map(|member| member.get("entity").and_then(Value::as_str))
        .collect();
    if in_loops.is_empty() || whole_loop_uses(sketch, features).is_none() {
        return BTreeSet::new();
    }
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
        .filter(|id| !in_loops.contains(id))
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
    let Some(used) = whole_loop_uses(sketch, features) else {
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
    used: &mut BTreeSet<String>,
    depth: usize,
) -> Option<bool> {
    if depth > MAX_SELECTION_DEPTH {
        return None;
    }
    if value.get("kind").and_then(Value::as_str) == Some("sketch_regions")
        && value.get("value")?.get("sketch").and_then(Value::as_str) == Some(id)
    {
        let regions = value.get("value")?.get("regions")?.as_array()?;
        if regions.is_empty() || regions.len() > MAX_LINES {
            return None;
        }
        let profiles = sketch.get("profiles")?.as_array()?;
        let mut members = 0usize;
        for region in regions {
            if region.get("region").and_then(Value::as_str) != Some("loops") {
                return None;
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
            Some(region_uses(value, id, identities, sketch, used, depth + 1)? || found)
        }),
        Value::Object(fields) => fields.values().try_fold(false, |found, value| {
            Some(region_uses(value, id, identities, sketch, used, depth + 1)? || found)
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

    fn line(id: &str, a: P2, b: P2) -> Value {
        json!({"id": id, "geometry": {"kind": "line",
            "start": {"u": a[0], "v": a[1]}, "end": {"u": b[0], "v": b[1]}}})
    }

    fn member(id: &str) -> Value {
        json!({"entity": id, "reversed": false})
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
}
