//! Planar sketches, as SolidCraft sketch features.
//!
//! Geometry is written at its solved positions, so a sketch is exact whatever happens to its
//! constraints. Curves that meet share a SolidCraft point: by the source's endpoint records
//! where it gives them, else by position.

use std::collections::BTreeMap;

use dutchman_core::{Report, Severity};
use serde_json::{json, Value};

use super::{constraints, name, Context};

/// Positions closer than this (mm) are one point.
const COINCIDENT: f64 = 1e-6;

/// Directions closer than this are the same axis.
const PARALLEL: f64 = 1e-9;

/// A 2D point in sketch coordinates.
pub(crate) type P2 = [f64; 2];

/// A 3D vector.
pub(crate) type V3 = [f64; 3];

/// A mapped sketch, as later features refer to it.
pub(crate) struct Mapped {
    /// The SolidCraft sketch feature's id.
    pub(crate) feature: u64,
    /// The sketch plane's unit normal, in model space.
    pub(crate) normal: V3,
    /// The sketch plane's origin, in model space.
    pub(crate) origin: V3,
    /// The sketch plane's unit x and y axes, in model space.
    pub(crate) x_dir: V3,
    pub(crate) y_dir: V3,
    /// The SolidCraft sketch's points, by index: their ids and positions in the sketch.
    pub(crate) points: Vec<(String, P2)>,
    /// What each source entity became, by its id.
    pub(crate) shapes: BTreeMap<String, Shape>,
}

impl Mapped {
    /// Expand source boundaries into every exact emitted piece in traversal order.
    pub(crate) fn boundaries(
        &self,
        entities: Vec<Vec<String>>,
    ) -> Result<Vec<Vec<String>>, String> {
        entities
            .into_iter()
            .map(|ring| {
                ring.into_iter()
                    .map(|entity| {
                        self.curve(&entity).map(Curve::ids).ok_or_else(|| {
                            "its profile uses a curve the sketch does not carry".to_owned()
                        })
                    })
                    .collect::<Result<Vec<_>, _>>()
                    .map(|chains| chains.into_iter().flatten().collect())
            })
            .collect()
    }

    /// Where the sketch point `at` is in model space.
    pub(crate) fn world(&self, at: P2) -> V3 {
        let [u, v] = at;
        [0, 1, 2].map(|i| {
            self.origin.get(i).copied().unwrap_or(0.0)
                + u * self.x_dir.get(i).copied().unwrap_or(0.0)
                + v * self.y_dir.get(i).copied().unwrap_or(0.0)
        })
    }
}

impl Mapped {
    /// The SolidCraft curve the source entity `entity` became.
    pub(crate) fn curve(&self, entity: &str) -> Option<&Curve> {
        match self.shapes.get(entity)? {
            Shape::Curve(curve) if curve.kind != "compound_conic" => Some(curve),
            Shape::Curve(_) => None,
            Shape::Point(_) => None,
        }
    }
}

/// Maps a sketch feature whose operation is `definition`.
pub(crate) fn map(
    feature: &Value,
    definition: &Value,
    ctx: &mut Context,
    report: &mut Report,
) -> Result<Value, String> {
    let binding = definition.get("sketch").unwrap_or(&Value::Null);
    if binding.get("space").and_then(Value::as_str) != Some("planar") {
        return Err("only planar sketches are mapped".to_owned());
    }
    let sketch_id = binding
        .get("sketch")
        .and_then(Value::as_str)
        .ok_or("the sketch it binds is not resolved")?;
    let sketch = ctx
        .arena("sketches")
        .iter()
        .find(|row| row.get("id").and_then(Value::as_str) == Some(sketch_id))
        .ok_or("the sketch it binds is missing")?;
    let (plane, frame) = plane(sketch)?;
    let entities: Vec<&Value> = ctx
        .arena("sketch_entities")
        .iter()
        .filter(|row| row.get("sketch").and_then(Value::as_str) == Some(sketch_id))
        .collect();
    // A plane at the height an earlier sketch's dimensions reach is offset by them.
    let (plane, frame, turn) = match follow_plane(ctx, &frame, &entities) {
        Some((plane, frame, turn)) => (plane, frame, Some(turn)),
        None => (plane, frame, None),
    };
    let Frame {
        normal,
        origin,
        x,
        y,
    } = frame;
    let super::crossing::Demoted {
        mut crossing,
        stray,
    } = super::crossing::demoted(sketch, &entities, ctx.arena("features"));
    let stray_count = stray.difference(&crossing).count();
    let crossing_count = crossing.len();
    crossing.extend(stray);
    let mut builder = Builder::default();
    for source in &entities {
        let moved = turn.and_then(|turn| turned(source, turn));
        let entity: &Value = moved.as_ref().unwrap_or(source);
        let is_crossing = entity
            .get("id")
            .and_then(Value::as_str)
            .is_some_and(|id| crossing.contains(id));
        if is_crossing {
            let mut construction = entity.clone();
            if let Some(fields) = construction.as_object_mut() {
                fields.insert("construction".to_owned(), Value::Bool(true));
            }
            builder.entity(&construction, report)?;
        } else {
            builder.entity(entity, report)?;
        }
    }
    let id = ctx.id();
    let name = name(feature)
        .map(str::to_owned)
        .unwrap_or_else(|| format!("Sketch{id}"));
    if stray_count > 0 {
        report.push(
            Severity::Info,
            format!("sketch \"{name}\""),
            format!(
                "{stray_count} sketch curve{} bounding no profile a feature uses written as \
                 construction",
                if stray_count == 1 { "" } else { "s" },
            ),
        );
    }
    if crossing_count > 0 {
        report.push(
            Severity::Info,
            format!("sketch \"{name}\""),
            format!(
                "{} drawn across a profile the sketch takes whole written as construction: \
                 solid, {} would split it",
                if crossing_count == 1 {
                    "1 line".to_owned()
                } else {
                    format!("{crossing_count} lines")
                },
                if crossing_count == 1 { "it" } else { "they" },
            ),
        );
    }
    let constraints = constraints::map(
        sketch_id,
        &format!("sketch \"{name}\""),
        ctx,
        &mut builder,
        report,
    );
    let mut constraints = constraints;
    for mut joint in builder.joints.clone() {
        if let Some(fields) = joint.as_object_mut() {
            fields.insert(
                "id".to_owned(),
                json!(format!("k{}", constraints.len() + 1)),
            );
        }
        constraints.push(joint);
    }
    let ties: Vec<&Value> = ctx
        .arena("sketch_constraints")
        .iter()
        .filter(|row| row.get("sketch").and_then(Value::as_str) == Some(sketch_id))
        .collect();
    if projected_origin(&entities) || origin_point_tied(&entities, &ties) {
        builder.anchor_at_origin(&mut constraints);
    }
    for mut hold in super::dof::holds(&builder.points, &builder.curves, &constraints) {
        if let Some(fields) = hold.as_object_mut() {
            fields.insert(
                "id".to_owned(),
                json!(format!("k{}", constraints.len() + 1)),
            );
        }
        constraints.push(hold);
    }
    ctx.coordinates.insert(
        sketch_id.to_owned(),
        super::chain::coordinates(&builder.points, &builder.curves, &constraints),
    );

    ctx.sketches.insert(
        sketch_id.to_owned(),
        Mapped {
            feature: id,
            normal,
            origin,
            x_dir: x,
            y_dir: y,
            points: builder
                .points
                .iter()
                .zip(&builder.positions)
                .map(|(point, at)| {
                    let id = point.get("id").and_then(Value::as_str).unwrap_or("");
                    (id.to_owned(), *at)
                })
                .collect(),
            shapes: builder.shapes.clone(),
        },
    );
    Ok(json!({
        "id": id,
        "name": name,
        "type": "sketch",
        "suppressed": false,
        "plane": plane,
        "sketch": {
            "points": builder.points,
            "curves": builder.curves,
            "constraints": constraints,
        },
    }))
}

/// A sketch made by the writer rather than read from the source: SolidCraft's feature, and the
/// mapped sketch later features refer to, for `entities` (CADIR sketch entity rows, in the
/// frame `origin`, `x`, `y`) on an explicit plane.
pub(crate) fn synthesized(
    id: u64,
    name: &str,
    frame: (V3, V3, V3),
    entities: &[Value],
) -> Result<(Value, Mapped), String> {
    let (origin, x, y) = frame;
    let normal = unit(cross(x, y)).ok_or("its plane's axes are parallel")?;
    let mut builder = Builder::default();
    let mut report = Report::default();
    for entity in entities {
        builder.entity(entity, &mut report)?;
    }
    let mapped = Mapped {
        feature: id,
        normal,
        origin,
        x_dir: x,
        y_dir: y,
        points: builder
            .points
            .iter()
            .zip(&builder.positions)
            .map(|(point, at)| {
                let id = point.get("id").and_then(Value::as_str).unwrap_or("");
                (id.to_owned(), *at)
            })
            .collect(),
        shapes: builder.shapes.clone(),
    };
    let feature = json!({
        "id": id,
        "name": name,
        "type": "sketch",
        "suppressed": false,
        "plane": {"type": "custom", "plane": {"origin": origin, "x_dir": x, "y_dir": y}},
        "sketch": {
            "points": builder.points,
            "curves": builder.curves,
            "constraints": [],
        },
    });
    Ok((feature, mapped))
}

