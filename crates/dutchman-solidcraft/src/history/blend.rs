//! Blends picked exclusively from their own historical input-edge geometry, including
//! stored base-feature edges preserved by the core before hidden shapes are removed, on the bodies
//! SolidCraft rebuilds from the history before the blend, never on the final body.
//!
//! Mapping keeps each selected input edge ([`Pending`]); once SolidCraft has replayed the
//! mapped history ([`crate::verify::replay_edges`]), [`prove`] finds it among the edges of the
//! bodies the blend starts from. SolidCraft reports each rebuilt edge's ends, the point
//! halfway along it and its length, but no carrier or tangent, so the rule is:
//!
//! - The input edge must be a straight line (its midpoint on the segment between its ends and
//!   its tangent along that segment) or a circular arc (its tangent square to the radius of
//!   the circle through its ends and midpoint, in that circle's plane). Other curves and
//!   closed edges are refused.
//! - Its midpoint must lie on exactly one rebuilt edge across every body: within the segment
//!   of a straight one (a length equal to its chord proves it straight), or at the midpoint of
//!   a curved one. Anything else, including two rebuilt edges through it, is refused.
//! - That edge must have the input edge's ends, in either order, and its length: the chord of
//!   a line, `radius × angle` of an arc. A straight rebuilt edge then lies along the input
//!   tangent; a curved one is not checked for a tangent SolidCraft does not report.

use dutchman_core::{Report, Severity};
use serde_json::{json, Map, Value};

use super::sketch::{cross, dot, number, unit, v3, Mapped, V3};
use super::{describe, name, Context};
use crate::verify::{ReplayedBodies, ReplayedEdge};

const LIMIT: usize = 256;
/// How far apart, in millimetres, two positions may be and still be the same.
const TOLERANCE: f64 = 1e-6;
/// How far from square or parallel two unit directions may be.
const ANGLE: f64 = 1e-6;
const INPUT: &str =
    "its input edge midpoint cannot be proven to select exactly one rebuilt edge yet";

type Segment = (V3, V3);

/// A blend whose input edges wait for SolidCraft's replay of the history before it.
#[derive(Clone, Debug)]
pub(crate) struct Pending {
    /// The blend's index in the mapped features.
    pub(crate) feature: usize,
    /// Its source feature's index in the mapping's outcomes.
    pub(crate) outcome: usize,
    /// Its source feature, as the report names it.
    pub(crate) subject: String,
    pub(super) edges: Vec<Input>,
    /// A 45-degree chamfer, written as an equal-distance one.
    pub(super) angle_chamfer: bool,
    /// For an extrude of a face of the model as it was earlier, instead of a blend: the face
    /// to find ([`super::face_profile`]); `edges` is then empty.
    pub(super) face: Option<super::face_profile::Proof>,
    /// The blend is the `n`th extra feature its source feature emits before its main one.
    pub(super) extra: Option<usize>,
    /// An earlier group of the same source blend may have shortened its straight edges.
    pub(super) trimmed: bool,
}

/// One selected input edge, from its feature input topology.
#[derive(Clone, Debug)]
pub(super) struct Input {
    /// The point at the middle of its carrier's parameter interval.
    point: V3,
    ends: Segment,
    tangent: V3,
    /// Its length: the chord of a line, or the arc's.
    length: f64,
    straight: bool,
}

pub(super) fn map(
    feature: &Value,
    definition: &Value,
    ctx: &mut Context,
    report: &mut Report,
) -> Result<Value, String> {
    let kind = definition
        .get("definition")
        .and_then(Value::as_str)
        .ok_or("its blend kind is not resolved")?;
    let groups = definition
        .get("groups")
        .and_then(Value::as_array)
        .filter(|groups| !groups.is_empty() && groups.len() <= LIMIT)
        .ok_or("its edge groups are missing")?;
    // SolidCraft blends take one size each: one feature per group, in source order. A later
    // group's edges may be shortened by an earlier one where they meet.
    let mut mapped = Vec::new();
    for (index, group) in groups.iter().enumerate() {
        mapped.push(self::group(feature, kind, group, ctx, report, index > 0)?);
    }
    let last = mapped.len().saturating_sub(1);
    let mut main = None;
    for (index, (out, mut pending)) in mapped.into_iter().enumerate() {
        if index == last {
            main = Some(out);
        } else {
            pending.extra = Some(ctx.extra.len());
            ctx.extra.push(out);
        }
        ctx.blends.push(pending);
    }
    main.ok_or_else(|| "its edge groups are missing".to_owned())
}

