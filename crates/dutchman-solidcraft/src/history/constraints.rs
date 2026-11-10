//! Sketch constraints and dimensions, as SolidCraft sketch constraints.
//!
//! Geometry is already exact (sketches are written at their solved positions), so constraints
//! carry design intent only: a constraint that does not map is dropped and reported, never a
//! reason to flatten. A dimension's value is measured from the solved geometry, so the solver
//! starts satisfied, and it is linked to its SolidCraft parameter where the source names one.

use std::collections::BTreeMap;
use std::f64::consts::{PI, TAU};

use dutchman_core::{Report, Severity};
use serde_json::{json, Value};

use super::sketch::{distance, number, Builder, Curve, Shape, P2};
use super::Context;

/// The constraints of sketch `sketch_id`, mapped onto `builder`'s points and curves. What does
/// not map is reported under `label`.
pub(crate) fn map(
    sketch_id: &str,
    label: &str,
    ctx: &Context,
    builder: &mut Builder,
    report: &mut Report,
) -> Vec<Value> {
    let mut out = Vec::new();
    let mut dropped: BTreeMap<String, usize> = BTreeMap::new();
    let mut total = 0usize;
    let mut mapper = Mapper { ctx, builder };
    for row in ctx
        .arena("sketch_constraints")
        .iter()
        .filter(|row| row.get("sketch").and_then(Value::as_str) == Some(sketch_id))
    {
        if row.get("active").and_then(Value::as_bool) == Some(false) {
            continue;
        }
        total += 1;
        if row.get("virtual_space").and_then(Value::as_bool) == Some(true) {
            *dropped
                .entry(
                    "virtual sketch space: SolidCraft has no separate virtual solver space"
                        .to_owned(),
                )
                .or_default() += 1;
            continue;
        }
        let definition = row.get("definition").unwrap_or(&Value::Null);
        match mapper.map(definition) {
            Ok(mapped) => {
                let driven = row.get("driving").and_then(Value::as_bool) == Some(false);
                for mut constraint in mapped {
                    if let Some(fields) = constraint.as_object_mut() {
                        fields.insert("id".to_owned(), json!(format!("k{}", out.len() + 1)));
                        if driven {
                            fields.insert("driven".to_owned(), json!(true));
                        }
                    }
                    out.push(constraint);
                }
            }
            Err(kind) => {
                fn names_chain(value: &Value, shapes: &BTreeMap<String, Shape>) -> bool {
                    match value {
                        Value::String(id) => {
                            matches!(shapes.get(id), Some(Shape::Curve(c)) if !c.pieces.is_empty())
                        }
                        Value::Array(values) => values.iter().any(|v| names_chain(v, shapes)),
                        Value::Object(fields) => fields.values().any(|v| names_chain(v, shapes)),
                        _ => false,
                    }
                }
                if names_chain(definition, &mapper.builder.shapes) {
                    report.push(Severity::Info, label, format!("a constraint on an exact spline chain is not carried ({kind}); its geometry is exact"));
                } else {
                    *dropped.entry(kind).or_default() += 1;
                }
            }
        }
    }
    if !dropped.is_empty() {
        let count: usize = dropped.values().sum();
        let kinds: Vec<String> = dropped
            .iter()
            .map(|(kind, n)| {
                if *n == 1 {
                    kind.clone()
                } else {
                    format!("{kind} ×{n}")
                }
            })
            .collect();
        report.push(
            Severity::Degraded,
            label,
            format!(
                "{count} of {total} constraints not carried ({}); its geometry is exact",
                kinds.join(", ")
            ),
        );
    }
    out
}

/// Maps one constraint definition onto the sketch's points and curves.
struct Mapper<'a, 'model> {
    ctx: &'a Context<'model>,
    builder: &'a mut Builder,
}

impl Mapper<'_, '_> {
    /// The SolidCraft constraints for one CADIR constraint definition (none when it is already
    /// true by construction), or the kind that did not map.
    fn map(&mut self, definition: &Value) -> Result<Vec<Value>, String> {
        let kind = definition
            .get("kind")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        let field = |key: &str| definition.get(key).unwrap_or(&Value::Null);
        let unmapped = || match kind {
            "native" => format!(
                "native {}",
                definition
                    .get("native_kind")
                    .and_then(Value::as_str)
                    .unwrap_or("constraint")
            ),
            "internal_alignment" => format!(
                "internal alignment {}",
                definition
                    .get("alignment")
                    .and_then(|value| value.get("alignment"))
                    .and_then(Value::as_str)
                    .unwrap_or("helper")
            ),
            other => other.replace('_', " "),
        };
        let mapped = match kind {
            "disabled" => Some(Vec::new()),
            "projected_copy" => {
                self.copy_relation(field("source"), field("result"), None, 0.0, [0.0, 0.0])
            }
            "rectangular_pattern" => self.rectangular_pattern(field("pattern")),
            "circular_pattern" => self.circular_pattern(field("pattern")),
            "same_coordinate" => self.same_coordinate(field("relation")),
            "point_coordinate_values" => self.point_coordinate_values(definition),
            "at_intersection" => self.intersection(definition),
            "coradial" => self
                .pair("concentric", definition, &["circle", "arc"])
                .zip(self.equal(definition))
                .map(|(mut center, radius)| {
                    center.extend(radius);
                    center
                }),
            "tangent_loci" => self.tangent_loci(definition),
            "curvature" => self.curvature(definition),
            "point_symmetric" => self.point_symmetric(definition),
            "polygon" => self.polygon(field("polygon")),
            "distance_loci_value" => self.loci_distance("distance_loci", definition),
            "repeated_distance" => self.repeated_distance(definition),
            "repeated_length" | "repeated_radius" | "repeated_diameter" => {
                self.repeated(kind, definition)
            }
            "parallel_line_set_distance" => self.line_set_distance(definition),
            "arc_angle" => self.arc_angle(definition),
            "polar_distance" => self.polar_distance(definition),
            "equal_distance" => self.equal_distance(definition),
            "midpoint_coordinate" => self.midpoint_coordinate(definition),
            "coincident" => self.entity_coincident(field("entities")),
            "coincident_loci" => field("loci")
                .as_array()
                .and_then(|loci| {
                    loci.iter()
                        .map(|l| self.point(l))
                        .collect::<Option<Vec<_>>>()
                })
                .map(|points| coincident(&points)),
            "point_on_object" => {
                let p = self.point(field("point"));
                match (p, self.shape(field("entity"))) {
                    (Some(p), Some(Shape::Curve(c)))
                        if !matches!(c.kind, "compound_conic" | "compound_spline") =>
                    {
                        Some(vec![
                            json!({"type": "point_on_curve", "p": p, "c": c.index}),
                        ])
                    }
                    (Some(p), Some(Shape::Point(q))) => Some(coincident(&[p, *q])),
                    _ => None,
                }
            }
            "midpoint" => self
                .point(field("point"))
                .zip(self.curve(field("entity"), &["line"]))
                .map(|(p, l)| vec![json!({"type": "midpoint", "p": p, "l": l.index})]),
            "concentric" => self.concentric(definition),
            "collinear" => self.pair("collinear", definition, &["line"]),
            "parallel" => self.pair("parallel", definition, &["line"]),
            "perpendicular" => self.pair("perpendicular", definition, &["line"]),
            "tangent" => self.pair(
                "tangent",
                definition,
                &["line", "circle", "arc", "spline", "conic"],
            ),
            "equal" => self.equal(definition),
            "horizontal" | "vertical" => self
                .curve(field("entity"), &["line"])
                .map(|l| vec![json!({"type": kind, "l": l.index})]),
            "fixed" => match self.shape(field("entity")).cloned() {
                Some(Shape::Point(p)) => Some(vec![json!({"type": "fix", "p": p})]),
                Some(Shape::Curve(c)) => {
                    let count = if c.kind == "compound_conic" {
                        4
                    } else {
                        c.pieces.len().max(1)
                    };
                    (c.index..c.index + count)
                        .map(|index| self.builder.fix_curve(index))
                        .collect::<Option<Vec<_>>>()
                        .map(|_| Vec::new())
                }
                _ => None,
            },
            "symmetric" => {
                let p = self.point(field("first"));
                let q = self.point(field("second"));
                let l = self.curve(field("axis"), &["line"]);
                match (p, q, l) {
                    (Some(p), Some(q), Some(l)) => Some(vec![
                        json!({"type": "symmetric", "p": p, "q": q, "l": l.index}),
                    ]),
                    _ => None,
                }
            }
            "distance_loci" | "horizontal_distance" | "vertical_distance" => {
                self.loci_distance(kind, definition)
            }
            "distance" => self.entity_distance(definition),
            "radius" | "diameter" => self.radius(kind, definition),
            "angle" => self.angle(definition),
            "angle_to_axis" => self.angle_to_axis(definition),
            _ => None,
        };
        mapped.ok_or_else(|| {
            let reason = match kind {
                "angle_difference" | "scalar_equality" => "solver scalar identities have no sketch entity binding",
                "snells_law" => "SolidCraft has no refraction constraint",
                "weight" => "SolidCraft has no driving rational spline weight dimension",
                "ellipse_angle" => "SolidCraft has no elliptical parameter sweep dimension",
                "internal_alignment" => "the source helper relation has no matching SolidCraft constraint",
                "midpoint_coordinate" => "requires two resolved point loci and a matching solved coordinate",
                "equal_distance" => "requires two resolved point pairs for equal-length construction carriers",
                "polar_distance" => "requires resolved points and a displacement matching their solved positions",
                "rectangular_pattern" | "circular_pattern" => "requires resolved matching instances, no driving count or span parameters, and at most 500 instances per direction",
                "offset" => "source offset pairs are not bound to a SolidCraft offset association",
                "projected_copy" => "requires matching resolved point or curve carriers; no external projection link can be reconstructed",
                "spline_group" => "SolidCraft has no source spline helper group constraint",
                "text_frame" | "text_path" | "text" => "the source text has no mapped SolidCraft text link",
                "group" => "SolidCraft has no source construction handle constraint",
                "native" => "its native operands have no resolved neutral relation",
                "distance" => "requires a resolved line length, point pair, point and line, or parallel line pair; nonparallel line spacing and unresolved external entities have no unambiguous distance; its value must match the solved line or endpoint measurement",
                "coincident" => "requires resolved points or a point and curve; curve-only entity loci do not identify which points must coincide",
                _ => "its operands or dimension value cannot be expressed by this SolidCraft constraint",
            };
            format!("{}: {reason}", unmapped())
        })
    }