/// Whether a profile touching the sketch origin is tied to a point of the sketch's own there:
/// more point entities lie on it than line and arc ends do, so one is the projected origin.
fn projected_origin(entities: &[&Value]) -> bool {
    let at_origin = |p: Option<P2>| p.is_some_and(|p| distance(p, [0.0, 0.0]) <= COINCIDENT);
    let (mut points, mut ends) = (0usize, 0usize);
    for entity in entities {
        let geometry = entity.get("geometry").unwrap_or(&Value::Null);
        match geometry.get("kind").and_then(Value::as_str) {
            Some("point") => points += usize::from(at_origin(p2(geometry.get("position")))),
            Some("line") => {
                ends += usize::from(at_origin(p2(geometry.get("start"))))
                    + usize::from(at_origin(p2(geometry.get("end"))));
            }
            Some("arc") => {
                let (Some(c), Some(r), Some(a0), Some(a1)) = (
                    p2(geometry.get("center")),
                    geometry.get("radius").and_then(Value::as_f64),
                    geometry.get("start_angle").and_then(Value::as_f64),
                    geometry.get("end_angle").and_then(Value::as_f64),
                ) else {
                    continue;
                };
                for angle in [a0, a1] {
                    ends += usize::from(at_origin(Some([
                        c[0] + r * angle.cos(),
                        c[1] + r * angle.sin(),
                    ])));
                }
            }
            _ => {}
        }
    }
    ends > 0 && points > ends
}

/// Whether a point entity at the sketch origin is what two or more coincidences tie to: the
/// projected origin of a sketch whose profile touches it, which the end-point count in
/// `projected_origin` cannot tell from the profile's own points when each end has its own.
fn origin_point_tied(entities: &[&Value], constraints: &[&Value]) -> bool {
    entities.iter().any(|entity| {
        let geometry = entity.get("geometry").unwrap_or(&Value::Null);
        let at_origin = geometry.get("kind").and_then(Value::as_str) == Some("point")
            && p2(geometry.get("position")).is_some_and(|p| distance(p, [0.0, 0.0]) <= COINCIDENT);
        let Some(id) = entity
            .get("id")
            .and_then(Value::as_str)
            .filter(|_| at_origin)
        else {
            return false;
        };
        constraints
            .iter()
            .filter(|row| {
                let definition = row.get("definition").unwrap_or(&Value::Null);
                matches!(
                    definition.get("kind").and_then(Value::as_str),
                    Some("coincident" | "coincident_loci")
                ) && definition.to_string().contains(&format!("\"{id}\""))
            })
            .count()
            >= 2
    })
}

/// The origin planes, with their axes and normals in model space.
const ORIGIN_PLANES: [(&str, V3, V3, V3); 3] = [
    ("XY", [1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]),
    ("XZ", [1.0, 0.0, 0.0], [0.0, 0.0, -1.0], [0.0, 1.0, 0.0]),
    ("YZ", [0.0, 1.0, 0.0], [0.0, 0.0, 1.0], [1.0, 0.0, 0.0]),
];

/// The plane of a sketch lying parallel to an origin plane at the height of a point of an
/// earlier sketch whose position the earlier sketch's dimensions set: that origin plane,
/// offset by those dimensions, so the sketch moves with them. Also the frame it then has and
/// how the source sketch's axes turn into that plane's (`[x.X, x.Y, y.X, y.Y]`; a sketch
/// drawn with its axes turned, as a face sketch often is, is written turned back). `None`
/// when the plane is not so placed or a curve could not be turned.
fn follow_plane(
    ctx: &Context,
    frame: &Frame,
    entities: &[&Value],
) -> Option<(Value, Frame, [f64; 4])> {
    let (name, xs, ys, ns) = ORIGIN_PLANES
        .into_iter()
        .find(|(_, _, _, ns)| same(frame.normal, *ns))?;
    let turn = [
        dot(frame.x, xs),
        dot(frame.x, ys),
        dot(frame.y, xs),
        dot(frame.y, ys),
    ];
    if !turn
        .iter()
        .all(|v| v.abs() <= 1e-9 || (v.abs() - 1.0).abs() <= 1e-9)
    {
        return None;
    }
    let height = dot(frame.origin, ns);
    if height.abs() <= COINCIDENT
        || dot(frame.origin, xs).abs() > COINCIDENT
        || dot(frame.origin, ys).abs() > COINCIDENT
    {
        return None;
    }
    let identity = (turn[0] - 1.0).abs() <= 1e-9 && (turn[3] - 1.0).abs() <= 1e-9;
    if !identity && !entities.iter().all(|e| turned(e, turn).is_some()) {
        return None;
    }
    let distance = earlier_height(ctx, ns, height)?;
    Some((
        json!({"type": "offset", "base": {"type": "origin", "name": name}, "distance": distance}),
        Frame {
            normal: ns,
            origin: [ns[0] * height, ns[1] * height, ns[2] * height],
            x: xs,
            y: ys,
        },
        turn,
    ))
}

/// The expression for `height` along `normal` (a principal axis) as an earlier sketch's
/// dimensions give it: a point of that sketch lies there, at a distance they set.
fn earlier_height(ctx: &Context, normal: V3, height: f64) -> Option<String> {
    let k = (0..3).find(|k| normal.get(*k).is_some_and(|n| n.abs() > 0.5))?;
    let sign = *normal.get(k)?;
    let mut sketches: Vec<(&String, &Mapped)> = ctx.sketches.iter().collect();
    sketches.sort_by_key(|(_, mapped)| std::cmp::Reverse(mapped.feature));
    for (id, mapped) in sketches {
        let Some(coordinates) = ctx.coordinates.get(id) else {
            continue;
        };
        let (cx, cy) = (*mapped.x_dir.get(k)?, *mapped.y_dir.get(k)?);
        let base = *mapped.origin.get(k)?;
        let axis = if (cx.abs() - 1.0).abs() <= 1e-9 && cy.abs() <= 1e-9 {
            0
        } else if (cy.abs() - 1.0).abs() <= 1e-9 && cx.abs() <= 1e-9 {
            1
        } else {
            continue;
        };
        let coefficient = if axis == 0 { cx } else { cy };
        for (index, (_, at)) in mapped.points.iter().enumerate() {
            let world = base + at.get(axis)? * coefficient;
            let Some(coordinate) = coordinates
                .get(index)
                .and_then(|c| c.get(axis)?.as_ref())
                .filter(|c| c.symbolic)
            else {
                continue;
            };
            if (world - height * sign).abs() > COINCIDENT {
                continue;
            }
            let term = if coefficient > 0.0 {
                coordinate.expression.clone()
            } else {
                format!("-({})", coordinate.expression)
            };
            let along = if base.abs() <= COINCIDENT {
                term
            } else {
                format!("{base} mm + ({term})")
            };
            return Some(if sign > 0.0 {
                along
            } else {
                format!("-({along})")
            });
        }
    }
    None
}

/// A source sketch entity with its coordinates in another plane's axes: `turn` is
/// `[x.X, x.Y, y.X, y.Y]`. `None` for a kind that is not turned.
fn turned(entity: &Value, turn: [f64; 4]) -> Option<Value> {
    let [a, b, c, d] = turn;
    let at = |value: Option<&Value>| -> Option<Value> {
        let [u, v] = p2(value)?;
        Some(json!({"u": a * u + c * v, "v": b * u + d * v}))
    };
    let angle =
        |theta: f64| (b * theta.cos() + d * theta.sin()).atan2(a * theta.cos() + c * theta.sin());
    let mut out = entity.clone();
    let geometry = out.get_mut("geometry")?.as_object_mut()?;
    match geometry.get("kind")?.as_str()? {
        "point" => {
            let position = at(geometry.get("position"))?;
            geometry.insert("position".to_owned(), position);
        }
        "line" => {
            let (start, end) = (at(geometry.get("start"))?, at(geometry.get("end"))?);
            geometry.insert("start".to_owned(), start);
            geometry.insert("end".to_owned(), end);
        }
        "circle" => {
            let center = at(geometry.get("center"))?;
            geometry.insert("center".to_owned(), center);
        }
        "arc" => {
            let center = at(geometry.get("center"))?;
            let (s, e) = (
                angle(geometry.get("start_angle")?.as_f64()?),
                angle(geometry.get("end_angle")?.as_f64()?),
            );
            geometry.insert("center".to_owned(), center);
            geometry.insert("start_angle".to_owned(), json!(s));
            geometry.insert("end_angle".to_owned(), json!(e));
        }
        _ => return None,
    }
    Some(out)
}

/// A sketch plane in model space: its origin and unit axes.
struct Frame {
    normal: V3,
    origin: V3,
    x: V3,
    y: V3,
}

/// The SolidCraft plane of a sketch, and its frame: an origin plane when the frame is one
/// exactly, else an explicit plane with the frame's axes.
fn plane(sketch: &Value) -> Result<(Value, Frame), String> {
    let placement = sketch.get("placement").unwrap_or(&Value::Null);
    if placement.get("kind").and_then(Value::as_str) != Some("resolved") {
        return Err("its plane is not resolved".to_owned());
    }
    let frame = placement.get("frame").unwrap_or(&Value::Null);
    let origin = v3(frame.get("origin")).ok_or("its plane has no origin")?;
    let normal = unit(v3(frame.get("normal")).ok_or("its plane has no normal")?)
        .ok_or("its plane's normal is zero")?;
    let x = unit(v3(frame.get("u_axis")).ok_or("its plane has no u axis")?)
        .ok_or("its plane's u axis is zero")?;
    if dot(x, normal).abs() > PARALLEL.sqrt() {
        return Err("its plane's u axis is not in the plane".to_owned());
    }
    let y = unit(cross(normal, x)).ok_or("its plane's axes are parallel")?;

    let at_origin = origin.iter().all(|c| c.abs() <= COINCIDENT);
    let named = [
        ("XY", [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]),
        ("XZ", [1.0, 0.0, 0.0], [0.0, 0.0, -1.0]),
        ("YZ", [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]),
    ]
    .into_iter()
    .find(|(_, px, py)| same(x, *px) && same(y, *py))
    .map(|(name, _, _)| name);
    let plane = match named {
        Some(name) if at_origin => json!({"type": "origin", "name": name}),
        _ => json!({
            "type": "custom",
            "plane": {"origin": origin, "x_dir": x, "y_dir": y},
        }),
    };
    Ok((
        plane,
        Frame {
            normal,
            origin,
            x,
            y,
        },
    ))
}

