//! Extrudes, as SolidCraft extrude features.

use serde_json::{json, Map, Value};

use super::profile::profile;
use super::sketch::{dot, number, unit, v3};
use super::{name, Context};

/// How close to the sketch normal (as a cosine) an explicit direction must be.
const ALONG_NORMAL: f64 = 1.0 - 1e-9;

/// Maps an extrude feature whose operation is `definition`.
pub(crate) fn map(
    feature: &Value,
    definition: &Value,
    ctx: &mut Context,
    preceding: &[Value],
) -> Result<Value, String> {
    let face_profile =
        definition.pointer("/profile/kind").and_then(Value::as_str) == Some("historical_faces");
    let (sketch_id, profiles) = if face_profile {
        // The sketch of the face's boundary runs along the face's outward normal; mapping
        // assumes that side and the replay corrects it, so nothing here may depend on the
        // side beyond travelling along or against the profile normal.
        if !matches!(
            definition
                .pointer("/direction/kind")
                .and_then(Value::as_str),
            None | Some("profile_normal" | "reversed_profile_normal")
        ) {
            return Err(
                "an extrude of a face in a direction of its own is not mapped yet".to_owned(),
            );
        }
        super::face_profile::map(
            feature,
            definition.pointer("/profile/value").unwrap_or(&Value::Null),
            ctx,
        )?
    } else {
        profile(definition.get("profile"), ctx)?
    };
    let profile_sketch = if face_profile {
        None
    } else {
        super::profile::whole_region_sketch(feature, &sketch_id, ctx, preceding)
    };
    let sketch = ctx
        .sketches
        .get(&sketch_id)
        .ok_or("its sketch is not an earlier mapped sketch")?;
    let sketch_feature = profile_sketch.unwrap_or(sketch.feature);
    let forward = forward(definition.get("direction"), sketch.normal)?;
    let mut extent = extent(
        feature,
        definition.get("extent"),
        forward,
        ctx,
        sketch.normal,
    )?;
    if face_profile
        && (extent.contains_key("to")
            || extent.contains_key("from")
            || definition.pointer("/start/kind").and_then(Value::as_str) == Some("from_face"))
    {
        return Err("an extrude of a face to or from an object is not mapped yet".to_owned());
    }
    // The blind distance of the first side keeps the parameter that sets it, so editing the
    // parameter lengthens the extrude.
    if !extent.contains_key("through_all") && !extent.contains_key("to") {
        let first = ["/extent/side", "/extent/first"]
            .iter()
            .find_map(|path| number(definition.pointer(path)?.pointer("/termination/length")));
        if let Some(expression) =
            first.and_then(|length| ctx.owned_parameter(feature, "AlongDistance", length))
        {
            if extent.get("direction").and_then(Value::as_str) != Some("symmetric") {
                extent.insert("distance".to_owned(), json!(expression));
            }
        }
    }
    let mut start_height = 0.0;
    if let Some(offset) = start_offset(definition.get("start"))? {
        start_height = offset;
        // Both offsets are measured along the profile normal, regardless of travel.
        extent.insert("start_offset".to_owned(), json!(mm(offset)));
    }
    if definition.pointer("/start/kind").and_then(Value::as_str) == Some("from_plane") {
        // A start plane parallel to the sketch is a start offset along the sketch normal.
        let plane = definition
            .pointer("/start/plane")
            .ok_or("its starting plane is missing")?;
        let plane_normal = v3(plane.get("normal"))
            .and_then(unit)
            .ok_or("its starting plane normal is malformed")?;
        if dot(plane_normal, sketch.normal).abs() < ALONG_NORMAL {
            return Err(
                "a starting plane not parallel to the extrude sketch is not supported yet"
                    .to_owned(),
            );
        }
        let origin = v3(plane.get("origin")).ok_or("its starting plane origin is malformed")?;
        let offset: f64 = origin
            .iter()
            .zip(sketch.origin)
            .zip(sketch.normal)
            .map(|((coordinate, origin), normal)| (coordinate - origin) * normal)
            .sum();
        if offset.abs() > 1e-9 {
            start_height = offset;
            extent.insert("start_offset".to_owned(), json!(mm(offset)));
        }
    }
    if definition.pointer("/start/kind").and_then(Value::as_str) == Some("from_face") {
        let start = definition.get("start").ok_or("its start is malformed")?;
        let point = super::faces::pick(ctx, feature, start.get("face"), sketch.normal)?;
        start_height = point
            .iter()
            .zip(sketch.origin)
            .zip(sketch.normal)
            .map(|((coordinate, origin), normal)| (coordinate - origin) * normal)
            .sum();
        extent.insert("from".to_owned(), json!(point));
        if let Some(offset) = number(start.get("offset")) {
            let offset = if forward { offset } else { -offset };
            start_height += offset;
            extent.insert("start_offset".to_owned(), json!(mm(offset)));
        }
    }
    if let Some(target) = extent.get("to") {
        // SolidCraft's To Object chooses a side from the point, whereas CADIR's
        // side is explicit. Do not silently reverse travel to reach the target.
        let height = |pick: &Value| -> Option<f64> {
            let coordinates = pick.as_array()?;
            if coordinates.len() != 3 {
                return None;
            }
            coordinates
                .iter()
                .zip(sketch.origin)
                .zip(sketch.normal)
                .try_fold(0.0, |sum, ((coordinate, origin), normal)| {
                    Some(sum + (coordinate.as_f64()? - origin) * normal)
                })
        };
        let travel =
            height(target).ok_or("its terminating object point is malformed")? - start_height;
        let sign = if extent.get("direction").and_then(Value::as_str) == Some("negative") {
            -1.0
        } else {
            1.0
        };
        if travel * sign <= 0.0 {
            return Err("its terminating object is not along the selected extrude side".to_owned());
        }
    }
    if extent.contains_key("taper")
        && definition.get("inner_wire_taper").and_then(Value::as_str) == Some("same_as_outer")
    {
        return Err(
            "extrude draft with inner wires tapering as the outer wire is not supported yet"
                .to_owned(),
        );
    }
    if definition.get("solid").and_then(Value::as_bool) == Some(false) {
        return Err("sheet extrudes are not mapped yet".to_owned());
    }
    let operation = match operation(definition.get("op"))? {
        // A join before any feature built a body has nothing to join: it makes the first body,
        // which is then named like any other new body.
        "join" if !ctx.builds => "new_body",
        operation => operation,
    };
    if operation == "cut"
        && extent.get("through_all").and_then(Value::as_bool) == Some(true)
        && ctx.named_bodies.is_empty()
    {
        return Err("its through-all cut has no earlier named body; resolve its body-producing feature first".to_owned());
    }
    let end = end_of_sweep(definition, &extent, sketch, start_height, forward);
    let body_names = body_names(feature, ctx, operation);
    if !face_profile {
        let proven = match (operation, end) {
            ("new_body", Some(end)) if ctx.sole_builder => {
                match super::cap::check(definition.get("profile"), ctx, &end) {
                    Ok(proven) => proven,
                    Err(reason) => {
                        ctx.profile_mismatch = true;
                        return Err(reason);
                    }
                }
            }
            _ => false,
        };
        ctx.profile_unverified = !proven;
    }

    let id = ctx.id();
    ctx.builds = true;
    let mut out = json!({
        "id": id,
        "name": name(feature).map(str::to_owned).unwrap_or_else(|| format!("Extrude{id}")),
        "type": "extrude",
        "suppressed": false,
        "sketch": sketch_feature,
        "profiles": profiles,
        "extent": extent,
        "operation": operation,
    });
    if let (Some(names), Some(fields)) = (body_names, out.as_object_mut()) {
        fields.insert("body_names".to_owned(), json!(names));
    }
    Ok(out)
}