    fn shape_points(&self, entity: &Value) -> Option<Vec<usize>> {
        match self.shape(entity)? {
            Shape::Point(p) => Some(vec![*p]),
            Shape::Curve(c) => {
                let row = self.builder_curve(c.index)?;
                if c.kind == "compound_spline" {
                    return c
                        .pieces
                        .iter()
                        .map(|piece| {
                            self.builder_curve(piece.index)?
                                .get("pts")?
                                .as_array()?
                                .iter()
                                .map(|p| usize::try_from(p.as_u64()?).ok())
                                .collect::<Option<Vec<_>>>()
                        })
                        .collect::<Option<Vec<_>>>()
                        .map(|pieces| {
                            pieces
                                .into_iter()
                                .enumerate()
                                .flat_map(|(index, points)| {
                                    points.into_iter().skip(usize::from(index > 0))
                                })
                                .collect()
                        });
                }
                if c.kind == "spline" {
                    return row
                        .get("pts")?
                        .as_array()?
                        .iter()
                        .map(|p| usize::try_from(p.as_u64()?).ok())
                        .collect();
                }
                let keys: &[&str] = match c.kind {
                    "line" => &["a", "b"],
                    "circle" => &["c"],
                    "arc" => &["c", "a", "b"],
                    "ellipse" => &["c", "m"],
                    "conic" => &["a", "b", "apex"],
                    _ => return None,
                };
                keys.iter()
                    .map(|key| usize::try_from(row.get(*key)?.as_u64()?).ok())
                    .collect()
            }
        }
    }

    fn copy_relation(
        &self,
        source: &Value,
        result: &Value,
        center: Option<usize>,
        angle: f64,
        offset: P2,
    ) -> Option<Vec<Value>> {
        let a = self.shape(source)?;
        let b = self.shape(result)?;
        let mut out = Vec::new();
        match (a, b) {
            (Shape::Point(_), Shape::Point(_)) => {}
            (Shape::Curve(a), Shape::Curve(b)) if a.kind == b.kind => {
                let first = self.builder_curve(a.index)?;
                let second = self.builder_curve(b.index)?;
                if a.kind == "conic" && first.get("rho") != second.get("rho") {
                    return None;
                }
                if a.kind == "compound_spline"
                    && (a.pieces.len() != b.pieces.len()
                        || a.pieces.iter().zip(&b.pieces).any(|(a, b)| {
                            self.builder_curve(a.index).and_then(|c| c.get("degree"))
                                != self.builder_curve(b.index).and_then(|c| c.get("degree"))
                        }))
                {
                    return None;
                }
                if a.kind == "spline" && first.get("degree") != second.get("degree") {
                    return None;
                }
                if matches!(a.kind, "circle" | "ellipse") {
                    if (number(first.get("r"))? - number(second.get("r"))?).abs() > 1e-6 {
                        return None;
                    }
                    out.push(json!({"type": "pattern_radius", "a": a.index, "b": b.index}));
                }
            }
            _ => return None,
        }
        let source = self.shape_points(source)?;
        let result = self.shape_points(result)?;
        if source.len() != result.len() || angle.abs() > TAU {
            return None;
        }
        let c = match center {
            Some(p) => self.position(p)?,
            None => [0.0, 0.0],
        };
        for (p, q) in source.into_iter().zip(result) {
            let at = self.position(p)?;
            let d = [at[0] - c[0], at[1] - c[1]];
            let expected = [
                c[0] + d[0] * angle.cos() - d[1] * angle.sin() + offset[0],
                c[1] + d[0] * angle.sin() + d[1] * angle.cos() + offset[1],
            ];
            if distance(expected, self.position(q)?) > 1e-6 {
                return None;
            }
            if p != q {
                out.push(
                    json!({"type": "pattern_point", "p": p, "q": q, "center": center,
                    "angle": angle, "offset": offset, "directions": [null, null]}),
                );
            }
        }
        Some(out)
    }

    fn rectangular_pattern(&self, pattern: &Value) -> Option<Vec<Value>> {
        let directions = pattern.get("directions")?.as_array()?;
        let displacement = |direction: &Value| {
            if direction.get("distance").is_some_and(|v| !v.is_null())
                || direction
                    .get("count_parameter")
                    .is_some_and(|v| !v.is_null())
            {
                return None;
            }
            let dir = direction.get("direction")?.as_array()?;
            let spacing = number(direction.get("spacing"))?;
            Some([
                number(dir.first())? * spacing,
                number(dir.get(1))? * spacing,
            ])
        };
        let first = displacement(directions.first()?)?;
        let second = displacement(directions.get(1)?)?;
        let rows = pattern.get("rows")?.as_array()?;
        let seed = rows
            .first()?
            .as_array()?
            .first()?
            .get("entities")?
            .as_array()?;
        if rows.len() > 500
            || rows
                .iter()
                .any(|row| row.as_array().is_none_or(|row| row.len() > 500))
        {
            return None;
        }
        let mut out = Vec::new();
        for (i, row) in rows.iter().enumerate() {
            for (j, instance) in row.as_array()?.iter().enumerate() {
                if i == 0 && j == 0 {
                    continue;
                }
                let entities = instance.get("entities")?.as_array()?;
                if entities.len() != seed.len() {
                    return None;
                }
                let offset = [
                    first[0] * i as f64 + second[0] * j as f64,
                    first[1] * i as f64 + second[1] * j as f64,
                ];
                for (a, b) in seed.iter().zip(entities) {
                    out.extend(self.copy_relation(a, b, None, 0.0, offset)?);
                }
            }
        }
        Some(out)
    }