/// What a source sketch entity became.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Shape {
    /// A point, by index.
    Point(usize),
    /// A curve.
    Curve(Curve),
}

/// A SolidCraft curve a source entity became.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Curve {
    /// Exact pieces, empty for a single curve. Aggregate endpoints belong to the whole chain.
    pub(crate) pieces: Vec<Curve>,
    /// Its index among the sketch's curves.
    pub(crate) index: usize,
    /// Its id.
    pub(crate) id: String,
    /// The SolidCraft curve family, including ellipses, splines and conics.
    pub(crate) kind: &'static str,
    /// The point indices of its stored start, end and centre, where it has them.
    pub(crate) start: Option<usize>,
    pub(crate) end: Option<usize>,
    pub(crate) center: Option<usize>,
}

impl Curve {
    /// All emitted ids in traversal order.
    pub(crate) fn ids(&self) -> Vec<String> {
        if self.pieces.is_empty() {
            vec![self.id.clone()]
        } else {
            self.pieces.iter().map(|piece| piece.id.clone()).collect()
        }
    }
}

/// SolidCraft sketch points and curves as they are built.
pub(crate) struct Builder {
    points: Vec<Value>,
    /// Positions of `points`, for sharing coincident ends.
    pub(crate) positions: Vec<P2>,
    /// Source endpoint records already placed, with their point index.
    endpoint_refs: Vec<(String, usize)>,
    curves: Vec<Value>,
    /// Continuity relations introduced between exact spline pieces.
    joints: Vec<Value>,
    /// The last number used per point or curve id prefix (conics use `q` to avoid constraint ids).
    counters: BTreeMap<&'static str, u64>,
    /// What each source entity became, by its id.
    pub(crate) shapes: BTreeMap<String, Shape>,
}

impl Default for Builder {
    fn default() -> Self {
        Builder {
            points: vec![json!({"id": "origin", "pos": [0.0, 0.0], "fixed": true})],
            positions: vec![[0.0, 0.0]],
            endpoint_refs: Vec::new(),
            curves: Vec::new(),
            joints: Vec::new(),
            counters: BTreeMap::new(),
            shapes: BTreeMap::new(),
        }
    }
}

impl Builder {
    /// The curve at `index`, as written.
    pub(crate) fn curve_json(&self, index: usize) -> Option<&Value> {
        self.curves.get(index)
    }

    fn fresh(&mut self, prefix: &'static str) -> String {
        let n = self.counters.entry(prefix).or_insert(0);
        *n = n.saturating_add(1);
        format!("{prefix}{n}")
    }

    /// The point at `pos`: the one already placed for `endpoint_ref` or at that position
    /// (never the fixed sketch origin), else a new one.
    fn point(&mut self, pos: P2, endpoint_ref: Option<&str>) -> usize {
        let by_ref = endpoint_ref.and_then(|r| {
            self.endpoint_refs
                .iter()
                .find(|(known, _)| known == r)
                .map(|(_, index)| *index)
        });
        let index = by_ref.or_else(|| {
            self.positions
                .iter()
                .enumerate()
                .skip(1)
                .find(|(_, at)| distance(**at, pos) <= COINCIDENT)
                .map(|(index, _)| index)
        });
        let index = index.unwrap_or_else(|| {
            let id = self.fresh("p");
            self.points
                .push(json!({"id": id, "pos": pos, "fixed": false}));
            self.positions.push(pos);
            self.positions.len() - 1
        });
        if let Some(r) = endpoint_ref {
            if !self.endpoint_refs.iter().any(|(known, _)| known == r) {
                self.endpoint_refs.push((r.to_owned(), index));
            }
        }
        index
    }

    /// A new point at `pos`, never shared with another curve's.
    pub(crate) fn own_point(&mut self, pos: P2) -> usize {
        let id = self.fresh("p");
        self.points
            .push(json!({"id": id, "pos": pos, "fixed": false}));
        self.positions.push(pos);
        self.positions.len() - 1
    }

    /// Adds a curve, recording it as the one the source `entity` became. Its start, end and
    /// centre are the `a`, `b` and `c` points of `kind`.
    pub(crate) fn curve(
        &mut self,
        entity: Option<&str>,
        prefix: &'static str,
        construction: bool,
        kind: Value,
        extra: &[(&str, Value)],
    ) {
        let id = self.fresh(prefix);
        if let Some(entity) = entity {
            let point = |key: &str| {
                kind.get(key)
                    .and_then(Value::as_u64)
                    .and_then(|i| usize::try_from(i).ok())
            };
            let curve = Curve {
                pieces: Vec::new(),
                index: self.curves.len(),
                id: id.clone(),
                kind: match kind.get("type").and_then(Value::as_str) {
                    Some("line") => "line",
                    Some("circle") => "circle",
                    Some("arc") => "arc",
                    Some("ellipse") => "ellipse",
                    Some("spline") => "spline",
                    Some("conic") => "conic",
                    _ => "",
                },
                start: point("a"),
                end: point("b"),
                center: point("c"),
            };
            self.shapes.insert(entity.to_owned(), Shape::Curve(curve));
        }
        let mut curve = json!({"id": id, "construction": construction});
        if let (Some(fields), Some(kind)) = (curve.as_object_mut(), kind.as_object()) {
            fields.extend(kind.clone());
            for (key, value) in extra {
                fields.insert((*key).to_owned(), value.clone());
            }
        }
        self.curves.push(curve);
    }

    /// Pins a sketch that nothing anchors to the origin its geometry touches. A source sketch
    /// is solved in place, so a profile that meets the sketch origin is tied to it there; the
    /// tie is a projected origin point that is not carried over, and without it a dimension
    /// edit slides the whole profile sideways (across a revolve axis, say). Only a sketch with
    /// no fixed point or curve, no reference line and no constraint on the origin gains the
    /// tie, to the first point lying exactly on it.
    pub(crate) fn anchor_at_origin(&self, constraints: &mut Vec<Value>) {
        let fixed = self
            .points
            .iter()
            .skip(1)
            .any(|p| p.get("fixed").and_then(Value::as_bool) == Some(true))
            || self.curves.iter().any(|c| {
                c.get("fixed").and_then(Value::as_bool) == Some(true)
                    || c.get("infinite").and_then(Value::as_bool) == Some(true)
            });
        let on_origin = constraints.iter().any(|c| {
            ["p", "q"]
                .iter()
                .any(|key| c.get(key).and_then(Value::as_u64) == Some(0))
                || c.get("type").and_then(Value::as_str) == Some("fix")
        });
        if fixed || on_origin {
            return;
        }
        let Some(point) = self
            .positions
            .iter()
            .enumerate()
            .skip(1)
            .find(|(_, at)| distance(**at, [0.0, 0.0]) <= COINCIDENT)
            .map(|(index, _)| index)
        else {
            return;
        };
        constraints.push(json!({
            "id": format!("k{}", constraints.len() + 1),
            "type": "coincident",
            "p": point,
            "q": 0,
        }));
    }

    /// Locks a complete curve, including its independent radius or conic shape.
    pub(crate) fn fix_curve(&mut self, index: usize) -> Option<()> {
        self.curves
            .get_mut(index)?
            .as_object_mut()?
            .insert("fixed".to_owned(), json!(true));
        Some(())
    }

    /// The index a newly appended helper curve will have.
    pub(crate) fn curve_count(&self) -> usize {
        self.curves.len()
    }