/// Where a one-sided blind sweep of `definition` ends, for the cap proof. `None` for every
/// other extent (drafted, symmetric, two-sided, to an object, through all).
fn end_of_sweep(
    definition: &Value,
    extent: &Map<String, Value>,
    sketch: &super::sketch::Mapped,
    start_height: f64,
    forward: bool,
) -> Option<super::cap::End> {
    if extent.contains_key("taper") || extent.contains_key("draft") {
        return None;
    }
    let side = definition.pointer("/extent/side")?;
    if definition.pointer("/extent/kind")?.as_str()? != "one_sided"
        || side.pointer("/termination/kind")?.as_str()? != "blind"
        || number(side.get("draft")).is_some_and(|draft| draft != 0.0)
    {
        return None;
    }
    let length = number(side.pointer("/termination/length"))?;
    if length <= 0.0 {
        return None;
    }
    let travel = if forward { 1.0 } else { -1.0 };
    Some(super::cap::End {
        origin: sketch.origin,
        normal: sketch.normal,
        x: sketch.x_dir,
        y: sketch.y_dir,
        height: start_height + travel * length,
        travel,
    })
}

/// Whether the extrude runs along the sketch normal (`false`: against it).
fn forward(direction: Option<&Value>, normal: [f64; 3]) -> Result<bool, String> {
    let direction = direction.unwrap_or(&Value::Null);
    match direction.get("kind").and_then(Value::as_str) {
        None | Some("profile_normal") => Ok(true),
        Some("reversed_profile_normal") => Ok(false),
        Some("explicit") => {
            let vector = v3(direction.get("vector"))
                .and_then(unit)
                .ok_or("its direction is malformed")?;
            let cos = dot(vector, normal);
            if cos >= ALONG_NORMAL {
                Ok(true)
            } else if cos <= -ALONG_NORMAL {
                Ok(false)
            } else {
                Err("an extrude oblique to its sketch is not mapped yet".to_owned())
            }
        }
        Some(other) => Err(format!("its direction is {other}")),
    }
}