    fn circular_pattern(&self, pattern: &Value) -> Option<Vec<Value>> {
        if ["angle_parameter", "count_parameter"]
            .iter()
            .any(|key| pattern.get(*key).is_some_and(|v| !v.is_null()))
        {
            return None;
        }
        let center = self.point(&json!({"kind": "entity", "entity": pattern.get("center")?}))?;
        let seed = pattern.get("seed")?.as_array()?;
        let instances = pattern.get("instances")?.as_array()?;
        if instances.len() >= 500 {
            return None;
        }
        let mut out = Vec::new();
        for instance in instances {
            let angle = number(instance.get("angle"))?;
            let entities = instance.get("entities")?.as_array()?;
            if entities.len() != seed.len() {
                return None;
            }
            for (a, b) in seed.iter().zip(entities) {
                out.extend(self.copy_relation(a, b, Some(center), angle, [0.0, 0.0])?);
            }
        }
        Some(out)
    }

    /// A construction carrier lets SolidCraft express relations between existing loci
    /// without replacing the profile's original entities.
    fn carrier(&mut self, p: usize, q: usize) -> Option<usize> {
        if p == q {
            return None;
        }
        let l = self.builder.curve_count();
        self.builder.curve(
            None,
            "l",
            true,
            json!({"type": "line", "a": p, "b": q}),
            &[],
        );
        Some(l)
    }

    fn polar_distance(&mut self, definition: &Value) -> Option<Vec<Value>> {
        let p = self.point(definition.get("first")?)?;
        let q = self.point(definition.get("second")?)?;
        let value = number(definition.get("distance"))?;
        if value <= 1e-12 {
            return Some(coincident(&[p, q]));
        }
        let a = self.position(p)?;
        let b = self.position(q)?;
        let angle = number(definition.get("angle"))?;
        if (distance(a, b) - value).abs() > 1e-6
            || wrap((b[1] - a[1]).atan2(b[0] - a[0]) - angle).abs() > ANGLE_TOLERANCE
        {
            return None;
        }
        let l = self.carrier(p, q)?;
        let mut length = json!({"type": "length", "l": l, "value": value});
        if let Some(param) = definition
            .get("distance_parameter")
            .and_then(Value::as_str)
            .and_then(|id| self.ctx.params.get(id))
        {
            length
                .as_object_mut()?
                .insert("param".to_owned(), json!(param));
        }
        Some(vec![
            length,
            json!({"type": "line_angle", "l": l, "value": angle}),
        ])
    }

    fn equal_distance(&mut self, definition: &Value) -> Option<Vec<Value>> {
        let pair = |key| {
            let pair = definition.get(key)?;
            Some((
                self.point(pair.get("first")?)?,
                self.point(pair.get("second")?)?,
            ))
        };
        let (p, q) = pair("first")?;
        let (r, s) = pair("second")?;
        if p == q || r == s {
            return Some([coincident(&[p, q]), coincident(&[r, s])].concat());
        }
        let a = self.carrier(p, q)?;
        let b = self.carrier(r, s)?;
        Some(vec![json!({"type": "equal", "a": a, "b": b})])
    }

    fn midpoint_coordinate(&mut self, definition: &Value) -> Option<Vec<Value>> {
        let p = self.point(definition.get("first")?)?;
        let q = self.point(definition.get("second")?)?;
        let a = self.position(p)?;
        let b = self.position(q)?;
        let midpoint = [a[0] * 0.5 + b[0] * 0.5, a[1] * 0.5 + b[1] * 0.5];
        let (axis, kind) = match definition.get("axis")?.as_str()? {
            "u" => (0, "distance_x"),
            "v" => (1, "distance_y"),
            _ => return None,
        };
        let value = number(definition.get("value"))?;
        if (midpoint.get(axis)? - value).abs() > 1e-6 {
            return None;
        }
        let (point, mut out) = if p == q {
            (p, Vec::new())
        } else {
            let l = self.carrier(p, q)?;
            let point = self.builder.own_point(midpoint);
            (point, vec![json!({"type": "midpoint", "p": point, "l": l})])
        };
        out.push(json!({"type": kind, "p": 0, "q": point, "value": value.abs()}));
        Some(out)
    }

    fn same_coordinate(&self, relation: &Value) -> Option<Vec<Value>> {
        let p = self.point(relation.get("first")?)?;
        let q = self.point(relation.get("second")?)?;
        if p == q {
            return Some(Vec::new());
        }
        let kind = match relation.get("axis")?.as_str()? {
            "u" => "vertical_points",
            "v" => "horizontal_points",
            _ => return None,
        };
        Some(vec![json!({"type": kind, "p": p, "q": q})])
    }

    fn point_coordinate_values(&self, definition: &Value) -> Option<Vec<Value>> {
        let p = self.point(definition.get("point")?)?;
        let values = definition.get("values")?.as_array()?;
        let [u, v] = self.position(p)?;
        if (number(values.first())? - u).abs() > 1e-6 || (number(values.get(1))? - v).abs() > 1e-6 {
            return None;
        }
        Some(vec![json!({"type": "fix", "p": p})])
    }

    fn intersection(&self, definition: &Value) -> Option<Vec<Value>> {
        let p = self.point(definition.get("point")?)?;
        ["first", "second"]
            .into_iter()
            .map(|key| {
                let c = self.curve(
                    definition.get(key)?,
                    &["line", "circle", "arc", "ellipse", "spline", "conic"],
                )?;
                Some(json!({"type": "point_on_curve", "p": p, "c": c.index}))
            })
            .collect()
    }

    fn concentric(&self, definition: &Value) -> Option<Vec<Value>> {
        let a = self.curve(
            definition.get("first")?,
            &["circle", "arc", "ellipse", "compound_conic"],
        )?;
        let b = self.curve(
            definition.get("second")?,
            &["circle", "arc", "ellipse", "compound_conic"],
        )?;
        // SolidCraft's Concentric accepts circular curves only; shared centers also
        // express the neutral relation for ellipses.
        if matches!(a.kind, "ellipse" | "compound_conic")
            || matches!(b.kind, "ellipse" | "compound_conic")
        {
            Some(coincident(&[a.center?, b.center?]))
        } else {
            self.pair("concentric", definition, &["circle", "arc"])
        }
    }

    fn tangent_loci(&self, definition: &Value) -> Option<Vec<Value>> {
        let first = definition.get("first")?;
        let second = definition.get("second")?;
        let endpoint = |locus: &Value| {
            matches!(
                locus.get("kind").and_then(Value::as_str),
                Some("start" | "end")
            )
        };
        if !endpoint(first) || !endpoint(second) {
            return None;
        }
        let p = self.point(first)?;
        let q = self.point(second)?;
        let endpoint_curve = |locus: &Value| {
            let Shape::Curve(curve) = self.shape(locus.get("entity")?)? else {
                return None;
            };
            let piece = if curve.pieces.is_empty() {
                curve
            } else if locus.get("kind")?.as_str()? == "start" {
                curve.pieces.first()?
            } else {
                curve.pieces.last()?
            };
            matches!(piece.kind, "line" | "arc" | "spline" | "conic").then_some(piece)
        };
        let a = endpoint_curve(first)?;
        let b = endpoint_curve(second)?;
        let kind = if a.kind == "line" && b.kind == "line" {
            "collinear"
        } else {
            "tangent"
        };
        let mut out = vec![json!({"type": kind, "a": a.index, "b": b.index})];
        out.extend(coincident(&[p, q]));
        Some(out)
    }

