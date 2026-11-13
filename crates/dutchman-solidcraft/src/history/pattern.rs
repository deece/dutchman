//! Patterns of features or named bodies, as SolidCraft patterns.
//!
//! A linear pattern (in one or two directions) maps to a rectangular one. A circular pattern
//! uses the same total span and instance count: partial spans include both ends, while a
//! full turn spaces the instances around the circle without repeating the original.

use std::f64::consts::TAU;

use serde_json::{json, Value};

use super::sketch::{cross, dot, number, unit, v3};
use super::{name, Context};

/// How far from a full turn (radians) a circular pattern may be and still be one.
const FULL_TURN: f64 = 1e-9;

/// Bound the work a pattern asks SolidCraft to replay.
const MAX_INSTANCES: u64 = 10_000;
/// Bound feature/body seed lists, including blends expanded after replay.
pub(super) const MAX_SEEDS: usize = 10_000;

/// Maps a pattern feature whose operation is `definition`.
pub(crate) fn map(
    feature: &Value,
    definition: &Value,
    ctx: &mut Context,
    preceding: &mut Vec<Value>,
) -> Result<Value, String> {
    let unresolved =
        definition.pointer("/pattern/kind").and_then(Value::as_str) == Some("unresolved");
    let seeds = definition
        .get("seeds")
        .and_then(Value::as_array)
        .filter(|seeds| !seeds.is_empty())
        .ok_or(if unresolved {
            "cadmpeg does not decode what it repeats or how yet"
        } else {
            "it patterns nothing"
        })?;
    if seeds.len() > MAX_SEEDS {
        return Err("patterns with more than 10000 seeds are not supported yet".to_owned());
    }
    let mut features = Vec::with_capacity(seeds.len());
    let mut bodies = Vec::new();
    for seed in seeds {
        match seed.get("kind").and_then(Value::as_str) {
            Some("feature") => {
                let id = seed
                    .get("value")
                    .and_then(Value::as_str)
                    .ok_or("its seed is malformed")?;
                let name = ctx
                    .feature_names
                    .get(id)
                    .ok_or("it patterns a feature that did not map")?;
                if let Some(group) = ctx.pattern_features.get(id) {
                    features.extend(group.iter().cloned());
                } else {
                    features.push(name.clone());
                }
                if features.len() > MAX_SEEDS {
                    return Err("patterns with more than 10000 seeds are not supported yet".to_owned());
                }
            }
            Some("bodies") => {
                bodies.extend(body_seeds(seed.get("value"), ctx)?);
                if bodies.len() > MAX_SEEDS {
                    return Err("patterns with more than 10000 body seeds are not supported yet".to_owned());
                }
            }
            Some("faces") => {
                return Err("patterns of faces are not supported yet: SolidCraft patterns repeat whole features or bodies".to_owned())
            }
            Some("occurrences") => {
                return Err("patterns of occurrences are not supported yet: occurrence selections do not identify SolidCraft component definitions".to_owned())
            }
            Some(other)
                if seed.pointer("/value/kind").and_then(Value::as_str) == Some("native") =>
            {
                return Err(format!(
                    "patterns of {other} are not mapped yet: cadmpeg does not resolve which"
                ))
            }
            Some(other) => return Err(format!("patterns of {other} are not mapped yet")),
            None => return Err("its seed is malformed".to_owned()),
        }
    }
    if !features.is_empty() && !bodies.is_empty() {
        return Err("patterns mixing feature and body seeds are not supported yet: SolidCraft requires one seed kind".to_owned());
    }
    // Older SolidCraft cannot use a pattern or mirror feature as a feature seed.
    if preceding.iter().any(|mapped| {
        matches!(
            mapped.get("type").and_then(Value::as_str),
            Some("pattern" | "mirror")
        ) && mapped
            .get("name")
            .and_then(Value::as_str)
            .is_some_and(|name| features.iter().any(|seed| seed == name))
    }) {
        require_chain_version(ctx)?;
    }
    let pattern = definition.get("pattern");
    if pattern.and_then(|p| p.get("kind")).and_then(Value::as_str) == Some("composite")
        && pattern
            .and_then(|p| p.get("stages"))
            .and_then(Value::as_array)
            .is_some_and(|stages| {
                stages.iter().any(|stage| {
                    stage.pointer("/pattern/kind").and_then(Value::as_str) == Some("mirror")
                })
            })
    {
        if !bodies.is_empty() {
            return Err("composite mirror patterns of bodies are not supported yet: intermediate copies have no mapped body names".to_owned());
        }
        return chain(feature, pattern, features, ctx, preceding);
    }
    if pattern.and_then(|p| p.get("kind")).and_then(Value::as_str) == Some("mirror") {
        if !bodies.is_empty() {
            return Err("mirrors of bodies are not mapped yet".to_owned());
        }
        let plane = mirror_plane(pattern)?;
        let id = ctx.id();
        ctx.builds = true;
        return Ok(json!({
            "id": id,
            "name": name(feature).map(str::to_owned).unwrap_or_else(|| format!("Mirror{id}")),
            "type": "mirror",
            "suppressed": false,
            "features": features,
            "plane": plane,
        }));
    }
    let layout = mapped_layout(pattern, ctx)?;
    if repeated_endpoint(pattern) {
        ctx.pattern_replay_required = true;
    }
    let id = ctx.id();
    ctx.builds = true;
    let mut out = json!({
        "id": id,
        "name": name(feature).map(str::to_owned).unwrap_or_else(|| format!("Pattern{id}")),
        "type": "pattern",
        "suppressed": false,
        "features": features,
        "pattern": layout,
    });
    if !bodies.is_empty() {
        if let Some(fields) = out.as_object_mut() {
            fields.insert("bodies".to_owned(), json!(bodies));
        }
    }
    Ok(out)
}

