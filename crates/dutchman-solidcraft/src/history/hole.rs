//! Holes at sketch locations or resolved spatial placements, with supported entry
//! forms, drill points, and terminations proven against earlier extrusion caps.

use std::collections::BTreeSet;

use dutchman_core::{Report, Severity};
use serde_json::{json, Value};

use super::sketch::{dot, number, unit, v3, Mapped, Shape, V3};
use super::{describe, name, Context};

/// Maps a hole feature whose operation is `definition`.
pub(crate) fn map(
    feature: &Value,
    definition: &Value,
    ctx: &mut Context,
    report: &mut Report,
) -> Result<Value, String> {
    let (position, direction, points) = placement(feature, definition, ctx)?;

    let shape = definition.get("shape").unwrap_or(&Value::Null);
    if shape.get("exit_kind").is_some_and(|v| !v.is_null()) {
        return Err("a hole with an exit treatment is not mapped yet".to_owned());
    }
    let construction = shape.get("construction").unwrap_or(&Value::Null);
    if construction.get("construction").and_then(Value::as_str) == Some("native_thread") {
        return Err("a hole with native thread geometry is not mapped yet".to_owned());
    }
    let form = construction.get("kind").unwrap_or(&Value::Null);
    // Newer SolidCraft drills an entry-recess hole to a point as well.
    let recess_tip = crate::VersionGate::EntryRecessTip.supported(ctx.target_version);
    if definition.pointer("/bottom/kind").and_then(Value::as_str) == Some("angled")
        && matches!(
            form.get("kind").and_then(Value::as_str),
            Some("counterbore" | "countersink" | "chamfer")
        )
        && !recess_tip
    {
        return Err("SolidCraft holes take a drill point only on a plain drilled hole".to_owned());
    }
    let diameter = number(shape.get("diameter")).ok_or("its diameter is not given")?;
    let (thread, thread_note) = cosmetic_thread(construction.get("specification"), diameter)?;
    let mut hole = match (
        construction.get("construction").and_then(Value::as_str),
        form.get("kind").and_then(Value::as_str),
    ) {
        (Some("form"), Some("simple")) => json!({"type": "simple"}),
        (Some("form"), Some("simple_drilled")) => {
            let angle = number(form.get("drill_point_angle"))
                .filter(|a| *a > 0.0 && *a < std::f64::consts::PI)
                .ok_or("its drill point's angle is not given")?;
            json!({"type": "drilled", "tip_angle": format!("{}", angle.to_degrees())})
        }
        (Some("form"), Some("counterbore" | "countersink" | "chamfer")) => {
            let entry = number(form.get("diameter"))
                .filter(|d| *d > diameter)
                .ok_or("its entry diameter must be greater than its bore diameter")?;
            if form.get("kind").and_then(Value::as_str) == Some("counterbore") {
                let depth = number(form.get("depth"))
                    .filter(|d| *d > 0.0)
                    .ok_or("its counterbore depth is not given")?;
                json!({"type": "counterbore", "cb_diameter": format!("{entry} mm"),
                    "cb_depth": format!("{depth} mm")})
            } else {
                let angle = number(form.get("angle"))
                    .filter(|a| *a > 0.0 && *a < std::f64::consts::PI)
                    .ok_or("its countersink angle is not given")?;
                json!({"type": "countersink", "cs_diameter": format!("{entry} mm"),
                    "cs_angle": format!("{}", angle.to_degrees())})
            }
        }
        (Some("form"), Some("counterbore_drilled")) => {
            return Err(
                "SolidCraft holes take a drill point only on a plain drilled hole".to_owned(),
            )
        }
        (Some("form"), Some("counterdrill")) => {
            return Err(
                "a counterdrill with a conical entry and cylindrical recess is not mapped yet"
                    .to_owned(),
            )
        }
        (Some("form"), Some("partial_counterbore")) => {
            return Err("its counterbore diameter or depth is not resolved".to_owned())
        }
        (Some("form"), Some("partial_countersink")) => {
            return Err("its countersink diameter or angle is not resolved".to_owned())
        }
        (Some("form"), Some("unresolved")) => {
            return Err("its hole entry form or dimensions are not resolved".to_owned())
        }
        (Some("form"), Some(kind)) => return Err(format!("a {kind} hole is not mapped yet")),
        _ => return Err("a hole of this construction is not mapped yet".to_owned()),
    };

    // Newer SolidCraft applies entry-recess tip angles. These source forms have
    // flat bottoms; 180 degrees makes that explicit on those releases.
    if crate::VersionGate::EntryRecessTip.supported(ctx.target_version)
        && matches!(
            hole.get("type").and_then(Value::as_str),
            Some("counterbore" | "countersink")
        )
    {
        if let Some(fields) = hole.as_object_mut() {
            fields.insert("tip_angle".to_owned(), json!("180"));
        }
    }

    let mut depth = None;
    let mut to = None;
    let extent = definition.get("extent").unwrap_or(&Value::Null);
    match extent.get("kind").and_then(Value::as_str) {
        None | Some("through_all") => {}
        Some("blind") => {
            let length = number(extent.get("length"))
                .filter(|length| *length > 0.0)
                .ok_or("its depth is not given")?;
            let bottom = definition.get("bottom").unwrap_or(&Value::Null);
            match bottom.get("kind").and_then(Value::as_str) {
                None => depth = Some(length),
                Some("flat") => {
                    if form.get("kind").and_then(Value::as_str) == Some("simple_drilled") {
                        return Err(
                            "its flat bottom conflicts with its drill-point form".to_owned()
                        );
                    }
                    depth = Some(length);
                }
                Some("angled") => {
                    let angle = number(bottom.get("included_angle"))
                        .filter(|a| *a > 0.0 && *a < std::f64::consts::PI)
                        .ok_or("its drill point's angle is not given")?;
                    // SolidCraft's depth is to the shoulder, where the point begins.
                    let to_tip = bottom.get("depth_to_tip").and_then(Value::as_bool) == Some(true);
                    let point = diameter / 2.0 / (angle / 2.0).tan();
                    let shoulder = if to_tip { length - point } else { length };
                    if shoulder <= 0.0 {
                        return Err("its drill point is deeper than the hole".to_owned());
                    }
                    depth = Some(shoulder);
                    let tip = format!("{}", angle.to_degrees());
                    match hole.as_object_mut() {
                        Some(fields)
                            if recess_tip
                                && matches!(
                                    fields.get("type").and_then(Value::as_str),
                                    Some("counterbore" | "countersink")
                                ) =>
                        {
                            fields.insert("tip_angle".to_owned(), json!(tip));
                        }
                        _ => hole = json!({"type": "drilled", "tip_angle": tip}),
                    }
                }
                Some(other) => {
                    return Err(format!("a hole with a {other} bottom is not mapped yet"))
                }
            }
        }
        Some("to_face") => {
            if number(extent.get("offset")).is_some_and(|offset| offset != 0.0) {
                return Err("a hole that ends offset from a face is not mapped yet".to_owned());
            }
            if hole.get("type").and_then(Value::as_str) == Some("drilled")
                || definition.pointer("/bottom/kind").and_then(Value::as_str) == Some("angled")
            {
                return Err("a hole to a face with a drill point is not mapped yet".to_owned());
            }
            let face = earlier_face(feature, extent.get("face"), ctx)?;
            if dot(sub(face.point, position), direction) <= 0.0
                || dot(face.normal, direction).abs() < 1.0 - 1e-9
            {
                return Err("its terminating face is not ahead of the hole".to_owned());
            }
            to = Some(face.point);
        }
        Some("unresolved") => return Err("its hole termination is not resolved".to_owned()),
        Some("through_next") => {
            return Err("a hole through the next material region is not mapped yet".to_owned())
        }
        Some("to_first") => {
            return Err("a hole ending at the first encountered face is not mapped yet".to_owned())
        }
        Some("to_last") => {
            return Err("a hole ending at the last encountered face is not mapped yet".to_owned())
        }
        Some("to_vertex") => {
            return Err("a hole ending at a source vertex is not mapped yet".to_owned())
        }
        Some("offset_from_face") => {
            return Err("a hole that ends offset from a face is not mapped yet".to_owned())
        }
        Some("to_shape") => {
            return Err("a hole ending at a selected shape is not mapped yet".to_owned())
        }
        Some(other) => return Err(format!("a hole that ends {other} is not mapped yet")),
    }
    if definition.get("taper_angle").is_some_and(|t| !t.is_null()) {
        return Err("a tapered hole is not mapped yet".to_owned());
    }

    let id = ctx.id();
    ctx.builds = true;
    let mut out = json!({
        "id": id,
        "name": name(feature).map(str::to_owned).unwrap_or_else(|| format!("Hole{id}")),
        "type": "hole",
        "suppressed": false,
        "diameter": format!("{diameter} mm"),
        "direction": direction,
        "hole": hole,
        "position": position,
    });
    if let (Some(depth), Some(fields)) = (depth, out.as_object_mut()) {
        fields.insert("depth".to_owned(), json!(format!("{depth} mm")));
    }
    if let Some(fields) = out.as_object_mut() {
        if let Some(thread) = thread {
            fields.insert("thread".to_owned(), json!(thread));
        }
        if let Some(points) = points {
            fields.insert("points".to_owned(), points);
        }
        if let Some(to) = to {
            fields.insert("to".to_owned(), json!(to));
        }
    }
    if let Some(note) = thread_note {
        report.push(Severity::Degraded, describe(feature), note);
    }
    Ok(out)
}