    fn curvature(&self, definition: &Value) -> Option<Vec<Value>> {
        self.pair("smooth", definition, &["line", "arc", "spline", "conic"])
    }

    fn point_symmetric(&mut self, definition: &Value) -> Option<Vec<Value>> {
        let p = self.point(definition.get("first")?)?;
        let q = self.point(definition.get("second")?)?;
        let center = self.point(definition.get("center")?)?;
        let a = self.position(p)?;
        let b = self.position(q)?;
        if distance(
            self.position(center)?,
            [a[0] * 0.5 + b[0] * 0.5, a[1] * 0.5 + b[1] * 0.5],
        ) > 1e-6
        {
            return None;
        }
        if p == q {
            return Some(coincident(&[p, center]));
        }
        let l = self.builder.curve_count();
        self.builder.curve(
            None,
            "l",
            true,
            json!({"type": "line", "a": p, "b": q}),
            &[],
        );
        Some(vec![json!({"type": "midpoint", "p": center, "l": l})])
    }

    fn polygon(&mut self, polygon: &Value) -> Option<Vec<Value>> {
        let lines: Vec<Curve> = polygon
            .get("entities")?
            .as_array()?
            .iter()
            .map(|id| self.curve(id, &["line"]).cloned())
            .collect::<Option<_>>()?;
        if !(3..=200).contains(&lines.len()) {
            return None;
        }
        let mut corners = Vec::new();
        let mut degrees = BTreeMap::new();
        for line in &lines {
            for p in [line.start?, line.end?] {
                *degrees.entry(p).or_insert(0usize) += 1;
                if !corners.contains(&p) {
                    corners.push(p);
                }
            }
        }
        if corners.len() != lines.len() || degrees.values().any(|n| *n != 2) {
            return None;
        }
        let positions: Vec<P2> = corners
            .iter()
            .map(|p| self.position(*p))
            .collect::<Option<_>>()?;
        let center = positions
            .iter()
            .fold([0.0, 0.0], |[x, y], p| [x + p[0], y + p[1]])
            .map(|v| v / positions.len() as f64);
        let radius =
            positions.iter().map(|p| distance(*p, center)).sum::<f64>() / positions.len() as f64;
        let length = lines
            .first()
            .and_then(|l| Some(distance(self.position(l.start?)?, self.position(l.end?)?)))?;
        if radius <= 1e-9
            || positions
                .iter()
                .any(|p| (distance(*p, center) - radius).abs() > 1e-6)
            || lines.iter().any(|l| {
                l.start
                    .zip(l.end)
                    .and_then(|(a, b)| Some(distance(self.position(a)?, self.position(b)?)))
                    .is_none_or(|v| (v - length).abs() > 1e-6)
            })
        {
            return None;
        }
        let c = self.builder.own_point(center);
        let circle = self.builder.curve_count();
        self.builder.curve(
            None,
            "c",
            true,
            json!({"type": "circle", "c": c, "r": radius}),
            &[],
        );
        let mut out: Vec<Value> = lines
            .windows(2)
            .filter_map(|pair| {
                Some(json!({"type": "equal", "a": pair.first()?.index, "b": pair.get(1)?.index}))
            })
            .collect();
        out.extend(
            corners
                .into_iter()
                .map(|p| json!({"type": "point_on_curve", "p": p, "c": circle})),
        );
        Some(out)
    }

    fn repeated_distance(&self, definition: &Value) -> Option<Vec<Value>> {
        let mut out = Vec::new();
        for measurement in definition.get("measurements")?.as_array()? {
            let kind = match measurement.get("kind")?.as_str()? {
                "distance" => "distance_loci",
                "horizontal" => "horizontal_distance",
                "vertical" => "vertical_distance",
                _ => return None,
            };
            let mut dimension = measurement.clone();
            dimension
                .as_object_mut()?
                .insert("parameter".to_owned(), definition.get("parameter")?.clone());
            out.extend(self.loci_distance(kind, &dimension)?);
        }
        Some(out)
    }

    fn repeated(&self, kind: &str, definition: &Value) -> Option<Vec<Value>> {
        let mut out = Vec::new();
        for entity in definition.get("entities")?.as_array()? {
            let dimension = json!({"entity": entity, "entities": [entity], "parameter": definition.get("parameter")?});
            out.extend(match kind {
                "repeated_length" => self.entity_distance(&dimension)?,
                "repeated_radius" => self.radius("radius", &dimension)?,
                "repeated_diameter" => self.radius("diameter", &dimension)?,
                _ => return None,
            });
        }
        Some(out)
    }

    fn line_set_distance(&self, definition: &Value) -> Option<Vec<Value>> {
        let first: Vec<&Curve> = definition
            .get("first")?
            .as_array()?
            .iter()
            .map(|id| self.curve(id, &["line"]))
            .collect::<Option<_>>()?;
        let second: Vec<&Curve> = definition
            .get("second")?
            .as_array()?
            .iter()
            .map(|id| self.curve(id, &["line"]))
            .collect::<Option<_>>()?;
        let a = *first.first()?;
        let b = *second.first()?;
        let mut out = vec![json!({"type": "parallel", "a": a.index, "b": b.index})];
        for set in [&first, &second] {
            for pair in set.windows(2) {
                out.push(
                    json!({"type": "collinear", "a": pair.first()?.index, "b": pair.get(1)?.index}),
                );
            }
        }
        let p = a.start?;
        let value = point_line(
            self.position(p)?,
            self.position(b.start?)?,
            self.position(b.end?)?,
        )?;
        out.extend(self.dimension(
            json!({"type": "point_line_distance", "p": p, "l": b.index, "value": value}),
            definition,
        ));
        Some(out)
    }

    fn arc_angle(&self, definition: &Value) -> Option<Vec<Value>> {
        let c = self.curve(definition.get("entity")?, &["arc"])?;
        let value = number(definition.get("angle"))?;
        if value > TAU {
            return None;
        }
        let center = self.position(c.center?)?;
        let a = self.position(c.start?)?;
        let b = self.position(c.end?)?;
        let sweep = signed_angle(
            [a[0] - center[0], a[1] - center[1]],
            [b[0] - center[0], b[1] - center[1]],
        )?
        .rem_euclid(TAU);
        if wrap(sweep - value).abs() > ANGLE_TOLERANCE {
            return None;
        }
        Some(vec![
            json!({"type": "arc_sweep", "c": c.index, "value": value}),
        ])
    }

    /// What the source entity `id` became.
    fn shape(&self, id: &Value) -> Option<&Shape> {
        self.builder.shapes.get(id.as_str()?)
    }

    /// The curve the source entity `id` became, if it is one of `kinds`.
    fn curve(&self, id: &Value, kinds: &[&str]) -> Option<&Curve> {
        match self.shape(id)? {
            Shape::Curve(curve) if kinds.contains(&curve.kind) => Some(curve),
            _ => None,
        }
    }

    /// The point a CADIR locus names: a point entity, or a curve's start, end or centre.
    fn point(&self, locus: &Value) -> Option<usize> {
        let shape = self.shape(locus.get("entity")?)?;
        match (locus.get("kind")?.as_str()?, shape) {
            ("entity", Shape::Point(p)) => Some(*p),
            ("start", Shape::Curve(c)) => c.start,
            ("end", Shape::Curve(c)) => c.end,
            ("center", Shape::Curve(c)) => c.center,
            _ => None,
        }
    }

    fn position(&self, point: usize) -> Option<P2> {
        self.builder.positions.get(point).copied()
    }

