//! Extrudes of a face of the model as it was earlier in the timeline.
//!
//! CADIR gives such a profile as faces of the feature's own input topology. It carries no
//! surface for them, but it keeps the sampled geometry of every edge that bounds a selected
//! face (its ends, the point halfway along its carrier and the tangent there), and each
//! edge's faces. So the face is rebuilt from its boundary:
//!
//! - Every edge must be a straight line, a circular arc or a full circle, all in one plane.
//!   The plane comes from the boundary's points, never from the final model.
//! - The edges must chain into closed loops. Loops inside loops are holes, as SolidCraft's
//!   own profile rule has them.
//! - The loops become a sketch on that plane, and the extrude takes every profile of it.
//!
//! The sketch's side is not known from the boundary alone: the extrude runs along the face's
//! outward normal, which only the body knows. Mapping assumes a side ([`Proof`]); once
//! SolidCraft has replayed the history before the extrude, [`prove`] finds the face among the
//! rebuilt ones (same plane, same area, every boundary edge on it) and either confirms the
//! side or says the sketch has to be mirrored ([`Proof::sketch`]). A face it cannot find
//! exactly once refuses the extrude.

use serde_json::{json, Value};

use super::sketch::{cross, dot, synthesized, unit, v3, V3};
use super::Context;
use crate::verify::{Replay, ReplayedEdge, ReplayedFace};

/// Most faces one profile may select, and edges all of them may have.
const FACES: usize = 64;
const EDGES: usize = 4096;
/// How far apart, in millimetres, two positions may be and still be the same.
const NEAR: f64 = 1e-5;
/// How far from square or parallel two unit directions may be.
const ANGLE: f64 = 1e-5;
const TAU: f64 = std::f64::consts::TAU;

/// One boundary edge of a face, as its samples describe it.
#[derive(Clone, Debug)]
enum Edge {
    Line {
        start: V3,
        end: V3,
    },
    /// A circular arc from `start` through `mid` to `end`; `axis` is the normal about which
    /// that order runs counter-clockwise.
    Arc {
        start: V3,
        mid: V3,
        end: V3,
        center: V3,
        radius: f64,
        axis: V3,
    },
    Circle {
        start: V3,
        mid: V3,
        center: V3,
        radius: f64,
        axis: V3,
    },
}

impl Edge {
    fn ends(&self) -> (V3, V3) {
        match self {
            Edge::Line { start, end } | Edge::Arc { start, end, .. } => (*start, *end),
            Edge::Circle { start, .. } => (*start, *start),
        }
    }

    /// The edge's length.
    fn length(&self) -> f64 {
        match self {
            Edge::Line { start, end } => norm(sub(*end, *start)),
            Edge::Arc {
                start,
                mid,
                end,
                center,
                radius,
                axis,
            } => {
                let sweep = |a: V3, b: V3| signed_angle(sub(a, *center), sub(b, *center), *axis);
                let first = sweep(*start, *mid).abs();
                let second = sweep(*mid, *end).abs();
                radius * (first + second)
            }
            Edge::Circle { radius, .. } => TAU * radius,
        }
    }
}

/// The boundary of one selected face.
#[derive(Clone, Debug)]
struct Boundary {
    edges: Vec<Edge>,
    /// The area of the face: its loops with holes taken away.
    area: f64,
}

/// What the replay has to confirm about a mapped face profile.
#[derive(Clone, Debug)]
pub(crate) struct Proof {
    /// The id of the sketch feature written for the profile.
    pub(crate) sketch: u64,
    name: String,
    origin: V3,
    x: V3,
    y: V3,
    faces: Vec<Boundary>,
}

/// Maps the faces `value` selects (a `historical_faces` profile) of `feature` to a sketch of
/// their boundary, left in `ctx` for the history, and returns its key in `ctx.sketches` and
/// the profile selection.
pub(super) fn map(
    feature: &Value,
    value: &Value,
    ctx: &mut Context,
) -> Result<(String, Value), String> {
    let mut faces = boundaries(feature, value, ctx)?;
    let points: Vec<V3> = faces
        .iter()
        .flat_map(|face| &face.edges)
        .flat_map(|edge| {
            let (start, end) = edge.ends();
            let mid = match edge {
                Edge::Line { start, end } => scale(add(*start, *end), 0.5),
                Edge::Arc { mid, .. } | Edge::Circle { mid, .. } => *mid,
            };
            [start, mid, end]
        })
        .collect();
    let (origin, normal) = plane(&faces, &points)?;
    let helper = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]
        .into_iter()
        .min_by(|a, b| dot(*a, normal).abs().total_cmp(&dot(*b, normal).abs()))
        .ok_or("its face has no plane")?;
    let x = unit(cross(helper, normal)).ok_or("its face has no plane")?;
    let y = unit(cross(normal, x)).ok_or("its face has no plane")?;
    for face in &mut faces {
        face.area = area(face, origin, x, y)?;
    }
    let id = ctx.id();
    let name = format!("Face Sketch{id}");
    let proof = Proof {
        sketch: id,
        name,
        origin,
        x,
        y,
        faces,
    };
    let (sketch, mapped) = proof.sketch(false)?;
    let key = format!(
        "historical-face:{}",
        feature.get("id").and_then(Value::as_str).unwrap_or("?")
    );
    ctx.sketches.insert(key.clone(), mapped);
    ctx.extra.push(sketch);
    ctx.blends.push(super::blend::Pending {
        feature: 0,
        outcome: 0,
        subject: String::new(),
        edges: Vec::new(),
        angle_chamfer: false,
        face: Some(proof),
        extra: None,
        trimmed: false,
    });
    Ok((key, json!({"type": "all"})))
}

