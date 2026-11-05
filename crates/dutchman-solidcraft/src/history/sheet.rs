//! Sheet-metal base flanges and edge flanges on proven input-state edges. Hems are refused
//! with the reason SolidCraft cannot express them.

use serde_json::{json, Value};

use super::profile::profile;
use super::sketch::{dot, number, v3, Shape, V3};
use super::{name, Context};

/// Maps a base flange whose operation is `definition`.
pub(super) fn base_flange(
    feature: &Value,
    definition: &Value,
    ctx: &mut Context,
) -> Result<Value, String> {
    let (sketch, selection) = profile(definition.get("profile"), ctx)?;
    let sketch = ctx
        .sketches
        .get(&sketch)
        .map(|mapped| mapped.feature)
        .ok_or("its sketch is not an earlier mapped sketch")?;
    let thickness = number(definition.get("thickness"))
        .filter(|t| *t > 0.0)
        .ok_or("its thickness is not given")?;
    // SolidCraft grows the sheet along the sketch's normal, or against it when flipped.
    let flip = match definition.get("side").and_then(Value::as_str) {
        None | Some("forward") => false,
        Some("reverse") => true,
        Some(_) => {
            return Err(
                "a base flange thick on both sides of its sketch is not mapped yet".to_owned(),
            )
        }
    };
    let rule = ctx.sheet_rule(thickness);
    let id = ctx.id();
    ctx.builds = true;
    let mut out = json!({
        "id": id,
        "name": name(feature).map(str::to_owned).unwrap_or_else(|| format!("BaseFlange{id}")),
        "type": "sheet_base",
        "suppressed": false,
        "sketch": sketch,
        "profiles": selection,
        "rule": rule,
    });
    if let (true, Some(fields)) = (flip, out.as_object_mut()) {
        fields.insert("flip".to_owned(), Value::Bool(true));
    }
    Ok(out)
}

/// A flange on a proven perimeter edge of the sole, unchanged rectangular base sheet.
/// Selection reads only this feature's input state; the final B-rep is never consulted.
pub(super) fn edge_flange(
    feature: &Value,
    definition: &Value,
    ctx: &mut Context,
    preceding: &[Value],
) -> Result<Value, String> {
    let selection = definition
        .get("edges")
        .ok_or("its edge selection is missing")?;
    if selection.get("kind").and_then(Value::as_str) != Some("historical") {
        return Err("its edge selection needs this feature's input edge geometry".to_owned());
    }
    let state = selection
        .pointer("/value/state")
        .and_then(Value::as_str)
        .and_then(|id| ctx.row("feature_input_topologies", id))
        .filter(|state| state.get("input_of") == feature.get("id"))
        .ok_or("its selected input topology does not belong to this feature")?;
    let selected = selection
        .pointer("/value/edges")
        .and_then(Value::as_array)
        .filter(|edges| !edges.is_empty() && edges.len() <= 4)
        .ok_or("a flange needs one to four resolved input edges")?;
    if definition.pointer("/height/kind").and_then(Value::as_str) != Some("distance") {
        return Err("a flange height ending at an object is not supported yet".to_owned());
    }
    if definition.get("height_datum").and_then(Value::as_str) != Some("outer_faces") {
        return Err("a flange height measured from inner faces is not supported yet".to_owned());
    }
    if definition.pointer("/width/kind").and_then(Value::as_str) != Some("full_edge") {
        return Err("a flange spanning only part of its edge is not supported yet".to_owned());
    }
    let position = match definition.get("bend_position").and_then(Value::as_str) {
        Some("inside") => "inside",
        Some("outside") => "outside",
        _ => return Err("this flange bend position is not supported yet".to_owned()),
    };
    let height = number(definition.pointer("/height/value"))
        .filter(|v| *v > 0.0)
        .ok_or("its flange height is not given")?;
    let angle = number(definition.get("angle"))
        .filter(|v| *v > 0.0 && *v < std::f64::consts::PI)
        .ok_or("a flange angle outside zero to 180 degrees is not supported yet")?;
    let radius = number(definition.get("bend_radius"))
        .filter(|v| *v > 0.0)
        .ok_or("its flange bend radius is not given")?;
    let rebuilt = base_edges(ctx, preceding)?;
    let geometry = state
        .get("edge_geometry")
        .and_then(Value::as_array)
        .ok_or("its input topology has no edge geometry")?;
    let members = state
        .get("edges")
        .and_then(Value::as_array)
        .ok_or("its input topology has no edges")?;
    let mut picks = Vec::new();
    let mut taken = Vec::new();
    for edge in selected {
        if !members.contains(edge) {
            return Err("its selected edge is absent from its input topology".to_owned());
        }
        let mut rows = geometry.iter().filter(|row| row.get("edge") == Some(edge));
        let row = rows
            .next()
            .ok_or("its selected input edge has no geometry")?;
        if rows.next().is_some() {
            return Err("its selected input edge has ambiguous geometry".to_owned());
        }
        let point = v3(row.get("interior")).ok_or("its input edge midpoint is missing")?;
        let start = v3(row.get("start")).ok_or("its input edge start vertex is missing")?;
        let end = v3(row.get("end")).ok_or("its input edge end vertex is missing")?;
        let tangent = v3(row.get("tangent")).ok_or("its input edge tangent is missing")?;
        let matches: Vec<_> = rebuilt
            .iter()
            .enumerate()
            .filter(|(_, (a, b))| on_segment(point, *a, *b))
            .collect();
        let (index, (a, b)) = match matches.as_slice() {
            [only] => *only,
            _ => {
                return Err(
                    "its input edge midpoint does not identify exactly one rebuilt edge".to_owned(),
                )
            }
        };
        // The last four edges are the plate's thickness edges, which cannot grow a flange.
        if index >= 8 || taken.contains(&(index % 4)) {
            return Err("its input edges repeat a pick or select a thickness edge".to_owned());
        }
        let direction = difference(*b, *a);
        let length = dot(direction, direction).sqrt();
        if !((near(start, *a) && near(end, *b)) || (near(start, *b) && near(end, *a)))
            || (dot(tangent, direction).abs() / length - 1.0).abs() > 1e-9
        {
            return Err(
                "its input edge vertices or tangent differ from the rebuilt edge".to_owned(),
            );
        }
        taken.push(index % 4);
        picks.push(point);
    }
    let id = ctx.id();
    Ok(json!({
        "id": id,
        "name": name(feature).map(str::to_owned).unwrap_or_else(|| format!("Flange{id}")),
        "type": "sheet_flange", "suppressed": false,
        "edges": picks, "height": format!("{height} mm"),
        "angle": format!("{} deg", angle.to_degrees()),
        "radius": format!("{radius} mm"), "position": position,
    }))
}