/// The SolidCraft extent, with `forward` saying which way the first side runs.
fn extent(
    feature: &Value,
    extent: Option<&Value>,
    forward: bool,
    ctx: &Context,
    normal: [f64; 3],
) -> Result<Map<String, Value>, String> {
    let extent = extent.ok_or("it has no extent")?;
    let way = if forward { "positive" } else { "negative" };
    let mut out = Map::new();
    match extent.get("kind").and_then(Value::as_str) {
        Some("one_sided") => {
            let side = side(feature, extent.get("side"), ctx, normal)?;
            out.insert("direction".to_owned(), json!(way));
            put(&mut out, side, "distance");
        }
        Some("symmetric") => {
            let side = side(feature, extent.get("side"), ctx, normal)?;
            if matches!(side, Side::ToObject(..)) {
                return Err("a symmetric extrude to an object is not supported yet".to_owned());
            }
            out.insert("direction".to_owned(), json!("symmetric"));
            out.insert("convention".to_owned(), json!("total_distance"));
            put(&mut out, side, "distance");
        }
        Some("two_sided") => {
            let first = side(feature, extent.get("first"), ctx, normal)?;
            let second = side(feature, extent.get("second"), ctx, normal)?;
            // SolidCraft stores distance2 against the normal, regardless of direction.
            let (positive, negative) = if forward {
                (first, second)
            } else {
                (second, first)
            };
            let Side::Blind(negative) = negative else {
                return Err(
                    "a non-blind extrude end against the sketch normal on a two-sided extent \
                     is not supported yet"
                        .to_owned(),
                );
            };
            out.insert("direction".to_owned(), json!("positive"));
            put(&mut out, positive, "distance");
            out.insert("distance2".to_owned(), json!(mm(negative)));
        }
        Some(other) => return Err(format!("its extent is {other}")),
        None => return Err("its extent is malformed".to_owned()),
    }
    for side in ["side", "first", "second"]
        .iter()
        .filter_map(|key| extent.get(*key))
    {
        if let Some(draft) = number(side.get("draft")).filter(|draft| *draft != 0.0) {
            if extent.get("kind").and_then(Value::as_str) != Some("one_sided") {
                return Err(
                    "draft on a symmetric or two-sided extrude is not supported yet".to_owned(),
                );
            }
            // Both angles are measured outward along travel. SolidCraft's positive
            // taper grows the section, including when travel opposes the normal.
            out.insert(
                "taper".to_owned(),
                json!(format!("{} deg", draft.to_degrees())),
            );
        }
    }
    Ok(out)
}

/// One side's travel, as far as SolidCraft can follow it.
#[derive(Clone, Copy)]
enum Side {
    Blind(f64),
    ThroughAll,
    ToObject([f64; 3], Option<f64>),
}

fn side(
    feature: &Value,
    side: Option<&Value>,
    ctx: &Context,
    normal: [f64; 3],
) -> Result<Side, String> {
    let side = side.ok_or("its extent has no side")?;
    let termination = side.get("termination").unwrap_or(&Value::Null);
    match termination.get("kind").and_then(Value::as_str) {
        Some("blind") => number(termination.get("length"))
            .filter(|length| *length > 0.0)
            .map(Side::Blind)
            .ok_or_else(|| "its blind length is malformed".to_owned()),
        Some("through_all") => Ok(Side::ThroughAll),
        Some("to_face") => Ok(Side::ToObject(
            super::faces::pick(ctx, feature, termination.get("face"), normal)?,
            number(termination.get("offset")),
        )),
        Some("to_plane") => {
            let plane = termination.get("plane").ok_or("its terminating plane is missing")?;
            let plane_normal = v3(plane.get("normal"))
                .and_then(unit)
                .ok_or("its terminating plane normal is malformed")?;
            if dot(plane_normal, normal).abs() < ALONG_NORMAL {
                return Err(
                    "a terminating plane not parallel to the extrude sketch is not supported yet"
                        .to_owned(),
                );
            }
            // A free point works before any body exists. Later, SolidCraft may
            // reinterpret it as a face pick. Final-body faces cannot prove that
            // the same face exists at this point of the timeline, unless the plane is
            // itself a face's surface: then a pick of that face ends at the same plane.
            let on_face = termination.get("source").and_then(Value::as_str) == Some("face_surface");
            if ctx.builds && !on_face {
                return Err(
                    "a to-plane extrude ending at a work plane or point after earlier geometry is not supported yet: CADIR does not supply the input face geometry needed to distinguish a plane point from an existing body face"
                        .to_owned(),
                );
            }
            let point = v3(plane.get("origin"))
                .ok_or("its terminating plane origin is malformed")?;
            Ok(Side::ToObject(point, None))
        }
        Some("to_vertex") => Err(
            "a to-vertex extrude needs a resolved input vertex position; native, generated, and historical vertex selections are not mapped yet"
                .to_owned(),
        ),
        Some(other) => Err(format!("{other} extrude ends are not mapped yet")),
        None => Err("its extent is malformed".to_owned()),
    }
}