/// One edge group of a blend as a SolidCraft blend, with the proof its edges wait for.
fn group(
    feature: &Value,
    kind: &str,
    group: &Value,
    ctx: &mut Context,
    report: &mut Report,
    trimmed: bool,
) -> Result<(Value, Pending), String> {
    let mut out = Map::new();
    out.insert("type".to_owned(), json!(kind));
    out.insert("suppressed".to_owned(), json!(false));
    // A tangency weight of exactly 1 blends with a circular cross-section: the fillet's faces
    // are circular cylinders and tori, as an unweighted fillet's are.
    let weighted = match group.get("tangency_weight").filter(|v| !v.is_null()) {
        None => false,
        Some(weight) if kind == "fillet" && number(Some(weight)) == Some(1.0) => true,
        Some(_) if kind == "fillet" => {
            return Err("fillets with a tangency weight are not supported yet".to_owned())
        }
        Some(_) => false,
    };
    if kind == "fillet" {
        let law = group.get("radius").ok_or("its radius is not given")?;
        match law.get("kind").and_then(Value::as_str) {
            Some("constant") => {
                out.insert("radius".to_owned(), json!(length(law.get("radius"))?));
                out.insert("style".to_owned(), json!({"type": "constant"}));
            }
            Some("variable") => {
                return Err(
                    "variable fillets need a proven chain direction, which is not supported yet"
                        .to_owned(),
                );
            }
            _ => {
                return Err("fillets other than constant-radius are not supported yet".to_owned());
            }
        }
    } else {
        let spec = group
            .get("spec")
            .ok_or("its chamfer dimensions are not given")?;
        let distance = match spec.get("kind").and_then(Value::as_str) {
            Some("distance") => spec.get("distance"),
            Some("two_distances") if number(spec.get("first")) == number(spec.get("second")) => {
                spec.get("first")
            }
            Some("distance_angle")
                if number(spec.get("angle"))
                    .is_some_and(|a| (a - std::f64::consts::FRAC_PI_4).abs() <= 1e-12) =>
            {
                spec.get("distance")
            }
            Some("two_distances" | "distance_angle") => {
                return Err("asymmetric chamfers need an input reference face ordering, which is not supported yet".to_owned());
            }
            _ => return Err("its chamfer dimensions are not resolved".to_owned()),
        };
        out.insert("distance".to_owned(), json!(length(distance)?));
    }
    let selection = group.get("edges").ok_or("its input edges are not given")?;
    let angle_chamfer =
        group.pointer("/spec/kind").and_then(Value::as_str) == Some("distance_angle");
    let (inputs, points) = match selection.get("kind").and_then(Value::as_str) {
        Some("historical") => historical(feature, selection, ctx, angle_chamfer)?,
        Some("generated") => generated(selection, ctx)?,
        found @ Some("native" | "unresolved") => {
            return Err(super::unsupported_selection("edge", found))
        }
        Some("historical_partial") => {
            return Err("some of its edges are not resolved in its input topology".to_owned())
        }
        Some("resolved") => {
            return Err("its resolved edges are not proven members of an earlier stored feature result; that edge selection is not supported yet".to_owned())
        }
        _ => return Err("blends require edges from their own feature input topology; edges of the final body are not supported yet".to_owned()),
    };
    out.insert("edges".to_owned(), json!(points));
    let id = ctx.id();
    out.insert("id".to_owned(), json!(id));
    out.insert(
        "name".to_owned(),
        json!(name(feature)
            .map(str::to_owned)
            .unwrap_or_else(|| format!("Blend{id}"))),
    );
    let pending = Pending {
        feature: 0,
        outcome: 0,
        subject: String::new(),
        edges: inputs,
        angle_chamfer,
        face: None,
        extra: None,
        trimmed,
    };
    if weighted {
        report.push(
            Severity::Info,
            describe(feature),
            "tangency weight 1.0 treated as a circular fillet",
        );
    }
    Ok((Value::Object(out), pending))
}