    /// A two-curve relation of SolidCraft type `kind` between `first` and `second`.
    fn pair(&self, kind: &str, definition: &Value, kinds: &[&str]) -> Option<Vec<Value>> {
        let first = self.shape(definition.get("first")?)?;
        let second = self.shape(definition.get("second")?)?;
        let (Shape::Curve(first), Shape::Curve(second)) = (first, second) else {
            return None;
        };
        let candidates = |curve: &Curve| {
            if curve.pieces.is_empty() {
                vec![curve.clone()]
            } else {
                curve.pieces.clone()
            }
        };
        let a_candidates = candidates(first);
        let b_candidates = candidates(second);
        let mut pairs = a_candidates
            .iter()
            .flat_map(|a| b_candidates.iter().map(move |b| (a, b)))
            .filter(|(a, b)| kinds.contains(&a.kind) && kinds.contains(&b.kind))
            .filter(|(a, b)| {
                let meet = [a.start, a.end]
                    .into_iter()
                    .flatten()
                    .any(|p| [b.start, b.end].contains(&Some(p)));
                if kind == "smooth" {
                    a.index != b.index && meet
                } else {
                    first.pieces.is_empty() && second.pieces.is_empty() || kind == "tangent" && meet
                }
            });
        let (a, b) = pairs.next()?;
        if pairs.next().is_some() {
            return None;
        }
        let kind = if kind == "tangent" && a.kind == "line" && b.kind == "line" {
            "collinear"
        } else {
            kind
        };
        Some(vec![json!({"type": kind, "a": a.index, "b": b.index})])
    }

    fn equal(&self, definition: &Value) -> Option<Vec<Value>> {
        let a = self.curve(definition.get("first")?, &["line", "circle", "arc"])?;
        let b = self.curve(definition.get("second")?, &["line", "circle", "arc"])?;
        let round = |kind: &str| kind == "circle" || kind == "arc";
        (a.kind == b.kind || (round(a.kind) && round(b.kind)))
            .then(|| vec![json!({"type": "equal", "a": a.index, "b": b.index})])
    }

    /// The SolidCraft parameter a dimension's `parameter` drives, as a `param` field.
    fn param(&self, definition: &Value) -> Option<String> {
        self.ctx
            .params
            .get(definition.get("parameter")?.as_str()?)
            .cloned()
    }

    fn dimension(&self, mut constraint: Value, definition: &Value) -> Vec<Value> {
        if let (Some(param), Some(fields)) = (self.param(definition), constraint.as_object_mut()) {
            fields.insert("param".to_owned(), json!(param));
        }
        vec![constraint]
    }

    fn loci_distance(&self, kind: &str, definition: &Value) -> Option<Vec<Value>> {
        let first = definition.get("first")?;
        let second = definition.get("second")?;
        let (p, q) = (self.point(first), self.point(second));
        let constraint = match (kind, p, q) {
            ("distance_loci", Some(p), Some(q)) => {
                let value = distance(self.position(p)?, self.position(q)?);
                json!({"type": "distance", "p": p, "q": q, "value": value})
            }
            ("distance_loci", Some(p), None) | ("distance_loci", None, Some(p)) => {
                let line = if q.is_none() { second } else { first };
                let l = self.curve(line.get("entity")?, &["line"])?;
                let value = point_line(
                    self.position(p)?,
                    self.position(l.start?)?,
                    self.position(l.end?)?,
                )?;
                json!({"type": "point_line_distance", "p": p, "l": l.index, "value": value})
            }
            ("horizontal_distance", Some(p), Some(q)) => {
                let value = (self.position(q)?[0] - self.position(p)?[0]).abs();
                json!({"type": "distance_x", "p": p, "q": q, "value": value})
            }
            ("vertical_distance", Some(p), Some(q)) => {
                let value = (self.position(q)?[1] - self.position(p)?[1]).abs();
                json!({"type": "distance_y", "p": p, "q": q, "value": value})
            }
            _ => return None,
        };
        Some(self.dimension(constraint, definition))
    }

    /// Entity coincidences include point-on-curve relations, not only point pairs.
    fn entity_coincident(&self, entities: &Value) -> Option<Vec<Value>> {
        let entities = entities.as_array()?;
        let first = entities.first()?;
        let mut out = Vec::new();
        for second in entities.iter().skip(1) {
            match (self.shape(first)?, self.shape(second)?) {
                (Shape::Point(p), Shape::Point(q)) => out.extend(coincident(&[*p, *q])),
                (Shape::Point(p), Shape::Curve(c)) | (Shape::Curve(c), Shape::Point(p))
                    if !matches!(c.kind, "compound_conic" | "compound_spline") =>
                {
                    out.push(json!({"type": "point_on_curve", "p": p, "c": c.index}));
                }
                _ => return None,
            }
        }
        Some(out)
    }

    fn distance_value(&self, definition: &Value) -> Option<f64> {
        definition
            .get("parameter")
            .and_then(Value::as_str)
            .and_then(|id| self.ctx.row("parameters", id))
            .and_then(|parameter| number(parameter.pointer("/value/value")))
    }

    /// A generic entity distance may refer to the infinite line or its finite end.
    /// Use the source-evaluated dimension to distinguish them, never force a value
    /// onto geometry that does not measure it.
    fn point_entity_distance(
        &self,
        p: usize,
        line: &Curve,
        definition: &Value,
    ) -> Option<Vec<Value>> {
        let a = self.position(line.start?)?;
        let b = self.position(line.end?)?;
        let at = self.position(p)?;
        let value = point_line(at, a, b)?;
        let expected = self.distance_value(definition);
        if let Some(expected) = expected {
            if (expected - value).abs() > 1e-6 {
                if self
                    .builder_curve(line.index)?
                    .get("infinite")
                    .and_then(Value::as_bool)
                    == Some(true)
                {
                    return None;
                }
                let delta = [b[0] - a[0], b[1] - a[1]];
                let t = ((at[0] - a[0]) * delta[0] + (at[1] - a[1]) * delta[1])
                    / (delta[0] * delta[0] + delta[1] * delta[1]);
                let q = if t < 0.0 {
                    line.start?
                } else if t > 1.0 {
                    line.end?
                } else {
                    return None;
                };
                let value = distance(at, self.position(q)?);
                if (expected - value).abs() > 1e-6 {
                    return None;
                }
                return Some(self.dimension(
                    json!({"type": "distance", "p": p, "q": q, "value": value}),
                    definition,
                ));
            }
        }
        Some(self.dimension(
            json!({"type": "point_line_distance", "p": p, "l": line.index, "value": value}),
            definition,
        ))
    }

    /// Disjoint parallel finite segments can instead measure between their nearest ends.
    /// A unique closest pair and the source value must both prove that interpretation.
    fn parallel_end_distance(&self, a: &Curve, b: &Curve, definition: &Value) -> Option<Value> {
        let expected = self.distance_value(definition)?;
        for line in [a, b] {
            if self
                .builder_curve(line.index)?
                .get("infinite")
                .and_then(Value::as_bool)
                == Some(true)
            {
                return None;
            }
        }
        let start = self.position(a.start?)?;
        let delta = self.direction(a)?;
        let length = delta[0].hypot(delta[1]);
        if length <= 1e-12 {
            return None;
        }
        let project =
            |p: P2| ((p[0] - start[0]) * delta[0] + (p[1] - start[1]) * delta[1]) / length;
        let t = project(self.position(b.start?)?);
        let u = project(self.position(b.end?)?);
        if t.max(u) >= 0.0 && t.min(u) <= length {
            return None;
        }
        let mut pairs = Vec::new();
        for p in [a.start?, a.end?] {
            for q in [b.start?, b.end?] {
                pairs.push((p, q, distance(self.position(p)?, self.position(q)?)));
            }
        }
        pairs.sort_by(|a, b| a.2.total_cmp(&b.2));
        let (p, q, value) = *pairs.first()?;
        if (value - expected).abs() > 1e-6
            || pairs
                .get(1)
                .is_some_and(|other| (other.2 - value).abs() <= 1e-6)
        {
            return None;
        }
        Some(json!({"type": "distance", "p": p, "q": q, "value": value}))
    }