fn put(out: &mut Map<String, Value>, side: Side, key: &str) {
    match side {
        Side::Blind(length) => {
            out.insert(key.to_owned(), json!(mm(length)));
        }
        Side::ToObject(point, offset) => {
            out.insert(key.to_owned(), json!("0 mm"));
            out.insert("to".to_owned(), json!(point));
            if let Some(offset) = offset {
                out.insert("to_offset".to_owned(), json!(mm(offset)));
            }
        }
        Side::ThroughAll => {
            out.insert(key.to_owned(), json!("0 mm"));
            out.insert("through_all".to_owned(), json!(true));
        }
    }
}

/// The start offset from the sketch plane, if any.
fn start_offset(start: Option<&Value>) -> Result<Option<f64>, String> {
    let start = start.unwrap_or(&Value::Null);
    match start.get("kind").and_then(Value::as_str) {
        None | Some("profile_plane") | Some("from_face") | Some("from_plane") => Ok(None),
        Some("offset_profile_plane") => number(start.get("offset"))
            .map(|offset| (offset != 0.0).then_some(offset))
            .ok_or_else(|| "its start offset is malformed".to_owned()),
        Some(other) => Err(format!("{other} extrude starts are not mapped yet")),
    }
}

/// SolidCraft's operation for a CADIR boolean.
pub(super) fn operation(op: Option<&Value>) -> Result<&'static str, String> {
    match op.and_then(Value::as_str) {
        Some("new_body") => Ok("new_body"),
        Some("join") => Ok("join"),
        Some("cut") => Ok("cut"),
        Some("intersect") => Ok("intersect"),
        Some(other) => Err(format!("its boolean is {other}")),
        None => Err("its boolean is malformed".to_owned()),
    }
}

/// The names of the bodies a new-body extrude makes. A body the source leaves unnamed gets
/// "Body N", the first number not yet taken, so later features can still target it.
///
/// Each named body's colour is recorded in `ctx.looks` under its name, so the replayed body
/// keeps the colour the source gave it.
pub(super) fn body_names(
    feature: &Value,
    ctx: &mut Context,
    operation: &str,
) -> Option<Vec<String>> {
    if operation != "new_body" {
        return None;
    }
    let outputs = feature.get("outputs").and_then(Value::as_array)?;
    if outputs.is_empty() {
        return None;
    }
    let rows: Vec<&Value> = outputs
        .iter()
        .map(|id| ctx.row("bodies", id.as_str()?))
        .collect::<Option<_>>()?;
    let mut taken: std::collections::BTreeSet<String> = ctx
        .named_bodies
        .iter()
        .map(|(_, name)| name.clone())
        .collect();
    let given: Vec<Option<String>> = rows
        .iter()
        .map(|row| {
            row.get("name")
                .and_then(Value::as_str)
                .filter(|name| !name.trim().is_empty())
                .map(str::to_owned)
        })
        .collect();
    taken.extend(given.iter().flatten().cloned());
    let mut next = 1;
    let names: Vec<String> = given
        .into_iter()
        .map(|name| {
            name.unwrap_or_else(|| loop {
                let candidate = format!("Body {next}");
                next += 1;
                if taken.insert(candidate.clone()) {
                    break candidate;
                }
            })
        })
        .collect();
    for (row, name) in rows.iter().zip(&names) {
        if let Some(look) = row.get("color").and_then(look) {
            ctx.looks.insert(name.clone(), look);
        }
        if let Some(id) = row.get("id").and_then(Value::as_str) {
            ctx.named_bodies.push((id.to_owned(), name.clone()));
        }
    }
    Some(names)
}