/// Input edges selected in the blend's own feature input topology, with their midpoints.
fn historical(
    feature: &Value,
    selection: &Value,
    ctx: &Context,
    angle_chamfer: bool,
) -> Result<(Vec<Input>, Vec<V3>), String> {
    let state_id = selection
        .pointer("/value/state")
        .and_then(Value::as_str)
        .ok_or("its input topology is not resolved")?;
    let states = ctx.arena("feature_input_topologies");
    if states.len() > 10_000 {
        return Err("its input topology exceeds the blend resolution limit".to_owned());
    }
    let mut states = states
        .iter()
        .filter(|state| state.get("id").and_then(Value::as_str) == Some(state_id));
    let state = states.next().ok_or("its input topology is missing")?;
    if states.next().is_some() || state.get("input_of") != feature.get("id") {
        return Err("its input topology does not uniquely belong to this feature".to_owned());
    }
    let ids = selection
        .pointer("/value/edges")
        .and_then(Value::as_array)
        .filter(|ids| !ids.is_empty() && ids.len() <= LIMIT)
        .ok_or("its input edge selection is empty or exceeds the blend limit")?;
    let members = state
        .get("edges")
        .and_then(Value::as_array)
        .ok_or("its input edges are missing")?;
    let geometry = state
        .get("edge_geometry")
        .and_then(Value::as_array)
        .ok_or("its input edge geometry is missing")?;
    if members.len() > 10_000 || geometry.len() > 10_000 {
        return Err("its input topology exceeds the blend resolution limit".to_owned());
    }
    let mut inputs = Vec::new();
    let mut points = Vec::new();
    for edge in ids {
        if !edge.is_string() || members.iter().filter(|member| *member == edge).count() != 1 {
            return Err(
                "its selected edge is not a unique member of its input topology".to_owned(),
            );
        }
        let mut samples = geometry
            .iter()
            .filter(|sample| sample.get("edge") == Some(edge));
        let sample = samples
            .next()
            .ok_or("its selected input edge has no geometry")?;
        if samples.next().is_some() {
            return Err("its selected input edge has repeated geometry".to_owned());
        }
        let point = v3(sample.get("interior")).ok_or("its input edge midpoint is missing")?;
        let ends = (
            v3(sample.get("start")).ok_or("its input edge start is missing")?,
            v3(sample.get("end")).ok_or("its input edge end is missing")?,
        );
        let tangent = v3(sample.get("tangent"))
            .and_then(unit)
            .ok_or("its input edge tangent is missing")?;
        if near(ends.0, ends.1) {
            return Err("blends on closed input edges are not supported yet".to_owned());
        }
        if near(point, ends.0) || near(point, ends.1) {
            return Err(INPUT.to_owned());
        }
        let input = if on_segment(point, ends) {
            if !parallel(sub(ends.1, ends.0), tangent) {
                return Err(INPUT.to_owned());
            }
            Input {
                point,
                ends,
                tangent,
                length: distance(ends.0, ends.1),
                straight: true,
            }
        } else {
            let length = arc_length(point, ends, tangent).ok_or(
                "blends on curved input edges other than circular arcs are not supported yet",
            )?;
            Input {
                point,
                ends,
                tangent,
                length,
                straight: false,
            }
        };
        if angle_chamfer && !input.straight {
            return Err("distance-and-angle chamfers on non-perpendicular input faces are not supported yet".to_owned());
        }
        inputs.push(input);
        points.push(point);
    }
    Ok((inputs, points))
}

/// Edges an extrusion or a sheet-metal base flange generated from its sketch, as cadmpeg names
/// them by the sketch entity and its role: a sketch point's side edge (role 18), or a sketch
/// line where the sweep starts (role 2) or ends (role 10). Each is a straight edge, proven on
/// SolidCraft's rebuilt body like any input edge.
fn generated(selection: &Value, ctx: &Context) -> Result<(Vec<Input>, Vec<V3>), String> {
    const GENERATED: &str = "its generated edge reference is malformed";
    let edges = selection
        .pointer("/value/edges")
        .and_then(Value::as_array)
        .filter(|edges| !edges.is_empty() && edges.len() <= LIMIT)
        .ok_or("its generated edge selection is empty or exceeds the blend limit")?;
    let mut inputs = Vec::new();
    let mut points = Vec::new();
    for edge in edges {
        let producer = edge
            .get("feature")
            .and_then(Value::as_str)
            .ok_or(GENERATED)?;
        let (entity, role) = edge
            .get("local_id")
            .and_then(Value::as_str)
            .and_then(|id| id.split_once(":sketch-entity:"))
            .and_then(|(_, rest)| rest.rsplit_once(":role:"))
            .ok_or(GENERATED)?;
        let definition = ctx
            .arena("features")
            .iter()
            .find(|feature| feature.get("id").and_then(Value::as_str) == Some(producer))
            .and_then(|feature| feature.get("definition"))
            .ok_or("the feature that generated its edge is missing")?;
        let (sketch, mapped, direction, depth) = sweep(definition, ctx)?;
        let geometry = ctx
            .arena("sketch_entities")
            .iter()
            .find(|row| {
                row.get("id").and_then(Value::as_str) == Some(entity)
                    && row.get("sketch").and_then(Value::as_str) == Some(sketch)
            })
            .and_then(|row| row.get("geometry"))
            .ok_or("the sketch entity its edge was generated from is missing")?;
        let at = |key: &str| -> Option<V3> {
            let position = geometry.get(key)?;
            Some(mapped.world([number(position.get("u"))?, number(position.get("v"))?]))
        };
        let travel = scale(direction, depth);
        let ends = match (role, geometry.get("kind").and_then(Value::as_str)) {
            ("18", Some("point")) => {
                let from = at("position").ok_or(GENERATED)?;
                (from, add(from, travel))
            }
            ("2" | "10", Some("line")) => {
                let offset = if role == "10" { travel } else { [0.0; 3] };
                (
                    add(at("start").ok_or(GENERATED)?, offset),
                    add(at("end").ok_or(GENERATED)?, offset),
                )
            }
            _ => return Err("this kind of generated edge is not mapped yet".to_owned()),
        };
        if near(ends.0, ends.1) {
            return Err("its generated edge has no length".to_owned());
        }
        let point = scale(add(ends.0, ends.1), 0.5);
        let tangent = unit(sub(ends.1, ends.0)).ok_or(GENERATED)?;
        inputs.push(Input {
            point,
            ends,
            tangent,
            length: distance(ends.0, ends.1),
            straight: true,
        });
        points.push(point);
    }
    Ok((inputs, points))
}