/// Enumerates all twelve edges of the pre-feature body from the mapped base sketch.
/// Refuse other histories rather than assume their final edges survived earlier operations.
fn base_edges(ctx: &Context, preceding: &[Value]) -> Result<Vec<(V3, V3)>, String> {
    let mut solids = preceding
        .iter()
        .filter(|f| f.get("type").and_then(Value::as_str) != Some("sketch"));
    let base = solids
        .next()
        .filter(|f| f.get("type").and_then(Value::as_str) == Some("sheet_base"))
        .ok_or("an edge flange on a body that is not a base sheet is not supported yet")?;
    if solids.next().is_some() {
        return Err(
            "an edge flange on a base sheet that earlier features changed is not supported yet"
                .to_owned(),
        );
    }
    let reason =
        "an edge flange on a base sheet that is not one plain rectangle is not supported yet";
    if base.pointer("/profiles/type").and_then(Value::as_str) != Some("all") {
        return Err(reason.to_owned());
    }
    let sketch = ctx
        .sketches
        .values()
        .find(|sketch| Some(sketch.feature) == base.get("sketch").and_then(Value::as_u64))
        .ok_or(reason)?;
    let mapped = preceding
        .iter()
        .find(|f| f.get("id") == base.get("sketch"))
        .ok_or(reason)?;
    let curves = mapped
        .pointer("/sketch/curves")
        .and_then(Value::as_array)
        .ok_or(reason)?;
    if curves.len() != 4
        || curves
            .iter()
            .any(|c| c.get("construction").and_then(Value::as_bool) == Some(true))
    {
        return Err(reason.to_owned());
    }
    let mut lines = Vec::new();
    for shape in sketch.shapes.values() {
        let Shape::Curve(curve) = shape else {
            continue;
        };
        if curve.kind != "line" {
            return Err(reason.to_owned());
        }
        let a = sketch
            .points
            .get(curve.start.ok_or(reason)?)
            .ok_or(reason)?
            .1;
        let b = sketch.points.get(curve.end.ok_or(reason)?).ok_or(reason)?.1;
        lines.push((a, b));
    }
    if lines.len() != 4 {
        return Err(reason.to_owned());
    }
    let mut min = [f64::INFINITY; 2];
    let mut max = [f64::NEG_INFINITY; 2];
    for (a, b) in &lines {
        for (i, (lo, hi)) in min.iter_mut().zip(max.iter_mut()).enumerate() {
            for p in [a, b] {
                let v = p.get(i).copied().ok_or(reason)?;
                *lo = lo.min(v);
                *hi = hi.max(v);
            }
        }
    }
    let [x0, y0] = min;
    let [x1, y1] = max;
    if x1 - x0 < 1e-6 || y1 - y0 < 1e-6 {
        return Err(reason.to_owned());
    }
    let corners = [[x0, y0], [x1, y0], [x1, y1], [x0, y1]];
    let ring: Vec<_> = corners
        .iter()
        .copied()
        .zip(corners.iter().copied().cycle().skip(1))
        .collect();
    for (a, b) in &ring {
        if lines
            .iter()
            .filter(|(c, d)| (a == c && b == d) || (a == d && b == c))
            .count()
            != 1
        {
            return Err(reason.to_owned());
        }
    }
    let rule = base.get("rule").and_then(Value::as_str).ok_or(reason)?;
    let thickness = ctx
        .sheet_rules
        .iter()
        .find(|(_, name)| name == rule)
        .map(|(thickness, _)| *thickness)
        .ok_or(reason)?;
    let sign = if base.get("flip").and_then(Value::as_bool) == Some(true) {
        -1.0
    } else {
        1.0
    };
    let shift = |p: V3| -> V3 {
        let [x, y, z] = p;
        let [nx, ny, nz] = sketch.normal;
        [
            x + sign * thickness * nx,
            y + sign * thickness * ny,
            z + sign * thickness * nz,
        ]
    };
    let mut edges = Vec::new();
    for top in [false, true] {
        for (a, b) in &ring {
            let (a, b) = (sketch.world(*a), sketch.world(*b));
            edges.push(if top { (shift(a), shift(b)) } else { (a, b) });
        }
    }
    for p in corners {
        let p = sketch.world(p);
        edges.push((p, shift(p)));
    }
    Ok(edges)
}

