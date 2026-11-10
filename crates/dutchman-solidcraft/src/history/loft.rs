//! Lofts, as SolidCraft loft features.
//!
//! A loft through sketch profiles maps when SolidCraft builds the same shape from it: open,
//! with no degree limit, through two to fifty sections (several may be profiles of one
//! sketch), guided by at most two rails of sketch curves or one-curve whole sketches (not by
//! a centreline). A ruled loft maps through two sections without rails, where SolidCraft's loft
//! is ruled too. A surface loft becomes a SolidCraft surface loft (walls only, a new surface
//! body). A first or last section of a solid loft that is a point (a position or a vertex)
//! becomes a sketch holding that point, written just before the loft, since SolidCraft takes a
//! loft's point end as a sketch point. Edge and face simplification after the loft changes no
//! shape and is left to SolidCraft.

use serde_json::{json, Value};

use super::extrude::{body_names, operation};
use super::profile::{profile, profile_count};
use super::sketch::v3;
use super::sketch::{Mapped, Shape};
use super::sweep::curves;
use super::{name, Context};

/// Why a loft over a spline written as exact pieces is refused: a loft runs its surface
/// through each section's edges, so more edges would make another surface (and can stall
/// SolidCraft), even though each piece is exact.
const SPLIT: &str = "lofts through a spline written as exact pieces are not mapped yet: the \
pieces would change the lofted surface";

/// Why a loft section that takes several of a sketch's loops is refused: the source's loft can
/// fill the outer loop where the loops read as a ring, so neither reading is proven.
const SEVERAL_LOOPS: &str = "a loft section that takes several loops of its sketch is not mapped \
yet: whether the inner loops are holes in the lofted shape is not known";

/// How many of its sketch's loops a section selects, when it selects whole loops by kind
/// (the whole sketch, or a list of its profiles). Other selections name their regions.
fn loops_selected(section: &Value, ctx: &Context) -> usize {
    let value = section.get("value");
    match section.get("kind").and_then(Value::as_str) {
        Some("sketch") => value
            .and_then(Value::as_str)
            .map_or(0, |sketch| profile_count(ctx, sketch)),
        Some("sketch_profiles") => value
            .and_then(|value| value.get("profiles"))
            .and_then(Value::as_array)
            .map_or(0, Vec::len),
        _ => 0,
    }
}

/// Whether `sketch` carries a spline split into exact pieces, among the curves `ids` when given.
fn split_spline(sketch: &Mapped, ids: Option<&[&str]>) -> bool {
    sketch.shapes.values().any(|shape| match shape {
        Shape::Curve(curve) if !curve.pieces.is_empty() => ids.is_none_or(|ids| {
            curve
                .pieces
                .iter()
                .any(|piece| ids.contains(&piece.id.as_str()))
        }),
        _ => false,
    })
}