/// The cosmetic thread label a threaded hole keeps, and a note on what of its thread does
/// not survive. SolidCraft labels a hole with an ISO metric thread (`M6x1`) and keeps the
/// source's resolved bore diameter; it has no hand, class or thread-length controls, so
/// those are reported rather than blocking a hole whose geometry maps exactly. A modeled
/// (helical) thread changes the geometry and is refused.
fn cosmetic_thread(
    specification: Option<&Value>,
    bore: f64,
) -> Result<(Option<String>, Option<String>), String> {
    let Some(specification) = specification.filter(|value| !value.is_null()) else {
        return Ok((None, None));
    };
    if specification.get("kind").and_then(Value::as_str) != Some("threaded") {
        return Ok((None, None));
    }
    if specification.get("modeled").and_then(Value::as_bool) == Some(true) {
        return Err("a hole with a modeled thread is not mapped yet".to_owned());
    }
    let designation = specification
        .get("designation")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let pitch = number(specification.get("pitch")).filter(|p| *p > 0.0);
    let label = match designation {
        None => Err("its thread size is not given".to_owned()),
        Some(designation) => match metric_thread(designation, pitch) {
            None => Err(format!(
                "SolidCraft labels ISO metric threads only, not {designation}"
            )),
            Some((major, pitch)) if !thread_fits(major, pitch, bore) => Err(format!(
                "an {designation} thread does not fit its {bore} mm bore"
            )),
            Some((major, pitch)) => Ok(format!("M{major}x{pitch}")),
        },
    };
    let label = match label {
        Ok(label) => label,
        Err(why) => {
            return Ok((
                None,
                Some(format!("its thread is left out ({why}); the bore is kept")),
            ))
        }
    };
    let mut lost = Vec::new();
    if specification.get("hand").and_then(Value::as_str) == Some("left") {
        lost.push("its left-hand direction".to_owned());
    }
    if let Some(class) = specification.get("class").and_then(Value::as_str) {
        lost.push(format!("its {class} tolerance class"));
    }
    match specification.pointer("/depth/kind").and_then(Value::as_str) {
        Some("blind") => match number(specification.pointer("/depth/depth")) {
            Some(depth) => lost.push(format!("its {depth} mm thread length")),
            None => lost.push("its thread length".to_owned()),
        },
        Some("tapped_standard") => lost.push("its standard tapped runout".to_owned()),
        _ => {}
    }
    let note = (!lost.is_empty()).then(|| {
        format!(
            "its thread is kept as a cosmetic {label} label over the whole hole, without {}",
            series(&lost)
        )
    });
    Ok((Some(label), note))
}

