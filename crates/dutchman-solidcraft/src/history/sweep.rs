//! Sweeps, as SolidCraft sweep features.
//!
//! A solid or surface sweep of one sketch profile along planar sketch curves maps with
//! rotation-minimizing, fixed or fixed-binormal orientation, optional twist and end scale,
//! or one guide rail. A whole-sketch path or rail maps when it has one curve. Taper, explicit
//! corner transitions, several sections, partial paths, tangent extension, simplification
//! and other orientation or profile interpolation laws are not mapped.

use serde_json::{json, Map, Value};

use super::extrude::{body_names, operation};
use super::profile::profile;
use super::sketch::number;
use super::{name, Context};

/// How far from the whole path (as a fraction of it) a sweep's extent may be and still be it.
const WHOLE: f64 = 1e-9;

/// Maps a sweep feature whose operation is `definition`.
pub(crate) fn map(feature: &Value, definition: &Value, ctx: &mut Context) -> Result<Value, String> {
    let shape = definition.get("shape").unwrap_or(&Value::Null);
    let surface = match shape.get("mode").and_then(Value::as_str) {
        Some("solid") => false,
        Some("surface") => true,
        _ => return Err("its swept shape is not resolved".to_owned()),
    };
    if shape
        .get("sections")
        .and_then(Value::as_array)
        .is_some_and(|sections| !sections.is_empty())
    {
        return Err("sweeps through several sections are not mapped yet".to_owned());
    }
    let section = shape.get("section").unwrap_or(&Value::Null);
    match section.get("kind").and_then(Value::as_str) {
        Some("profile") => {}
        Some("generated") => {
            return Err("sweeps with generated circular sections are not mapped yet".to_owned())
        }
        _ => return Err("its sweep section is not resolved".to_owned()),
    }
    if definition.get("path_tangent").and_then(Value::as_bool) == Some(true) {
        return Err(
            "sweeps that extend the path through tangent-connected edges are not mapped yet"
                .to_owned(),
        );
    }
    if definition.get("linearize").and_then(Value::as_bool) == Some(true) {
        return Err(
            "sweeps that simplify linear edges and planar faces are not mapped yet".to_owned(),
        );
    }
    let (sketch_id, profiles) = profile(section.get("value"), ctx)?;
    let sketch = ctx
        .sketches
        .get(&sketch_id)
        .ok_or("its sketch is not an earlier mapped sketch")?
        .feature;

    let (path_sketch, path) = curves(definition.get("path"), ctx, "path")?;
    whole(definition.get("path_extent"), "path")?;

    let mut extra = Map::new();
    match definition
        .pointer("/orientation/kind")
        .and_then(Value::as_str)
    {
        None | Some("corrected_frenet") => {}
        Some("fixed") => {
            extra.insert("orientation".to_owned(), json!("parallel"));
        }
        Some("binormal") => {
            let direction = definition.pointer("/orientation/direction");
            let coordinates = ["x", "y", "z"]
                .map(|axis| number(direction.and_then(|direction| direction.get(axis))));
            let [Some(x), Some(y), Some(z)] = coordinates else {
                return Err("its sweep binormal direction is not resolved".to_owned());
            };
            extra.insert("orientation".to_owned(), json!({"direction": [x, y, z]}));
        }
        Some("auxiliary") => {
            return Err("sweeps oriented by an auxiliary path are not mapped yet".to_owned())
        }
        Some("guide_surface") => {
            return Err("sweeps oriented by guide faces are not mapped yet".to_owned())
        }
        Some(other) => {
            return Err(format!(
                "sweeps oriented by {} are not mapped yet",
                other.replace('_', " ")
            ))
        }
    }
    match definition.get("transition").and_then(Value::as_str) {
        None | Some("transformed") => {}
        Some(transition) => {
            return Err(format!(
                "sweeps with {} corner transitions are not mapped yet",
                transition.trim_end_matches("_corner").replace('_', " ")
            ))
        }
    }
    match definition.get("transformation").and_then(Value::as_str) {
        None | Some("constant") => {}
        Some(transformation) => {
            let law = match transformation {
                "multi_section" => "multiple",
                "s_shape" => "S-shaped",
                "interpolation" => "native smooth",
                other => other,
            };
            return Err(format!(
                "sweeps with {law} section interpolation are not mapped yet"
            ));
        }
    }
    if number(definition.get("taper")).is_some_and(|taper| taper != 0.0) {
        return Err("tapered sweeps are not mapped yet".to_owned());
    }
    if let Some(scale) = number(definition.get("scale")).filter(|scale| *scale != 1.0) {
        if !(0.01..=100.0).contains(&scale) {
            return Err("sweep end scales outside 0.01 to 100 are not mapped yet".to_owned());
        }
        extra.insert("scale".to_owned(), json!(scale.to_string()));
    }
    if let Some(twist) = number(definition.get("twist")).filter(|twist| *twist != 0.0) {
        if twist.abs() > 4.0 * std::f64::consts::TAU {
            return Err("sweep twists beyond four turns are not mapped yet".to_owned());
        }
        extra.insert(
            "twist".to_owned(),
            json!(format!("{} deg", twist.to_degrees())),
        );
    }
    if let Some(rail) = definition.get("guide_rail").filter(|rail| !rail.is_null()) {
        if ["orientation", "twist", "scale"]
            .iter()
            .any(|key| extra.contains_key(*key))
        {
            return Err(
                "sweeps combining a guide rail with orientation, twist or scale are not mapped yet"
                    .to_owned(),
            );
        }
        let (rail_sketch, rail_curves) = curves(rail.get("path"), ctx, "guide rail")?;
        whole(rail.get("extent"), "guide rail")?;
        extra.insert(
            "rails".to_owned(),
            json!([{"sketch": rail_sketch, "curves": rail_curves}]),
        );
    }
    let operation = if surface {
        "new_body"
    } else {
        operation(shape.get("op"))?
    };
    let body_names = body_names(feature, ctx, operation);

    let id = ctx.id();
    ctx.builds = true;
    let mut out = json!({
        "id": id,
        "name": name(feature).map(str::to_owned).unwrap_or_else(|| format!("Sweep{id}")),
        "type": if surface { "surface_sweep" } else { "sweep" },
        "suppressed": false,
        "sketch": sketch,
        "profiles": profiles,
        "path_sketch": path_sketch,
        "path": path,
        "operation": operation,
    });
    if let Some(fields) = out.as_object_mut() {
        if surface {
            fields.remove("operation");
        }
        fields.extend(extra);
        if let Some(names) = body_names {
            fields.insert("body_names".to_owned(), json!(names));
        }
    }
    Ok(out)
}