/// The sketch a generating feature sweeps, with its mapping, and the direction and length of
/// the sweep from the sketch plane: an extrusion's one-sided blind depth, or a base flange's
/// thickness on the side it is drawn.
fn sweep<'a>(
    definition: &'a Value,
    ctx: &'a Context,
) -> Result<(&'a str, &'a Mapped, V3, f64), String> {
    const GENERATED: &str = "its generated edge reference is malformed";
    let kind = definition.get("definition").and_then(Value::as_str);
    if !matches!(kind, Some("extrude" | "sheet_metal_base_flange")) {
        return Err(
            "its edge was generated by a feature other than an extrusion or a base flange, which is not supported yet"
                .to_owned(),
        );
    }
    let sketch = definition
        .pointer("/profile/value/sketch")
        .and_then(Value::as_str)
        .ok_or(GENERATED)?;
    let mapped = ctx
        .sketches
        .get(sketch)
        .ok_or("the sketch its edge was generated from is not an earlier mapped sketch")?;
    if kind == Some("sheet_metal_base_flange") {
        let thickness = number(definition.get("thickness"))
            .filter(|t| t.is_finite() && *t > 0.0)
            .ok_or("its edge's base flange has no thickness")?;
        let normal = match definition.get("side").and_then(Value::as_str) {
            None | Some("forward") => mapped.normal,
            Some("reverse") => scale(mapped.normal, -1.0),
            Some(_) => {
                return Err(
                    "its edge's base flange is thick on both sides of its sketch, which is not supported yet"
                        .to_owned(),
                )
            }
        };
        let direction = unit(normal).ok_or("its edge's base flange direction is not resolved")?;
        return Ok((sketch, mapped, direction, thickness));
    }
    if definition.pointer("/start/kind").and_then(Value::as_str) != Some("profile_plane") {
        return Err(
            "its edge's extrusion starts away from its sketch, which is not supported yet"
                .to_owned(),
        );
    }
    let side = definition
        .pointer("/extent/side")
        .filter(|_| definition.pointer("/extent/kind").and_then(Value::as_str) == Some("one_sided"))
        .and_then(Value::as_object)
        .filter(|side| side.keys().all(|key| key == "termination"))
        .ok_or("its edge's extrusion is not one-sided, which is not supported yet")?;
    let depth = side
        .get("termination")
        .filter(|end| end.get("kind").and_then(Value::as_str) == Some("blind"))
        .and_then(|end| number(end.get("length")))
        .filter(|depth| depth.is_finite() && *depth > 0.0)
        .ok_or("its edge's extrusion has no blind depth")?;
    let direction = match definition
        .pointer("/direction/kind")
        .and_then(Value::as_str)
    {
        Some("profile_normal") => Some(mapped.normal),
        Some("explicit") => definition
            .pointer("/direction/vector")
            .and_then(|vector| v3(Some(vector))),
        _ => None,
    }
    .and_then(unit)
    .ok_or("its edge's extrusion direction is not resolved")?;
    Ok((sketch, mapped, direction, depth))
}

/// One independent body's selection, in order of first appearance in the input edges.
#[derive(Debug, PartialEq)]
pub(crate) struct Selection {
    pub(crate) body: String,
    pub(crate) points: Vec<V3>,
}