/// Each stage adds copies of the whole instance set produced so far. A pattern feature
/// selects only its added copies, so the next stage also needs the original seed names.
fn chain(
    feature: &Value,
    pattern: Option<&Value>,
    mut features: Vec<String>,
    ctx: &mut Context,
    preceding: &mut Vec<Value>,
) -> Result<Value, String> {
    let stages = pattern
        .and_then(|pattern| pattern.get("stages"))
        .and_then(Value::as_array)
        .ok_or("its pattern stages are malformed")?;
    if stages.is_empty() {
        return Err("its pattern stages are empty".to_owned());
    }
    if stages.len() > 32 {
        return Err("composite patterns with more than 32 stages are not supported yet".to_owned());
    }
    if stages.len() > 1 {
        require_chain_version(ctx)?;
    }
    let mut mapped = Vec::with_capacity(stages.len());
    let mut instances = 1_u64;
    for (index, stage) in stages.iter().enumerate() {
        let pattern = stage.get("pattern");
        let id = ctx.id();
        let base = name(feature).unwrap_or("Pattern");
        let (mut out, count) =
            if pattern.and_then(|p| p.get("kind")).and_then(Value::as_str) == Some("mirror") {
                (
                    json!({"id": id, "type": "mirror", "suppressed": false,
                "features": features, "plane": mirror_plane(pattern)?}),
                    2,
                )
            } else {
                let layout = mapped_layout(pattern, ctx)?;
                let count = |field: &str| {
                    layout
                        .get(field)
                        .and_then(Value::as_str)
                        .and_then(|s| s.parse::<u64>().ok())
                        .unwrap_or(1)
                };
                let count = count("count")
                    .saturating_mul(count("count1"))
                    .saturating_mul(count("count2"));
                (
                    json!({"id": id, "type": "pattern", "suppressed": false,
                "features": features, "pattern": layout}),
                    count,
                )
            };
        instances = instances.saturating_mul(count);
        if instances > MAX_INSTANCES || features.len() > MAX_SEEDS {
            return Err("patterns with more than 10000 instances are not supported yet".to_owned());
        }
        let stage_name = if index + 1 == stages.len() {
            base.to_owned()
        } else {
            format!("{base} stage {}", index + 1)
        };
        if let Some(fields) = out.as_object_mut() {
            fields.insert("name".to_owned(), json!(stage_name));
        }
        let unique = ctx.unique_name(&out, preceding.iter().chain(mapped.iter()));
        if let Some(fields) = out.as_object_mut() {
            fields.insert("name".to_owned(), json!(unique));
        }
        features.push(unique);
        mapped.push(out);
    }
    let last = mapped.pop().ok_or("its pattern stages are empty")?;
    if let Some(id) = feature.get("id").and_then(Value::as_str) {
        ctx.pattern_features.insert(id.to_owned(), features);
    }
    preceding.extend(mapped);
    ctx.builds = true;
    ctx.pattern_replay_required = true;
    Ok(last)
}