impl Proof {
    /// The sketch feature of the boundary and the sketch as later features see it, on the side
    /// the face's normal `origin → x × y` points to, or the other one when `mirrored`.
    pub(crate) fn sketch(&self, mirrored: bool) -> Result<(Value, super::sketch::Mapped), String> {
        let y = if mirrored {
            scale(self.y, -1.0)
        } else {
            self.y
        };
        let normal = unit(cross(self.x, y)).ok_or("its face has no plane")?;
        let mut entities = Vec::new();
        for (k, edge) in self.faces.iter().flat_map(|face| &face.edges).enumerate() {
            let at = |p: V3| {
                let d = sub(p, self.origin);
                json!({"u": dot(d, self.x), "v": dot(d, y)})
            };
            let geometry = match edge {
                Edge::Line { start, end } => {
                    json!({"kind": "line", "start": at(*start), "end": at(*end)})
                }
                Edge::Circle { center, radius, .. } => {
                    json!({"kind": "circle", "center": at(*center), "radius": radius})
                }
                Edge::Arc {
                    start,
                    end,
                    center,
                    radius,
                    axis,
                    ..
                } => {
                    let angle = |p: V3| {
                        let d = sub(p, *center);
                        dot(d, y).atan2(dot(d, self.x))
                    };
                    // Counter-clockwise on this sketch's side runs from the start to the end
                    // when the arc turns about the same way as the sketch normal.
                    let (from, to) = if dot(*axis, normal) > 0.0 {
                        (*start, *end)
                    } else {
                        (*end, *start)
                    };
                    json!({
                        "kind": "arc",
                        "center": at(*center),
                        "radius": radius,
                        "start_angle": angle(from),
                        "end_angle": angle(to),
                    })
                }
            };
            entities.push(json!({
                "id": format!("face-edge-{k}"),
                "construction": false,
                "geometry": geometry,
            }));
        }
        synthesized(self.sketch, &self.name, (self.origin, self.x, y), &entities)
    }
}

/// One point on each face `value` selects, from the boundary samples of its input topology:
/// inside a planar face (clear of its edges and its holes), on the middle of a cylindrical one.
/// Faces of any other surface, and boundaries that are not lines, arcs and circles, are refused.
pub(super) fn interior_points(
    feature: &Value,
    value: &Value,
    ctx: &Context,
) -> Result<Vec<V3>, String> {
    let state = value.get("state").cloned().unwrap_or(Value::Null);
    let ids = value
        .get("faces")
        .and_then(Value::as_array)
        .filter(|ids| !ids.is_empty() && ids.len() <= FACES)
        .ok_or("its face selection is empty or exceeds the face limit")?;
    let mut points = Vec::new();
    for id in ids {
        let one = json!({"state": state, "faces": [id]});
        let faces = boundaries(feature, &one, ctx)?;
        let [face] = faces.as_slice() else {
            return Err(BAD.to_owned());
        };
        points.push(point_in(face)?);
    }
    Ok(points)
}

/// A point inside the one planar face `value` selects, clear of its edges and its holes, and
/// the unit normal of its plane, from the boundary samples of its input topology.
pub(super) fn planar_face(
    feature: &Value,
    value: &Value,
    ctx: &Context,
) -> Result<(V3, V3), String> {
    let faces = boundaries(feature, value, ctx)?;
    let [face] = faces.as_slice() else {
        return Err("a selection without exactly one face is not supported yet".to_owned());
    };
    let (origin, normal) = plane(std::slice::from_ref(face), &samples(face))
        .map_err(|_| "an extrude face that is not planar is not supported yet".to_owned())?;
    Ok((planar_point(face, origin, normal)?, normal))
}

/// The ends and middle of every boundary edge of `face`.
fn samples(face: &Boundary) -> Vec<V3> {
    face.edges
        .iter()
        .flat_map(|edge| {
            let (start, end) = edge.ends();
            let mid = match edge {
                Edge::Line { start, end } => scale(add(*start, *end), 0.5),
                Edge::Arc { mid, .. } | Edge::Circle { mid, .. } => *mid,
            };
            [start, mid, end]
        })
        .collect()
}

/// A point on one face: planar when its boundary is flat, else the cylinder it is part of.
fn point_in(face: &Boundary) -> Result<V3, String> {
    let samples: Vec<V3> = face
        .edges
        .iter()
        .flat_map(|edge| {
            let (start, end) = edge.ends();
            let mid = match edge {
                Edge::Line { start, end } => scale(add(*start, *end), 0.5),
                Edge::Arc { mid, .. } | Edge::Circle { mid, .. } => *mid,
            };
            [start, mid, end]
        })
        .collect();
    match plane(std::slice::from_ref(face), &samples) {
        Ok((origin, normal)) => planar_point(face, origin, normal),
        Err(_) => cylinder_point(face),
    }
}