/// Match every input edge against the entire rebuilt history before grouping by body.
/// Only constant circular fillets and equal-setback chamfers reach this proof: applying
/// their identical parameters independently to disjoint bodies is equivalent.
pub(crate) fn prove(pending: &Pending, bodies: &ReplayedBodies) -> Result<Vec<Selection>, String> {
    let mut chosen: Vec<(usize, usize)> = Vec::new();
    for input in &pending.edges {
        let mut on = Vec::new();
        for (b, (_, edges)) in bodies.iter().enumerate() {
            for (e, edge) in edges.iter().enumerate() {
                let hit = if straight(edge) {
                    on_segment(input.point, (edge.start, edge.end))
                } else {
                    near(input.point, edge.mid)
                };
                if hit {
                    on.push((b, e));
                }
            }
        }
        let [(b, e)] = on.as_slice() else {
            return Err(INPUT.to_owned());
        };
        let edges = bodies
            .get(*b)
            .map(|(_, edges)| edges.as_slice())
            .ok_or(INPUT)?;
        let edge = edges.get(*e).ok_or(INPUT)?;
        // A straight edge an earlier group shortened still runs along the input edge, inside it.
        let shortened = pending.trimmed
            && input.straight
            && straight(edge)
            && on_segment(edge.start, input.ends)
            && on_segment(edge.end, input.ends);
        if !shortened
            && (!same(input.ends, (edge.start, edge.end))
                || !same_length(edge.length, input.length)
                || straight(edge) != input.straight)
            || (input.straight && !parallel(sub(edge.end, edge.start), input.tangent))
        {
            return Err(INPUT.to_owned());
        }
        // A 45-degree chamfer has equal setbacks only at a right-angle corner.
        if pending.angle_chamfer && !right_angle(edge, edges) {
            return Err("distance-and-angle chamfers on non-perpendicular input faces are not supported yet".to_owned());
        }
        if chosen.contains(&(*b, *e)) {
            return Err("its input edge selection repeats a rebuilt edge".to_owned());
        }
        chosen.push((*b, *e));
    }
    if chosen.is_empty() {
        return Err(INPUT.to_owned());
    }
    let mut selections: Vec<Selection> = Vec::new();
    for ((body, _), input) in chosen.iter().zip(&pending.edges) {
        let name = bodies.get(*body).map(|(name, _)| name).ok_or(INPUT)?;
        if let Some(selection) = selections.iter_mut().find(|s| s.body == *name) {
            selection.points.push(input.point);
        } else {
            selections.push(Selection {
                body: name.clone(),
                points: vec![input.point],
            });
        }
    }
    Ok(selections)
}

/// Whether the faces either side of a straight `edge` meet square, judged at both its ends by
/// the two other edges there, which must be straight: the corner of a prism.
fn right_angle(edge: &ReplayedEdge, edges: &[ReplayedEdge]) -> bool {
    let Some(along) = unit(sub(edge.end, edge.start)) else {
        return false;
    };
    [edge.start, edge.end].into_iter().all(|vertex| {
        let mut directions = Vec::new();
        for other in edges {
            if same((other.start, other.end), (edge.start, edge.end)) {
                continue;
            }
            let far = if near(vertex, other.start) {
                other.end
            } else if near(vertex, other.end) {
                other.start
            } else {
                continue;
            };
            if !straight(other) {
                return false;
            }
            let d = sub(far, vertex);
            let across = sub(d, scale(along, dot(d, along)));
            match unit(across) {
                Some(across) => directions.push(across),
                None => return false,
            }
        }
        match directions.as_slice() {
            [a, b] => dot(*a, *b).abs() <= ANGLE,
            _ => false,
        }
    })
}

/// The length of the circular arc from `ends.0` through `point` to `ends.1`, when `tangent`
/// is the tangent at `point` of the circle through the three; `None` for any other curve.
fn arc_length(point: V3, ends: Segment, tangent: V3) -> Option<f64> {
    let a = sub(ends.0, point);
    let b = sub(ends.1, point);
    let normal = cross(a, b);
    let n2 = dot(normal, normal);
    if !n2.is_finite() || n2 <= TOLERANCE * TOLERANCE * dot(a, a) * dot(b, b) {
        return None;
    }
    let towards = sub(scale(b, dot(a, a)), scale(a, dot(b, b)));
    let offset = scale(cross(towards, normal), 1.0 / (2.0 * n2));
    let center = add(point, offset);
    let radius = distance(point, center);
    let out = unit(sub(point, center))?;
    if dot(out, tangent).abs() > ANGLE || dot(unit(normal)?, tangent).abs() > ANGLE {
        return None;
    }
    let angle = |p: V3, q: V3| {
        let (u, v) = (sub(p, center), sub(q, center));
        let c = cross(u, v);
        dot(c, c).sqrt().atan2(dot(u, v))
    };
    let length = radius * (angle(ends.0, point) + angle(point, ends.1));
    (length.is_finite() && length > 0.0).then_some(length)
}