fn require_chain_version(ctx: &Context) -> Result<(), String> {
    if !crate::VersionGate::MirrorChain.supported(ctx.target_version) {
        return Err(format!(
            "needs SolidCraft 0.25 patch 1 or newer (target {})",
            ctx.target_version.minor()
        ));
    }
    Ok(())
}

fn repeated_endpoint(pattern: Option<&Value>) -> bool {
    pattern.is_some_and(|pattern| {
        pattern.get("kind").and_then(Value::as_str) == Some("circular_angles")
            && pattern
                .get("angles")
                .and_then(Value::as_array)
                .and_then(|angles| number(angles.last()))
                .is_some_and(|last| (last - TAU).abs() <= FULL_TURN)
    })
}

/// Resolve path references as well as the layout's numeric fields.
fn mapped_layout(pattern: Option<&Value>, ctx: &Context) -> Result<Value, String> {
    let pattern = pattern.ok_or("it has no layout")?;
    match pattern.get("kind").and_then(Value::as_str) {
        Some("curve_driven") => path_layout(Some(pattern), ctx),
        Some("composite") => {
            let stages = pattern
                .get("stages")
                .and_then(Value::as_array)
                .ok_or("its pattern stages are malformed")?;
            if stages.len() == 1 {
                let stage = stages.first().and_then(|stage| stage.get("pattern"));
                if stage
                    .and_then(|stage| stage.get("kind"))
                    .and_then(Value::as_str)
                    == Some("curve_driven")
                {
                    return path_layout(stage, ctx);
                }
            }
            composite_layout(pattern)
        }
        _ => layout(Some(pattern)),
    }
}

/// Resolve only bodies whose earlier history actually assigned a SolidCraft name.
fn body_seeds(selection: Option<&Value>, ctx: &Context) -> Result<Vec<String>, String> {
    let selection = selection.ok_or("its body selection is malformed")?;
    let value = selection.get("value").unwrap_or(&Value::Null);
    let ids: Vec<&Value> = match selection.get("kind").and_then(Value::as_str) {
        Some("bodies") => value.as_array().map(|rows| rows.iter().collect()),
        Some("resolved") => value
            .get("bodies")
            .and_then(Value::as_array)
            .map(|rows| rows.iter().collect()),
        Some("resolved_set") => value
            .get("members")
            .and_then(Value::as_array)
            .map(|rows| rows.iter().filter_map(|row| row.get("body")).collect()),
        Some("historical" | "historical_set") => {
            return Err("patterns of historical bodies are not supported yet: their input topology has no mapped body names".to_owned())
        }
        Some("generated" | "local") => {
            return Err("patterns of feature-local bodies are not supported yet: their persistent identities have no mapped body names".to_owned())
        }
        Some("native" | "native_set" | "unresolved") => {
            return Err("patterns of bodies are not supported yet: cadmpeg does not resolve which bodies".to_owned())
        }
        _ => return Err("its body selection is malformed".to_owned()),
    }
    .filter(|ids: &Vec<&Value>| !ids.is_empty())
    .ok_or("its body selection is empty")?;
    if ids.len() > 10_000 {
        return Err("patterns with more than 10000 body seeds are not supported yet".to_owned());
    }
    ids.into_iter()
        .map(|id| {
            let id = id.as_str().ok_or("its body selection is malformed")?;
            let (_, name) = ctx
                .named_bodies
                .iter()
                .find(|(known, _)| known == id)
                .ok_or("it patterns a body without an earlier mapped name")?;
            if ctx
                .named_bodies
                .iter()
                .any(|(known, other)| known != id && other == name)
            {
                return Err("it patterns a body whose name is ambiguous".to_owned());
            }
            Ok(name.clone())
        })
        .collect()
}

