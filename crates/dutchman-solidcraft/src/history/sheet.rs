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
    let (state, selected) = selected_edges(feature, selection, ctx)?;
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
    let base = base_edges(ctx, preceding)?;
    let picks: Vec<V3> = prove_picks(state, selected, &base.edges)?
        .into_iter()
        .map(|pick| pick.point)
        .collect();
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

/// One proven input edge of the rebuilt base sheet.
struct Pick {
    /// The input edge's midpoint.
    point: V3,
    /// Its index among the rebuilt edges: 0 to 3 on the sketch face, 4 to 7 on the far face.
    index: usize,
    /// The rebuilt edge's end points.
    ends: (V3, V3),
}

/// The input topology row and the selected edge ids of a historical selection.
fn selected_edges<'a, 'b>(
    feature: &Value,
    selection: &'b Value,
    ctx: &Context<'a>,
) -> Result<(&'a Value, &'b [Value]), String> {
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
    Ok((state, selected))
}

/// Proves each selected input edge is exactly one rebuilt edge of the base sheet.
fn prove_picks(
    state: &Value,
    selected: &[Value],
    rebuilt: &[(V3, V3)],
) -> Result<Vec<Pick>, String> {
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
        picks.push(Pick {
            point,
            index,
            ends: (*a, *b),
        });
    }
    Ok(picks)
}

/// The rebuilt base sheet: its twelve edges and the numbers a flange on it needs.
struct Base {
    edges: Vec<(V3, V3)>,
    thickness: f64,
    /// Unit direction the sheet grows from its sketch face.
    growth: V3,
    /// A point on the sketch face.
    origin: V3,
    /// The centre of the sketch face.
    center: V3,
}