/// Whether SolidCraft's edge is straight: as long as its chord.
fn straight(edge: &ReplayedEdge) -> bool {
    let chord = distance(edge.start, edge.end);
    chord > TOLERANCE && same_length(edge.length, chord)
}

fn same_length(a: f64, b: f64) -> bool {
    (a - b).abs() <= TOLERANCE * a.abs().max(b.abs()).max(1.0)
}

fn parallel(direction: V3, tangent: V3) -> bool {
    unit(direction).is_some_and(|direction| (dot(direction, tangent).abs() - 1.0).abs() <= ANGLE)
}

fn length(value: Option<&Value>) -> Result<String, String> {
    number(value)
        .filter(|v| *v > 0.0)
        .map(|v| format!("{v} mm"))
        .ok_or_else(|| "its blend dimension is not a positive finite length".to_owned())
}

fn near(a: V3, b: V3) -> bool {
    a.iter().zip(b).all(|(a, b)| (a - b).abs() <= TOLERANCE)
}

fn on_segment(point: V3, (a, b): Segment) -> bool {
    let direction = sub(b, a);
    let length2 = dot(direction, direction);
    if !length2.is_finite() || length2 <= 0.0 {
        return false;
    }
    let t = dot(sub(point, a), direction) / length2;
    if !(0.0..=1.0).contains(&t) {
        return false;
    }
    let closest = [0, 1, 2]
        .map(|i| a.get(i).copied().unwrap_or(0.0) + t * direction.get(i).copied().unwrap_or(0.0));
    near(point, closest)
}

fn same(a: Segment, b: Segment) -> bool {
    (near(a.0, b.0) && near(a.1, b.1)) || (near(a.0, b.1) && near(a.1, b.0))
}

fn sub(a: V3, b: V3) -> V3 {
    [0, 1, 2].map(|i| a.get(i).copied().unwrap_or(0.0) - b.get(i).copied().unwrap_or(0.0))
}

fn add(a: V3, b: V3) -> V3 {
    [0, 1, 2].map(|i| a.get(i).copied().unwrap_or(0.0) + b.get(i).copied().unwrap_or(0.0))
}

fn scale(a: V3, k: f64) -> V3 {
    a.map(|v| v * k)
}