/// `a`, `a and b`, or `a, b, and c`.
fn series(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [one] => one.clone(),
        [first, second] => format!("{first} and {second}"),
        [rest @ .., last] => format!("{}, and {last}", rest.join(", ")),
    }
}

/// ISO 261 coarse pitches, for a metric designation that gives no pitch.
const ISO_COARSE: [(f64, f64); 34] = [
    (1.0, 0.25),
    (1.2, 0.25),
    (1.4, 0.3),
    (1.6, 0.35),
    (1.8, 0.35),
    (2.0, 0.4),
    (2.5, 0.45),
    (3.0, 0.5),
    (3.5, 0.6),
    (4.0, 0.7),
    (5.0, 0.8),
    (6.0, 1.0),
    (7.0, 1.0),
    (8.0, 1.25),
    (10.0, 1.5),
    (12.0, 1.75),
    (14.0, 2.0),
    (16.0, 2.0),
    (18.0, 2.5),
    (20.0, 2.5),
    (22.0, 2.5),
    (24.0, 3.0),
    (27.0, 3.0),
    (30.0, 3.5),
    (33.0, 3.5),
    (36.0, 4.0),
    (39.0, 4.0),
    (42.0, 4.5),
    (45.0, 4.5),
    (48.0, 5.0),
    (52.0, 5.0),
    (56.0, 5.5),
    (60.0, 5.5),
    (64.0, 6.0),
];

/// The major diameter and pitch (mm) of an ISO metric designation such as `M6`, `M6x0.75`
/// or `M6x1-6H`; a pitch the designation leaves out comes from `pitch`, then the coarse
/// series. A class or hand after the size is not part of it.
pub(super) fn metric_thread(designation: &str, pitch: Option<f64>) -> Option<(f64, f64)> {
    let upper = designation.trim().to_ascii_uppercase();
    let body = upper.strip_prefix('M')?;
    let size: String = body
        .chars()
        .take_while(|c| c.is_ascii_digit() || matches!(c, '.' | 'X' | '\u{d7}' | ' '))
        .filter(|c| *c != ' ')
        .collect();
    let (major, own_pitch) = match size.split_once(['X', '\u{d7}']) {
        Some((major, pitch)) => (major.parse::<f64>().ok()?, Some(pitch.parse::<f64>().ok()?)),
        None => (size.parse::<f64>().ok()?, None),
    };
    let pitch = own_pitch.or(pitch).or_else(|| {
        ISO_COARSE
            .iter()
            .find(|(size, _)| (size - major).abs() < 1e-9)
            .map(|(_, pitch)| *pitch)
    })?;
    (major.is_finite() && pitch.is_finite() && major > 0.0 && pitch > 0.0 && pitch < major)
        .then_some((major, pitch))
}

/// Whether SolidCraft accepts a thread of `major` and `pitch` on a hole of `bore`: the bore
/// is near the thread's minor diameter or its tap drill.
fn thread_fits(major: f64, pitch: f64, bore: f64) -> bool {
    (major - pitch * 1.0825 - bore).abs() < 0.2 * major
        || (major - pitch - bore).abs() < 0.15 * major
}

/// A complete placement, or sketch locations that keep following their sketch.
fn placement(
    feature: &Value,
    definition: &Value,
    ctx: &Context,
) -> Result<(V3, V3, Option<Value>), String> {
    if let Some(placements) = definition.get("placements").and_then(Value::as_array) {
        let [placement] = placements.as_slice() else {
            return sketch_placements(definition, ctx, placements);
        };
        return match placement.get("kind").and_then(Value::as_str) {
            Some("directed") => {
                let position = v3(placement.get("position")).ok_or("its position is not given")?;
                let direction = v3(placement.get("direction"))
                    .and_then(unit)
                    .ok_or("its direction is zero")?;
                if definition.get("face").is_some_and(|face| !face.is_null()) {
                    let face = earlier_face(feature, definition.get("face"), ctx)?;
                    if dot(sub(position, face.point), face.normal).abs() > 1e-7
                        || !face.contains(position)
                        || dot(direction, face.normal) >= 0.0
                    {
                        return Err("its placement does not drill into its entry face".to_owned());
                    }
                }
                Ok((position, direction, None))
            }
            Some("axis") => {
                let face = earlier_face(feature, definition.get("face"), ctx)?;
                let origin = v3(placement.get("origin")).ok_or("its axis origin is not given")?;
                let axis = v3(placement.get("axis"))
                    .and_then(unit)
                    .ok_or("its axis is zero")?;
                let cos = dot(axis, face.normal);
                if cos.abs() < 1e-9 {
                    return Err("its hole axis is parallel to its entry face".to_owned());
                }
                let position = add(
                    origin,
                    axis,
                    dot(sub(face.point, origin), face.normal) / cos,
                );
                if !position.iter().all(|c| c.is_finite()) || !face.contains(position) {
                    return Err("its hole axis misses its entry face".to_owned());
                }
                // An axis has no sign: the entry face supplies the inward direction.
                let direction = axis.map(|c| if cos > 0.0 { -c } else { c });
                Ok((position, direction, None))
            }
            _ => Err("a hole of this placement is not mapped yet".to_owned()),
        };
    }
    let (position, direction, points, _) = sketch_locations(definition, ctx, None)?;
    Ok((position, direction, points))
}