/// Fixed arc-length copies keep their seed orientation; CADIR carries no orientation law.
fn path_layout(pattern: Option<&Value>, ctx: &Context) -> Result<Value, String> {
    let pattern = pattern.ok_or("it has no layout")?;
    let path = pattern
        .get("path")
        .ok_or("its pattern path is not resolved")?;
    let kind = path
        .get("kind")
        .and_then(Value::as_str)
        .unwrap_or("unresolved");
    let (sketch, curves) = if kind == "sketch" {
        let id = path
            .get("value")
            .and_then(Value::as_str)
            .ok_or("its pattern path sketch is malformed")?;
        // A whole sketch supplies its neutral entity order, without construction geometry.
        let curves: Vec<Value> = ctx
            .arena("sketch_entities")
            .iter()
            .filter(|row| row.get("sketch").and_then(Value::as_str) == Some(id))
            .filter(|row| row.get("construction").and_then(Value::as_bool) != Some(true))
            .filter_map(|row| {
                let entity = row.get("id")?.as_str()?;
                ctx.sketches.get(id)?.curve(entity)?;
                Some(json!(entity))
            })
            .collect();
        let selection = json!({"kind": "sketch_curves", "value": {"sketch": id, "curves": curves}});
        super::sweep::curves(Some(&selection), ctx, "pattern path")?
    } else if kind == "sketch_curves" {
        super::sweep::curves(Some(path), ctx, "pattern path")?
    } else {
        return Err(format!("pattern paths from {} are not supported yet: SolidCraft requires curves of an earlier mapped planar sketch", kind.replace('_', " ")));
    };
    let spacing = number(pattern.get("spacing"))
        .filter(|spacing| *spacing > 0.0)
        .ok_or("its spacing is malformed")?;
    Ok(
        json!({"type": "path", "path_sketch": sketch, "path": curves,
        "count": instance_count(pattern.get("count"))?.to_string(),
        "spacing": format!("{spacing} mm")}),
    )
}

fn instance_count(value: Option<&Value>) -> Result<u64, String> {
    let count = value
        .and_then(Value::as_u64)
        .filter(|count| *count >= 1)
        .ok_or("its count is malformed")?;
    if count > MAX_INSTANCES {
        return Err("patterns with more than 10000 instances are not supported yet".to_owned());
    }
    Ok(count)
}

/// The SolidCraft plane of a CADIR mirror: through its point, square to its normal. Which two
/// directions span it does not matter to a reflection, so any pair square to the normal does.
fn mirror_plane(pattern: Option<&Value>) -> Result<Value, String> {
    let pattern = pattern.ok_or("it has no layout")?;
    let origin = v3(pattern.get("plane_origin")).ok_or("its mirror plane is malformed")?;
    let normal = v3(pattern.get("plane_normal"))
        .and_then(unit)
        .ok_or("its mirror plane is malformed")?;
    let away = if normal[0].abs() < 0.9 {
        [1.0, 0.0, 0.0]
    } else {
        [0.0, 1.0, 0.0]
    };
    let along = dot(away, normal);
    let x = unit([
        away[0] - along * normal[0],
        away[1] - along * normal[1],
        away[2] - along * normal[2],
    ])
    .ok_or("its mirror plane is malformed")?;
    let y = cross(normal, x);
    Ok(json!({"type": "custom", "plane": {"origin": origin, "x_dir": x, "y_dir": y}}))
}