/// Maps a loft feature whose operation is `definition`, appending any apex sketches
/// to `preceding` only once the complete loft has mapped.
pub(crate) fn map(
    feature: &Value,
    definition: &Value,
    ctx: &mut Context,
    preceding: &mut Vec<Value>,
) -> Result<Value, String> {
    let guidance = definition.get("guidance").unwrap_or(&Value::Null);
    let guides = match guidance.get("kind").and_then(Value::as_str) {
        None => &[][..],
        Some("guides") => guidance
            .get("path")
            .and_then(Value::as_array)
            .map_or(&[][..], Vec::as_slice),
        Some("centerline") => return Err("lofts along a centreline are not mapped yet".to_owned()),
        Some(_) => return Err("its guidance is malformed".to_owned()),
    };
    if guides.len() > 2 {
        return Err("lofts along more than two guide rails are not mapped yet".to_owned());
    }
    let rails = guides
        .iter()
        .map(|guide| {
            curves(Some(guide), ctx, "guide rail")
                .map(|(sketch, curves)| json!({"sketch": sketch, "curves": curves}))
        })
        .collect::<Result<Vec<_>, _>>()?;
    for rail in &rails {
        let feature = rail.get("sketch").and_then(Value::as_u64);
        let ids: Vec<&str> = rail
            .get("curves")
            .and_then(Value::as_array)
            .map(|ids| ids.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default();
        if ctx
            .sketches
            .values()
            .filter(|mapped| Some(mapped.feature) == feature)
            .any(|mapped| split_spline(mapped, Some(&ids)))
        {
            return Err(SPLIT.to_owned());
        }
    }
    let flag = |key: &str, default: bool| {
        definition
            .get(key)
            .and_then(Value::as_bool)
            .unwrap_or(default)
    };
    if flag("closed", false) {
        return Err("closed lofts are not mapped yet".to_owned());
    }
    let surface = !flag("solid", true);
    let ruled = flag("ruled", false);
    if definition.get("max_degree").is_some_and(|d| !d.is_null()) {
        return Err("lofts with a degree limit are not mapped yet".to_owned());
    }

    let sources = definition
        .get("sections")
        .and_then(Value::as_array)
        .ok_or("its sections are malformed")?;
    if !(2..=50).contains(&sources.len()) {
        return Err("a loft needs between two and fifty sections".to_owned());
    }
    // SolidCraft rules a loft between two sections and runs one smooth surface through
    // three or more, so a ruled loft is the same shape only through two, without rails.
    if ruled && sources.len() > 2 {
        return Err("ruled lofts through more than two sections are not mapped yet".to_owned());
    }
    if ruled && !rails.is_empty() {
        return Err("ruled lofts along guide rails are not mapped yet".to_owned());
    }
    let mut sections = Vec::with_capacity(sources.len());
    let mut apex_sketches = Vec::new();
    for (index, section) in sources.iter().enumerate() {
        if matches!(
            section.get("kind").and_then(Value::as_str),
            Some("point" | "native_point" | "vertex")
        ) {
            if surface {
                return Err("surface lofts to a point are not mapped yet".to_owned());
            }
            if index != 0 && index + 1 != sources.len() {
                return Err("lofts with an interior point section are not mapped yet".to_owned());
            }
            let position = match section.get("kind").and_then(Value::as_str) {
                Some("point") => v3(section.get("value")),
                Some("vertex") => section
                    .get("value")
                    .and_then(Value::as_str)
                    .and_then(|id| ctx.row("vertices", id))
                    .and_then(|vertex| vertex.get("point"))
                    .and_then(Value::as_str)
                    .and_then(|id| ctx.row("points", id))
                    .and_then(|point| v3(point.get("position"))),
                _ => {
                    return Err(
                        "lofts that close to an unresolved point are not mapped yet".to_owned()
                    )
                }
            }
            .ok_or("a loft point section has no resolved position")?;
            let id = ctx.id();
            let apex_name = ctx.unique_name(
                &json!({"name": format!("Loft apex {id}")}),
                preceding.iter(),
            );
            apex_sketches.push(json!({
                "id": id,
                "name": apex_name,
                "type": "sketch",
                "suppressed": false,
                "plane": {"type": "custom", "plane": {
                    "origin": position,
                    "x_dir": [1.0, 0.0, 0.0],
                    "y_dir": [0.0, 1.0, 0.0],
                }},
                "sketch": {
                    "points": [{"id": "origin", "pos": [0.0, 0.0], "fixed": true}],
                    "curves": [],
                    "constraints": [],
                },
            }));
            sections.push(json!({"sketch": id, "point": "origin", "profiles": {"type": "all"}}));
            continue;
        }
        if loops_selected(section, ctx) > 1 {
            return Err(SEVERAL_LOOPS.to_owned());
        }
        let (sketch_id, profiles) = profile(Some(section), ctx)?;
        let sketch = ctx
            .sketches
            .get(&sketch_id)
            .ok_or("a section's sketch is not an earlier mapped sketch")?;
        if split_spline(sketch, None) {
            return Err(SPLIT.to_owned());
        }
        sections.push(json!({"sketch": sketch.feature, "profiles": profiles}));
    }
    // A surface loft is a sheet of its own; SolidCraft makes it a new surface body.
    let operation = if surface {
        "new_body"
    } else {
        operation(definition.get("op"))?
    };
    let body_names = body_names(feature, ctx, operation);

    let id = ctx.id();
    ctx.builds = true;
    let mut out = json!({
        "id": id,
        "name": name(feature).map(str::to_owned).unwrap_or_else(|| format!("Loft{id}")),
        "type": if surface { "surface_loft" } else { "loft" },
        "suppressed": false,
        "sections": sections,
        "operation": operation,
    });
    if let Some(fields) = out.as_object_mut() {
        if surface {
            fields.remove("operation");
        }
        if !rails.is_empty() {
            fields.insert("rails".to_owned(), json!(rails));
        }
        if let Some(names) = body_names {
            fields.insert("body_names".to_owned(), json!(names));
        }
    }
    preceding.extend(apex_sketches);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::super::sketch::{Curve, Mapped, Shape};

    fn curve(id: &str, pieces: Vec<Curve>) -> Curve {
        Curve {
            pieces,
            index: 0,
            id: id.to_owned(),
            kind: "spline",
            start: None,
            end: None,
            center: None,
        }
    }

    fn sketch(shapes: Vec<(&str, Curve)>) -> Mapped {
        Mapped {
            feature: 7,
            normal: [0.0, 0.0, 1.0],
            origin: [0.0; 3],
            x_dir: [1.0, 0.0, 0.0],
            y_dir: [0.0, 1.0, 0.0],
            points: Vec::new(),
            shapes: shapes
                .into_iter()
                .map(|(id, c)| (id.to_owned(), Shape::Curve(c)))
                .collect::<BTreeMap<_, _>>(),
        }
    }

    #[test]
    fn a_loft_section_or_rail_through_a_split_spline_is_found() {
        let split = curve("s1", vec![curve("s1", vec![]), curve("s2", vec![])]);
        let mapped = sketch(vec![("whole", split), ("line", curve("l1", vec![]))]);
        assert!(super::split_spline(&mapped, None));
        assert!(super::split_spline(&mapped, Some(&["s2"])));
        assert!(!super::split_spline(&mapped, Some(&["l1"])));
        let plain = sketch(vec![("one", curve("s1", vec![]))]);
        assert!(!super::split_spline(&plain, None));
    }
}