/// Several directed placements, drilled at the sketch locations they stand on: every
/// placement must be one of the hole's sketch locations, every location one of the
/// placements, and all of them must drill the same way.
fn sketch_placements(
    definition: &Value,
    ctx: &Context,
    placements: &[Value],
) -> Result<(V3, V3, Option<Value>), String> {
    const NEAR: f64 = 1e-6;
    let mut spots = Vec::new();
    let mut direction = None::<V3>;
    for placement in placements {
        if placement.get("kind").and_then(Value::as_str) != Some("directed") {
            return Err("multiple spatial hole placements are not mapped yet".to_owned());
        }
        let position = v3(placement.get("position")).ok_or("its position is not given")?;
        let along = v3(placement.get("direction"))
            .and_then(unit)
            .ok_or("its direction is zero")?;
        if direction.is_some_and(|first| dot(first, along) < 1.0 - 1e-9) {
            return Err("holes drilled in different directions are not mapped yet".to_owned());
        }
        direction.get_or_insert(along);
        spots.push(position);
    }
    let direction = direction.ok_or("its placements are missing")?;
    let (position, direction, points, worlds) = sketch_locations(definition, ctx, Some(direction))
        .map_err(|reason| {
            format!("multiple spatial hole placements without sketch locations: {reason}")
        })?;
    let near = |a: V3, b: V3| dot(sub(a, b), sub(a, b)).sqrt() <= NEAR;
    if worlds.len() != spots.len()
        || !spots.iter().all(|s| worlds.iter().any(|w| near(*s, *w)))
        || !worlds.iter().all(|w| spots.iter().any(|s| near(*s, *w)))
    {
        return Err("its placements are not its sketch locations".to_owned());
    }
    Ok((position, direction, points))
}

/// Sketch locations that keep following their sketch, the first one's position, the drill
/// direction (`direction`, else against the sketch normal) and every location's position.
fn sketch_locations(
    definition: &Value,
    ctx: &Context,
    direction: Option<V3>,
) -> Result<(V3, V3, Option<Value>, Vec<V3>), String> {
    let profile = definition.get("profile").unwrap_or(&Value::Null);
    let kind = profile.get("kind").and_then(Value::as_str);
    let value = profile.get("value").unwrap_or(&Value::Null);
    let sketch_id = match kind {
        Some("sketch") => value.as_str(),
        Some("sketch_entities") => value.get("sketch").and_then(Value::as_str),
        Some("sketch_profiles" | "sketch_regions") => {
            return Err("holes placed from sketch profile regions are not mapped yet".to_owned())
        }
        Some("sketch_selection") => {
            return Err("its selected sketch hole locations are not resolved".to_owned())
        }
        Some("historical_faces" | "faces") => {
            return Err("holes placed from face profiles are not mapped yet".to_owned())
        }
        Some("feature" | "generated") => {
            return Err("holes placed from generated curve profiles are not mapped yet".to_owned())
        }
        _ => return Err("a hole placed without resolved points is not mapped yet".to_owned()),
    }
    .ok_or("its sketch is not resolved")?;
    let sketch = ctx
        .sketches
        .get(sketch_id)
        .ok_or("its sketch is not an earlier mapped sketch")?;
    let selected = if kind == Some("sketch_entities") {
        let entities = value
            .get("entities")
            .and_then(Value::as_array)
            .filter(|ids| !ids.is_empty() && ids.len() <= 4096)
            .ok_or("its selected hole locations are not resolved")?;
        let selected = entities
            .iter()
            .map(|id| {
                id.as_str()
                    .filter(|id| sketch.shapes.contains_key(*id))
                    .ok_or("its selected hole locations are not in its sketch")
            })
            .collect::<Result<BTreeSet<_>, _>>()?;
        Some(selected)
    } else {
        // Construction references become hole locations only when explicitly selected.
        let construction: BTreeSet<&str> = ctx
            .arena("sketch_entities")
            .iter()
            .filter(|entity| entity.get("sketch").and_then(Value::as_str) == Some(sketch_id))
            .filter(|entity| entity.get("construction").and_then(Value::as_bool) == Some(true))
            .filter_map(|entity| entity.get("id").and_then(Value::as_str))
            .collect();
        Some(
            sketch
                .shapes
                .keys()
                .map(String::as_str)
                .filter(|id| !construction.contains(id))
                .collect(),
        )
    };
    let locations = locations(sketch, definition.get("profile_filter"), selected.as_ref())?;
    let first = locations
        .first()
        .ok_or("its sketch has no points to drill at")?;
    let direction = match direction.or_else(|| v3(definition.get("direction"))) {
        Some(direction) => unit(direction).ok_or("its direction is zero")?,
        None => sketch.normal.map(|c| -c),
    };
    // SolidCraft's multi-point placement drills perpendicular to the sketch.
    if dot(direction, sketch.normal).abs() < 1.0 - 1e-9 {
        return Err("a hole oblique to its location sketch is not mapped yet".to_owned());
    }
    let position = sketch.world(first.1);
    let worlds = locations.iter().map(|(_, p)| sketch.world(*p)).collect();
    let ids: Vec<String> = locations.into_iter().map(|(id, _)| id).collect();
    Ok((
        position,
        direction,
        Some(json!({"sketch": sketch.feature, "ids": ids})),
        worlds,
    ))
}