/// Enumerates all twelve edges of the pre-feature body from the mapped base sketch.
/// Refuse other histories rather than assume their final edges survived earlier operations.
fn base_edges(ctx: &Context, preceding: &[Value]) -> Result<Base, String> {
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
    let center = sketch.world([(x0 + x1) / 2.0, (y0 + y1) / 2.0]);
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
    let [nx, ny, nz] = sketch.normal;
    Ok(Base {
        edges,
        thickness,
        growth: [sign * nx, sign * ny, sign * nz],
        origin: sketch.origin,
        center,
    })
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

/// A plate attached to a sheet edge together with its paired bend, written as one 90 degree
/// edge flange. SolidCraft has no feature that joins a separate plate, so only a rectangular
/// plate that spans the whole edge, stands square to the sheet and starts where the bend ends
/// is the same shape as an edge flange. The bend feature is consumed with its plate.
pub(super) fn attached_plate(
    feature: &Value,
    definition: &Value,
    ctx: &mut Context,
    preceding: &[Value],
) -> Result<Value, String> {
    const TOL: f64 = 1e-4;
    let selection = definition
        .get("attachment")
        .ok_or("an attached plate has no selected sheet edge to attach to")?;
    let (state, selected) = selected_edges(feature, selection, ctx)?;
    let plate_id = feature.get("id").and_then(Value::as_str);
    let bend = ctx
        .model
        .get("features")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .find(|f| {
            let op = super::operation(f);
            op.get("definition").and_then(Value::as_str) == Some("sheet_metal_bend")
                && op.get("plate").and_then(Value::as_str) == plate_id
        })
        .ok_or("its paired bend is missing, so the plate has no joining region")?;
    let bend_id = bend
        .get("id")
        .and_then(Value::as_str)
        .ok_or("its paired bend has no id")?
        .to_owned();
    let radius = number(super::operation(bend).get("inside_radius"))
        .filter(|r| *r > 0.0)
        .ok_or("its bend radius is not given")?;
    let thickness = number(definition.get("thickness"))
        .filter(|t| *t > 0.0)
        .ok_or("its thickness is not given")?;
    let base = base_edges(ctx, preceding)?;
    if (thickness - base.thickness).abs() > TOL {
        return Err("its thickness differs from the base sheet's".to_owned());
    }
    let picks = prove_picks(state, selected, &base.edges)?;
    let [pick] = picks.as_slice() else {
        return Err("a plate attached along several edges is not supported yet".to_owned());
    };
    let (sketch_id, _) = profile(definition.get("profile"), ctx)?;
    let sketch = ctx
        .sketches
        .get(&sketch_id)
        .ok_or("its sketch is not an earlier mapped sketch")?;
    let corners = rectangle(sketch)
        .ok_or("an attached plate whose profile is not one rectangle is not supported yet")?;
    let (a, b) = pick.ends;
    let along = difference(b, a);
    let length = dot(along, along).sqrt();
    let e = scale(along, 1.0 / length);
    let growth = base.growth;
    let normal = sketch.normal;
    if dot(normal, e).abs() > 1e-6 || dot(normal, growth).abs() > 1e-6 {
        return Err("a plate not square to the base sheet is not supported yet".to_owned());
    }
    // Outward from the base across the picked edge, in the base plane.
    let out = difference(pick.point, base.center);
    let out = difference(out, scale(e, dot(out, e)));
    let out = difference(out, scale(growth, dot(out, growth)));
    let reach = dot(out, out).sqrt();
    if reach < TOL {
        return Err("its picked edge has no outward direction".to_owned());
    }
    let out = scale(out, 1.0 / reach);
    let offset = dot(difference(sketch.origin, a), out);
    if offset.abs() > TOL && (offset + thickness).abs() > TOL {
        return Err(
            "a plate not flush with the end face of the sheet is not supported yet".to_owned(),
        );
    }
    // The flange rises from the picked face: up from the far face, down from the sketch face.
    let top = pick.index >= 4;
    let rise = if top { growth } else { scale(growth, -1.0) };
    let outer = if top {
        base.origin
    } else {
        let [x, y, z] = base.origin;
        let [gx, gy, gz] = growth;
        [x + thickness * gx, y + thickness * gy, z + thickness * gz]
    };
    let mut near = f64::INFINITY;
    let mut far = f64::NEG_INFINITY;
    for corner in corners {
        let at = dot(difference(corner, a), e);
        if at.abs() > TOL && (at - length).abs() > TOL {
            return Err(
                "a plate that does not span the whole edge is not supported yet".to_owned(),
            );
        }
        let up = dot(difference(corner, outer), rise);
        near = near.min(up);
        far = far.max(up);
    }
    if (near - (radius + thickness)).abs() > TOL {
        return Err(
            "a plate that does not start where its 90 degree bend ends is not supported yet"
                .to_owned(),
        );
    }
    ctx.consumed_bends.insert(bend_id);
    let id = ctx.id();
    Ok(json!({
        "id": id,
        "name": name(feature).map(str::to_owned).unwrap_or_else(|| format!("Flange{id}")),
        "type": "sheet_flange", "suppressed": false,
        "edges": [pick.point], "height": format!("{far} mm"),
        "angle": "90 deg", "radius": format!("{radius} mm"), "position": "inside",
    }))
}

/// The four world-space corners of a sketch that is exactly one rectangle of four lines.
fn rectangle(sketch: &super::sketch::Mapped) -> Option<[V3; 4]> {
    let mut corners: Vec<V3> = Vec::new();
    let mut lines = Vec::new();
    for shape in sketch.shapes.values() {
        let Shape::Curve(curve) = shape else {
            continue;
        };
        if curve.kind != "line" {
            return None;
        }
        let a = sketch.world(sketch.points.get(curve.start?)?.1);
        let b = sketch.world(sketch.points.get(curve.end?)?.1);
        for p in [a, b] {
            if !corners.iter().any(|c| near(*c, p)) {
                corners.push(p);
            }
        }
        lines.push((a, b));
    }
    let [c0, c1, c2, c3] = <[V3; 4]>::try_from(corners).ok()?;
    if lines.len() != 4 {
        return None;
    }
    // The corner farthest from the first is its diagonal; the other two are its neighbours.
    let mut rest = [c1, c2, c3];
    rest.sort_by(|p, q| {
        let (dp, dq) = (difference(*p, c0), difference(*q, c0));
        dot(dp, dp).total_cmp(&dot(dq, dq))
    });
    let [n1, n2, diagonal] = rest;
    let (s1, s2) = (difference(n1, c0), difference(n2, c0));
    let scale_of = dot(s1, s1).max(dot(s2, s2));
    if scale_of < 1e-12
        || dot(s1, s2).abs() > 1e-9 * scale_of
        || !near(
            diagonal,
            [
                c0[0] + s1[0] + s2[0],
                c0[1] + s1[1] + s2[1],
                c0[2] + s1[2] + s2[2],
            ],
        )
    {
        return None;
    }
    let (l1, l2) = (dot(s1, s1).sqrt(), dot(s2, s2).sqrt());
    let sides = lines.iter().all(|(a, b)| {
        let d = difference(*b, *a);
        let len = dot(d, d).sqrt();
        (len - l1).abs() < 1e-6 || (len - l2).abs() < 1e-6
    });
    sides.then_some([c0, n1, diagonal, n2])
}

fn scale(a: V3, k: f64) -> V3 {
    let [x, y, z] = a;
    [x * k, y * k, z * k]
}

/// The reason a bend not consumed by its plate cannot be mapped.
pub(super) fn bend_refusal() -> String {
    "a bend whose attached plate was not written with it as an edge flange is not supported yet"
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