/// The point of a planar face farthest from its boundary among the centres of the triangles
/// its outline corners make that lie inside it, holes left out.
fn planar_point(face: &Boundary, origin: V3, normal: V3) -> Result<V3, String> {
    let helper = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]
        .into_iter()
        .min_by(|a, b| dot(*a, normal).abs().total_cmp(&dot(*b, normal).abs()))
        .ok_or("its face has no plane")?;
    let x = unit(cross(helper, normal)).ok_or("its face has no plane")?;
    let y = unit(cross(normal, x)).ok_or("its face has no plane")?;
    let flat = |p: V3| {
        let d = sub(p, origin);
        [dot(d, x), dot(d, y)]
    };
    let mut rings: Vec<Vec<[f64; 2]>> = Vec::new();
    for ring in loops(&face.edges)? {
        let mut polygon = Vec::new();
        for (index, forward) in ring {
            let edge = face.edges.get(index).ok_or(BAD)?;
            let (a, b) = edge.ends();
            let from = if forward { a } else { b };
            polygon.push(flat(from));
            match edge {
                Edge::Line { .. } => {}
                Edge::Arc {
                    center,
                    radius,
                    axis,
                    ..
                } => {
                    let c = flat(*center);
                    let p = flat(from);
                    let turn = edge_sweep(edge) * dot(*axis, normal).signum();
                    let turn = if forward { turn } else { -turn };
                    let first = (p[1] - c[1]).atan2(p[0] - c[0]);
                    for k in 1..ARC_STEPS {
                        let at = first + turn * f64::from(k) / f64::from(ARC_STEPS);
                        polygon.push([c[0] + radius * at.cos(), c[1] + radius * at.sin()]);
                    }
                }
                Edge::Circle { center, radius, .. } => {
                    let c = flat(*center);
                    polygon.clear();
                    for k in 0..CIRCLE_STEPS {
                        let at = TAU * f64::from(k) / f64::from(CIRCLE_STEPS);
                        polygon.push([c[0] + radius * at.cos(), c[1] + radius * at.sin()]);
                    }
                }
            }
        }
        rings.push(polygon);
    }
    let inside_face = |p: [f64; 2]| rings.iter().filter(|ring| inside(ring, p)).count() % 2 == 1;
    let clearance = |p: [f64; 2]| {
        rings
            .iter()
            .flat_map(|ring| {
                (0..ring.len()).filter_map(move |i| {
                    Some(segment_distance(
                        p,
                        *ring.get(i)?,
                        *ring.get((i + 1) % ring.len())?,
                    ))
                })
            })
            .fold(f64::INFINITY, f64::min)
    };
    let mut best: Option<([f64; 2], f64)> = None;
    for ring in &rings {
        let n = ring.len();
        for i in 0..n {
            let (Some(a), Some(b), Some(c)) = (
                ring.get((i + n - 1) % n),
                ring.get(i),
                ring.get((i + 1) % n),
            ) else {
                continue;
            };
            let centre = [(a[0] + b[0] + c[0]) / 3.0, (a[1] + b[1] + c[1]) / 3.0];
            if !inside_face(centre) {
                continue;
            }
            let room = clearance(centre);
            if room > 1e-3 && best.is_none_or(|(_, most)| room > most) {
                best = Some((centre, room));
            }
        }
    }
    let ([u, v], _) = best.ok_or("its face has no resolved planar interior point")?;
    Ok(add(origin, add(scale(x, u), scale(y, v))))
}