/// A planar face with a point inside its boundary, as in `faces.rs`. The final B-rep
/// alone cannot establish when a face existed: `earlier_face` also proves its cap
/// against the last preceding solid feature's sketch and extrusion distance.
struct PlanarFace {
    point: V3,
    normal: V3,
    u: V3,
    v: V3,
    corners: Vec<V3>,
}

impl PlanarFace {
    fn contains(&self, point: V3) -> bool {
        let flat = |p: V3| [dot(p, self.u), dot(p, self.v)];
        let polygon: Vec<[f64; 2]> = self.corners.iter().map(|p| flat(*p)).collect();
        inside(&polygon, flat(point))
    }
}

fn earlier_face(
    feature: &Value,
    selection: Option<&Value>,
    ctx: &Context,
) -> Result<PlanarFace, String> {
    let selection = selection.ok_or("its face is not resolved")?;
    let ids = match selection.get("kind").and_then(Value::as_str) {
        Some("faces") => selection.get("value"),
        Some("resolved") => selection.pointer("/value/faces"),
        _ => None,
    }
    .and_then(Value::as_array)
    .ok_or("its face has no resolved geometry")?;
    let [id] = ids.as_slice() else {
        return Err("a hole referring to several faces is not mapped yet".to_owned());
    };
    let face = id
        .as_str()
        .and_then(|id| ctx.row("faces", id))
        .ok_or("its face is missing")?;
    let geometry = face
        .get("surface")
        .and_then(Value::as_str)
        .and_then(|id| ctx.row("surfaces", id))
        .and_then(|row| row.get("geometry"))
        .ok_or("its face's surface is missing")?;
    if geometry.get("kind").and_then(Value::as_str) != Some("plane") {
        return Err("a hole referring to a curved face is not mapped yet".to_owned());
    }
    let normal = v3(geometry.get("normal"))
        .and_then(unit)
        .ok_or("its face's normal is missing")?;
    let u = v3(geometry.get("u_axis"))
        .and_then(unit)
        .ok_or("its face's axes are missing")?;
    let v = [
        normal[1] * u[2] - normal[2] * u[1],
        normal[2] * u[0] - normal[0] * u[2],
        normal[0] * u[1] - normal[1] * u[0],
    ];
    let origin = v3(geometry.get("origin")).ok_or("its face's origin is missing")?;
    // Only the outer loop can be compared with an earlier extrusion cap. Inner
    // loops in the saved result may have been introduced by this very hole; the
    // proven earlier cap has the single, filled profile reconstructed below.
    let boundary = face.get("loops").ok_or("its face's boundary is missing")?;
    let id = match boundary.get("classification").and_then(Value::as_str) {
        Some("classified") => boundary.get("outer"),
        _ => boundary
            .get("loops")
            .and_then(Value::as_array)
            .and_then(|ids| ids.first()),
    }
    .ok_or("its face's outer boundary is missing")?;
    let lp = id
        .as_str()
        .and_then(|id| ctx.row("loops", id))
        .ok_or("its face's boundary is missing")?;
    let coedges = lp
        .pointer("/boundary/coedges")
        .and_then(Value::as_array)
        .filter(|rows| rows.len() <= 4096)
        .ok_or("its face's polygonal boundary is not resolved")?;
    let mut corners = Vec::new();
    for id in coedges {
        let coedge = id
            .as_str()
            .and_then(|id| ctx.row("coedges", id))
            .ok_or("its face's boundary is missing")?;
        let edge = coedge
            .get("edge")
            .and_then(Value::as_str)
            .and_then(|id| ctx.row("edges", id))
            .ok_or("its face's edge is missing")?;
        // A curved edge's endpoints do not describe the boundary polygon.
        let curve = edge
            .pointer("/carrier/curve")
            .and_then(Value::as_str)
            .and_then(|id| ctx.row("curves", id));
        if curve
            .and_then(|row| row.pointer("/geometry/kind"))
            .and_then(Value::as_str)
            != Some("line")
        {
            return Err(
                "a hole referring to a face with curved edges is not mapped yet".to_owned(),
            );
        }
        let reversed = coedge.get("sense").and_then(Value::as_str) == Some("reversed");
        let vertex = edge
            .get(if reversed { "end" } else { "start" })
            .and_then(Value::as_str)
            .and_then(|id| ctx.row("vertices", id))
            .ok_or("its face's vertex is missing")?;
        let point = vertex
            .get("point")
            .and_then(Value::as_str)
            .and_then(|id| ctx.row("points", id))
            .and_then(|row| v3(row.get("position")))
            .ok_or("its face's point is missing")?;
        if dot(sub(point, origin), normal).abs() > 1e-7 {
            return Err("its face's boundary is not on its plane".to_owned());
        }
        corners.push(point);
    }
    if corners.len() < 3 {
        return Err("its face has no polygonal boundary".to_owned());
    }
    let mut planar = PlanarFace {
        point: [0.0; 3],
        normal,
        u,
        v,
        corners,
    };
    let outer = &planar.corners;
    // As in faces.rs, choose the centroid of a nondegenerate corner triangle
    // inside the earlier cap's filled outline.
    planar.point = (0..outer.len())
        .find_map(|i| {
            let a = *outer.get((i + outer.len() - 1) % outer.len())?;
            let b = *outer.get(i)?;
            let c = *outer.get((i + 1) % outer.len())?;
            let centre = [
                (a[0] + b[0] + c[0]) / 3.0,
                (a[1] + b[1] + c[1]) / 3.0,
                (a[2] + b[2] + c[2]) / 3.0,
            ];
            let ab = sub(b, a);
            let ac = sub(c, a);
            let area = [
                ab[1] * ac[2] - ab[2] * ac[1],
                ab[2] * ac[0] - ab[0] * ac[2],
                ab[0] * ac[1] - ab[1] * ac[0],
            ];
            (dot(area, area) > 1e-18 && planar.contains(centre)).then_some(centre)
        })
        .ok_or("its face has no resolved interior point")?;
    if !earlier_cap(feature, &planar, ctx) {
        return Err("its face is not proven to exist before the hole".to_owned());
    }
    // The underlying surface normal need not be the face's outward normal.
    if face.get("sense").and_then(Value::as_str) == Some("reversed") {
        planar.normal = planar.normal.map(|c| -c);
    }
    Ok(planar)
}