    fn entity_distance(&self, definition: &Value) -> Option<Vec<Value>> {
        let entities = definition.get("entities")?.as_array()?;
        match entities.as_slice() {
            [line] => {
                let l = self.curve(line, &["line"])?;
                let value = distance(self.position(l.start?)?, self.position(l.end?)?);
                Some(self.dimension(
                    json!({"type": "length", "l": l.index, "value": value}),
                    definition,
                ))
            }
            [first, second] => {
                match (self.shape(first)?, self.shape(second)?) {
                    (Shape::Point(p), Shape::Curve(c)) | (Shape::Curve(c), Shape::Point(p))
                        if c.kind == "line" =>
                    {
                        return self.point_entity_distance(*p, c, definition);
                    }
                    _ => {}
                }
                if let (Some(a), Some(b)) =
                    (self.curve(first, &["line"]), self.curve(second, &["line"]))
                {
                    let u = self.direction(a)?;
                    let v = self.direction(b)?;
                    let scale = u[0].hypot(u[1]) * v[0].hypot(v[1]);
                    if scale <= 1e-12 || (u[0] * v[1] - u[1] * v[0]).abs() > scale * 1e-9 {
                        return None;
                    }
                    let p = a.start?;
                    let value = point_line(
                        self.position(p)?,
                        self.position(b.start?)?,
                        self.position(b.end?)?,
                    )?;
                    let mut out = vec![json!({"type": "parallel", "a": a.index, "b": b.index})];
                    let constraint = if self
                        .distance_value(definition)
                        .is_some_and(|expected| (expected - value).abs() > 1e-6)
                    {
                        self.parallel_end_distance(a, b, definition)?
                    } else {
                        json!({"type": "point_line_distance", "p": p, "l": b.index, "value": value})
                    };
                    out.extend(self.dimension(constraint, definition));
                    return Some(out);
                }
                let mut loci = definition.clone();
                let fields = loci.as_object_mut()?;
                fields.insert(
                    "first".to_owned(),
                    json!({"kind": "entity", "entity": first}),
                );
                fields.insert(
                    "second".to_owned(),
                    json!({"kind": "entity", "entity": second}),
                );
                self.loci_distance("distance_loci", &loci)
            }
            _ => None,
        }
    }

    fn radius(&self, kind: &str, definition: &Value) -> Option<Vec<Value>> {
        let kinds: &[&str] = if kind == "radius" {
            &["circle", "arc", "ellipse"]
        } else {
            &["circle", "arc"]
        };
        let c = self.curve(definition.get("entity")?, kinds)?;
        let radius = match c.kind {
            "circle" | "ellipse" => number(self.builder_curve(c.index)?.get("r"))?,
            _ => distance(self.position(c.center?)?, self.position(c.start?)?),
        };
        let value = if kind == "diameter" {
            radius * 2.0
        } else {
            radius
        };
        Some(self.dimension(
            json!({"type": kind, "c": c.index, "value": value}),
            definition,
        ))
    }

    /// An angle between two lines. SolidCraft holds the signed counter-clockwise angle from
    /// `a` to `b` (plus a half turn with `flip`); the source's parameter holds the angle as
    /// shown. Of the four ways to state it, the one the solved geometry satisfies with the
    /// parameter's value is written, so the parameter keeps driving the same angle.
    fn angle(&self, definition: &Value) -> Option<Vec<Value>> {
        let a = self.curve(definition.get("first")?, &["line"])?;
        let b = self.curve(definition.get("second")?, &["line"])?;
        let measured = signed_angle(self.direction(a)?, self.direction(b)?)?;
        let constraint = match self.angle_value(definition) {
            Some(value) => {
                let (x, y, flip) = [(a, b, false), (a, b, true), (b, a, false), (b, a, true)]
                    .into_iter()
                    .find(|(x, _, flip)| {
                        let turn = if x.index == a.index {
                            measured
                        } else {
                            -measured
                        };
                        let target = value + if *flip { PI } else { 0.0 };
                        wrap(turn - target).abs() <= ANGLE_TOLERANCE
                    })?;
                let mut constraint =
                    json!({"type": "angle", "a": x.index, "b": y.index, "value": value});
                if let (true, Some(fields)) = (flip, constraint.as_object_mut()) {
                    fields.insert("flip".to_owned(), json!(true));
                }
                constraint
            }
            None => json!({"type": "angle", "a": a.index, "b": b.index,
                           "value": measured.rem_euclid(TAU)}),
        };
        Some(self.dimension(constraint, definition))
    }

    /// A line's angle from the sketch's horizontal axis, as SolidCraft's signed line angle.
    /// Only written when the geometry and the parameter agree on the sign, so the parameter
    /// drives it the same way.
    fn angle_to_axis(&mut self, definition: &Value) -> Option<Vec<Value>> {
        if definition.get("axis")?.as_str()? == "vertical" {
            let l = self.curve(definition.get("entity")?, &["line"])?.clone();
            let d = self.direction(&l)?;
            let measured = d[1].atan2(d[0]) - PI * 0.5;
            let value = self.angle_value(definition).unwrap_or(measured);
            let (reverse, flip) = [(false, false), (false, true), (true, false), (true, true)]
                .into_iter()
                .find(|(reverse, flip)| {
                    let turn = if *reverse { -measured } else { measured };
                    wrap(turn - value - if *flip { PI } else { 0.0 }).abs() <= ANGLE_TOLERANCE
                })?;
            let a = self.builder.own_point([0.0, 0.0]);
            let b = self.builder.own_point([0.0, 1.0]);
            let axis = self.carrier(a, b)?;
            self.builder.fix_curve(axis)?;
            let (first, second) = if reverse {
                (l.index, axis)
            } else {
                (axis, l.index)
            };
            return Some(self.dimension(
                json!({"type": "angle", "a": first, "b": second, "value": value, "flip": flip}),
                definition,
            ));
        }
        if definition.get("axis")?.as_str()? != "horizontal" {
            return None;
        }
        let l = self.curve(definition.get("entity")?, &["line"])?;
        let d = self.direction(l)?;
        let measured = d[1].atan2(d[0]);
        let value = match self.angle_value(definition) {
            Some(value) if wrap(measured - value).abs() <= ANGLE_TOLERANCE => value,
            Some(_) => return None,
            None => measured,
        };
        Some(self.dimension(
            json!({"type": "line_angle", "l": l.index, "value": value}),
            definition,
        ))
    }

    /// A line's stored start-to-end direction.
    fn direction(&self, line: &Curve) -> Option<P2> {
        let (a, b) = (self.position(line.start?)?, self.position(line.end?)?);
        Some([b[0] - a[0], b[1] - a[1]])
    }

    /// The evaluated angle (radians) of a dimension's parameter.
    fn angle_value(&self, definition: &Value) -> Option<f64> {
        let id = definition.get("parameter")?.as_str()?;
        let value = self.ctx.row("parameters", id)?.get("value")?;
        (value.get("kind")?.as_str()? == "angle")
            .then(|| number(value.get("value")))
            .flatten()
    }

    fn builder_curve(&self, index: usize) -> Option<&Value> {
        self.builder.curve_json(index)
    }
}

/// How far apart (radians) two angles may be and still be the same.
const ANGLE_TOLERANCE: f64 = 1e-6;

/// The signed angle from `a` to `b`, counter-clockwise.
fn signed_angle(a: P2, b: P2) -> Option<f64> {
    let cross = a[0] * b[1] - a[1] * b[0];
    let dot = a[0] * b[0] + a[1] * b[1];
    (cross != 0.0 || dot != 0.0).then(|| cross.atan2(dot))
}

/// An angle wrapped into (-π, π].
fn wrap(angle: f64) -> f64 {
    let wrapped = angle.rem_euclid(TAU);
    if wrapped > PI {
        wrapped - TAU
    } else {
        wrapped
    }
}