/// How far `p` is from the segment `a`-`b`.
fn segment_distance(p: [f64; 2], a: [f64; 2], b: [f64; 2]) -> f64 {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let length = dx * dx + dy * dy;
    let t = if length > 0.0 {
        (((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / length).clamp(0.0, 1.0)
    } else {
        0.0
    };
    ((p[0] - a[0] - t * dx).powi(2) + (p[1] - a[1] - t * dy).powi(2)).sqrt()
}

/// A point in the middle of a cylindrical face: its boundary is circles or arcs of one radius
/// about one axis, joined by straight edges along the axis. A full cylinder gets the point
/// opposite its seam, a partial one the middle of its arcs.
fn cylinder_point(face: &Boundary) -> Result<V3, String> {
    const NOT: &str = "a face other than a plane or a cylinder is not supported yet";
    let curved: Vec<(V3, f64, V3, bool)> = face
        .edges
        .iter()
        .filter_map(|edge| match edge {
            Edge::Arc {
                mid,
                center,
                radius,
                ..
            } => Some((*center, *radius, *mid, false)),
            Edge::Circle {
                start,
                center,
                radius,
                ..
            } => Some((*center, *radius, *start, true)),
            Edge::Line { .. } => None,
        })
        .collect();
    let (first, rest) = curved.split_first().ok_or(NOT)?;
    let far = rest
        .iter()
        .max_by(|a, b| norm(sub(a.0, first.0)).total_cmp(&norm(sub(b.0, first.0))))
        .ok_or(NOT)?;
    let axis = unit(sub(far.0, first.0)).ok_or(NOT)?;
    let radius = first.1;
    let seams = face
        .edges
        .iter()
        .filter(|edge| matches!(edge, Edge::Line { .. }))
        .count();
    if seams == 0
        || curved.iter().any(|(center, r, _, _)| {
            (r - radius).abs() > NEAR * radius.max(1.0)
                || norm(cross(axis, sub(*center, first.0))) > NEAR * radius.max(1.0)
        })
        || face.edges.iter().any(|edge| match edge {
            Edge::Line { start, end } => {
                unit(sub(*end, *start)).is_none_or(|line| norm(cross(line, axis)) > ANGLE)
            }
            _ => false,
        })
    {
        return Err(NOT.to_owned());
    }
    let along: Vec<f64> = curved
        .iter()
        .map(|c| dot(sub(c.0, first.0), axis))
        .collect();
    let low = along.iter().copied().fold(f64::INFINITY, f64::min);
    let high = along.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    if high - low <= NEAR {
        return Err(NOT.to_owned());
    }
    let middle = add(first.0, scale(axis, (low + high) / 2.0));
    let radial = |point: V3, center: V3| {
        let d = sub(point, center);
        sub(d, scale(axis, dot(d, axis)))
    };
    let direction = match curved.iter().find(|c| c.3) {
        Some(circle) => scale(radial(circle.2, circle.0), -1.0),
        None => curved
            .iter()
            .filter_map(|c| unit(radial(c.2, c.0)))
            .fold([0.0; 3], add),
    };
    let direction = unit(direction).ok_or(NOT)?;
    Ok(add(middle, scale(direction, radius)))
}

/// Reads the boundary of every selected face from the feature's input topology.
fn boundaries(feature: &Value, value: &Value, ctx: &Context) -> Result<Vec<Boundary>, String> {
    let state_id = value
        .get("state")
        .and_then(Value::as_str)
        .ok_or("its input topology is not resolved")?;
    let states = ctx.arena("feature_input_topologies");
    if states.len() > 10_000 {
        return Err("its input topology exceeds the face resolution limit".to_owned());
    }
    let mut states = states
        .iter()
        .filter(|state| state.get("id").and_then(Value::as_str) == Some(state_id));
    let state = states.next().ok_or("its input topology is missing")?;
    if states.next().is_some() || state.get("input_of") != feature.get("id") {
        return Err("its input topology does not uniquely belong to this feature".to_owned());
    }
    let ids = value
        .get("faces")
        .and_then(Value::as_array)
        .filter(|ids| !ids.is_empty() && ids.len() <= FACES)
        .ok_or("its face selection is empty or exceeds the face limit")?;
    let members = state
        .get("faces")
        .and_then(Value::as_array)
        .ok_or("its input faces are missing")?;
    let geometry = state
        .get("edge_geometry")
        .and_then(Value::as_array)
        .filter(|rows| rows.len() <= 10_000)
        .ok_or(
            "its input topology carries no edge geometry for the selected face, so the face's \
             boundary is not known",
        )?;
    let mut used: Vec<&str> = Vec::new();
    let mut faces = Vec::new();
    let mut total = 0usize;
    for id in ids {
        if !id.is_string() || members.iter().filter(|member| *member == id).count() != 1 {
            return Err(
                "its selected face is not a unique member of its input topology".to_owned(),
            );
        }
        let mut edges = Vec::new();
        for row in geometry.iter().filter(|row| {
            row.get("faces")
                .and_then(Value::as_array)
                .is_some_and(|faces| faces.contains(id))
        }) {
            let name = row.get("edge").and_then(Value::as_str).ok_or(BAD)?;
            if used.contains(&name) {
                return Err(
                    "its selected faces share an edge, or an edge is sampled twice".to_owned(),
                );
            }
            used.push(name);
            edges.push(edge(row)?);
        }
        if edges.is_empty() {
            return Err(
                "its input topology carries no edge geometry for the selected face, so the \
                 face's boundary is not known"
                    .to_owned(),
            );
        }
        total = total.saturating_add(edges.len());
        if total > EDGES {
            return Err("its faces have more boundary edges than the face limit".to_owned());
        }
        faces.push(Boundary { edges, area: 0.0 });
    }
    Ok(faces)
}

const BAD: &str = "a boundary edge of its face is malformed";

/// One sampled edge as a line, an arc or a circle.
fn edge(row: &Value) -> Result<Edge, String> {
    let mid = v3(row.get("interior")).ok_or(BAD)?;
    let start = v3(row.get("start")).ok_or("a boundary edge of its face has no end position")?;
    let end = v3(row.get("end")).ok_or("a boundary edge of its face has no end position")?;
    let tangent = v3(row.get("tangent")).and_then(unit);
    let chord = sub(end, start);
    if norm(chord) <= NEAR {
        // A closed edge: the circle whose diameter runs from its end to its far point.
        let diameter = sub(mid, start);
        let tangent = tangent.ok_or("a closed boundary edge of its face has no tangent")?;
        let radius = norm(diameter) / 2.0;
        let straight = unit(diameter).ok_or(BAD)?;
        if radius <= NEAR || dot(straight, tangent).abs() > ANGLE {
            return Err("a closed boundary edge of its face is not a circle".to_owned());
        }
        let axis = unit(cross(diameter, tangent)).ok_or(BAD)?;
        return Ok(Edge::Circle {
            start,
            mid,
            center: scale(add(start, mid), 0.5),
            radius,
            axis,
        });
    }
    let along = cross(sub(mid, start), chord);
    if norm(along) <= NEAR * norm(chord) {
        let t = dot(sub(mid, start), chord) / dot(chord, chord);
        if !(0.0..=1.0).contains(&t) {
            return Err(BAD.to_owned());
        }
        if let Some(tangent) = tangent {
            if (dot(unit(chord).ok_or(BAD)?, tangent).abs() - 1.0).abs() > ANGLE {
                return Err(BAD.to_owned());
            }
        }
        return Ok(Edge::Line { start, end });
    }
    // The circle through the ends and the midpoint, if the tangent there is square to its
    // radius and along the chord, as it is on a circular arc.
    let (ab, ac) = (sub(mid, start), chord);
    let w = cross(ab, ac);
    let center = add(
        start,
        scale(
            add(
                scale(cross(w, ab), dot(ac, ac)),
                scale(cross(ac, w), dot(ab, ab)),
            ),
            1.0 / (2.0 * dot(w, w)),
        ),
    );
    let radius = norm(sub(center, start));
    let tangent = tangent
        .ok_or("a boundary edge of its face is neither a line nor an arc with a known tangent")?;
    let radial = unit(sub(mid, center)).ok_or(BAD)?;
    if !radius.is_finite()
        || dot(radial, tangent).abs() > ANGLE
        || norm(cross(unit(chord).ok_or(BAD)?, tangent)) > ANGLE
    {
        return Err("a boundary edge of its face is neither a line nor a circular arc".to_owned());
    }
    Ok(Edge::Arc {
        start,
        mid,
        end,
        center,
        radius,
        axis: unit(w).ok_or(BAD)?,
    })
}

/// A point on the faces' plane and its unit normal, which every boundary point must lie on.
fn plane(faces: &[Boundary], points: &[V3]) -> Result<(V3, V3), String> {
    let origin = *points.first().ok_or("its face has no boundary")?;
    let far = points
        .iter()
        .copied()
        .max_by(|a, b| norm(sub(*a, origin)).total_cmp(&norm(sub(*b, origin))))
        .ok_or("its face has no boundary")?;
    let size = norm(sub(far, origin)).max(1.0);
    let line = unit(sub(far, origin));
    let third = line.and_then(|line| {
        points
            .iter()
            .copied()
            .map(|p| (p, norm(cross(line, sub(p, origin)))))
            .max_by(|a, b| a.1.total_cmp(&b.1))
            .filter(|(_, off)| *off > NEAR * size)
            .map(|(p, _)| p)
    });
    // A boundary of one circle, or of points on one line, takes its normal from the curve.
    let normal = match third {
        Some(third) => unit(cross(sub(far, origin), sub(third, origin))),
        None => faces
            .iter()
            .flat_map(|face| &face.edges)
            .find_map(|edge| match edge {
                Edge::Arc { axis, .. } | Edge::Circle { axis, .. } => Some(*axis),
                Edge::Line { .. } => None,
            }),
    }
    .ok_or("its face boundary does not span a plane")?;
    let flat = NEAR * size;
    if points
        .iter()
        .any(|p| dot(sub(*p, origin), normal).abs() > flat)
        || faces
            .iter()
            .flat_map(|face| &face.edges)
            .any(|edge| match edge {
                Edge::Arc { axis, .. } | Edge::Circle { axis, .. } => {
                    norm(cross(*axis, normal)) > ANGLE
                }
                Edge::Line { .. } => false,
            })
    {
        return Err("its face boundary is not planar".to_owned());
    }
    Ok((origin, normal))
}

/// The area of a face: its loops chained from its edges, with the loops inside an odd number
/// of others taken away, in the frame `origin`, `x`, `y`.
fn area(face: &Boundary, origin: V3, x: V3, y: V3) -> Result<f64, String> {
    let normal = unit(cross(x, y)).ok_or("its face has no plane")?;
    let loops = loops(&face.edges)?;
    let flat = |p: V3| {
        let d = sub(p, origin);
        [dot(d, x), dot(d, y)]
    };
    let mut rings: Vec<(f64, Vec<[f64; 2]>)> = Vec::new();
    for ring in &loops {
        let mut twice = 0.0;
        let mut polygon = Vec::new();
        for &(index, forward) in ring {
            let edge = face.edges.get(index).ok_or(BAD)?;
            let (a, b) = edge.ends();
            let (from, to) = if forward { (a, b) } else { (b, a) };
            let (p, q) = (flat(from), flat(to));
            polygon.push(p);
            match edge {
                Edge::Line { .. } => twice += p[0] * q[1] - p[1] * q[0],
                Edge::Arc {
                    center,
                    radius,
                    axis,
                    ..
                } => {
                    let c = flat(*center);
                    let turn = edge_sweep(edge) * dot(*axis, normal).signum();
                    let turn = if forward { turn } else { -turn };
                    twice += radius * radius * turn + c[0] * (q[1] - p[1]) - c[1] * (q[0] - p[0]);
                    let first = (p[1] - c[1]).atan2(p[0] - c[0]);
                    for k in 1..ARC_STEPS {
                        let at = first + turn * f64::from(k) / f64::from(ARC_STEPS);
                        polygon.push([c[0] + radius * at.cos(), c[1] + radius * at.sin()]);
                    }
                }
                Edge::Circle { center, radius, .. } => {
                    let c = flat(*center);
                    twice += TAU * radius * radius;
                    for k in 0..CIRCLE_STEPS {
                        let at = TAU * f64::from(k) / f64::from(CIRCLE_STEPS);
                        polygon.push([c[0] + radius * at.cos(), c[1] + radius * at.sin()]);
                    }
                }
            }
        }
        rings.push((twice.abs() / 2.0, polygon));
    }
    let mut total = 0.0;
    for (i, (size, polygon)) in rings.iter().enumerate() {
        let Some(at) = polygon.first() else {
            return Err(BAD.to_owned());
        };
        let depth = rings
            .iter()
            .enumerate()
            .filter(|(j, (_, other))| *j != i && inside(other, *at))
            .count();
        total += if depth % 2 == 0 { *size } else { -*size };
    }
    if total.is_finite() && total > 0.0 {
        Ok(total)
    } else {
        Err("its face has no positive area".to_owned())
    }
}

const ARC_STEPS: u32 = 16;
const CIRCLE_STEPS: u32 = 64;

/// The signed angle an arc turns from its start through its midpoint to its end, about its axis.
fn edge_sweep(edge: &Edge) -> f64 {
    match edge {
        Edge::Arc {
            start,
            mid,
            end,
            center,
            axis,
            ..
        } => {
            signed_angle(sub(*start, *center), sub(*mid, *center), *axis)
                + signed_angle(sub(*mid, *center), sub(*end, *center), *axis)
        }
        _ => 0.0,
    }
}

/// Chains edges into closed loops, each as (edge index, whether it runs start to end).
fn loops(edges: &[Edge]) -> Result<Vec<Vec<(usize, bool)>>, String> {
    let mut left: Vec<usize> = Vec::new();
    let mut out = Vec::new();
    for (index, edge) in edges.iter().enumerate() {
        if matches!(edge, Edge::Circle { .. }) {
            out.push(vec![(index, true)]);
        } else {
            left.push(index);
        }
    }
    while let Some(first) = left.pop() {
        let (begin, mut at) = edges.get(first).ok_or(BAD)?.ends();
        let mut ring = vec![(first, true)];
        while !close(at, begin) {
            let mut next = left.iter().copied().filter(|&i| {
                edges.get(i).is_some_and(|edge| {
                    let (a, b) = edge.ends();
                    close(a, at) || close(b, at)
                })
            });
            let (Some(found), None) = (next.next(), next.next()) else {
                return Err("its face boundary does not close into loops".to_owned());
            };
            left.retain(|&i| i != found);
            let (a, b) = edges.get(found).ok_or(BAD)?.ends();
            let forward = close(a, at);
            at = if forward { b } else { a };
            ring.push((found, forward));
        }
        out.push(ring);
    }
    Ok(out)
}

fn close(a: V3, b: V3) -> bool {
    norm(sub(a, b)) <= NEAR
}

/// Whether `p` is inside the polygon (even-odd).
fn inside(polygon: &[[f64; 2]], p: [f64; 2]) -> bool {
    let mut inside = false;
    let n = polygon.len();
    for i in 0..n {
        let (Some(a), Some(b)) = (polygon.get(i), polygon.get((i + 1) % n)) else {
            continue;
        };
        if (a[1] > p[1]) != (b[1] > p[1])
            && p[0] < (b[0] - a[0]) * (p[1] - a[1]) / (b[1] - a[1]) + a[0]
        {
            inside = !inside;
        }
    }
    inside
}

/// Finds each mapped face among the faces SolidCraft rebuilt before the extrude, and says
/// whether the sketch has to be mirrored to run along their outward normal.
pub(crate) fn prove(proof: &Proof, replay: &Replay) -> Result<bool, String> {
    if replay.faces.len() != replay.bodies.len() {
        return Err("SolidCraft's replay of the history before it has no faces".to_owned());
    }
    let normal = unit(cross(proof.x, proof.y)).ok_or("its face has no plane")?;
    let mut sides = Vec::new();
    for face in &proof.faces {
        // SolidCraft measures a face with curved edges from its mesh, a little under the exact
        // area, so those faces are compared to a percent and the edges decide.
        let curved = face
            .edges
            .iter()
            .any(|edge| !matches!(edge, Edge::Line { .. }));
        let slack = if curved { 0.02 } else { 1e-4 };
        let mut found = Vec::new();
        // The nearest in area of the faces on its plane, for the reason a face is not found.
        let mut nearest: Option<(f64, &ReplayedFace, &[ReplayedEdge])> = None;
        for ((_, edges), faces) in replay.bodies.iter().zip(&replay.faces) {
            for candidate in faces {
                let Some(outward) = candidate.normal.and_then(unit) else {
                    continue;
                };
                if (dot(outward, normal).abs() - 1.0).abs() > ANGLE
                    || dot(sub(candidate.point, proof.origin), normal).abs() > 1e-4
                {
                    continue;
                }
                let off = (candidate.area - face.area).abs();
                if nearest.is_none_or(|(best, _, _)| off < best) {
                    nearest = Some((off, candidate, edges));
                }
                if off > slack * face.area.max(1.0) {
                    continue;
                }
                // Its edges are the face's, no more and no fewer.
                let named: Vec<&ReplayedEdge> = edges
                    .iter()
                    .filter(|rebuilt| rebuilt.name.split('|').any(|name| name == candidate.name))
                    .collect();
                let bounded = bounds(face, &named);
                if bounded {
                    found.push(dot(outward, normal) < 0.0);
                }
            }
        }
        match found.as_slice() {
            [side] => sides.push(*side),
            [] => return Err(not_found(face, nearest)),
            _ => {
                return Err(
                    "its face matches several faces SolidCraft rebuilds before it".to_owned(),
                )
            }
        }
    }
    let mirrored = sides.first().copied().ok_or("its face is not found")?;
    if sides.iter().any(|side| *side != mirrored) {
        return Err("its faces do not face the same way".to_owned());
    }
    Ok(mirrored)
}

/// Why a face is not among the rebuilt ones: the face on its plane nearest in area, and how its
/// edges differ.
fn not_found(face: &Boundary, nearest: Option<(f64, &ReplayedFace, &[ReplayedEdge])>) -> String {
    let head = format!(
        "its face ({:.4} mm2, {} boundary edges) is not among the faces SolidCraft rebuilds \
         before it",
        face.area,
        face.edges.len()
    );
    let Some((_, candidate, edges)) = nearest else {
        return format!("{head}: none lies on its plane");
    };
    let named: Vec<&ReplayedEdge> = edges
        .iter()
        .filter(|rebuilt| rebuilt.name.split('|').any(|name| name == candidate.name))
        .collect();
    let unmatched = face
        .edges
        .iter()
        .position(|edge| !named.iter().any(|rebuilt| lies_on(edge, rebuilt)));
    let detail = match unmatched {
        Some(k) => {
            let shown = |p: V3| format!("({:.3}, {:.3}, {:.3})", p[0], p[1], p[2]);
            let mine = face.edges.get(k).map_or_else(String::new, |edge| {
                let (a, b) = edge.ends();
                let kind = match edge {
                    Edge::Line { .. } => "line",
                    Edge::Arc { .. } => "arc",
                    Edge::Circle { .. } => "circle",
                };
                format!("{kind} {}-{} long {:.4}", shown(a), shown(b), edge.length())
            });
            let theirs: Vec<String> = named
                .iter()
                .take(4)
                .map(|rebuilt| {
                    format!(
                        "{}-{} long {:.4}",
                        shown(rebuilt.start),
                        shown(rebuilt.end),
                        rebuilt.length
                    )
                })
                .collect();
            format!(
                "its edge {k} of {} ({mine}) has no match among [{}]",
                face.edges.len(),
                theirs.join("; ")
            )
        }
        None => "its edges all match".to_owned(),
    };
    format!(
        "{head}: the nearest face on its plane is {:.4} mm2 with {} edges, and {detail}",
        candidate.area,
        named.len()
    )
}

/// Whether `named`, the edges SolidCraft names for a face, are exactly the face's edges: each
/// lies along one of them, and together they run its whole length. SolidCraft may split a
/// circle in two, and measures a curved edge from its mesh, a little short.
fn bounds(face: &Boundary, named: &[&ReplayedEdge]) -> bool {
    let mut covered = vec![0.0; face.edges.len()];
    for rebuilt in named {
        let mut along = face
            .edges
            .iter()
            .enumerate()
            .filter(|(_, edge)| lies_on(edge, rebuilt));
        let (Some((k, _)), None) = (along.next(), along.next()) else {
            return false;
        };
        if let Some(total) = covered.get_mut(k) {
            *total += rebuilt.length;
        }
    }
    face.edges.iter().zip(&covered).all(|(edge, total)| {
        let slack = if matches!(edge, Edge::Line { .. }) {
            1e-4
        } else {
            2e-2
        };
        (total - edge.length()).abs() <= slack * edge.length().max(1.0)
    })
}

/// Whether SolidCraft's edge, by its ends and midpoint, lies along the sampled `edge`.
fn lies_on(edge: &Edge, rebuilt: &ReplayedEdge) -> bool {
    [rebuilt.start, rebuilt.mid, rebuilt.end]
        .into_iter()
        .all(|p| match edge {
            Edge::Line { start, end } => {
                let along = sub(*end, *start);
                let t = dot(sub(p, *start), along) / dot(along, along);
                (-1e-6..=1.0 + 1e-6).contains(&t)
                    && norm(sub(p, add(*start, scale(along, t)))) <= 1e-4
            }
            Edge::Circle { center, radius, .. } => {
                (norm(sub(p, *center)) - radius).abs() <= 1e-3 * radius.max(1.0)
            }
            Edge::Arc {
                start,
                center,
                radius,
                axis,
                ..
            } => {
                let turn = edge_sweep(edge);
                let at = signed_angle(sub(*start, *center), sub(p, *center), *axis);
                // Within the arc's own turn, give or take the ends.
                let from_start = if turn >= 0.0 { at } else { -at };
                let from_start = if from_start < -1e-3 {
                    from_start + TAU
                } else {
                    from_start
                };
                (norm(sub(p, *center)) - radius).abs() <= 1e-3 * radius.max(1.0)
                    && from_start <= turn.abs() + 1e-3
            }
        })
}

fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn add(a: V3, b: V3) -> V3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn scale(a: V3, k: f64) -> V3 {
    [a[0] * k, a[1] * k, a[2] * k]
}

fn norm(a: V3) -> f64 {
    dot(a, a).sqrt()
}

/// The angle from `a` to `b` about `axis`, in (-pi, pi].
fn signed_angle(a: V3, b: V3, axis: V3) -> f64 {
    dot(cross(a, b), axis).atan2(dot(a, b))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]

    use super::*;

    fn point(x: f64, y: f64) -> Value {
        json!({"x": x, "y": y, "z": 0.0})
    }

    fn row(start: (f64, f64), mid: (f64, f64), end: (f64, f64), tangent: (f64, f64)) -> Value {
        json!({
            "edge": "e", "interior": point(mid.0, mid.1),
            "start": point(start.0, start.1), "end": point(end.0, end.1),
            "tangent": point(tangent.0, tangent.1),
        })
    }

    const FRAME: (V3, V3, V3) = ([0.0; 3], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]);

    fn face(edges: Vec<Edge>) -> f64 {
        let boundary = Boundary { edges, area: 0.0 };
        area(&boundary, FRAME.0, FRAME.1, FRAME.2).unwrap()
    }

    fn circle(radius: f64) -> Edge {
        let r = row((radius, 0.0), (-radius, 0.0), (radius, 0.0), (0.0, -1.0));
        edge(&r).unwrap()
    }

    #[test]
    fn a_circle_inside_a_circle_is_a_hole() {
        let area = face(vec![circle(10.0), circle(4.0)]);
        assert!((area - std::f64::consts::PI * 84.0).abs() < 1e-6, "{area}");
    }

    #[test]
    fn an_arc_and_its_chord_close_a_half_disc_either_way_round() {
        for flip in [1.0, -1.0] {
            let top = edge(&row(
                (10.0, 0.0),
                (0.0, 10.0 * flip),
                (-10.0, 0.0),
                (-1.0, 0.0),
            ))
            .unwrap();
            assert!(matches!(top, Edge::Arc { .. }));
            let chord = edge(&row((-10.0, 0.0), (0.0, 0.0), (10.0, 0.0), (1.0, 0.0))).unwrap();
            let area = face(vec![top.clone(), chord]);
            assert!(
                (area - std::f64::consts::PI * 50.0).abs() < 1e-6,
                "{flip}: {area}"
            );
            assert!((top.length() - std::f64::consts::PI * 10.0).abs() < 1e-6);
        }
    }

    #[test]
    fn an_open_boundary_is_refused() {
        let line = |a: (f64, f64), b: (f64, f64)| {
            edge(&row(
                a,
                ((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0),
                b,
                (b.0 - a.0, b.1 - a.1),
            ))
            .unwrap()
        };
        let boundary = Boundary {
            edges: vec![
                line((0.0, 0.0), (10.0, 0.0)),
                line((10.0, 0.0), (10.0, 10.0)),
            ],
            area: 0.0,
        };
        assert!(area(&boundary, FRAME.0, FRAME.1, FRAME.2)
            .unwrap_err()
            .contains("does not close"));
    }

    #[test]
    fn a_curve_that_is_not_a_circle_is_refused() {
        // Its tangent at the midpoint is not along the chord.
        let bad = edge(&row((0.0, 0.0), (5.0, 3.0), (10.0, 0.0), (1.0, 1.0)));
        assert!(bad.is_err());
    }

    fn arc_row(centre_z: f64, from: f64, to: f64) -> Value {
        // A quarter arc of radius 5 about the z axis at height `centre_z`.
        let at = |a: f64| json!({"x": 5.0 * a.cos(), "y": 5.0 * a.sin(), "z": centre_z});
        let mid = (from + to) / 2.0;
        json!({
            "edge": "e", "interior": at(mid), "start": at(from), "end": at(to),
            "tangent": {"x": -mid.sin(), "y": mid.cos(), "z": 0.0},
        })
    }

    #[test]
    fn a_partial_cylinder_gets_a_point_on_its_middle() {
        let quarter = std::f64::consts::FRAC_PI_2;
        let line = |angle: f64| {
            let at = |z: f64| json!({"x": 5.0 * angle.cos(), "y": 5.0 * angle.sin(), "z": z});
            json!({
                "edge": "e", "interior": at(5.0), "start": at(0.0), "end": at(10.0),
                "tangent": {"x": 0.0, "y": 0.0, "z": 1.0},
            })
        };
        let rows = [
            arc_row(0.0, 0.0, quarter),
            arc_row(10.0, 0.0, quarter),
            line(0.0),
            line(quarter),
        ];
        let edges: Vec<Edge> = rows.iter().map(|r| edge(r).unwrap()).collect();
        let p = point_in(&Boundary { edges, area: 1.0 }).unwrap();
        let half = quarter / 2.0;
        assert!((p[0] - 5.0 * half.cos()).abs() < 1e-6, "{p:?}");
        assert!((p[1] - 5.0 * half.sin()).abs() < 1e-6, "{p:?}");
        assert!((p[2] - 5.0).abs() < 1e-6, "{p:?}");
    }

    #[test]
    fn a_square_face_gets_a_point_well_inside() {
        let corners = [(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)];
        let edges: Vec<Edge> = (0..4)
            .map(|k| {
                let (a, b) = (corners[k], corners[(k + 1) % 4]);
                edge(&row(
                    a,
                    ((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0),
                    b,
                    ((b.0 - a.0) / 10.0, (b.1 - a.1) / 10.0),
                ))
                .unwrap()
            })
            .collect();
        let p = point_in(&Boundary { edges, area: 100.0 }).unwrap();
        assert!(
            p[0] > 1.0 && p[0] < 9.0 && p[1] > 1.0 && p[1] < 9.0,
            "{p:?}"
        );
    }
}