    /// Adds one CADIR sketch entity.
    pub(crate) fn entity(&mut self, entity: &Value, report: &mut Report) -> Result<(), String> {
        let construction = entity
            .get("construction")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let refs: Vec<&str> = entity
            .get("endpoint_refs")
            .and_then(Value::as_array)
            .map(|refs| refs.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default();
        let source = entity.get("id").and_then(Value::as_str);
        let geometry = entity.get("geometry").unwrap_or(&Value::Null);
        let kind = geometry
            .get("kind")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        let bad = || format!("its {kind} sketch curve is malformed");
        match kind {
            "point" => {
                let at = p2(geometry.get("position")).ok_or_else(bad)?;
                let index = self.point(at, refs.first().copied());
                if let Some(source) = source {
                    self.shapes.insert(source.to_owned(), Shape::Point(index));
                }
            }
            "line" => {
                let start = p2(geometry.get("start")).ok_or_else(bad)?;
                let end = p2(geometry.get("end")).ok_or_else(bad)?;
                let a = self.point(start, refs.first().copied());
                let b = self.point(end, refs.get(1).copied());
                if a == b {
                    return Err(bad());
                }
                self.curve(
                    source,
                    "l",
                    construction,
                    json!({"type": "line", "a": a, "b": b}),
                    &[],
                );
            }
            "reference_line" => {
                let origin = p2(geometry.get("origin")).ok_or_else(bad)?;
                let direction = p2(geometry.get("direction")).ok_or_else(bad)?;
                let length = direction[0].hypot(direction[1]);
                if length <= PARALLEL {
                    return Err(bad());
                }
                let along = [
                    origin[0] + direction[0] / length,
                    origin[1] + direction[1] / length,
                ];
                let a = self.point(origin, None);
                let b = self.point(along, None);
                self.curve(
                    source,
                    "l",
                    true,
                    json!({"type": "line", "a": a, "b": b}),
                    &[("infinite", json!(true))],
                );
            }
            "circle" => {
                let center = p2(geometry.get("center")).ok_or_else(bad)?;
                let radius = number(geometry.get("radius")).ok_or_else(bad)?;
                let c = self.point(center, None);
                self.curve(
                    source,
                    "c",
                    construction,
                    json!({"type": "circle", "c": c, "r": radius}),
                    &[],
                );
            }
            "arc" => {
                let center = p2(geometry.get("center")).ok_or_else(bad)?;
                let radius = number(geometry.get("radius")).ok_or_else(bad)?;
                let start = number(geometry.get("start_angle")).ok_or_else(bad)?;
                let end = number(geometry.get("end_angle")).ok_or_else(bad)?;
                let at = |angle: f64| {
                    [
                        center[0] + radius * angle.cos(),
                        center[1] + radius * angle.sin(),
                    ]
                };
                let c = self.point(center, None);
                let a = self.point(at(start), refs.first().copied());
                let b = self.point(at(end), refs.get(1).copied());
                self.curve(
                    source,
                    "a",
                    construction,
                    json!({"type": "arc", "c": c, "a": a, "b": b}),
                    &[],
                );
            }
            "nurbs" if rational_conic(geometry.get("curve")).is_some() => {
                let (start, apex, end, rho) = rational_conic(geometry.get("curve"))
                    .ok_or("its rational quadratic cannot be expressed by a SolidCraft conic")?;
                let a = self.point(start, refs.first().copied());
                let b = self.point(end, refs.get(1).copied());
                let apex = self.own_point(apex);
                self.curve(
                    source,
                    "q",
                    construction,
                    json!({"type": "conic", "a": a, "b": b, "apex": apex, "rho": rho}),
                    &[],
                );
            }
            "nurbs" => match spline(geometry.get("curve")) {
                Ok(Spline {
                    degree,
                    pieces,
                    smooth,
                }) => {
                    let mut chain = Vec::new();
                    let mut previous_end = None;
                    let count = pieces.len();
                    for (piece_index, poles) in pieces.iter().enumerate() {
                        let last = poles.len().saturating_sub(1);
                        let points: Vec<usize> = poles
                            .iter()
                            .enumerate()
                            .map(|(i, pole)| {
                                if i == 0 {
                                    previous_end
                                        .unwrap_or_else(|| self.point(*pole, refs.first().copied()))
                                } else if i == last {
                                    if piece_index + 1 == count {
                                        self.point(*pole, refs.get(1).copied())
                                    } else {
                                        self.own_point(*pole)
                                    }
                                } else {
                                    self.own_point(*pole)
                                }
                            })
                            .collect();
                        self.curve(source, "s", construction,
                            json!({"type": "spline", "pts": points, "control": true, "degree": degree}), &[]);
                        previous_end = points.last().copied();
                        if let Some(Shape::Curve(curve)) =
                            source.and_then(|id| self.shapes.get_mut(id))
                        {
                            curve.start = points.first().copied();
                            curve.end = previous_end;
                            chain.push(curve.clone());
                        }
                        if piece_index > 0 {
                            let before = pieces
                                .get(piece_index - 1)
                                .ok_or("its spline piece is missing")?;
                            let tangent = before.iter().rev().take(2).copied().collect::<Vec<_>>();
                            if let (Some(&end), Some(&a), Some(&start), Some(&b)) =
                                (tangent.first(), tangent.get(1), poles.first(), poles.get(1))
                            {
                                let u = [end[0] - a[0], end[1] - a[1]];
                                let v = [b[0] - start[0], b[1] - start[1]];
                                let scale = u[0].hypot(u[1]) * v[0].hypot(v[1]);
                                if smooth.get(piece_index - 1).copied().unwrap_or(false)
                                    || scale > 0.0
                                        && (u[0] * v[1] - u[1] * v[0]).abs() <= 1e-12 * scale
                                        && u[0] * v[0] + u[1] * v[1] > 0.0
                                {
                                    let index = self.curves.len() - 1;
                                    self.joints.push(
                                        json!({"type": "tangent", "a": index - 1, "b": index}),
                                    );
                                }
                            }
                        }
                    }
                    if chain.len() > 1 {
                        let mut whole = chain.first().cloned().ok_or("its spline has no pieces")?;
                        whole.kind = "compound_spline";
                        whole.end = chain.last().and_then(|piece| piece.end);
                        whole.pieces = chain;
                        if let Some(source) = source {
                            self.shapes.insert(source.to_owned(), Shape::Curve(whole));
                        }
                    }
                }
                Err(why) if construction => report.push(
                    Severity::Degraded,
                    "sketch",
                    format!("a construction spline is not carried: {why}"),
                ),
                Err(why) => return Err(why),
            },
            "ellipse" if geometry.get("bounds").is_none_or(Value::is_null) => {
                let center = p2(geometry.get("center")).ok_or_else(bad)?;
                let angle = number(geometry.get("major_angle")).ok_or_else(bad)?;
                let major = number(geometry.get("major_radius")).ok_or_else(bad)?;
                let minor = number(geometry.get("minor_radius")).ok_or_else(bad)?;
                if !(major > 0.0 && minor > 0.0) {
                    return Err(bad());
                }
                let end = [
                    center[0] + major * angle.cos(),
                    center[1] + major * angle.sin(),
                ];
                let c = self.point(center, None);
                let m = self.point(end, None);
                self.curve(
                    source,
                    "e",
                    construction,
                    json!({"type": "ellipse", "c": c, "m": m, "r": minor}),
                    &[],
                );
            }
            "ellipse"
                if geometry
                    .get("bounds")
                    .and_then(Value::as_array)
                    .is_some_and(|bounds| {
                        number(bounds.first())
                            .zip(number(bounds.get(1)))
                            .is_some_and(|(a, b)| (b - a).abs() >= std::f64::consts::PI - PARALLEL)
                    }) =>
            {
                let bounds = geometry
                    .get("bounds")
                    .and_then(Value::as_array)
                    .ok_or_else(bad)?;
                let first = number(bounds.first()).ok_or_else(bad)?;
                let last = number(bounds.get(1)).ok_or_else(bad)?;
                let span = last - first;
                if span.abs() > std::f64::consts::TAU + PARALLEL {
                    let why = "an elliptical arc spanning more than one turn is not supported yet";
                    if construction {
                        report.push(Severity::Degraded, "sketch", why);
                        return Ok(());
                    }
                    return Err(why.to_owned());
                }
                // One source curve can become several exact conics in the whole sketch.
                // Do not expose just the first piece as a selectable whole source curve.
                let count = 4usize;
                let index = self.curves.len();
                let mut ends = None;
                let mut id = String::new();
                for piece in 0..count {
                    let start = first + span * piece as f64 / count as f64;
                    let end = first + span * (piece + 1) as f64 / count as f64;
                    let mut part = geometry.clone();
                    part.as_object_mut()
                        .ok_or_else(bad)?
                        .insert("bounds".to_owned(), json!([start, end]));
                    let (start, apex, end, rho) = conic(&part)?;
                    let a = self.point(
                        start,
                        if piece == 0 {
                            refs.first().copied()
                        } else {
                            None
                        },
                    );
                    let b = self.point(
                        end,
                        if piece + 1 == count {
                            refs.get(1).copied()
                        } else {
                            None
                        },
                    );
                    let apex = self.own_point(apex);
                    self.curve(
                        None,
                        "q",
                        construction,
                        json!({"type": "conic", "a": a, "b": b, "apex": apex, "rho": rho}),
                        &[],
                    );
                    if piece == 0 {
                        id = self
                            .curves
                            .get(index)
                            .and_then(|curve| curve.get("id"))
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_owned();
                        ends = Some((a, b));
                    } else if let Some((_, end)) = &mut ends {
                        *end = b;
                    }
                }
                let center = self.point(p2(geometry.get("center")).ok_or_else(bad)?, None);
                if let (Some(source), Some((start, end))) = (source, ends) {
                    self.shapes.insert(
                        source.to_owned(),
                        Shape::Curve(Curve {
                            pieces: Vec::new(),
                            index,
                            id,
                            kind: "compound_conic",
                            start: Some(start),
                            end: Some(end),
                            center: Some(center),
                        }),
                    );
                }
            }
            "ellipse" | "hyperbola" | "parabola" => match conic(geometry) {
                Ok((start, apex, end, rho)) => {
                    let a = self.point(start, refs.first().copied());
                    let b = self.point(end, refs.get(1).copied());
                    let apex = self.own_point(apex);
                    self.curve(
                        source,
                        "q",
                        construction,
                        json!({"type": "conic", "a": a, "b": b, "apex": apex, "rho": rho}),
                        &[],
                    );
                }
                Err(why) if construction => report.push(
                    Severity::Degraded,
                    "sketch",
                    format!("a construction {kind} is not carried: {why}"),
                ),
                Err(why) => return Err(why),
            },
            "text" | "external_reference" | "native" => {
                let why = match kind {
                    "text" => "sketch text is not supported yet: source font metrics, width scaling and vertical anchoring cannot be preserved as solved outlines",
                    "external_reference" => "an external sketch reference has no solved sketch-space geometry",
                    _ => "source-native sketch geometry has no resolved neutral curve",
                };
                if construction {
                    report.push(
                        Severity::Degraded,
                        "sketch",
                        format!("a construction entity is not carried: {why}"),
                    );
                } else {
                    return Err(why.to_owned());
                }
            }
            other if construction => report.push(
                Severity::Degraded,
                "sketch",
                format!("a construction {other} curve is not carried"),
            ),
            other => return Err(format!("{other} sketch curves are not mapped yet")),
        }
        Ok(())
    }
}

/// A single rational quadratic Bézier has the same locus after normalizing its
/// endpoint weights to one. Other weighted splines need knots or weights that
/// SolidCraft's control spline does not store.
fn rational_conic(curve: Option<&Value>) -> Option<(P2, P2, P2, f64)> {
    let curve = curve?;
    if curve.get("degree")?.as_u64()? != 2 || curve.get("periodic")?.as_bool()? {
        return None;
    }
    let knots = curve.get("knots")?.as_array()?;
    if knots.len() != 6 {
        return None;
    }
    let first = number(knots.first())?;
    let last = number(knots.last())?;
    if last <= first
        || !knots.iter().take(3).all(|k| number(Some(k)) == Some(first))
        || !knots.iter().skip(3).all(|k| number(Some(k)) == Some(last))
    {
        return None;
    }
    let poles = curve.get("poles")?;
    if poles.get("form")?.as_str()? != "rational" {
        return None;
    }
    let points = poles.get("points")?.as_array()?;
    if points.len() != 3 {
        return None;
    }
    let [a, apex, b] = [points.first()?, points.get(1)?, points.get(2)?];
    let [w0, w1, w2] = [
        number(a.get("weight"))?,
        number(apex.get("weight"))?,
        number(b.get("weight"))?,
    ];
    if w0.signum() != w1.signum() || w0.signum() != w2.signum() {
        return None;
    }
    let weight = (w1.abs() / w0.abs().sqrt()) / w2.abs().sqrt();
    let rho = weight / (1.0 + weight);
    if !(1e-6..1.0 - 1e-6).contains(&rho) || rho == 1e-6 {
        return None;
    }
    Some((
        p2(a.get("point"))?,
        p2(apex.get("point"))?,
        p2(b.get("point"))?,
        rho,
    ))
}

/// A bounded analytic conic as a rational quadratic Bézier. The middle pole is
/// the intersection of the endpoint tangents; its weight determines SolidCraft's rho.
fn conic(geometry: &Value) -> Result<(P2, P2, P2, f64), String> {
    let kind = geometry.get("kind").and_then(Value::as_str).unwrap_or("");
    let bounds = geometry
        .get("bounds")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            format!("an unbounded {kind} cannot be expressed by SolidCraft's bounded conic")
        })?;
    let first = number(bounds.first()).ok_or("its conic has no start parameter")?;
    let last = number(bounds.get(1)).ok_or("its conic has no end parameter")?;
    let middle = first * 0.5 + last * 0.5;
    let half = (last - first) * 0.5;
    if !half.is_finite() || half.abs() <= PARALLEL {
        return Err("its conic parameter span is zero or too large".to_owned());
    }
    let (origin, angle, start, apex, end, weight) = if kind == "parabola" {
        let origin = p2(geometry.get("vertex")).ok_or("its parabola has no vertex")?;
        let angle = number(geometry.get("axis_angle")).ok_or("its parabola has no axis angle")?;
        let focal = number(geometry.get("focal_length"))
            .filter(|f| *f > 0.0)
            .ok_or("its parabola has no positive focal length")?;
        let at = |y: f64| [y * y / (4.0 * focal), y];
        (
            origin,
            angle,
            at(first),
            [first * last / (4.0 * focal), middle],
            at(last),
            1.0,
        )
    } else {
        let origin = p2(geometry.get("center")).ok_or("its conic has no center")?;
        let angle = number(geometry.get("major_angle")).ok_or("its conic has no major angle")?;
        let major = number(geometry.get("major_radius")).ok_or("its conic has no major radius")?;
        let minor = number(geometry.get("minor_radius")).ok_or("its conic has no minor radius")?;
        if kind == "ellipse" {
            if half.abs() >= std::f64::consts::FRAC_PI_2 - PARALLEL {
                return Err("an elliptical arc spanning 180 degrees or more is not supported yet: it needs multiple SolidCraft conics".to_owned());
            }
            let weight = half.cos();
            let at = |t: f64| [major * t.cos(), minor * t.sin()];
            (
                origin,
                angle,
                at(first),
                [major * middle.cos() / weight, minor * middle.sin() / weight],
                at(last),
                weight,
            )
        } else {
            let weight = half.cosh();
            let at = |t: f64| [major * t.cosh(), minor * t.sinh()];
            (
                origin,
                angle,
                at(first),
                [
                    major * middle.cosh() / weight,
                    minor * middle.sinh() / weight,
                ],
                at(last),
                weight,
            )
        }
    };
    let rho = weight / (1.0 + weight);
    let transform = |[x, y]: P2| {
        [
            origin[0] + x * angle.cos() - y * angle.sin(),
            origin[1] + x * angle.sin() + y * angle.cos(),
        ]
    };
    let points = [transform(start), transform(apex), transform(end)];
    if !points
        .iter()
        .flatten()
        .all(|v| v.is_finite() && v.abs() < 1e9)
        || !(1e-6..1.0 - 1e-6).contains(&rho)
        || rho == 1e-6
    {
        return Err("its conic exceeds SolidCraft's finite point or rho range".to_owned());
    }
    let [start, apex, end] = points;
    Ok((start, apex, end, rho))
}