fn difference(a: V3, b: V3) -> V3 {
    let [ax, ay, az] = a;
    let [bx, by, bz] = b;
    [ax - bx, ay - by, az - bz]
}

fn near(a: V3, b: V3) -> bool {
    let d = difference(a, b);
    dot(d, d) < 1e-12
}

fn on_segment(p: V3, a: V3, b: V3) -> bool {
    let d = difference(b, a);
    let t = dot(difference(p, a), d) / dot(d, d);
    let [x, y, z] = a;
    let [dx, dy, dz] = d;
    (0.0..=1.0).contains(&t) && near(p, [x + t * dx, y + t * dy, z + t * dz])
}

/// CADIR hems retain developed lengths; SolidCraft accepts an outer-face length.
pub(super) fn hem_refusal(definition: &Value) -> String {
    let reason = match definition.get("direction").and_then(Value::as_str) {
        Some("unresolved") => "a hem with unresolved fold direction is not supported yet",
        _ => match definition.pointer("/form/kind").and_then(Value::as_str) {
            Some("flat" | "open") => {
                "a hem specified by developed length is not supported yet; SolidCraft requires length to the outer face"
            }
            Some("gap_length") => "an unresolved flat-or-open hem is not supported yet",
            Some("rolled") => "a rolled hem is not supported yet",
            Some("teardrop") => "a teardrop hem is not supported yet",
            _ => "this hem form is not supported yet",
        },
    };
    reason.to_owned()
}

/// An attached plate is a flat plate joined to a sheet edge by its paired bend. SolidCraft's base
/// flange always makes a separate sheet body and nothing else joins a plate to an existing sheet,
/// so the plate is refused with that reason, or with the missing sheet when there is none.
pub(super) fn attached_plate_refusal(definition: &Value) -> String {
    if definition.get("attachment").is_none() {
        return "an attached plate has no selected sheet edge to attach to".to_owned();
    }
    "an attached plate joined to an existing sheet by a bend is not supported yet; SolidCraft \
     base flanges make separate sheet bodies and cannot join a plate to a sheet edge"
        .to_owned()
}

/// A SolidCraft sheet rule of `thickness` mm, named after it, with SolidCraft's own defaults
/// for everything else.
pub(super) fn rule(name: &str, thickness: f64) -> Value {
    json!({
        "name": name,
        "thickness": format!("{thickness} mm"),
        "bend_radius": "Thickness",
        "k_factor": "0.44",
        "relief_width": "Thickness",
        "relief_depth": "Thickness * 0.5",
        "corner_relief": "Thickness * 4.0",
        "corner_relief_shape": "notch",
        "hem_gap": "0.01 mm",
        "gap": "Thickness",
    })
}