/// A deliberately narrow proof: one polygonal profile, extruded as a new body,
/// with no intervening solid operation. Matching the cap's entire outer boundary
/// rules out a blind bottom or shoulder introduced by this hole or a later cut.
fn earlier_cap(feature: &Value, face: &PlanarFace, ctx: &Context) -> bool {
    let Some(previous) = ctx
        .arena("features")
        .iter()
        .filter(|row| {
            row.get("id")
                .and_then(Value::as_str)
                .is_some_and(|id| ctx.feature_names.contains_key(id))
                && super::operation(row)
                    .get("definition")
                    .and_then(Value::as_str)
                    != Some("sketch")
                && row.get("ordinal").and_then(Value::as_u64)
                    < feature.get("ordinal").and_then(Value::as_u64)
        })
        .max_by_key(|row| row.get("ordinal").and_then(Value::as_u64))
    else {
        return false;
    };
    let op = previous.get("definition").unwrap_or(&Value::Null);
    if op.get("definition").and_then(Value::as_str) != Some("extrude")
        || op.get("op").and_then(Value::as_str) != Some("new_body")
        || op.pointer("/extent/kind").and_then(Value::as_str) != Some("one_sided")
        || op
            .pointer("/extent/side/termination/kind")
            .and_then(Value::as_str)
            != Some("blind")
        || number(op.pointer("/extent/side/draft")).is_some_and(|a| a != 0.0)
        || op
            .get("start")
            .is_some_and(|s| s.get("kind").and_then(Value::as_str) != Some("profile_plane"))
    {
        return false;
    }
    if !matches!(
        op.pointer("/profile/kind").and_then(Value::as_str),
        Some("sketch" | "sketch_profiles")
    ) {
        return false;
    }
    let Some(length) = number(op.pointer("/extent/side/termination/length")) else {
        return false;
    };
    let Ok((sketch_id, _)) = super::profile::profile(op.get("profile"), ctx) else {
        return false;
    };
    let Some(sketch) = ctx.sketches.get(&sketch_id) else {
        return false;
    };
    let Some(profiles) = ctx
        .row("sketches", &sketch_id)
        .and_then(|row| row.get("profiles"))
        .and_then(Value::as_array)
    else {
        return false;
    };
    let [profile] = profiles.as_slice() else {
        return false;
    };
    let Some(uses) = profile.as_array() else {
        return false;
    };
    let outer = &face.corners;
    if uses.len() != outer.len() || dot(face.normal, sketch.normal).abs() < 1.0 - 1e-9 {
        return false;
    }
    let unique: BTreeSet<_> = uses
        .iter()
        .filter_map(|entry| entry.get("entity").and_then(Value::as_str))
        .collect();
    if unique.len() != uses.len() {
        return false;
    }
    let direction = match op.pointer("/direction/kind").and_then(Value::as_str) {
        None | Some("profile_normal") => 1.0,
        Some("reversed_profile_normal") => -1.0,
        _ => return false,
    };
    [0.0, direction * length].into_iter().any(|height| {
        uses.iter().all(|entry| {
            let Some(curve) = entry
                .get("entity")
                .and_then(Value::as_str)
                .and_then(|id| sketch.curve(id))
            else {
                return false;
            };
            if curve.kind != "line" {
                return false;
            }
            let Some((_, start)) = curve.start.and_then(|i| sketch.points.get(i)) else {
                return false;
            };
            let Some((_, end)) = curve.end.and_then(|i| sketch.points.get(i)) else {
                return false;
            };
            let start = add(sketch.world(*start), sketch.normal, height);
            let end = add(sketch.world(*end), sketch.normal, height);
            let close = |a: V3, b: V3| dot(sub(a, b), sub(a, b)) < 1e-16;
            outer.iter().enumerate().any(|(i, a)| {
                outer.get((i + 1) % outer.len()).is_some_and(|b| {
                    (close(start, *a) && close(end, *b)) || (close(start, *b) && close(end, *a))
                })
            })
        })
    })
}

fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn add(a: V3, direction: V3, distance: f64) -> V3 {
    [
        a[0] + direction[0] * distance,
        a[1] + direction[1] * distance,
        a[2] + direction[2] * distance,
    ]
}