fn distance(a: V3, b: V3) -> f64 {
    let d = sub(a, b);
    dot(d, d).sqrt()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use serde_json::{json, Value};

    use super::{generated, Context};
    use crate::history::sketch::Mapped;

    fn proof(edges: Vec<super::Input>) -> super::Pending {
        super::Pending {
            feature: 0,
            outcome: 0,
            subject: "Treatment".to_owned(),
            edges,
            angle_chamfer: false,
            face: None,
            extra: None,
            trimmed: false,
        }
    }

    fn straight_input(x: f64) -> super::Input {
        super::Input {
            point: [x, 0.0, 5.0],
            ends: ([x, 0.0, 0.0], [x, 0.0, 10.0]),
            tangent: [0.0, 0.0, 1.0],
            length: 10.0,
            straight: true,
        }
    }

    fn rebuilt(input: &super::Input) -> crate::verify::ReplayedEdge {
        crate::verify::ReplayedEdge {
            start: input.ends.0,
            end: input.ends.1,
            mid: input.point,
            length: input.length,
            name: String::new(),
        }
    }

    #[test]
    fn proof_groups_bodies_and_edges_by_first_appearance() {
        let (a, b, c) = (
            straight_input(0.0),
            straight_input(20.0),
            straight_input(30.0),
        );
        let bodies = vec![
            ("A".to_owned(), vec![rebuilt(&a)]),
            ("B".to_owned(), vec![rebuilt(&b), rebuilt(&c)]),
        ];
        let pending = proof(vec![c.clone(), a.clone(), b.clone()]);
        assert_eq!(
            super::prove(&pending, &bodies).unwrap(),
            vec![
                super::Selection {
                    body: "B".to_owned(),
                    points: vec![c.point, b.point]
                },
                super::Selection {
                    body: "A".to_owned(),
                    points: vec![a.point]
                },
            ]
        );
    }

    #[test]
    fn splitting_keeps_every_edge_check_and_trimmed_group_check() {
        let (a, b) = (straight_input(0.0), straight_input(20.0));
        let bodies = vec![
            ("A".to_owned(), vec![rebuilt(&a)]),
            ("B".to_owned(), vec![rebuilt(&b)]),
        ];
        let pending = proof(vec![a.clone(), b.clone()]);
        for changed in 0..4 {
            let mut bad = bodies.clone();
            match changed {
                0 => bad[1].1[0].end[2] = 9.0,
                1 => bad[1].1[0].length = 11.0,
                2 => bad[1].1[0].start[0] += 1.0,
                _ => bad.push(("Ambiguous".to_owned(), vec![rebuilt(&b)])),
            }
            assert!(super::prove(&pending, &bad).is_err());
        }
        let mut repeated = pending.clone();
        repeated.edges.push(a);
        assert!(super::prove(&repeated, &bodies)
            .unwrap_err()
            .contains("repeats"));
        let mut tangent = pending.clone();
        tangent.edges[1].tangent = [1.0, 0.0, 0.0];
        assert!(super::prove(&tangent, &bodies).is_err());
        let mut shortened = bodies.clone();
        shortened[1].1[0].start[2] = 1.0;
        shortened[1].1[0].end[2] = 9.0;
        shortened[1].1[0].length = 8.0;
        assert!(super::prove(&pending, &shortened).is_err());
        let mut trimmed = pending.clone();
        trimmed.trimmed = true;
        assert_eq!(super::prove(&trimmed, &shortened).unwrap().len(), 2);
        let mut angle = pending;
        angle.angle_chamfer = true;
        // Bare edges have no perpendicular incident faces to prove at either end.
        assert!(super::prove(&angle, &bodies)
            .unwrap_err()
            .contains("non-perpendicular"));
    }

    #[test]
    fn unsupported_radius_laws_never_reach_body_splitting() {
        for radius in [
            json!({"kind": "variable"}),
            json!({"kind": "chordal"}),
            json!({"kind": "asymmetric"}),
            json!({"kind": "unresolved"}),
            json!({"kind": "setback"}),
        ] {
            let model = json!({});
            let mut ctx = Context::new(&model);
            let error = super::map(
                &json!({"name": "Treatment"}),
                &json!({"definition": "fillet", "groups": [{"radius": radius}]}),
                &mut ctx,
                &mut dutchman_core::Report::default(),
            )
            .unwrap_err();
            assert!(error.contains("not supported yet"), "{error}");
            assert!(ctx.blends.is_empty());
            assert!(ctx.extra.is_empty());
        }
    }

    /// An extrusion of sketch `s` (XY plane, raised 2 mm) 10 mm along `direction`, a corner
    /// point and the near side line in that sketch, and a native feature.
    fn model(direction: Value) -> Value {
        json!({
            "features": [
                {"id": "extrude", "definition": {"definition": "extrude",
                    "profile": {"kind": "sketch_regions", "value": {"sketch": "s"}},
                    "direction": direction, "start": {"kind": "profile_plane"},
                    "extent": {"kind": "one_sided", "side": {"termination": {"kind": "blind", "length": 10.0}}}}},
                {"id": "native", "definition": {"definition": "native"}},
            ],
            "sketch_entities": [
                {"id": "corner", "sketch": "s", "geometry": {"kind": "point", "position": {"u": 10.0, "v": 15.0}}},
                {"id": "side", "sketch": "s", "geometry": {"kind": "line",
                    "start": {"u": 10.0, "v": 15.0}, "end": {"u": -10.0, "v": 15.0}}},
            ],
        })
    }

    fn select(model: &Value, edges: Value) -> Result<Vec<[f64; 3]>, String> {
        let mut ctx = Context::new(model);
        ctx.sketches.insert(
            "s".to_owned(),
            Mapped {
                feature: 1,
                normal: [0.0, 0.0, 1.0],
                origin: [0.0, 0.0, 2.0],
                x_dir: [1.0, 0.0, 0.0],
                y_dir: [0.0, 1.0, 0.0],
                points: Vec::new(),
                shapes: BTreeMap::new(),
            },
        );
        generated(
            &json!({"kind": "generated", "value": {"edges": edges, "native": "n"}}),
            &ctx,
        )
        .map(|(_, points)| points)
    }

    fn edge(feature: &str, entity: &str, role: u32) -> Value {
        json!({"feature": feature,
            "local_id": format!("inventor:nmx:INV_NMX_SWEEPGENERATED_TAG:sketch-entity:{entity}:role:{role}")})
    }

    #[test]
    fn each_radius_group_becomes_its_own_blend_in_source_order() {
        let sample = |edge: &str, start: [f64; 3], end: [f64; 3]| {
            let p = |c: [f64; 3]| json!({"x": c[0], "y": c[1], "z": c[2]});
            let mid = [(start[0] + end[0]) / 2.0, (start[1] + end[1]) / 2.0, 0.0];
            json!({"edge": edge, "start": p(start), "end": p(end), "interior": p(mid),
                "tangent": p([end[0] - start[0], end[1] - start[1], 0.0])})
        };
        let model = json!({"feature_input_topologies": [{"id": "st", "input_of": "f",
            "edges": ["a", "b"],
            "edge_geometry": [sample("a", [0.0; 3], [10.0, 0.0, 0.0]),
                sample("b", [10.0, 0.0, 0.0], [10.0, 10.0, 0.0])]}]});
        let group = |edge: &str, radius: f64| {
            json!({"edges": {"kind": "historical", "value": {"state": "st", "edges": [edge], "native": "n"}},
                "radius": {"kind": "constant", "radius": radius}})
        };
        let definition =
            json!({"definition": "fillet", "groups": [group("a", 3.0), group("b", 2.0)]});
        let mut ctx = Context::new(&model);
        let main = super::map(
            &json!({"id": "f", "name": "Fillet"}),
            &definition,
            &mut ctx,
            &mut dutchman_core::Report::default(),
        )
        .unwrap();
        assert_eq!(main["radius"], "2 mm");
        assert_eq!(main["edges"], json!([[10.0, 5.0, 0.0]]));
        let [first] = ctx.extra.as_slice() else {
            panic!("one earlier group: {:?}", ctx.extra);
        };
        assert_eq!(first["radius"], "3 mm");
        assert_ne!(first["id"], main["id"]);
        let pending: Vec<_> = ctx.blends.iter().map(|p| (p.extra, p.trimmed)).collect();
        assert_eq!(pending, [(Some(0), false), (None, true)]);
    }

    #[test]
    fn generated_edges_are_found_from_their_sketch_entity_and_extrusion() {
        let normal = model(json!({"kind": "profile_normal"}));
        // A point's side edge runs the extrusion's depth from the sketch plane.
        assert_eq!(
            select(&normal, json!([edge("extrude", "corner", 18)])),
            Ok(vec![[10.0, 15.0, 7.0]])
        );
        // A line's edge where the extrusion ends (role 10) or starts (role 2).
        assert_eq!(
            select(
                &normal,
                json!([edge("extrude", "side", 10), edge("extrude", "side", 2)])
            ),
            Ok(vec![[0.0, 15.0, 12.0], [0.0, 15.0, 2.0]])
        );
        let down = model(json!({"kind": "explicit", "vector": {"x": 0.0, "y": 0.0, "z": -1.0}}));
        assert_eq!(
            select(&down, json!([edge("extrude", "corner", 18)])),
            Ok(vec![[10.0, 15.0, -3.0]])
        );
    }

    #[test]
    fn other_generated_edges_say_why_they_are_not_mapped() {
        let normal = model(json!({"kind": "profile_normal"}));
        for (edges, reason) in [
            (
                json!([edge("extrude", "side", 18)]),
                "this kind of generated edge is not mapped yet",
            ),
            (
                json!([edge("extrude", "corner", 10)]),
                "this kind of generated edge is not mapped yet",
            ),
            (
                json!([edge("native", "corner", 18)]),
                "other than an extrusion",
            ),
            (json!([edge("missing", "corner", 18)]), "is missing"),
            (
                json!([{"feature": "extrude", "local_id": "inventor:nmx:other"}]),
                "malformed",
            ),
            (json!([]), "empty"),
        ] {
            let error = select(&normal, edges).unwrap_err();
            assert!(error.contains(reason), "{error}");
        }
    }

    #[test]
    fn base_flange_edges_run_its_thickness_on_its_side() {
        let flange = |side: &str| {
            let mut model = model(json!({"kind": "profile_normal"}));
            model["features"][0] = json!({"id": "plate", "definition": {
                "definition": "sheet_metal_base_flange",
                "profile": {"kind": "sketch_regions", "value": {"sketch": "s"}},
                "thickness": 3.0, "side": side}});
            model
        };
        // A point's side edge runs the thickness from the sketch plane, on the flange's side.
        assert_eq!(
            select(&flange("forward"), json!([edge("plate", "corner", 18)])),
            Ok(vec![[10.0, 15.0, 3.5]])
        );
        assert_eq!(
            select(&flange("reverse"), json!([edge("plate", "corner", 18)])),
            Ok(vec![[10.0, 15.0, 0.5]])
        );
        // A line's edge on the far face sits at the thickness; on the sketch plane, at none.
        assert_eq!(
            select(
                &flange("forward"),
                json!([edge("plate", "side", 10), edge("plate", "side", 2)])
            ),
            Ok(vec![[0.0, 15.0, 5.0], [0.0, 15.0, 2.0]])
        );
        let error = select(&flange("symmetric"), json!([edge("plate", "corner", 18)])).unwrap_err();
        assert!(error.contains("both sides of its sketch"), "{error}");
    }
}