/// Coincidences joining every point to the first, leaving out points that are already one.
fn coincident(points: &[usize]) -> Vec<Value> {
    let Some((first, rest)) = points.split_first() else {
        return Vec::new();
    };
    rest.iter()
        .filter(|p| *p != first)
        .map(|q| json!({"type": "coincident", "p": first, "q": q}))
        .collect()
}

/// The distance from `p` to the line through `a` and `b`.
fn point_line(p: P2, a: P2, b: P2) -> Option<f64> {
    let length = distance(a, b);
    (length > 0.0)
        .then(|| ((b[0] - a[0]) * (a[1] - p[1]) - (a[0] - p[0]) * (b[1] - a[1])).abs() / length)
}

// Test helpers outside `#[test]` functions are not covered by clippy.toml's test allowances.
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    const SKETCH: &str = "f3d:model:sketch#1";

    fn entity(key: &str, geometry: Value) -> Value {
        json!({"id": key, "sketch": SKETCH, "geometry": geometry})
    }

    fn line(key: &str, a: P2, b: P2) -> Value {
        entity(
            key,
            json!({"kind": "line",
            "start": {"u": a[0], "v": a[1]}, "end": {"u": b[0], "v": b[1]}}),
        )
    }

    /// A 40 × 30 rectangle (`b`, `r`, `t`, `l`), a circle `c` of radius 5 at (20, 15), a
    /// diagonal construction line `d` and a point `m` at the middle of the bottom edge.
    fn entities() -> Vec<Value> {
        vec![
            line("b", [0.0, 0.0], [40.0, 0.0]),
            line("r", [40.0, 0.0], [40.0, 30.0]),
            line("t", [40.0, 30.0], [0.0, 30.0]),
            line("l", [0.0, 30.0], [0.0, 0.0]),
            entity(
                "c",
                json!({"kind": "circle", "center": {"u": 20.0, "v": 15.0}, "radius": 5.0}),
            ),
            line("d", [0.0, 0.0], [40.0, 30.0]),
            entity(
                "m",
                json!({"kind": "point", "position": {"u": 20.0, "v": 0.0}}),
            ),
        ]
    }

    fn run(constraints: Vec<Value>) -> (Vec<Value>, Report) {
        let rows: Vec<Value> = constraints
            .into_iter()
            .enumerate()
            .map(|(i, definition)| {
                json!({"id": format!("f3d:model:sketch-constraint#{i}"), "sketch": SKETCH,
                       "definition": definition})
            })
            .collect();
        let model = json!({"sketch_constraints": rows});
        let mut ctx = Context::new(&model);
        ctx.params
            .insert("f3d:model:parameter#1".to_owned(), "width".to_owned());
        let mut builder = Builder::default();
        let mut report = Report::default();
        for e in entities() {
            builder.entity(&e, &mut report).unwrap();
        }
        let out = map(SKETCH, "sketch \"Base\"", &ctx, &mut builder, &mut report);
        (out, report)
    }

    fn locus(kind: &str, entity: &str) -> Value {
        json!({"kind": kind, "entity": entity})
    }

    #[test]
    fn geometric_relations_map_onto_curves_and_points() {
        let (out, report) = run(vec![
            json!({"kind": "horizontal", "entity": "b"}),
            json!({"kind": "vertical", "entity": "r"}),
            json!({"kind": "parallel", "first": "b", "second": "t"}),
            json!({"kind": "perpendicular", "first": "b", "second": "l"}),
            json!({"kind": "midpoint", "point": locus("entity", "m"), "entity": "b"}),
            json!({"kind": "point_on_object", "point": locus("center", "c"), "entity": "d"}),
            json!({"kind": "fixed", "entity": "m"}),
            json!({"kind": "coincident_loci", "loci": [locus("end", "b"), locus("start", "r")]}),
        ]);
        assert!(report.losses().is_empty(), "{:?}", report.losses());
        let types: Vec<&str> = out.iter().map(|c| c["type"].as_str().unwrap()).collect();
        assert_eq!(
            types,
            [
                "horizontal",
                "vertical",
                "parallel",
                "perpendicular",
                "midpoint",
                "point_on_curve",
                "fix"
            ]
        );
        // The shared corner is one point already: its coincidence adds nothing.
        assert_eq!(out[0], json!({"id": "k1", "type": "horizontal", "l": 0}));
        assert_eq!(out[5]["c"], 5);
    }

    #[test]
    fn entity_point_curve_coincidences_and_distances_map() {
        let (out, report) = run(vec![
            json!({"kind": "coincident", "entities": ["m", "b"]}),
            json!({"kind": "coincident", "entities": ["b", "m"]}),
            json!({"kind": "distance", "entities": ["m", "t"], "parameter": "f3d:model:parameter#1"}),
            json!({"kind": "distance", "entities": ["t", "m"], "parameter": "f3d:model:parameter#1"}),
            json!({"kind": "distance", "entities": ["b", "t"], "parameter": "f3d:model:parameter#1"}),
        ]);
        assert!(report.losses().is_empty(), "{:?}", report.losses());
        assert_eq!(out[0]["type"], "point_on_curve");
        assert_eq!(out[1]["type"], "point_on_curve");
        for i in [2, 3, 5] {
            assert_eq!(out[i]["type"], "point_line_distance");
            assert_eq!(out[i]["value"], 30.0);
            assert_eq!(out[i]["param"], "width");
        }
        assert_eq!(out[4]["type"], "parallel");
    }

    #[test]
    fn construction_and_reference_entities_keep_driven_dimensions() {
        let model = json!({"sketch_constraints": [{"sketch": SKETCH, "driving": false,
            "definition": {"kind": "distance", "entities": ["m", "axis"], "parameter": "p"}}]});
        let mut ctx = Context::new(&model);
        ctx.params.insert("p".to_owned(), "Length".to_owned());
        let mut builder = Builder::default();
        let mut report = Report::default();
        for e in entities() {
            builder.entity(&e, &mut report).unwrap();
        }
        builder
            .entity(
                &entity(
                    "axis",
                    json!({"kind": "reference_line",
            "origin": {"u": 0.0, "v": 5.0}, "direction": {"u": 1.0, "v": 0.0}}),
                ),
                &mut report,
            )
            .unwrap();
        let out = map(SKETCH, "sketch", &ctx, &mut builder, &mut report);
        assert!(report.losses().is_empty());
        assert_eq!(out[0]["type"], "point_line_distance");
        assert_eq!(out[0]["driven"], true);
        assert_eq!(out[0]["param"], "Length");
        assert_eq!(out[0]["value"], 5.0);
    }

    #[test]
    fn finite_point_line_distance_uses_the_endpoint_only_when_its_value_matches() {
        let model = json!({"parameters": [{"id": "p", "value": {"kind": "length", "value": 2.0}}]});
        let mut ctx = Context::new(&model);
        ctx.params.insert("p".to_owned(), "Length".to_owned());
        let mut builder = Builder::default();
        let mut report = Report::default();
        builder
            .entity(&line("segment", [0.0, 0.0], [10.0, 0.0]), &mut report)
            .unwrap();
        builder
            .entity(
                &entity(
                    "outside",
                    json!({"kind": "point", "position": {"u": 12.0, "v": 0.0}}),
                ),
                &mut report,
            )
            .unwrap();
        let mut mapper = Mapper {
            ctx: &ctx,
            builder: &mut builder,
        };
        let d = json!({"kind": "distance", "entities": ["outside", "segment"], "parameter": "p"});
        let out = mapper.map(&d).unwrap();
        assert_eq!(out[0]["type"], "distance");
        assert_eq!(out[0]["value"], 2.0);
        assert_eq!(out[0]["q"], 2);
        assert_eq!(out[0]["param"], "Length");
        let mut reversed = d.clone();
        reversed["entities"] = json!(["segment", "outside"]);
        assert_eq!(mapper.map(&reversed).unwrap(), out);
        let mismatch =
            json!({"parameters": [{"id": "p", "value": {"kind": "length", "value": 3.0}}]});
        let ctx = Context::new(&mismatch);
        assert!(Mapper {
            ctx: &ctx,
            builder: &mut builder
        }
        .map(&d)
        .is_err());
    }

    #[test]
    fn reference_axes_have_no_artificial_finite_end_distance() {
        let model = json!({"parameters": [{"id": "p", "value": {"kind": "length", "value": 2.0}}]});
        let ctx = Context::new(&model);
        let mut builder = Builder::default();
        let mut report = Report::default();
        builder.entity(&entity("axis", json!({"kind": "reference_line", "origin": {"u": 0.0, "v": 0.0}, "direction": {"u": 1.0, "v": 0.0}})), &mut report).unwrap();
        builder
            .entity(
                &entity(
                    "p",
                    json!({"kind": "point", "position": {"u": 3.0, "v": 0.0}}),
                ),
                &mut report,
            )
            .unwrap();
        assert!(Mapper {
            ctx: &ctx,
            builder: &mut builder
        }
        .map(&json!({"kind": "distance", "entities": ["p", "axis"], "parameter": "p"}))
        .is_err());
    }

    #[test]
    fn disjoint_parallel_segments_can_measure_their_unique_nearest_ends() {
        let model = json!({"parameters": [{"id": "p", "value": {"kind": "length", "value": 5.0_f64.sqrt()}}]});
        let ctx = Context::new(&model);
        let mut builder = Builder::default();
        let mut report = Report::default();
        builder
            .entity(&line("a", [0.0, 0.0], [1.0, 0.0]), &mut report)
            .unwrap();
        builder
            .entity(&line("b", [2.0, 2.0], [3.0, 2.0]), &mut report)
            .unwrap();
        let out = Mapper {
            ctx: &ctx,
            builder: &mut builder,
        }
        .map(&json!({"kind": "distance", "entities": ["a", "b"], "parameter": "p"}))
        .unwrap();
        assert_eq!(out[0]["type"], "parallel");
        assert_eq!(out[1]["type"], "distance");
        assert_eq!(out[1]["p"], 2);
        assert_eq!(out[1]["q"], 3);
        assert_eq!(out[1]["value"], 5.0_f64.sqrt());
    }

    #[test]
    fn nonparallel_entity_lines_have_no_spacing_dimension() {
        let (out, report) = run(vec![
            json!({"kind": "distance", "entities": ["b", "r"], "parameter": "p"}),
        ]);
        assert!(out.is_empty());
        assert!(report.losses()[0]
            .detail
            .contains("nonparallel line spacing"));
    }

    #[test]
    fn dimensions_measure_the_geometry_and_link_their_parameter() {
        let (out, _) = run(vec![
            json!({"kind": "distance", "entities": ["b"], "parameter": "f3d:model:parameter#1"}),
            json!({"kind": "horizontal_distance", "first": locus("start", "b"),
                   "second": locus("center", "c"), "parameter": "f3d:model:parameter#9"}),
            json!({"kind": "distance_loci", "first": locus("center", "c"),
                   "second": locus("entity", "b"), "parameter": "f3d:model:parameter#9"}),
            json!({"kind": "diameter", "entity": "c", "parameter": "f3d:model:parameter#9"}),
        ]);
        assert_eq!(
            out[0],
            json!({"id": "k1", "type": "length", "l": 0, "value": 40.0, "param": "width"})
        );
        assert_eq!(out[1]["type"], "distance_x");
        assert_eq!(out[1]["value"], 20.0);
        assert!(
            out[1].get("param").is_none(),
            "an unmapped parameter is not linked"
        );
        assert_eq!(out[2]["type"], "point_line_distance");
        assert_eq!(out[2]["value"], 15.0);
        assert_eq!(
            out[3],
            json!({"id": "k4", "type": "diameter", "c": 4, "value": 10.0})
        );
    }

    fn run_with_params(constraints: Vec<Value>, parameters: Value) -> Vec<Value> {
        let rows: Vec<Value> = constraints
            .into_iter()
            .enumerate()
            .map(|(i, definition)| {
                json!({"id": format!("f3d:model:sketch-constraint#{i}"), "sketch": SKETCH,
                       "definition": definition})
            })
            .collect();
        let model = json!({"sketch_constraints": rows, "parameters": parameters});
        let mut ctx = Context::new(&model);
        ctx.params.insert("a".to_owned(), "theta".to_owned());
        let mut builder = Builder::default();
        let mut report = Report::default();
        for e in entities() {
            builder.entity(&e, &mut report).unwrap();
        }
        map(SKETCH, "sketch", &ctx, &mut builder, &mut report)
    }

    fn angle_param(degrees: f64) -> Value {
        json!([{"id": "a", "name": "theta", "expression": format!("{degrees} deg"),
                "value": {"kind": "angle", "value": degrees.to_radians()}}])
    }

    #[test]
    fn angles_keep_the_parameter_driving_the_same_angle() {
        // The diagonal `d` runs at atan2(30, 40) ≈ 36.87° from the bottom edge `b`.
        let theta = 30.0_f64.atan2(40.0).to_degrees();
        let shown = |degrees: f64| {
            run_with_params(
                vec![json!({"kind": "angle", "first": "b", "second": "d", "parameter": "a"})],
                angle_param(degrees),
            )
        };
        let acute = shown(theta);
        assert_eq!(acute[0]["a"], 0);
        assert_eq!(acute[0]["b"], 5);
        assert!(acute[0].get("flip").is_none());
        assert_eq!(acute[0]["param"], "theta");
        // The obtuse angle between the same lines is the reversed direction of one of them.
        let obtuse = shown(180.0 - theta);
        assert_eq!(
            (obtuse[0]["a"].clone(), obtuse[0]["flip"].clone()),
            (json!(5), json!(true))
        );
        // An angle the geometry does not have is not written.
        assert!(shown(50.0).is_empty());
    }

    #[test]
    fn angles_to_the_horizontal_axis_are_line_angles() {
        let to_axis = |degrees: f64, axis: &str| {
            run_with_params(
                vec![json!({"kind": "angle_to_axis", "entity": "d", "axis": axis,
                            "parameter": "a"})],
                angle_param(degrees),
            )
        };
        let theta = 30.0_f64.atan2(40.0).to_degrees();
        let out = to_axis(theta, "horizontal");
        assert_eq!(out[0]["type"], "line_angle");
        assert_eq!(out[0]["l"], 5);
        assert!(to_axis(-theta, "horizontal").is_empty());
        assert_eq!(to_axis(90.0 - theta, "vertical")[0]["type"], "angle");
    }

    #[test]
    fn unmapped_constraints_are_dropped_and_counted() {
        let (out, report) = run(vec![
            json!({"kind": "native", "native_kind": "Linear Dimension-1", "entities": []}),
            json!({"kind": "native", "native_kind": "Linear Dimension-1", "entities": []}),
            json!({"kind": "angle", "first": "b", "second": "c", "parameter": "x"}),
            json!({"kind": "horizontal", "entity": "c"}),
            json!({"kind": "vertical", "entity": "l"}),
        ]);
        assert_eq!(out.len(), 1);
        let loss = report.losses().first().unwrap();
        assert_eq!(loss.severity, Severity::Degraded);
        assert_eq!(loss.subject, "sketch \"Base\"");
        assert!(loss.detail.starts_with("4 of 5 constraints not carried ("));
        assert!(loss.detail.contains(
            "native Linear Dimension-1: its native operands have no resolved neutral relation ×2"
        ));
        assert!(loss.detail.ends_with("; its geometry is exact"));
    }

    #[test]
    fn reference_dimensions_stay_reference() {
        let rows = vec![json!({"kind": "radius", "entity": "c", "parameter": "p"})];
        let (out, _) = run(rows);
        assert!(out[0].get("driven").is_none());
    }
}