/// SolidCraft's highest spline degree.
const MAX_DEGREE: u64 = 9;

/// SolidCraft's most spline points.
const MAX_POLES: usize = 500;

/// How far apart (relative to the knot span) knots may be and still be evenly spaced.
const EVEN: f64 = 1e-9;

/// Exact target geometry and the source's continuity at the inserted joints.
#[derive(Debug)]
struct Spline {
    degree: u64,
    pieces: Vec<Vec<P2>>,
    smooth: Vec<bool>,
}

/// A CADIR spline as one uniform control spline or exact Bézier pieces. Periodic
/// curves are clamped first; nonuniform knots are inserted to full degree multiplicity.
fn spline(curve: Option<&Value>) -> Result<Spline, String> {
    let curve = curve.ok_or("its spline is malformed")?;
    let periodic = curve.get("periodic").and_then(Value::as_bool) == Some(true);
    let poles = curve.get("poles").ok_or("its spline is malformed")?;
    let rows = poles
        .get("points")
        .and_then(Value::as_array)
        .ok_or("its spline is malformed")?;
    if rows.len() > MAX_POLES {
        return Err("its spline has more than SolidCraft's 500 control points".to_owned());
    }
    let rational = poles.get("form").and_then(Value::as_str) == Some("rational");
    if rational {
        let first = rows
            .first()
            .and_then(|p| number(p.get("weight")))
            .filter(|w| *w != 0.0)
            .ok_or("its spline has no nonzero weight")?;
        if !rows.iter().all(|p| number(p.get("weight")) == Some(first)) {
            if periodic {
                return Err("weighted periodic splines are not supported yet: SolidCraft control splines have no weights".to_owned());
            }
            return Err("weighted splines other than a positive-weight quadratic Bézier are not supported yet: SolidCraft control splines have no weights".to_owned());
        }
    }
    let mut points: Vec<P2> = rows
        .iter()
        .map(|p| p2(if rational { p.get("point") } else { Some(p) }))
        .collect::<Option<_>>()
        .ok_or("its spline is malformed")?;
    let degree = curve
        .get("degree")
        .and_then(Value::as_u64)
        .filter(|d| (1..=MAX_DEGREE).contains(d))
        .ok_or("its spline degree is out of SolidCraft's range")?;
    let order = usize::try_from(degree).map_err(|_| "its spline degree is malformed")? + 1;
    if points.len() < order || points.len() > MAX_POLES {
        return Err("its spline has too few or too many control points".to_owned());
    }
    let knot_values = curve
        .get("knots")
        .and_then(Value::as_array)
        .ok_or("its spline is malformed")?;
    if knot_values.len() != points.len() + order {
        return Err("its spline's knots do not fit its control points".to_owned());
    }
    let mut knots: Vec<f64> = knot_values
        .iter()
        .map(Value::as_f64)
        .collect::<Option<_>>()
        .ok_or("its spline is malformed")?;
    if periodic {
        (knots, points) = clamp_periodic(order - 1, knots, points)
            .ok_or("its periodic spline cannot be clamped on its active knot domain")?;
        if points.len() > MAX_POLES {
            return Err(
                "clamping its periodic spline needs more than SolidCraft's 500 control points"
                    .to_owned(),
            );
        }
    }
    let (Some(&first), Some(&last)) = (knots.first(), knots.last()) else {
        return Err("its spline is malformed".to_owned());
    };
    let span = last - first;
    if !span.is_finite() {
        return Err("its spline knot span is not finite".to_owned());
    }
    let clamped = span > 0.0
        && knots
            .iter()
            .take(order)
            .all(|k| (k - first).abs() <= EVEN * span)
        && knots
            .iter()
            .rev()
            .take(order)
            .all(|k| (k - last).abs() <= EVEN * span);
    let inner: Vec<f64> = knots
        .get(order..knots.len() - order)
        .unwrap_or_default()
        .to_vec();
    let pieces = inner.len() + 1;
    // Exact periodic conversion must not round nearly uniform source knots into shape.
    let tolerance = if periodic { 8.0 * f64::EPSILON } else { EVEN };
    let even = inner.iter().enumerate().all(|(i, k)| {
        let want = first + span * (i + 1) as f64 / pieces as f64;
        (k - want).abs() <= tolerance * span
    });
    if knots.iter().any(|k| !k.is_finite()) || knots.windows(2).any(|w| w.first() > w.get(1)) {
        return Err("its spline knots are not finite and nondecreasing".to_owned());
    }
    if clamped && even {
        return Ok(Spline {
            degree,
            pieces: vec![points],
            smooth: Vec::new(),
        });
    }
    // Decomposition requires actual clamping, without rounding endpoint knots.
    if !span.is_finite()
        || span <= 0.0
        || !knots.iter().take(order).all(|k| *k == first)
        || !knots.iter().rev().take(order).all(|k| *k == last)
    {
        return Err("its spline is not clamped; this knot layout is not supported yet".to_owned());
    }
    let mut interior = Vec::new();
    for knot in inner {
        if knot <= first || knot >= last {
            return Err("its spline interior knots are outside its active domain".to_owned());
        }
        if interior.last().is_none_or(|(value, _)| *value != knot) {
            interior.push((knot, 1usize));
        } else if let Some((_, count)) = interior.last_mut() {
            *count += 1;
        }
    }
    if interior.len() + 1 > 200 {
        return Err(
            "its spline needs more than 200 exact pieces; this is not supported yet".to_owned(),
        );
    }
    let degree_usize = order - 1;
    let smooth = interior
        .iter()
        .map(|(_, multiplicity)| *multiplicity < degree_usize)
        .collect();
    for (knot, multiplicity) in interior {
        if multiplicity > degree_usize {
            return Err("disconnected spline pieces are not supported yet".to_owned());
        }
        for _ in multiplicity..degree_usize {
            insert_spline_knot(degree_usize, &mut knots, &mut points, knot)
                .ok_or("its spline cannot be split by exact knot insertion")?;
        }
    }
    let pieces = points
        .windows(order)
        .step_by(degree_usize)
        .map(<[P2]>::to_vec)
        .collect();
    Ok(Spline {
        degree,
        pieces,
        smooth,
    })
}

