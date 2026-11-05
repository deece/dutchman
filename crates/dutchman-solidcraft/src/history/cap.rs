//! Proof that an extrude swept the region the source swept.
//!
//! The mapper knows the loops it selects. CADIR gives no face-to-feature link for the final
//! model, so the evidence is only taken where it cannot be confused with another feature's:
//! the extrude is the only feature that builds geometry, and so the final model's planar faces
//! at the extrude's far end, facing outward, are its end cap. The cap's area then has to equal
//! the selected region's area.

use serde_json::Value;

use super::sketch::{cross, dot, unit, v3, P2, V3};
use super::Context;

/// Points per full turn when a circular edge is followed as a polygon.
const TURN: usize = 256;
/// How far (mm) a face's plane may be from the extrude's end plane.
const PLANE: f64 = 1e-4;

/// The sketch frame and the far end of the sweep, all in model space.
pub(super) struct End {
    pub(super) origin: V3,
    pub(super) normal: V3,
    pub(super) x: V3,
    pub(super) y: V3,
    /// Distance from the sketch plane to the end plane, along the sketch normal.
    pub(super) height: f64,
    /// `1.0` when the sweep runs along the sketch normal, `-1.0` against it.
    pub(super) travel: f64,
}

/// Whether the source proves the selected region: `Ok(true)` when the end cap's area matches,
/// `Ok(false)` when there is no usable evidence, and the reason when it does not match.
pub(super) fn check(profile: Option<&Value>, ctx: &Context, end: &End) -> Result<bool, String> {
    let Some(swept) = super::profile::swept_area(profile, ctx) else {
        return Ok(false);
    };
    let Some(cap) = cap_area(ctx, end) else {
        return Ok(false);
    };
    if (swept - cap).abs() > 1.0_f64.max(0.03 * cap) {
        return Err(format!(
            "its profile region doesn't match the source (swept {swept:.1} mm², the source \
             {cap:.1} mm²)"
        ));
    }
    Ok(true)
}

/// The area of the final model's outward faces on the end plane, or `None` when there is none
/// or one cannot be measured.
fn cap_area(ctx: &Context, end: &End) -> Option<f64> {
    let outward = end.normal.map(|c| c * end.travel);
    let mut total = 0.0;
    let mut found = false;
    for face in ctx.arena("faces") {
        let Some(surface) = face
            .get("surface")
            .and_then(Value::as_str)
            .and_then(|id| ctx.row("surfaces", id))
        else {
            continue;
        };
        let geometry = surface.get("geometry")?;
        if geometry.get("kind").and_then(Value::as_str) != Some("plane") {
            continue;
        }
        let (Some(normal), Some(origin)) = (
            v3(geometry.get("normal")).and_then(unit),
            v3(geometry.get("origin")),
        ) else {
            continue;
        };
        let sign = if face.get("sense").and_then(Value::as_str) == Some("reversed") {
            -1.0
        } else {
            1.0
        };
        let facing = dot(normal, outward) * sign;
        let height = dot(sub(origin, end.origin), end.normal);
        if facing < 1.0 - 1e-6 || (height - end.height).abs() > PLANE {
            continue;
        }
        total += face_area(ctx, face, end)?;
        found = true;
    }
    found.then_some(total)
}

/// A planar face's area: its largest loop less the others.
fn face_area(ctx: &Context, face: &Value, end: &End) -> Option<f64> {
    let loops = face.get("loops")?;
    let mut ids: Vec<&str> = Vec::new();
    if let Some(outer) = loops.get("outer").and_then(Value::as_str) {
        ids.push(outer);
    }
    for key in ["inner", "loops"] {
        ids.extend(
            loops
                .get(key)
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str),
        );
    }
    let mut areas = Vec::new();
    for id in ids {
        let ring = loop_ring(ctx, ctx.row("loops", id)?, end)?;
        areas.push(super::crossing::ring_area(&ring));
    }
    let largest = areas.iter().copied().max_by(f64::total_cmp)?;
    Some(largest - (areas.iter().sum::<f64>() - largest))
}

/// A loop followed as a polygon in the sketch's plane coordinates.
fn loop_ring(ctx: &Context, lp: &Value, end: &End) -> Option<Vec<P2>> {
    let mut ring: Vec<P2> = Vec::new();
    for coedge in lp.pointer("/boundary/coedges")?.as_array()? {
        let coedge = ctx.row("coedges", coedge.as_str()?)?;
        let edge = ctx.row("edges", coedge.get("edge")?.as_str()?)?;
        let reversed = coedge.get("sense").and_then(Value::as_str) == Some("reversed");
        let mut points = edge_points(ctx, edge)?;
        if reversed {
            points.reverse();
        }
        points.pop();
        ring.extend(points.into_iter().map(|p| {
            let d = sub(p, end.origin);
            [dot(d, end.x), dot(d, end.y)]
        }));
    }
    (ring.len() >= 3).then_some(ring)
}

/// Points along an edge from its start to its end, both included.
fn edge_points(ctx: &Context, edge: &Value) -> Option<Vec<V3>> {
    let vertex = |key: &str| -> Option<V3> {
        let point = ctx.row(
            "points",
            ctx.row("vertices", edge.get(key)?.as_str()?)?
                .get("point")?
                .as_str()?,
        )?;
        v3(point.get("position"))
    };
    let (start, finish) = (vertex("start")?, vertex("end")?);
    let curve = ctx.row("curves", edge.pointer("/carrier/curve")?.as_str()?)?;
    let geometry = curve.get("geometry")?;
    match geometry.get("kind")?.as_str()? {
        "line" => Some(vec![start, finish]),
        "circle" => {
            let center = v3(geometry.get("center"))?;
            let axis = v3(geometry.get("axis")).and_then(unit)?;
            let reference = v3(geometry.get("ref_direction")).and_then(unit)?;
            let side = cross(axis, reference);
            let radius = geometry.get("radius")?.as_f64()?;
            let angle = |p: V3| {
                let d = sub(p, center);
                dot(d, side).atan2(dot(d, reference))
            };
            let from = angle(start);
            let mut sweep = (angle(finish) - from).rem_euclid(std::f64::consts::TAU);
            if sweep < 1e-9 {
                sweep = std::f64::consts::TAU;
            }
            let steps = ((TURN as f64 * sweep / std::f64::consts::TAU).ceil() as usize).max(2);
            let mut points: Vec<V3> = (0..=steps)
                .map(|i| {
                    let a = from + sweep * i as f64 / steps as f64;
                    let mut point = center;
                    for ((coordinate, r), s) in point.iter_mut().zip(reference).zip(side) {
                        *coordinate += radius * (a.cos() * r + a.sin() * s);
                    }
                    point
                })
                .collect();
            // The stored ends are exact.
            *points.first_mut()? = start;
            *points.last_mut()? = finish;
            Some(points)
        }
        _ => None,
    }
}

fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