/// A CADIR colour (`{r, g, b, a}`, 0 to 1) as a SolidCraft look.
pub(super) fn look(color: &Value) -> Option<Value> {
    let channel = |key: &str| number(color.get(key)).filter(|c| (0.0..=1.0).contains(c));
    let rgb = [channel("r")?, channel("g")?, channel("b")?].map(|c| (c * 255.0).round() as u8);
    let opacity = channel("a").unwrap_or(1.0);
    Some(json!({
        "name": format!("#{:02x}{:02x}{:02x}", rgb[0], rgb[1], rgb[2]),
        "color": rgb,
        "opacity": opacity,
    }))
}

/// A length as a SolidCraft expression.
fn mm(length: f64) -> String {
    format!("{length} mm")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn extent(extent: Option<&Value>, forward: bool) -> Result<Map<String, Value>, String> {
        super::extent(
            &Value::Null,
            extent,
            forward,
            &Context::new(&Value::Null),
            [0.0, 0.0, 1.0],
        )
    }

    fn blind(length: f64) -> Value {
        json!({"termination": {"kind": "blind", "length": length}})
    }

    #[test]
    fn directions_follow_or_oppose_the_sketch_normal() {
        let n = [0.0, 0.0, 1.0];
        assert_eq!(forward(None, n), Ok(true));
        assert_eq!(
            forward(Some(&json!({"kind": "profile_normal"})), n),
            Ok(true)
        );
        assert_eq!(
            forward(Some(&json!({"kind": "reversed_profile_normal"})), n),
            Ok(false)
        );
        let explicit =
            |z: f64, x: f64| json!({"kind": "explicit", "vector": {"x": x, "y": 0.0, "z": z}});
        assert_eq!(forward(Some(&explicit(-2.0, 0.0)), n), Ok(false));
        assert!(forward(Some(&explicit(1.0, 1.0)), n).is_err());
        assert!(forward(Some(&json!({"kind": "unresolved"})), n).is_err());
    }

    #[test]
    fn extents_map_to_solidcraft_extents() {
        let one = json!({"kind": "one_sided", "side": blind(5.0)});
        assert_eq!(
            Value::Object(extent(Some(&one), false).unwrap()),
            json!({"direction": "negative", "distance": "5 mm"})
        );
        let symmetric = json!({"kind": "symmetric", "side": blind(8.0)});
        assert_eq!(
            Value::Object(extent(Some(&symmetric), true).unwrap()),
            json!({"direction": "symmetric", "convention": "total_distance", "distance": "8 mm"})
        );
        let two = json!({"kind": "two_sided", "first": blind(3.0), "second": blind(2.0)});
        assert_eq!(
            Value::Object(extent(Some(&two), true).unwrap()),
            json!({"direction": "positive", "distance": "3 mm", "distance2": "2 mm"})
        );
    }

    #[test]
    fn unsupported_extents_stop_the_history() {
        let drafted = json!({"kind": "one_sided",
            "side": {"termination": {"kind": "blind", "length": 5.0}, "draft": 0.1}});
        assert!(extent(Some(&drafted), true).unwrap().contains_key("taper"));
        let to_face = json!({"kind": "one_sided", "side": {"termination": {"kind": "to_face"}}});
        assert!(extent(Some(&to_face), true).is_err());
        let mixed = json!({"kind": "two_sided", "first": blind(3.0),
            "second": {"termination": {"kind": "through_all"}}});
        assert!(extent(Some(&mixed), true).is_err());
    }

    #[test]
    fn colours_become_looks() {
        assert_eq!(
            look(&json!({"r": 1.0, "g": 0.5, "b": 0.0, "a": 0.25})),
            Some(json!({"name": "#ff8000", "color": [255, 128, 0], "opacity": 0.25}))
        );
        assert_eq!(look(&json!({"r": 2.0, "g": 0.5, "b": 0.0})), None);
    }

    #[test]
    fn starts_and_booleans() {
        assert_eq!(start_offset(None), Ok(None));
        assert_eq!(
            start_offset(Some(&json!({"kind": "profile_plane"}))),
            Ok(None)
        );
        assert_eq!(
            start_offset(Some(
                &json!({"kind": "offset_profile_plane", "offset": 2.5})
            )),
            Ok(Some(2.5))
        );
        assert_eq!(start_offset(Some(&json!({"kind": "from_face"}))), Ok(None));
        for op in ["new_body", "join", "cut", "intersect"] {
            assert_eq!(operation(Some(&json!(op))), Ok(op));
        }
        assert!(operation(Some(&json!("unresolved"))).is_err());
    }
}