/// Restrict the full CADIR knot vector to [U[p], U[pole_count]]. Repeating each
/// boundary knot p+1 times makes the first and last poles points on the curve;
/// discarding the outside knots and poles then leaves the same polynomial curve.
fn clamp_periodic(
    degree: usize,
    mut knots: Vec<f64>,
    mut points: Vec<P2>,
) -> Option<(Vec<f64>, Vec<P2>)> {
    if degree == 0
        || degree > MAX_DEGREE as usize
        || points.len() > MAX_POLES
        || points.len() <= degree
        || knots.len() != points.len() + degree + 1
        || knots.iter().any(|k| !k.is_finite())
        || knots.windows(2).any(|pair| pair.first() > pair.get(1))
    {
        return None;
    }
    let start = *knots.get(degree)?;
    let end = *knots.get(points.len())?;
    if start >= end || !(end - start).is_finite() {
        return None;
    }
    for boundary in [start, end] {
        let multiplicity = knots.iter().filter(|k| **k == boundary).count();
        if multiplicity > degree + 1 {
            return None;
        }
        for _ in multiplicity..=degree {
            insert_spline_knot(degree, &mut knots, &mut points, boundary)?;
        }
    }
    let first = knots.iter().position(|k| *k == start)?;
    let last = knots.iter().rposition(|k| *k == end)?;
    let count = (last + 1).checked_sub(first + degree + 1)?;
    Some((
        knots.get(first..=last)?.to_vec(),
        points.get(first..first + count)?.to_vec(),
    ))
}

/// One Boehm insertion: untouched poles on either side, affine blends between.
fn insert_spline_knot(
    degree: usize,
    knots: &mut Vec<f64>,
    points: &mut Vec<P2>,
    value: f64,
) -> Option<()> {
    let span = knots.iter().rposition(|k| *k <= value)?;
    let multiplicity = knots.iter().filter(|k| **k == value).count();
    let left = span.checked_sub(degree)?;
    let right = span.checked_sub(multiplicity)?;
    if left > right || right >= points.len() {
        return None;
    }
    let mut inserted = Vec::with_capacity(points.len() + 1);
    for i in 0..=points.len() {
        let point = if i <= left {
            *points.get(i)?
        } else if i > right {
            *points.get(i - 1)?
        } else {
            let low = *knots.get(i)?;
            let high = *knots.get(i + degree)?;
            let width = high - low;
            if !width.is_finite() || width <= 0.0 {
                return None;
            }
            let alpha = (value - low) / width;
            if !alpha.is_finite() || !(0.0..=1.0).contains(&alpha) {
                return None;
            }
            let [ax, ay] = *points.get(i - 1)?;
            let [bx, by] = *points.get(i)?;
            [
                (1.0 - alpha) * ax + alpha * bx,
                (1.0 - alpha) * ay + alpha * by,
            ]
        };
        if point.iter().any(|v| !v.is_finite()) {
            return None;
        }
        inserted.push(point);
    }
    knots.insert(span + 1, value);
    *points = inserted;
    Some(())
}

pub(crate) fn number(value: Option<&Value>) -> Option<f64> {
    let value = value?;
    value
        .as_f64()
        .or_else(|| value.get("value").and_then(Value::as_f64))
        .filter(|n| n.is_finite())
}

pub(crate) fn p2(value: Option<&Value>) -> Option<P2> {
    let value = value?;
    Some([number(value.get("u"))?, number(value.get("v"))?])
}

pub(crate) fn v3(value: Option<&Value>) -> Option<V3> {
    let value = value?;
    Some([
        number(value.get("x"))?,
        number(value.get("y"))?,
        number(value.get("z"))?,
    ])
}

pub(crate) fn distance(a: P2, b: P2) -> f64 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}