fn inside(polygon: &[[f64; 2]], p: [f64; 2]) -> bool {
    let mut inside = false;
    let n = polygon.len();
    for i in 0..n {
        let (Some(a), Some(b)) = (polygon.get(i), polygon.get((i + n - 1) % n)) else {
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

/// The sketch points holes are drilled at, with their positions in the sketch: the centres of
/// its circles and arcs, and its points, as far as `filter` takes each, in the sketch's order.
fn locations(
    sketch: &Mapped,
    filter: Option<&Value>,
    selected: Option<&BTreeSet<&str>>,
) -> Result<Vec<(String, [f64; 2])>, String> {
    let filter = filter.and_then(Value::as_str).unwrap_or("all");
    let (points, circles, arcs) = match filter {
        "points" => (true, false, false),
        "circles" => (false, true, false),
        "points_and_circles" => (true, true, false),
        "arcs" => (false, false, true),
        "points_and_arcs" => (true, false, true),
        "circles_and_arcs" => (false, true, true),
        "all" => (true, true, true),
        other => return Err(format!("holes at a sketch's {other} are not mapped yet")),
    };
    let mut indices: Vec<usize> = sketch
        .shapes
        .iter()
        .filter(|(id, _)| selected.is_none_or(|ids| ids.contains(id.as_str())))
        .filter_map(|(_, shape)| match shape {
            Shape::Point(index) if points => Some(*index),
            Shape::Curve(curve)
                if (circles && curve.kind == "circle") || (arcs && curve.kind == "arc") =>
            {
                curve.center
            }
            _ => None,
        })
        .collect();
    indices.sort_unstable();
    indices.dedup();
    indices
        .into_iter()
        .map(|index| {
            sketch
                .points
                .get(index)
                .cloned()
                .ok_or_else(|| "its sketch's points are malformed".to_owned())
        })
        .collect()
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use std::collections::BTreeMap;

    use super::super::sketch::Curve;
    use super::*;

    /// A sketch 10 mm up on XY with a circle about (3, 4) and a lone point at (-1, 0).
    fn sketch() -> Mapped {
        let mut shapes = BTreeMap::new();
        shapes.insert(
            "circle".to_owned(),
            Shape::Curve(Curve {
                pieces: Vec::new(),
                index: 0,
                id: "c1".to_owned(),
                kind: "circle",
                start: None,
                end: None,
                center: Some(1),
            }),
        );
        shapes.insert("point".to_owned(), Shape::Point(2));
        Mapped {
            feature: 7,
            normal: [0.0, 0.0, 1.0],
            origin: [0.0, 0.0, 10.0],
            x_dir: [1.0, 0.0, 0.0],
            y_dir: [0.0, 1.0, 0.0],
            points: vec![
                ("origin".to_owned(), [0.0, 0.0]),
                ("p1".to_owned(), [3.0, 4.0]),
                ("p2".to_owned(), [-1.0, 0.0]),
            ],
            shapes,
        }
    }

    fn hole(extra: Value) -> Value {
        let mut hole = json!({
            "definition": "hole",
            "profile": {"kind": "sketch", "value": "s"},
            "profile_filter": "circles",
            "direction": {"x": 0.0, "y": 0.0, "z": -1.0},
            "shape": {"construction": {"construction": "form", "kind": {"kind": "simple"}}, "diameter": 5.0},
            "extent": {"kind": "through_all"},
        });
        if let (Some(fields), Some(more)) = (hole.as_object_mut(), extra.as_object()) {
            fields.extend(more.clone());
        }
        hole
    }

    fn mapped(definition: &Value) -> Result<Value, String> {
        let model = json!({});
        let mut ctx = Context::new(&model);
        ctx.sketches.insert("s".to_owned(), sketch());
        map(
            &json!({"name": "Hole"}),
            definition,
            &mut ctx,
            &mut Report::default(),
        )
    }

    #[test]
    fn a_simple_hole_through_all_drills_at_each_circle() {
        let out = mapped(&hole(json!({}))).unwrap();
        assert_eq!(out["type"], "hole");
        assert_eq!(out["diameter"], "5 mm");
        assert_eq!(out["direction"], json!([0.0, 0.0, -1.0]));
        assert_eq!(out["hole"], json!({"type": "simple"}));
        assert_eq!(out["points"], json!({"sketch": 7, "ids": ["p1"]}));
        assert_eq!(out["position"], json!([3.0, 4.0, 10.0]));
        assert!(out.get("depth").is_none(), "through all");
    }

    #[test]
    fn several_placements_on_sketch_locations_drill_at_those_locations() {
        let at = |x: f64, y: f64, z: f64| {
            json!({"kind": "directed", "position": {"x": x, "y": y, "z": z},
                "direction": {"x": 0.0, "y": 0.0, "z": z.signum()}})
        };
        let both = json!({"profile_filter": "all", "placements": [
            at(3.0, 4.0, 10.0), at(-1.0, 0.0, 10.0)]});
        let out = mapped(&hole(both)).unwrap();
        assert_eq!(out["points"]["ids"], json!(["p1", "p2"]));
        // The placements' direction wins over the sketch's default drilling side.
        assert_eq!(out["direction"], json!([0.0, 0.0, 1.0]));

        let elsewhere = json!({"profile_filter": "all", "placements": [
            at(3.0, 4.0, 10.0), at(5.0, 5.0, 10.0)]});
        assert_eq!(
            mapped(&hole(elsewhere)).unwrap_err(),
            "its placements are not its sketch locations"
        );
        let opposed = json!({"profile_filter": "all", "placements": [
            at(3.0, 4.0, 10.0), at(-1.0, 0.0, -10.0)]});
        assert_eq!(
            mapped(&hole(opposed)).unwrap_err(),
            "holes drilled in different directions are not mapped yet"
        );
    }

    #[test]
    fn every_location_the_filter_takes_is_drilled() {
        let out = mapped(&hole(json!({"profile_filter": "all"}))).unwrap();
        assert_eq!(out["points"]["ids"], json!(["p1", "p2"]));
        let none = mapped(&hole(json!({"profile_filter": "arcs"}))).unwrap_err();
        assert_eq!(none, "its sketch has no points to drill at");
    }

    #[test]
    fn construction_locations_require_an_explicit_selection() {
        for (entity, expected) in [("point", "p1"), ("circle", "p2")] {
            let model = json!({"sketch_entities": [
                {"id": entity, "sketch": "s", "construction": true}]});
            let mut ctx = Context::new(&model);
            ctx.sketches.insert("s".to_owned(), sketch());
            let definition = hole(json!({"profile_filter": "all"}));
            let feature = json!({"name": "Hole"});
            let implicit = map(&feature, &definition, &mut ctx, &mut Report::default()).unwrap();
            assert_eq!(implicit["points"]["ids"], json!([expected]));
            let explicit = hole(json!({"profile_filter": "all", "profile": {
                "kind": "sketch_entities", "value": {"sketch": "s", "entities": ["circle", "point"]}}}));
            let explicit = map(&feature, &explicit, &mut ctx, &mut Report::default()).unwrap();
            assert_eq!(explicit["points"]["ids"], json!(["p1", "p2"]));
        }
    }

    #[test]
    fn a_blind_hole_with_a_drill_point_is_drilled_to_its_shoulder() {
        let angle = 118_f64.to_radians();
        let flat = mapped(&hole(json!({"extent": {"kind": "blind", "length": 6.0}}))).unwrap();
        assert_eq!(flat["depth"], "6 mm");
        let to_shoulder = mapped(&hole(json!({
            "extent": {"kind": "blind", "length": 6.0},
            "bottom": {"kind": "angled", "included_angle": angle, "depth_to_tip": false},
        })))
        .unwrap();
        assert_eq!(to_shoulder["depth"], "6 mm");
        assert_eq!(
            to_shoulder["hole"],
            json!({"type": "drilled", "tip_angle": "118.00000000000001"})
        );
        let to_tip = mapped(&hole(json!({
            "extent": {"kind": "blind", "length": 6.0},
            "bottom": {"kind": "angled", "included_angle": angle, "depth_to_tip": true},
        })))
        .unwrap();
        let depth: f64 = to_tip["depth"]
            .as_str()
            .unwrap()
            .trim_end_matches(" mm")
            .parse()
            .unwrap();
        assert!((depth - (6.0 - 2.5 / (59_f64.to_radians()).tan())).abs() < 1e-9);
    }

    #[test]
    fn entry_forms_require_their_dimensions() {
        for (form, reason) in [
            (
                json!({"kind": "counterbore"}),
                "its entry diameter must be greater than its bore diameter",
            ),
            (
                json!({"kind": "counterbore", "diameter": 5.0, "depth": 2.0}),
                "its entry diameter must be greater than its bore diameter",
            ),
            (
                json!({"kind": "counterbore", "diameter": 8.0}),
                "its counterbore depth is not given",
            ),
            (
                json!({"kind": "counterbore", "diameter": 8.0, "depth": 0.0}),
                "its counterbore depth is not given",
            ),
            (
                json!({"kind": "countersink", "diameter": 8.0}),
                "its countersink angle is not given",
            ),
            (
                json!({"kind": "countersink", "diameter": 8.0, "angle": std::f64::consts::PI}),
                "its countersink angle is not given",
            ),
        ] {
            let definition = hole(json!({"shape": {"construction": {
                "construction": "form", "kind": form}, "diameter": 5.0}}));
            assert_eq!(mapped(&definition).unwrap_err(), reason);
        }
    }

    #[test]
    fn entry_forms_with_a_drill_point_are_not_silently_simplified() {
        for (form, kind) in [
            (
                json!({"kind": "counterbore", "diameter": 8.0, "depth": 2.0}),
                "counterbore",
            ),
            (
                json!({"kind": "countersink", "diameter": 8.0, "angle": std::f64::consts::FRAC_PI_2}),
                "countersink",
            ),
            (
                json!({"kind": "chamfer", "diameter": 8.0, "angle": std::f64::consts::FRAC_PI_2}),
                "countersink",
            ),
        ] {
            for to_tip in [false, true] {
                let definition = hole(json!({
                    "shape": {"construction": {"construction": "form", "kind": form}, "diameter": 5.0},
                    "extent": {"kind": "blind", "length": 6.0},
                    "bottom": {"kind": "angled", "included_angle": 118_f64.to_radians(), "depth_to_tip": to_tip},
                }));
                // SolidCraft that drills an entry-recess hole to a point keeps both.
                let out = mapped(&definition).unwrap();
                assert_eq!(out["hole"]["type"], kind);
                assert_eq!(out["hole"]["tip_angle"], "118.00000000000001");
                // Older SolidCraft would silently drop the point: refused.
                let model = json!({});
                let mut ctx = Context::new(&model);
                ctx.target_version = "0.22.0".parse().unwrap();
                ctx.sketches.insert("s".to_owned(), sketch());
                assert_eq!(
                    map(
                        &json!({"name": "Hole"}),
                        &definition,
                        &mut ctx,
                        &mut Report::default()
                    )
                    .unwrap_err(),
                    "SolidCraft holes take a drill point only on a plain drilled hole"
                );
            }
        }
    }

    #[test]
    fn other_holes_say_why_they_are_not_mapped() {
        let counterbore = hole(json!({"shape": {"construction": {"construction": "form",
            "kind": {"kind": "counterbore_drilled", "diameter": 8.0, "depth": 2.0,
                "drill_point_angle": 118_f64.to_radians()}}, "diameter": 5.0},
            "extent": {"kind": "blind", "length": 6.0}}));
        assert_eq!(
            mapped(&counterbore).unwrap_err(),
            "SolidCraft holes take a drill point only on a plain drilled hole"
        );
        let to_face = hole(json!({"extent": {"kind": "to_face"}}));
        assert_eq!(mapped(&to_face).unwrap_err(), "its face is not resolved");
    }
}