/// The SolidCraft pattern layout for a CADIR pattern transform.
fn layout(pattern: Option<&Value>) -> Result<Value, String> {
    let pattern = pattern.ok_or("it has no layout")?;
    match pattern.get("kind").and_then(Value::as_str) {
        Some("unresolved") => {
            let form = pattern.get("form").and_then(Value::as_str).unwrap_or("unknown");
            Err(format!("cadmpeg does not decode its {} pattern layout yet", form.replace('_', " ")))
        }
        Some("linear") => {
            // SolidCraft's rectangular pattern fills its whole grid; building it would add
            // the instances the source suppressed.
            if let Some(omitted) = pattern
                .get("omitted_instances")
                .and_then(Value::as_array)
                .filter(|omitted| !omitted.is_empty())
            {
                return Err(format!(
                    "patterns that skip grid positions are not supported yet: SolidCraft's rectangular pattern fills the whole grid ({} skipped)",
                    omitted.len()
                ));
            }
            let dir = v3(pattern.get("direction"))
                .and_then(unit)
                .ok_or("its direction is not resolved")?;
            let spacing = number(pattern.get("spacing"))
                .filter(|s| *s > 0.0)
                .ok_or("its spacing is malformed")?;
            let mut out = json!({
                "type": "rectangular",
                "dir1": dir,
                "count1": instance_count(pattern.get("count"))?.to_string(),
                "spacing1": format!("{spacing} mm"),
            });
            if let Some(second) = pattern.get("second").filter(|s| !s.is_null()) {
                let dir2 = v3(second.get("direction"))
                    .and_then(unit)
                    .ok_or("its second direction is malformed")?;
                let spacing2 = number(second.get("spacing"))
                    .filter(|s| *s > 0.0)
                    .ok_or("its second spacing is malformed")?;
                if let Some(fields) = out.as_object_mut() {
                    fields.insert("dir2".to_owned(), json!(dir2));
                    fields.insert(
                        "count2".to_owned(),
                        json!(instance_count(second.get("count"))?.to_string()),
                    );
                    fields.insert("spacing2".to_owned(), json!(format!("{spacing2} mm")));
                }
            }
            let count1 = instance_count(pattern.get("count"))?;
            let count2 = pattern
                .get("second")
                .filter(|second| !second.is_null())
                .map(|second| instance_count(second.get("count")))
                .transpose()?
                .unwrap_or(1);
            if count1.saturating_mul(count2) > MAX_INSTANCES {
                return Err("patterns with more than 10000 instances are not supported yet".to_owned());
            }
            Ok(out)
        }
        Some("linear_offsets") => {
            let (count, spacing) = regular_locations(pattern.get("offsets"), "linear offsets")?;
            layout(Some(&json!({"kind": "linear", "direction": pattern.get("direction"),
                "spacing": spacing, "count": count})))
        }
        Some("circular_angles") => {
            let (mut count, spacing) = regular_locations(pattern.get("angles"), "circular angles")?;
            let span = spacing * (count.saturating_sub(1) as f64);
            if count > 1 && (span - TAU).abs() <= FULL_TURN {
                // The endpoint coincides with the seed. SolidCraft's full-turn layout
                // already omits it, so retain just the distinct angular locations.
                count -= 1;
            }
            let angle = if count == 1 || (spacing * count as f64 - TAU).abs() <= FULL_TURN {
                TAU
            } else {
                span
            };
            layout(Some(&json!({"kind": "circular", "axis_origin": pattern.get("axis_origin"),
                "axis_dir": pattern.get("axis_dir"), "angle": angle, "count": count})))
        }
        Some("circular") => {
            let origin = v3(pattern.get("axis_origin")).ok_or("its axis is malformed")?;
            let axis = v3(pattern.get("axis_dir"))
                .and_then(unit)
                .ok_or("its axis is malformed")?;
            let angle = number(pattern.get("angle"))
                .filter(|angle| *angle > 0.0)
                .ok_or("its angle is malformed")?;
            // CADIR 0.6.3 PatternTransform::Circular defines angle as the angular span,
            // and count includes the original. SolidCraft 0.19 uses the same convention
            // (three instances over 90 degrees land at 0, 45 and 90 degrees).
            let degrees = if (TAU - angle).abs() <= FULL_TURN {
                360.0
            } else {
                angle.to_degrees()
            };
            if !degrees.is_finite() {
                return Err("its angle is malformed".to_owned());
            }
            Ok(json!({
                "type": "circular",
                "origin": origin,
                "axis": axis,
                "count": instance_count(pattern.get("count"))?.to_string(),
                "angle": format!("{degrees} deg"),
            }))
        }
        Some("composite") => composite_layout(pattern),
        Some("scale") => Err("progressively scaled patterns are not supported yet: SolidCraft pattern commands have no scale option".to_owned()),
        Some("mirror") => Err("composite patterns containing a mirror are not supported yet: SolidCraft has no ordered composite pattern command".to_owned()),
        Some("curve_driven") => Err("composite patterns combining a path with other stages are not supported yet: SolidCraft has no ordered composite pattern command".to_owned()),
        Some("mirror_reference") => Err("mirrors using a referenced plane are not mapped yet: its plane frame is not resolved".to_owned()),
        Some(other) => Err(format!("{other} patterns are not mapped yet")),
        None => Err("its layout is malformed".to_owned()),
    }
}