pub(crate) fn dot(a: V3, b: V3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

pub(crate) fn cross(a: V3, b: V3) -> V3 {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

pub(crate) fn unit(v: V3) -> Option<V3> {
    let length = dot(v, v).sqrt();
    (length > PARALLEL && length.is_finite()).then(|| [v[0] / length, v[1] / length, v[2] / length])
}

fn same(a: V3, b: V3) -> bool {
    a.iter()
        .zip(b)
        .all(|(x, y)| (x - y).abs() <= PARALLEL.sqrt())
}

// Test helpers outside `#[test]` functions are not covered by clippy.toml's test allowances.
#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use std::path::Path;

    use super::*;

    fn frame(origin: V3, normal: V3, u: V3) -> Value {
        let v = |c: V3| json!({"x": c[0], "y": c[1], "z": c[2]});
        json!({"placement": {"kind": "resolved", "frame": {
            "origin": v(origin), "normal": v(normal), "u_axis": v(u)
        }}})
    }

    #[test]
    fn principal_frames_become_origin_planes() {
        let o = [0.0; 3];
        let cases = [
            ([0.0, 0.0, 1.0], [1.0, 0.0, 0.0], "XY"),
            ([0.0, 1.0, 0.0], [1.0, 0.0, 0.0], "XZ"),
            ([1.0, 0.0, 0.0], [0.0, 1.0, 0.0], "YZ"),
        ];
        for (normal, u, name) in cases {
            let (plane, frame) = plane(&frame(o, normal, u)).unwrap();
            assert_eq!(plane, json!({"type": "origin", "name": name}));
            assert_eq!(frame.normal, normal);
        }
    }

    #[test]
    fn other_frames_become_explicit_planes() {
        let (offset, _) = plane(&frame([0.0, 0.0, 5.0], [0.0, 0.0, 1.0], [1.0, 0.0, 0.0])).unwrap();
        assert_eq!(
            offset,
            json!({"type": "custom", "plane": {
                "origin": [0.0, 0.0, 5.0], "x_dir": [1.0, 0.0, 0.0], "y_dir": [0.0, 1.0, 0.0]
            }})
        );
        // XY seen from below: the normal is reversed, so it is not the XY origin plane.
        let (below, _) = plane(&frame([0.0; 3], [0.0, 0.0, -1.0], [1.0, 0.0, 0.0])).unwrap();
        assert_eq!(below["type"], "custom");
        assert_eq!(below["plane"]["y_dir"], json!([0.0, -1.0, 0.0]));
    }

    #[test]
    fn unresolved_or_degenerate_frames_are_refused() {
        assert!(plane(&json!({"placement": {"kind": "unresolved"}})).is_err());
        assert!(plane(&frame([0.0; 3], [0.0, 0.0, 1.0], [0.0, 0.0, 1.0])).is_err());
    }

    fn entity(geometry: Value) -> Value {
        json!({"construction": false, "geometry": geometry})
    }

    fn line(a: P2, b: P2) -> Value {
        entity(json!({"kind": "line",
            "start": {"u": a[0], "v": a[1]}, "end": {"u": b[0], "v": b[1]}}))
    }

    fn point(key: &str, at: P2) -> Value {
        json!({"id": key, "construction": false,
            "geometry": {"kind": "point", "position": {"u": at[0], "v": at[1]}}})
    }

    #[test]
    fn a_profile_tied_to_the_projected_origin_is_pinned_there() {
        let mut builder = Builder::default();
        let mut report = Report::default();
        // A revolve profile touching the origin: its two line ends plus the projected origin.
        let entities = [
            line([0.0, 0.0], [10.0, 0.0]),
            line([10.0, 0.0], [10.0, -30.0]),
            line([10.0, -30.0], [0.0, -30.0]),
            line([0.0, -30.0], [0.0, 0.0]),
        ];
        for e in &entities {
            builder.entity(e, &mut report).unwrap();
        }
        // The source also lists a point per curve end, then the projected origin itself.
        let mut with_origin: Vec<Value> = entities.to_vec();
        with_origin.push(point("end1", [0.0, 0.0]));
        with_origin.push(point("end2", [0.0, 0.0]));
        let refs: Vec<&Value> = with_origin.iter().collect();
        assert!(!projected_origin(&refs));
        with_origin.push(point("origin", [0.0, 0.0]));
        let refs: Vec<&Value> = with_origin.iter().collect();
        assert!(projected_origin(&refs));
        // A lone point at the origin with no curve there is not a tie.
        assert!(!projected_origin(&[&point("origin", [0.0, 0.0])]));

        let mut constraints = vec![json!({"id": "k1", "type": "vertical", "l": 3})];
        builder.anchor_at_origin(&mut constraints);
        assert_eq!(
            constraints[1],
            json!({"id": "k2", "type": "coincident", "p": 1, "q": 0})
        );
        // Anchored already, by a dimension from the origin: nothing is added.
        let mut anchored =
            vec![json!({"id": "k1", "type": "distance", "p": 0, "q": 2, "value": 5.0})];
        builder.anchor_at_origin(&mut anchored);
        assert_eq!(anchored.len(), 1);
        // A sketch that does not touch the origin gains nothing.
        let mut away = Builder::default();
        away.entity(&line([1.0, 1.0], [5.0, 1.0]), &mut report)
            .unwrap();
        let mut none = Vec::new();
        away.anchor_at_origin(&mut none);
        assert!(none.is_empty());
    }

    #[test]
    fn a_rectangle_shares_its_corners() {
        let mut builder = Builder::default();
        let mut report = Report::default();
        for (a, b) in [
            ([0.0, 0.0], [40.0, 0.0]),
            ([40.0, 0.0], [40.0, 30.0]),
            ([40.0, 30.0], [0.0, 30.0]),
            ([0.0, 30.0], [0.0, 0.0]),
        ] {
            builder.entity(&line(a, b), &mut report).unwrap();
        }
        // The origin, then four corners: the corner at (0, 0) is not the fixed origin.
        assert_eq!(builder.points.len(), 5);
        assert_eq!(
            builder.curves,
            [
                json!({"id": "l1", "type": "line", "a": 1, "b": 2, "construction": false}),
                json!({"id": "l2", "type": "line", "a": 2, "b": 3, "construction": false}),
                json!({"id": "l3", "type": "line", "a": 3, "b": 4, "construction": false}),
                json!({"id": "l4", "type": "line", "a": 4, "b": 1, "construction": false}),
            ]
        );
    }

    #[test]
    fn arcs_run_counter_clockwise_between_their_angles() {
        let mut builder = Builder::default();
        let arc = entity(json!({"kind": "arc", "center": {"u": 10.0, "v": 20.0},
            "radius": 30.0, "start_angle": 0.0, "end_angle": std::f64::consts::PI}));
        builder.entity(&arc, &mut Report::default()).unwrap();
        let curve = builder.curves.first().unwrap();
        assert_eq!(curve["type"], "arc");
        let pos = |i: &Value| builder.positions[usize::try_from(i.as_u64().unwrap()).unwrap()];
        assert_eq!(pos(&curve["c"]), [10.0, 20.0]);
        assert_eq!(pos(&curve["a"]), [40.0, 20.0]);
        let end = pos(&curve["b"]);
        assert!(distance(end, [-20.0, 20.0]) < 1e-9, "{end:?}");
    }

    #[test]
    fn endpoint_records_join_curves_whatever_their_positions() {
        let mut builder = Builder::default();
        let mut first = line([0.0, 0.0], [10.0, 0.0]);
        first["endpoint_refs"] = json!(["s", "m"]);
        let mut second = line([10.0, 0.001], [10.0, 10.0]);
        second["endpoint_refs"] = json!(["m", "e"]);
        let mut report = Report::default();
        builder.entity(&first, &mut report).unwrap();
        builder.entity(&second, &mut report).unwrap();
        assert_eq!(builder.curves[0]["b"], builder.curves[1]["a"]);
    }

    fn nurbs(degree: u64, knots: &[f64], points: &[P2]) -> Value {
        json!({"degree": degree, "knots": knots, "periodic": false,
               "poles": {"form": "polynomial",
                         "points": points.iter().map(|p| json!({"u": p[0], "v": p[1]})).collect::<Vec<_>>()}})
    }

    #[test]
    fn clamped_splines_map_as_one_uniform_curve_or_exact_pieces() {
        let five = [[0.0, 0.0], [1.0, 1.0], [2.0, 0.0], [3.0, 1.0], [4.0, 0.0]];
        let uniform = nurbs(3, &[0.0, 0.0, 0.0, 0.0, 0.5, 1.0, 1.0, 1.0, 1.0], &five);
        assert_eq!(spline(Some(&uniform)).unwrap().degree, 3);
        assert_eq!(spline(Some(&uniform)).unwrap().pieces, vec![five.to_vec()]);
        let bezier = nurbs(2, &[0.0, 0.0, 0.0, 2.0, 2.0, 2.0], &five[..3]);
        assert!(spline(Some(&bezier)).is_ok());
        let uneven = nurbs(3, &[0.0, 0.0, 0.0, 0.0, 0.3, 1.0, 1.0, 1.0, 1.0], &five);
        let Spline { pieces, smooth, .. } = spline(Some(&uneven)).unwrap();
        assert_eq!(smooth, vec![true]);
        assert_eq!(pieces.len(), 2);
        assert_eq!(pieces[0].last(), pieces[1].first());
        for (i, piece) in pieces.iter().enumerate() {
            let (low, high) = if i == 0 { (0.0, 0.3) } else { (0.3, 1.0) };
            for j in 0..=20 {
                let t = j as f64 / 20.0;
                let actual = evaluate(3, &[0., 0., 0., 0., 1., 1., 1., 1.], piece, t);
                let expected = evaluate(
                    3,
                    &[0., 0., 0., 0., 0.3, 1., 1., 1., 1.],
                    &five,
                    low + (high - low) * t,
                );
                assert!(
                    distance(actual, expected) < 1e-12,
                    "{actual:?} != {expected:?}"
                );
            }
        }
        let mut rational = uniform.clone();
        rational["poles"]["form"] = json!("rational");
        assert!(spline(Some(&rational)).is_err());
        let wrong_count = nurbs(3, &[0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0], &five);
        assert!(spline(Some(&wrong_count)).is_err());
    }

    #[test]
    fn full_multiplicity_spline_pieces_keep_the_source_poles() {
        let poles: Vec<P2> = (0..10).map(|i| [i as f64, (i % 3) as f64]).collect();
        let curve = nurbs(
            3,
            &[0., 0., 0., 0., 0.2, 0.2, 0.2, 0.7, 0.7, 0.7, 1., 1., 1., 1.],
            &poles,
        );
        let Spline { pieces, smooth, .. } = spline(Some(&curve)).unwrap();
        assert_eq!(smooth, vec![false, false]);
        assert_eq!(
            pieces,
            vec![
                poles[0..4].to_vec(),
                poles[3..7].to_vec(),
                poles[6..10].to_vec()
            ]
        );
        let mut builder = Builder::default();
        let source = json!({"id": "chain", "construction": true, "geometry": {"kind": "nurbs", "curve": curve}});
        builder.entity(&source, &mut Report::default()).unwrap();
        assert_eq!(builder.curves.len(), 3);
        assert!(builder.curves.iter().all(|c| c["construction"] == true));
        for pair in builder.curves.windows(2) {
            assert_eq!(
                pair[0]["pts"].as_array().unwrap().last(),
                pair[1]["pts"].as_array().unwrap().first()
            );
        }
        let Shape::Curve(mapped) = &builder.shapes["chain"] else {
            panic!("missing chain");
        };
        assert_eq!(mapped.ids(), vec!["s1", "s2", "s3"]);
    }

    #[test]
    fn spline_decomposition_bounds_work_and_rejects_unequal_weights() {
        let poles: Vec<P2> = (0..204).map(|i| [i as f64, 0.0]).collect();
        let mut knots = vec![0.; 4];
        knots.extend((1..=200).map(|i| (i as f64 / 201.).powi(2)));
        knots.extend([1.; 4]);
        let curve = nurbs(3, &knots, &poles);
        assert!(spline(Some(&curve))
            .unwrap_err()
            .contains("200 exact pieces"));
        let mut curve = nurbs(3, &[0., 0., 0., 0., 0.3, 1., 1., 1., 1.], &poles[..5]);
        curve["poles"] = json!({"form": "rational", "points": poles[..5].iter().enumerate().map(|(i,p)|
            json!({"point": {"u": p[0], "v": p[1]}, "weight": i+1})).collect::<Vec<_>>()});
        assert!(spline(Some(&curve))
            .unwrap_err()
            .contains("weighted splines"));
        for p in curve["poles"]["points"].as_array_mut().unwrap() {
            p["weight"] = json!(2.0);
        }
        assert_eq!(spline(Some(&curve)).unwrap().pieces.len(), 2);
    }

    // Cox–de Boor basis evaluation, independent of the insertion implementation.
    #[allow(clippy::indexing_slicing)]
    fn evaluate(degree: usize, knots: &[f64], points: &[P2], t: f64) -> P2 {
        let mut basis: Vec<f64> = knots
            .windows(2)
            .map(|pair| {
                if (pair[0] <= t && t < pair[1] && t != knots[points.len()])
                    || (t == knots[points.len()] && pair[0] < t && t <= pair[1])
                {
                    1.0
                } else {
                    0.0
                }
            })
            .collect();
        for p in 1..=degree {
            basis = (0..basis.len() - 1)
                .map(|i| {
                    let a = knots[i + p] - knots[i];
                    let b = knots[i + p + 1] - knots[i + 1];
                    (if a > 0.0 {
                        (t - knots[i]) / a * basis[i]
                    } else {
                        0.0
                    }) + (if b > 0.0 {
                        (knots[i + p + 1] - t) / b * basis[i + 1]
                    } else {
                        0.0
                    })
                })
                .collect();
        }
        points
            .iter()
            .zip(basis)
            .fold([0.0, 0.0], |[x, y], (p, b)| [x + p[0] * b, y + p[1] * b])
    }

    fn periodic_curve(degree: usize) -> (Vec<f64>, Vec<P2>) {
        let count = degree + 3;
        let mut points: Vec<P2> = (0..count)
            .map(|i| {
                let angle = std::f64::consts::TAU * i as f64 / count as f64;
                [10.0 * angle.cos(), 10.0 * angle.sin()]
            })
            .collect();
        points.extend_from_within(..degree);
        let knots = (0..points.len() + degree + 1)
            .map(|i| i as f64 - degree as f64)
            .collect();
        (knots, points)
    }

    #[test]
    fn periodic_clamping_preserves_sampled_positions() {
        for degree in 1..=9 {
            let (knots, points) = periodic_curve(degree);
            let (clamped, poles) = clamp_periodic(degree, knots.clone(), points.clone()).unwrap();
            let start = knots[degree];
            let end = knots[points.len()];
            assert!(clamped[..=degree].iter().all(|k| *k == start));
            assert!(clamped[clamped.len() - degree - 1..]
                .iter()
                .all(|k| *k == end));
            assert!(distance(poles[0], *poles.last().unwrap()) < 1e-12);
            for i in 0..=2000 {
                let t = start + (end - start) * i as f64 / 2000.0;
                let a = evaluate(degree, &knots, &points, t);
                let b = evaluate(degree, &clamped, &poles, t);
                assert!(
                    distance(a, b) < 1e-9,
                    "degree {degree}, t {t}: {a:?} != {b:?}"
                );
            }
            assert!(
                distance(
                    evaluate(degree, &knots, &points, end),
                    *poles.last().unwrap()
                ) < 1e-9
            );
            let mut curve = nurbs(degree as u64, &knots, &points);
            curve["periodic"] = json!(true);
            assert!(spline(Some(&curve)).is_ok());
        }
    }

    #[test]
    fn clamping_handles_repeated_nonuniform_knots_and_clamped_ends() {
        let degree = 3;
        let knots = vec![-2.0, -1.0, 0.0, 0.0, 0.2, 0.2, 0.7, 1.0, 1.0, 2.0, 3.0];
        let points: Vec<P2> = (0..7).map(|i| [i as f64, (i * i % 5) as f64]).collect();
        let (clamped, poles) = clamp_periodic(degree, knots.clone(), points.clone()).unwrap();
        for i in 0..1000 {
            let t = i as f64 / 1000.0;
            assert!(
                distance(
                    evaluate(degree, &knots, &points, t),
                    evaluate(degree, &clamped, &poles, t)
                ) < 1e-9
            );
        }
        let (again, same) = clamp_periodic(degree, clamped.clone(), poles.clone()).unwrap();
        assert_eq!((again, same), (clamped, poles));
        let mut curve = nurbs(degree as u64, &knots, &points);
        curve["periodic"] = json!(true);
        let pieces = spline(Some(&curve)).unwrap().pieces;
        assert_eq!(pieces.len(), 3);
        for (piece, (low, high)) in pieces.iter().zip([(0., 0.2), (0.2, 0.7), (0.7, 1.)]) {
            for i in 0..=20 {
                let t = i as f64 / 20.;
                assert!(
                    distance(
                        evaluate(3, &[0., 0., 0., 0., 1., 1., 1., 1.], piece, t),
                        evaluate(3, &knots, &points, low + (high - low) * t)
                    ) < 1e-12
                );
            }
        }
    }

    #[test]
    fn periodic_spline_refusals_name_the_missing_target_data() {
        let (knots, points) = periodic_curve(3);
        let mut curve = nurbs(3, &knots, &points);
        curve["periodic"] = json!(true);
        curve["poles"] = json!({"form": "rational", "points": points.iter().enumerate()
            .map(|(i, p)| json!({"point": {"u": p[0], "v": p[1]}, "weight": i + 1})).collect::<Vec<_>>()});
        assert!(spline(Some(&curve))
            .unwrap_err()
            .contains("weighted periodic splines"));
        for point in curve["poles"]["points"].as_array_mut().unwrap() {
            point["weight"] = json!(2.0);
        }
        assert!(spline(Some(&curve)).is_ok());
        assert!(clamp_periodic(3, vec![], vec![]).is_none());
        assert!(clamp_periodic(3, vec![0.0; 11], points.clone()).is_none());
        let mut almost_uniform = nurbs(3, &knots, &points);
        almost_uniform["periodic"] = json!(true);
        almost_uniform["knots"][4] = json!(1.0 + 1e-10);
        assert!(spline(Some(&almost_uniform)).unwrap().pieces.len() > 1);
        let mut decreasing = knots.clone();
        decreasing[4] = -100.0;
        assert!(clamp_periodic(3, decreasing, points.clone()).is_none());
        let mut nonfinite = knots;
        nonfinite[0] = f64::NAN;
        assert!(clamp_periodic(3, nonfinite, points).is_none());
    }

    #[test]
    fn a_periodic_spline_shares_its_seam_point() {
        let (knots, points) = periodic_curve(3);
        let mut curve = nurbs(3, &knots, &points);
        curve["periodic"] = json!(true);
        let mut builder = Builder::default();
        builder
            .entity(
                &entity(json!({"kind": "nurbs", "curve": curve})),
                &mut Report::default(),
            )
            .unwrap();
        let spline = &builder.curves[0];
        assert_eq!(spline["control"], true);
        assert_eq!(spline["degree"], 3);
        let pts = spline["pts"].as_array().unwrap();
        assert_eq!(pts.first(), pts.last());
        assert_eq!(pts.len(), points.len());
    }

    #[test]
    fn a_spline_shares_only_its_ends() {
        let mut builder = Builder::default();
        let mut report = Report::default();
        builder
            .entity(&line([0.0, 0.0], [30.0, 0.0]), &mut report)
            .unwrap();
        let arch = json!({"id": "arch", "construction": false, "geometry": {"kind": "nurbs",
            "curve": nurbs(2, &[0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
                           &[[0.0, 0.0], [15.0, 30.0], [30.0, 0.0]])}});
        builder.entity(&arch, &mut report).unwrap();
        let spline = builder.curves.last().unwrap();
        assert_eq!(spline["type"], "spline");
        assert_eq!(spline["control"], true);
        // The arch's ends are the line's; its middle control point is its own.
        assert_eq!(spline["pts"], json!([1, 3, 2]));
    }

    #[test]
    fn whole_ellipses_map_by_their_major_axis_end() {
        let mut builder = Builder::default();
        let ellipse = entity(json!({"kind": "ellipse", "center": {"u": 5.0, "v": 5.0},
            "major_angle": std::f64::consts::FRAC_PI_2, "major_radius": 4.0, "minor_radius": 2.0}));
        builder.entity(&ellipse, &mut Report::default()).unwrap();
        let curve = builder.curves.first().unwrap();
        assert_eq!(curve["type"], "ellipse");
        assert_eq!(curve["r"], 2.0);
        let m = builder.positions[usize::try_from(curve["m"].as_u64().unwrap()).unwrap()];
        assert!(distance(m, [5.0, 9.0]) < 1e-9, "{m:?}");
    }

    #[test]
    fn unsupported_curves_drop_only_as_construction() {
        let ellipse = json!({"kind": "hyperbola", "center": {"u": 0.0, "v": 0.0},
            "major_angle": 0.0, "major_radius": 2.0, "minor_radius": 1.0});
        let mut report = Report::default();
        let mut builder = Builder::default();
        let mut construction = entity(ellipse.clone());
        construction["construction"] = json!(true);
        builder.entity(&construction, &mut report).unwrap();
        assert_eq!(report.losses().len(), 1);
        assert_eq!(
            builder.entity(&entity(ellipse), &mut report).unwrap_err(),
            "an unbounded hyperbola cannot be expressed by SolidCraft's bounded conic"
        );
    }

    /// The sketch of cadmpeg's synthetic `design_sketch_constraints.f3d`, compared with a
    /// golden copy. Set `UPDATE_GOLDEN=1` to rewrite it after an intended change.
    #[test]
    fn a_decoded_fusion_sketch_matches_its_golden_copy() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let model = dutchman_core::read(
            dutchman_core::Format::FusionArchive,
            &root.join("../dutchman-core/tests/fixtures/design_sketch_constraints.f3d"),
            &mut Report::default(),
        )
        .unwrap();
        let wire = serde_json::to_value(&model.ir.model).unwrap();
        let mut ctx = Context::new(&wire);
        ctx.params = super::super::params::map(&model, &mut Report::default()).1;
        let feature = ctx.arena("features").first().unwrap();
        let sketch = map(
            feature,
            super::super::operation(feature),
            &mut ctx,
            &mut Report::default(),
        )
        .unwrap();

        let pretty = serde_json::to_string_pretty(&sketch).unwrap() + "\n";
        let golden = root.join("tests/golden/design_sketch_constraints.sketch.json");
        if std::env::var_os("UPDATE_GOLDEN").is_some() {
            std::fs::write(&golden, &pretty).unwrap();
        }
        assert_eq!(pretty, std::fs::read_to_string(&golden).unwrap());
    }
}