/// The SolidCraft sketch feature and curve ids a path of sketch curves uses; `what` names it
/// in the reason when it cannot be mapped.
pub(super) fn curves(
    path: Option<&Value>,
    ctx: &Context,
    what: &str,
) -> Result<(u64, Vec<String>), String> {
    let path = path.ok_or_else(|| format!("it has no {what}"))?;
    // A whole sketch with one non-construction curve has an unambiguous path. For
    // several curves, the sketch's entity table does not establish traversal order.
    if path.get("kind").and_then(Value::as_str) == Some("sketch") {
        let sketch = path
            .get("value")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("its {what} sketch is not resolved"))?;
        let mapped = ctx
            .sketches
            .get(sketch)
            .ok_or_else(|| format!("its {what} sketch is not an earlier mapped sketch"))?;
        let mut entities = ctx.arena("sketch_entities").iter().filter(|entity| {
            entity.get("sketch").and_then(Value::as_str) == Some(sketch)
                && entity.get("construction").and_then(Value::as_bool) != Some(true)
                && entity.pointer("/geometry/kind").and_then(Value::as_str) != Some("point")
        });
        let entity = entities
            .next()
            .ok_or_else(|| format!("its {what} sketch has no path curves"))?;
        if entities.next().is_some() {
            return Err(format!(
                "a whole-sketch {what} with several curves has no resolved traversal order"
            ));
        }
        let curve = entity
            .get("id")
            .and_then(Value::as_str)
            .and_then(|id| mapped.curve(id))
            .ok_or_else(|| format!("its {what} uses a curve the sketch does not carry"))?;
        return Ok((mapped.feature, curve.ids()));
    }
    match path.get("kind").and_then(Value::as_str) {
        Some("sketch_curves") => {}
        Some("spatial_sketch_curves") => return spatial_curves(path.get("value"), ctx, what),
        Some("spatial_sketch_selection") => {
            return Err(format!(
                "a {what} in a 3D sketch given by native selection records is not mapped yet"
            ))
        }
        Some("unresolved") => return Err(format!("its {what} is not resolved")),
        Some("native") => {
            return Err(format!(
                "a {what} given by a native reference is not mapped yet"
            ))
        }
        Some("edges" | "historical_edges") => {
            return Err(format!("a {what} of topological edges is not mapped yet"))
        }
        Some("curves") => {
            return Err(format!(
                "a {what} of geometric curves outside a sketch is not mapped yet"
            ))
        }
        _ => return Err(format!("its {what} is not a set of sketch curves")),
    }
    let value = path.get("value").unwrap_or(&Value::Null);
    let sketch = value
        .get("sketch")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("its {what} is malformed"))?;
    let mapped = ctx
        .sketches
        .get(sketch)
        .ok_or_else(|| format!("its {what} sketch is not an earlier mapped sketch"))?;
    let ids = value
        .get("curves")
        .and_then(Value::as_array)
        .filter(|curves| !curves.is_empty())
        .ok_or_else(|| format!("its {what} is malformed"))?
        .iter()
        .map(|entity| {
            entity
                .as_str()
                .and_then(|entity| mapped.curve(entity))
                .map(|curve| curve.ids())
                .ok_or_else(|| format!("its {what} uses a curve the sketch does not carry"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok((mapped.feature, ids.into_iter().flatten().collect()))
}

/// The SolidCraft sketch feature and wire ids of curves picked in a 3D sketch.
fn spatial_curves(
    value: Option<&Value>,
    ctx: &Context,
    what: &str,
) -> Result<(u64, Vec<String>), String> {
    let value = value.unwrap_or(&Value::Null);
    let sketch = value
        .get("sketch")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("its {what} is malformed"))?;
    let mapped = ctx
        .spatial
        .get(sketch)
        .ok_or_else(|| format!("its {what} sketch is not an earlier mapped 3D sketch"))?;
    let mut ids = Vec::new();
    for entity in value
        .get("curves")
        .and_then(Value::as_array)
        .filter(|curves| !curves.is_empty())
        .ok_or_else(|| format!("its {what} is malformed"))?
    {
        let wires = entity
            .as_str()
            .and_then(|entity| mapped.wires.get(entity))
            .ok_or_else(|| format!("its {what} uses a curve the 3D sketch does not carry"))?;
        ids.extend(wires.iter().cloned());
    }
    Ok((mapped.feature, ids))
}

/// Refuses an extent that is not the whole of a path or rail.
fn whole(extent: Option<&Value>, what: &str) -> Result<(), String> {
    let Some(extent) = extent.filter(|extent| !extent.is_null()) else {
        return Ok(());
    };
    let along = number(extent.get("along_fraction")).unwrap_or(1.0);
    let against = number(extent.get("against_fraction")).unwrap_or(0.0);
    if (along - 1.0).abs() > WHOLE || against.abs() > WHOLE {
        return Err(format!(
            "sweeps along part of their {what} are not mapped yet"
        ));
    }
    Ok(())
}