/// Explicit locations fit a regular pattern only when every interval is equal.
fn regular_locations(value: Option<&Value>, what: &str) -> Result<(u64, f64), String> {
    let locations = value
        .and_then(Value::as_array)
        .filter(|rows| !rows.is_empty())
        .ok_or_else(|| format!("its {what} are malformed"))?;
    let count = u64::try_from(locations.len()).map_err(|_| "its count is too large")?;
    instance_count(Some(&json!(count)))?;
    if number(locations.first()) != Some(0.0) {
        return Err(format!("its {what} do not start at zero"));
    }
    let spacing = if count == 1 {
        1.0
    } else {
        number(locations.get(1))
            .filter(|value| *value > 0.0)
            .ok_or_else(|| format!("its {what} are malformed"))?
    };
    for (index, location) in locations.iter().enumerate() {
        let expected = spacing * index as f64;
        let actual = number(Some(location)).ok_or_else(|| format!("its {what} are malformed"))?;
        if (actual - expected).abs() > 1e-9 * expected.abs().max(1.0) {
            return Err(format!("nonuniform {what} are not supported yet: SolidCraft patterns have no per-instance locations or skipped instances"));
        }
    }
    Ok((count, spacing))
}

/// A pair of independent linear stages is exactly a two-direction rectangular pattern.
fn composite_layout(pattern: &Value) -> Result<Value, String> {
    let stages = pattern
        .get("stages")
        .and_then(Value::as_array)
        .ok_or("its pattern stages are malformed")?;
    if stages.len() == 1 {
        return layout(stages.first().and_then(|stage| stage.get("pattern")));
    }
    if stages.len() == 2 {
        let mut layouts = stages.iter().map(|stage| layout(stage.get("pattern")));
        let first = layouts.next().transpose()?;
        let second = layouts.next().transpose()?;
        if let (Some(first), Some(second)) = (first, second) {
            if first.get("type").and_then(Value::as_str) == Some("rectangular")
                && second.get("type").and_then(Value::as_str) == Some("rectangular")
                && first.get("dir2").is_none()
                && second.get("dir2").is_none()
            {
                let count1 = first
                    .get("count1")
                    .and_then(Value::as_str)
                    .and_then(|value| value.parse::<u64>().ok())
                    .ok_or("its count is malformed")?;
                let count2 = second
                    .get("count1")
                    .and_then(Value::as_str)
                    .and_then(|value| value.parse::<u64>().ok())
                    .ok_or("its count is malformed")?;
                if count1.saturating_mul(count2) > MAX_INSTANCES {
                    return Err(
                        "patterns with more than 10000 instances are not supported yet".to_owned(),
                    );
                }
                let mut out = first;
                if let Some(fields) = out.as_object_mut() {
                    for (from, to) in [
                        ("dir1", "dir2"),
                        ("count1", "count2"),
                        ("spacing1", "spacing2"),
                    ] {
                        if let Some(value) = second.get(from) {
                            fields.insert(to.to_owned(), value.clone());
                        }
                    }
                }
                return Ok(out);
            }
        }
    }
    let kinds = stages
        .iter()
        .take(8)
        .map(|stage| {
            stage
                .pointer("/pattern/kind")
                .and_then(Value::as_str)
                .unwrap_or("unknown")
        })
        .collect::<Vec<_>>()
        .join(" then ");
    Err(format!("composite patterns with {} stages ({kinds}) are not supported yet: SolidCraft has no ordered composite pattern command", stages.len()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn malformed_or_oversized_chains_do_not_emit_partial_features() {
        let mirror = json!({"pattern": {"kind": "mirror",
            "plane_origin": {"x": 20.0, "y": 0.0, "z": 0.0},
            "plane_normal": {"x": 1.0, "y": 0.0, "z": 0.0}}});
        for (stages, reason) in [
            (json!([]), "stages are empty"),
            (json!(vec![mirror.clone(); 33]), "more than 32 stages"),
            (
                json!([mirror, {"pattern": {"kind": "scale", "center": {"kind": "first_seed_centroid"},
                "final_factor": 2.0, "count": 3}}]),
                "progressively scaled patterns",
            ),
            (
                json!([mirror, {"pattern": {"kind": "mirror",
                "plane_origin": {"x": 0.0, "y": 0.0, "z": 0.0},
                "plane_normal": {"x": 0.0, "y": 0.0, "z": 0.0}}}]),
                "mirror plane is malformed",
            ),
        ] {
            let wire = Value::Null;
            let mut ctx = Context::new(&wire);
            ctx.target_version = crate::SolidCraftVersion::V0_25_1;
            let mut preceding = vec![json!({"name": "Earlier"})];
            let result = chain(
                &json!({"id": "composite", "name": "Copies"}),
                Some(&json!({"stages": stages})),
                vec!["Block".to_owned()],
                &mut ctx,
                &mut preceding,
            );
            assert!(result.unwrap_err().contains(reason));
            assert_eq!(preceding, vec![json!({"name": "Earlier"})]);
            assert!(ctx.pattern_features.is_empty());
            assert!(!ctx.builds);
        }
    }

    #[test]
    fn linear_patterns_become_rectangular() {
        let linear = json!({"kind": "linear", "direction": {"x": 2.0, "y": 0.0, "z": 0.0},
            "spacing": 20.0, "count": 3,
            "second": {"direction": {"x": 0.0, "y": 1.0, "z": 0.0}, "spacing": 15.0, "count": 2}});
        assert_eq!(
            layout(Some(&linear)).unwrap(),
            json!({"type": "rectangular", "dir1": [1.0, 0.0, 0.0], "count1": "3",
                   "spacing1": "20 mm", "dir2": [0.0, 1.0, 0.0], "count2": "2",
                   "spacing2": "15 mm"})
        );
        let mut skipping = linear.clone();
        skipping["omitted_instances"] = json!([[1, 1], [2, 0]]);
        assert_eq!(
            layout(Some(&skipping)).unwrap_err(),
            "patterns that skip grid positions are not supported yet: SolidCraft's rectangular pattern fills the whole grid (2 skipped)"
        );
        skipping["omitted_instances"] = json!([]);
        assert_eq!(layout(Some(&skipping)), layout(Some(&linear)));
    }

    #[test]
    fn circular_patterns_keep_their_span_and_total_count() {
        let circular = |angle: f64, count: u32| {
            json!({"kind": "circular",
            "axis_origin": {"x": 0.0, "y": 0.0, "z": 0.0},
            "axis_dir": {"x": 0.0, "y": 0.0, "z": 2.0}, "angle": angle, "count": count})
        };
        for (angle, expected) in [
            (TAU / 4.0, "90 deg"),
            (TAU / 2.0, "180 deg"),
            (TAU, "360 deg"),
            (TAU + FULL_TURN / 2.0, "360 deg"),
            (TAU * 1.25, "450 deg"),
        ] {
            for count in [1, 2, 3, 6] {
                assert_eq!(
                    layout(Some(&circular(angle, count))).unwrap(),
                    json!({"type": "circular", "origin": [0.0, 0.0, 0.0],
                        "axis": [0.0, 0.0, 1.0], "count": count.to_string(), "angle": expected})
                );
            }
        }
        for angle in [0.0, -1.0, f64::MAX] {
            assert_eq!(
                layout(Some(&circular(angle, 3))).unwrap_err(),
                "its angle is malformed"
            );
        }
        assert!(layout(Some(&circular(TAU / 4.0, 0))).is_err());
        assert!(layout(Some(&json!({"kind": "linear", "spacing": 1.0, "count": 2}))).is_err());
    }
    #[test]
    fn patterns_cadmpeg_does_not_decode_say_so() {
        let model = json!({});
        let reason = |definition: Value| {
            let mut ctx = Context::new(&model);
            map(
                &json!({"name": "Pattern"}),
                &definition,
                &mut ctx,
                &mut Vec::new(),
            )
            .unwrap_err()
        };
        assert_eq!(
            reason(json!({"seeds": [], "pattern": {"kind": "unresolved", "form": "linear"}})),
            "cadmpeg does not decode what it repeats or how yet"
        );
        assert_eq!(
            reason(json!({"seeds": [], "pattern": {"kind": "linear"}})),
            "it patterns nothing"
        );
        assert_eq!(
            reason(
                json!({"seeds": [{"kind": "faces", "value": {"kind": "native", "value": "n"}}],
                "pattern": {"kind": "circular"}})
            ),
            "patterns of faces are not supported yet: SolidCraft patterns repeat whole features or bodies"
        );
        assert_eq!(
            layout(Some(&json!({"kind": "unresolved", "form": "mirror"}))).unwrap_err(),
            "cadmpeg does not decode its mirror pattern layout yet"
        );
    }
}
