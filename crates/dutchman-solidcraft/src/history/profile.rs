//! Which closed profiles of a sketch a feature uses, as a SolidCraft profile selection.

use std::collections::BTreeSet;

use serde_json::{json, Value};

use super::sketch::{Curve, Mapped, Shape, P2};
use super::Context;

/// The sketch a profile reference uses, and the SolidCraft profile selection for it.
pub(super) fn profile(profile: Option<&Value>, ctx: &Context) -> Result<(String, Value), String> {
    let profile = profile.ok_or("it has no profile")?;
    let kind = profile
        .get("kind")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let value = profile.get("value").unwrap_or(&Value::Null);
    if kind == "sketch" {
        let sketch = value.as_str().ok_or("its profile sketch is malformed")?;
        // Every loop of the sketch, as the extrude takes them: loops inside loops are holes.
        if let Some(mapped) = ctx.sketches.get(sketch) {
            let every: Vec<usize> = (0..profile_count(ctx, sketch)).collect();
            if let Some(selection) = nested(ctx, sketch, &every, mapped) {
                return Ok((sketch.to_owned(), selection));
            }
        }
        return Ok((sketch.to_owned(), json!({"type": "all"})));
    }
    let sketch = value
        .get("sketch")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{kind} profiles are not mapped yet"))?;
    let mapped = ctx
        .sketches
        .get(sketch)
        .ok_or("its sketch is not an earlier mapped sketch")?;
    let selection = match kind {
        "sketch_profiles" => {
            let indices = value
                .get("profiles")
                .and_then(Value::as_array)
                .ok_or("its profile selection is malformed")?;
            let loops = indices
                .iter()
                .map(|index| source_loop(ctx, sketch, index, mapped))
                .collect::<Result<Vec<_>, _>>()?;
            let wanted: Option<Vec<usize>> = indices
                .iter()
                .map(|index| index.as_u64().and_then(|i| usize::try_from(i).ok()))
                .collect();
            if let Some(selection) = wanted.and_then(|wanted| nested(ctx, sketch, &wanted, mapped))
            {
                return Ok((sketch.to_owned(), selection));
            }
            let mut selected = Vec::new();
            for (index, ring) in indices.iter().zip(loops) {
                if needs_split(ctx, sketch, index, &ring) {
                    match split_loop(ctx, sketch, index, mapped) {
                        Ok(rings) => selected.extend(rings),
                        // Preserve the existing whole-sketch mapping when every source
                        // loop is taken, including arrangements we cannot split yet.
                        Err(_) if takes_every_loop(ctx, sketch, indices) => {
                            return Ok((sketch.to_owned(), json!({"type": "all"})));
                        }
                        Err(reason) => return Err(reason),
                    }
                } else {
                    selected.push(ring);
                }
            }
            curves(selected)?
        }
        "sketch_regions" => regions(value.get("regions"), ctx, sketch, mapped)?,
        "sketch_entities" => entities(value.get("entities"), mapped)?,
        // Native picks carry no neutral point or index. Infer only a single material
        // region, including a surrounding loop with holes under even-odd nesting.
        "sketch_selection" => {
            let count = profile_count(ctx, sketch);
            let source = ctx
                .row("sketches", sketch)
                .ok_or("its sketch has no profiles")?;
            let entities = sketch_entities(ctx, sketch);
            if count == 1 {
                // Other curves of the sketch can divide its one stored loop into several
                // regions, and the source picked one of them: cutting the whole sketch would
                // take them all.
                if split_loop(ctx, sketch, &json!(0), mapped).is_ok_and(|rings| rings.len() > 1) {
                    return Err(
                        "a profile cadmpeg does not resolve, in a sketch whose curves \
                                divide its one profile into several regions, is not mapped yet"
                            .to_owned(),
                    );
                }
                json!({"type": "all"})
            } else if let Some(outer) = super::crossing::single_region(source, &entities) {
                let boundary = source_loop(ctx, sketch, &json!(outer), mapped)?;
                let holes = (0..count)
                    .filter(|index| *index != outer)
                    .map(|index| source_loop(ctx, sketch, &json!(index), mapped))
                    .collect::<Result<Vec<_>, _>>()?;
                let holes: Vec<_> = holes
                    .into_iter()
                    .map(|hole| canonical_loop(hole, mapped))
                    .collect();
                sorted_whole(json!({"type": "whole_boundaries", "profiles": [{
                    "outer": canonical_loop(boundary, mapped),
                    "holes": holes,
                }]}))
            } else {
                return Err(format!(
                    "a profile cadmpeg does not resolve, in a sketch of {count} profiles, is not \
                     mapped yet"
                ));
            }
        }
        other => return Err(format!("{other} profiles are not mapped yet")),
    };
    Ok((sketch.to_owned(), selection))
}

/// A whole source region may span subdivisions needed by another feature's trimmed picks.
/// Give that explicit whole-loop consumer a copy with its unused curves construction, while
/// the shared sketch retains every trimmed boundary. Geometry, constraints, and curve ids
/// are copied unchanged; SolidCraft still has to match the selected boundaries uniquely.
pub(super) fn whole_region_sketch(
    feature: &Value,
    sketch: &str,
    ctx: &mut Context,
    preceding: &[Value],
) -> Option<u64> {
    let reference = super::operation(feature).get("profile")?;
    match reference.get("kind")?.as_str()? {
        "sketch_profiles" => {}
        "sketch_regions"
            if reference
                .pointer("/value/regions")?
                .as_array()?
                .iter()
                .all(|region| region.get("region").and_then(Value::as_str) == Some("loops")) => {}
        _ => return None,
    }
    let source = ctx.row("sketches", sketch)?;
    let entities = sketch_entities(ctx, sketch);
    let demoted = super::crossing::demoted(source, &entities, std::slice::from_ref(feature));
    let mapped = ctx.sketches.get(sketch)?;
    let crossing_ids: BTreeSet<String> = demoted
        .crossing
        .iter()
        .filter_map(|entity| mapped.curve(entity))
        .flat_map(Curve::ids)
        .collect();
    let demoted: BTreeSet<String> = demoted.crossing.into_iter().chain(demoted.stray).collect();
    let ids: BTreeSet<String> = demoted
        .iter()
        .filter_map(|entity| mapped.curve(entity))
        .flat_map(Curve::ids)
        .collect();
    let original = preceding
        .iter()
        .find(|row| row.get("id").and_then(Value::as_u64) == Some(mapped.feature))?;
    let curves = original.pointer("/sketch/curves")?.as_array()?;
    if !curves.iter().any(|curve| {
        curve.get("construction").and_then(Value::as_bool) != Some(true)
            && curve
                .get("id")
                .and_then(Value::as_str)
                .is_some_and(|id| crossing_ids.contains(id))
    }) {
        return None;
    }
    let mut copy = original.clone();
    for curve in copy.pointer_mut("/sketch/curves")?.as_array_mut()? {
        if curve
            .get("id")
            .and_then(Value::as_str)
            .is_some_and(|id| ids.contains(id))
        {
            curve
                .as_object_mut()?
                .insert("construction".to_owned(), json!(true));
        }
    }
    let id = ctx.id();
    let fields = copy.as_object_mut()?;
    fields.insert("id".to_owned(), json!(id));
    fields.insert(
        "name".to_owned(),
        json!(format!(
            "{} profile",
            super::name(feature).unwrap_or("Extrude")
        )),
    );
    ctx.extra.push(copy);
    Some(id)
}

/// The area (mm squared) of the sketch region a profile reference selects, from the source's
/// own loops, or `None` when the reference is not a whole sketch or a list of its profiles, or
/// the loops cannot be followed.
pub(super) fn swept_area(profile: Option<&Value>, ctx: &Context) -> Option<f64> {
    let profile = profile?;
    let value = profile.get("value")?;
    let (sketch, wanted): (&str, Vec<usize>) = match profile.get("kind")?.as_str()? {
        "sketch" => {
            let sketch = value.as_str()?;
            (sketch, (0..profile_count(ctx, sketch)).collect())
        }
        "sketch_profiles" => (
            value.get("sketch")?.as_str()?,
            value
                .get("profiles")?
                .as_array()?
                .iter()
                .map(|index| index.as_u64().and_then(|i| usize::try_from(i).ok()))
                .collect::<Option<Vec<_>>>()?,
        ),
        _ => return None,
    };
    let row = ctx.row("sketches", sketch)?;
    let entities = sketch_entities(ctx, sketch);
    super::crossing::selected_area(row, &entities, &wanted)
}

/// The selected source loops as whole boundaries when some lie inside others: a loop inside
/// one selected loop is its hole, as Fusion's even-odd profile nesting has it, so concentric
/// circles are a ring, not a disc. `None` when nothing nests, a loop is not followed whole, or
/// the loops touch or cross.
fn nested(ctx: &Context, sketch: &str, wanted: &[usize], mapped: &Mapped) -> Option<Value> {
    let row = ctx.row("sketches", sketch)?;
    let entities = sketch_entities(ctx, sketch);
    let nesting = super::crossing::nesting(row, &entities, wanted)?;
    let mut whole = Vec::with_capacity(nesting.len());
    for (outer, holes) in nesting {
        let outer = source_loop(ctx, sketch, &json!(outer), mapped).ok()?;
        let holes = holes
            .into_iter()
            .map(|hole| {
                source_loop(ctx, sketch, &json!(hole), mapped).map(|h| canonical_loop(h, mapped))
            })
            .collect::<Result<Vec<_>, _>>()
            .ok()?;
        whole.push(json!({"outer": canonical_loop(outer, mapped), "holes": holes}));
    }
    Some(sorted_whole(
        json!({"type": "whole_boundaries", "profiles": whole}),
    ))
}

/// A whole-boundary selection in the form every SolidCraft release validates when it loads a
/// design: each boundary's curve ids in ascending order, and its holes in ascending order. The
/// identity SolidCraft matches is the set of curves in each loop, so sorting changes no match;
/// a hole list out of order (`c10` sorts before `c2`) is refused as damaged by 0.18.2 to 0.22.1.
fn sorted_whole(mut selection: Value) -> Value {
    let sort = |boundary: &mut Value| {
        if let Some(ids) = boundary.as_array_mut() {
            ids.sort_by(|a, b| a.as_str().cmp(&b.as_str()));
        }
    };
    let profiles = selection
        .get_mut("profiles")
        .and_then(Value::as_array_mut)
        .into_iter()
        .flatten();
    for profile in profiles {
        if let Some(outer) = profile.get_mut("outer") {
            sort(outer);
        }
        if let Some(holes) = profile.get_mut("holes").and_then(Value::as_array_mut) {
            holes.iter_mut().for_each(sort);
            holes.sort_by(|a, b| {
                let ids = |v: &Value| -> Vec<String> {
                    v.as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(Value::as_str)
                        .map(str::to_owned)
                        .collect()
                };
                ids(a).cmp(&ids(b))
            });
        }
    }
    selection
}

/// A whole boundary as SolidCraft lists the loops it finds itself, which is the only order it
/// matches: counter-clockwise, holes included, starting at the curve through the loop's leftmost
/// point, or at the curve leaving it when that point is a vertex. Of vertices equally far left,
/// the one with the lowest sketch point index starts the loop.
/// A loop of curves other than lines and arcs, or one that does not close, is left as it is.
fn canonical_loop(ring: Vec<String>, mapped: &Mapped) -> Vec<String> {
    let Some(curves) = ring
        .iter()
        .map(|id| {
            mapped.shapes.values().find_map(|shape| match shape {
                Shape::Curve(curve) => {
                    if curve.pieces.is_empty() {
                        (curve.id == *id).then_some(curve)
                    } else {
                        curve.pieces.iter().find(|piece| piece.id == *id)
                    }
                }
                _ => None,
            })
        })
        .collect::<Option<Vec<&Curve>>>()
    else {
        return ring;
    };
    if curves.len() < 2 || ring.len() > LOOP_LIMIT {
        return ring;
    }
    let Some(mut steps) = chain(&curves) else {
        return ring;
    };
    let Some(mut samples) = steps
        .iter()
        .map(|&(index, forward)| run_points(curves.get(index).copied(), forward, mapped))
        .collect::<Option<Vec<_>>>()
    else {
        return ring;
    };
    if signed_area(&samples) < 0.0 {
        steps.reverse();
        for step in &mut steps {
            step.1 = !step.1;
        }
        samples = match steps
            .iter()
            .map(|&(index, forward)| run_points(curves.get(index).copied(), forward, mapped))
            .collect::<Option<Vec<_>>>()
        {
            Some(samples) => samples,
            None => return ring,
        };
    }
    let mut first = 0;
    let mut best: Option<(f64, usize)> = None;
    for (position, points) in samples.iter().enumerate() {
        for (point, vertex) in points {
            // A point inside an arc ranks after every vertex at the same x.
            let rank = vertex.unwrap_or(usize::MAX);
            let left = match best {
                None => true,
                Some((x, best_rank)) => {
                    point[0] < x - EPSILON || ((point[0] - x).abs() <= EPSILON && rank < best_rank)
                }
            };
            if left {
                best = Some((point[0], rank));
                first = position;
            }
        }
    }
    steps.rotate_left(first);
    steps
        .iter()
        .filter_map(|&(index, _)| ring.get(index).cloned())
        .collect()
}

/// More curves than this in one loop are left in their source order.
const LOOP_LIMIT: usize = 10_000;

/// Positions within this (mm) are the same for the leftmost-point test.
const EPSILON: f64 = 1e-9;

/// The loop's curves in order, each with whether it runs from its start to its end.
fn chain(curves: &[&Curve]) -> Option<Vec<(usize, bool)>> {
    let ends = |curve: &Curve, forward: bool| -> Option<(usize, usize)> {
        let (start, end) = (curve.start?, curve.end?);
        Some(if forward { (start, end) } else { (end, start) })
    };
    if curves
        .iter()
        .any(|curve| !matches!(curve.kind, "line" | "arc"))
    {
        return None;
    }
    let first = *curves.first()?;
    let second = *curves.get(1)?;
    let (start, end) = ends(first, true)?;
    let (next_start, next_end) = ends(second, true)?;
    let forward = if end == next_start || end == next_end {
        true
    } else if start == next_start || start == next_end {
        false
    } else {
        return None;
    };
    let mut steps = vec![(0, forward)];
    let mut at = ends(first, forward)?.1;
    for (index, curve) in curves.iter().enumerate().skip(1) {
        let (start, end) = ends(curve, true)?;
        let forward = if start == at {
            true
        } else if end == at {
            false
        } else {
            return None;
        };
        steps.push((index, forward));
        at = if forward { end } else { start };
    }
    (at == ends(first, forward)?.0).then_some(steps)
}

/// Points along a curve as the loop runs it, from its start and leaving out its end, each with
/// its sketch point index when it is a vertex. An arc's run includes its leftmost point when the
/// arc passes it.
fn run_points(
    curve: Option<&Curve>,
    forward: bool,
    mapped: &Mapped,
) -> Option<Vec<(P2, Option<usize>)>> {
    let curve = curve?;
    let at = |index: Option<usize>| mapped.points.get(index?).map(|(_, position)| *position);
    let (start, end) = (at(curve.start)?, at(curve.end)?);
    let first = if forward {
        (start, curve.start)
    } else {
        (end, curve.end)
    };
    if curve.kind == "line" {
        return Some(vec![first]);
    }
    // SolidCraft arcs run counter-clockwise from their start to their end about their centre.
    let center = at(curve.center)?;
    let radius = ((start[0] - center[0]).powi(2) + (start[1] - center[1]).powi(2)).sqrt();
    let from = (start[1] - center[1]).atan2(start[0] - center[0]);
    let to = (end[1] - center[1]).atan2(end[0] - center[0]);
    let tau = std::f64::consts::TAU;
    let mut sweep = (to - from).rem_euclid(tau);
    if sweep <= EPSILON {
        sweep = tau;
    }
    let point = |angle: f64| {
        [
            center[0] + radius * angle.cos(),
            center[1] + radius * angle.sin(),
        ]
    };
    let mut angles: Vec<f64> = (0..16)
        .map(|step| from + sweep * f64::from(step) / 16.0)
        .collect();
    let leftmost = (std::f64::consts::PI - from).rem_euclid(tau);
    if leftmost > EPSILON && leftmost < sweep - EPSILON {
        angles.push(from + leftmost);
        angles.sort_by(f64::total_cmp);
    }
    // Interior points only: the stored ends are exact.
    let interior: Vec<(P2, Option<usize>)> = angles
        .into_iter()
        .skip(1)
        .map(|angle| (point(angle), None))
        .collect();
    Some(if forward {
        std::iter::once(first).chain(interior).collect()
    } else {
        // Run backwards: from the end, leaving out the start.
        std::iter::once(first)
            .chain(interior.into_iter().rev())
            .collect()
    })
}

/// Twice the signed area enclosed by the samples, positive counter-clockwise.
fn signed_area(samples: &[Vec<(P2, Option<usize>)>]) -> f64 {
    let points: Vec<P2> = samples.iter().flatten().map(|(point, _)| *point).collect();
    points
        .iter()
        .zip(points.iter().cycle().skip(1))
        .map(|(a, b)| a[0] * b[1] - b[0] * a[1])
        .sum()
}

/// Whether a loop runs along one of its curves more than once.
fn crossed(curves: &[String]) -> bool {
    curves
        .iter()
        .enumerate()
        .any(|(index, curve)| curves.iter().skip(index + 1).any(|other| other == curve))
}

/// How many profile loops the source sketch has.
pub(super) fn profile_count(ctx: &Context, sketch: &str) -> usize {
    ctx.row("sketches", sketch)
        .and_then(|row| row.get("profiles"))
        .and_then(Value::as_array)
        .map_or(0, Vec::len)
}

fn takes_every_loop(ctx: &Context, sketch: &str, indices: &[Value]) -> bool {
    (0..profile_count(ctx, sketch)).all(|wanted| {
        indices
            .iter()
            .any(|index| index.as_u64().and_then(|i| usize::try_from(i).ok()) == Some(wanted))
    })
}

fn needs_split(ctx: &Context, sketch: &str, index: &Value, ring: &[String]) -> bool {
    if crossed(ring) {
        return true;
    }
    source_members(ctx, sketch, index)
        .is_ok_and(|members| super::crossing::crossed_loop(members, &sketch_entities(ctx, sketch)))
}

/// Source entities belonging to this sketch.
fn sketch_entities<'a>(ctx: &'a Context, sketch: &str) -> Vec<&'a Value> {
    ctx.arena("sketch_entities")
        .iter()
        .filter(|entity| entity.get("sketch").and_then(Value::as_str) == Some(sketch))
        .collect()
}

fn split_loop(
    ctx: &Context,
    sketch: &str,
    index: &Value,
    mapped: &Mapped,
) -> Result<Vec<Vec<String>>, String> {
    let members = source_members(ctx, sketch, index)?;
    let source = sketch_entities(ctx, sketch);
    let sketch = ctx
        .row("sketches", sketch)
        .ok_or("its sketch has no profiles")?;
    let mut construction = super::crossing::crossing(sketch, &source);
    construction.extend(super::crossing::region_lines(
        sketch,
        &source,
        ctx.arena("features"),
    ));
    construction.extend(super::crossing::stray_curves(
        sketch,
        &source,
        ctx.arena("features"),
    ));
    let entities: Vec<&Value> = source
        .into_iter()
        .filter(|entity| {
            !entity
                .get("id")
                .and_then(Value::as_str)
                .is_some_and(|id| construction.contains(id))
        })
        .collect();
    let rings = super::crossing::split(members, &entities)
        .ok_or_else(|| crossed_message(members, &entities))?;
    mapped.boundaries(rings)
}

/// Why a crossed profile is refused: its own loop crosses itself, or other curves cross it.
fn crossed_message(members: &[Value], entities: &[&Value]) -> String {
    if super::crossing::merged_regions(members, entities) {
        "its profile is several selected regions merged into one self-crossing loop; this is not mapped yet"
    } else {
        "a profile crossed by its own sketch's curves is not mapped yet"
    }
    .to_owned()
}

/// Profiles whose outer loops are these curve loops.
fn curves(loops: Vec<Vec<String>>) -> Result<Value, String> {
    if loops.is_empty() {
        return Err("it selects no profile".to_owned());
    }
    Ok(json!({"type": "curves", "loops": loops}))
}

/// Loop `index` of the source sketch's profile loops, as SolidCraft curve ids.
fn source_loop(
    ctx: &Context,
    sketch: &str,
    index: &Value,
    mapped: &Mapped,
) -> Result<Vec<String>, String> {
    source_members(ctx, sketch, index)?
        .iter()
        .map(|uses| {
            curve_ids(
                uses.get("entity"),
                mapped,
                uses.get("reversed").and_then(Value::as_bool) == Some(true),
            )
        })
        .collect::<Result<Vec<_>, _>>()
        .map(|chains| chains.into_iter().flatten().collect())
}

fn source_members<'a>(
    ctx: &'a Context,
    sketch: &str,
    index: &Value,
) -> Result<&'a [Value], String> {
    let loops = ctx
        .row("sketches", sketch)
        .and_then(|row| row.get("profiles"))
        .and_then(Value::as_array)
        .ok_or("its sketch has no profiles")?;
    index
        .as_u64()
        .and_then(|i| usize::try_from(i).ok())
        .and_then(|i| loops.get(i))
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .ok_or_else(|| "it selects a sketch profile that does not exist".to_owned())
}

/// The SolidCraft curve id of a source entity.
fn curve_ids(
    entity: Option<&Value>,
    mapped: &Mapped,
    reversed: bool,
) -> Result<Vec<String>, String> {
    entity
        .and_then(Value::as_str)
        .and_then(|entity| mapped.curve(entity))
        .map(|curve| {
            let mut ids = curve.ids();
            if reversed {
                ids.reverse();
            }
            ids
        })
        .ok_or_else(|| "its profile uses a curve the sketch does not carry".to_owned())
}

/// Profile regions. Regions made of whole source loops become SolidCraft's exact whole
/// boundaries (outer loop and holes); when any region is bounded by pieces of curves, every
/// region is selected by the curves of its outer boundary, which SolidCraft also matches
/// against pieces.
fn regions(
    regions: Option<&Value>,
    ctx: &Context,
    sketch: &str,
    mapped: &Mapped,
) -> Result<Value, String> {
    let regions = regions
        .and_then(Value::as_array)
        .filter(|regions| !regions.is_empty())
        .ok_or("its profile regions are malformed")?;
    // The one loop of a one-loop sketch, whole: every profile, which needs no boundary order.
    if let [region] = regions.as_slice() {
        let holes = region
            .get("holes")
            .and_then(Value::as_array)
            .is_some_and(|holes| !holes.is_empty());
        if region.get("region").and_then(Value::as_str) == Some("loops")
            && region.get("outer").and_then(Value::as_u64) == Some(0)
            && !holes
            && profile_count(ctx, sketch) == 1
            && !source_loop(ctx, sketch, &json!(0), mapped)
                .is_ok_and(|ring| needs_split(ctx, sketch, &json!(0), &ring))
        {
            return Ok(json!({"type": "all"}));
        }
    }
    let mut whole = Vec::with_capacity(regions.len());
    let mut outers = Vec::with_capacity(regions.len());
    let mut trimmed = false;
    for region in regions {
        match region.get("region").and_then(Value::as_str) {
            Some("loops") => {
                let outer = source_loop(
                    ctx,
                    sketch,
                    region.get("outer").unwrap_or(&Value::Null),
                    mapped,
                )?;
                let holes = region
                    .get("holes")
                    .and_then(Value::as_array)
                    .map(|holes| {
                        holes
                            .iter()
                            .map(|hole| source_loop(ctx, sketch, hole, mapped))
                            .collect::<Result<Vec<_>, _>>()
                    })
                    .transpose()?
                    .unwrap_or_default();
                let index = region.get("outer").unwrap_or(&Value::Null);
                if needs_split(ctx, sketch, index, &outer) {
                    if !holes.is_empty() {
                        return Err(source_members(ctx, sketch, index).map_or_else(
                            |_| {
                                "a profile crossed by its own sketch's curves is not mapped yet"
                                    .to_owned()
                            },
                            |members| crossed_message(members, &sketch_entities(ctx, sketch)),
                        ));
                    }
                    trimmed = true;
                    outers.extend(split_loop(ctx, sketch, index, mapped)?);
                } else {
                    outers.push(outer.clone());
                    whole.push(json!({
                        "outer": canonical_loop(outer, mapped),
                        "holes": holes
                            .into_iter()
                            .map(|hole| canonical_loop(hole, mapped))
                            .collect::<Vec<_>>(),
                    }));
                }
            }
            Some("trimmed") => {
                trimmed = true;
                let mut outer: Vec<String> = Vec::new();
                for boundary in region
                    .get("outer_boundary")
                    .and_then(Value::as_array)
                    .ok_or("its trimmed region is malformed")?
                {
                    for id in curve_ids(
                        boundary.get("entity"),
                        mapped,
                        boundary.get("reversed").and_then(Value::as_bool) == Some(true),
                    )? {
                        if !outer.contains(&id) {
                            outer.push(id);
                        }
                    }
                }
                outers.push(outer);
            }
            _ => return Err("its profile regions are malformed".to_owned()),
        }
    }
    if trimmed {
        if let Some(points) = region_points(regions, ctx, sketch, mapped, false) {
            return Ok(json!({"type": "points", "points": points}));
        }
        curves(outers)
    } else {
        // A loop that runs along part of another solid line is split there in SolidCraft, and
        // loops of one region that cross bound it by their union: either way its own curves
        // bound none of SolidCraft's profiles, so name each region by a point.
        if shares_an_edge(regions, ctx, sketch) || loops_cross(regions, ctx, sketch) {
            if let Some(points) = region_points(regions, ctx, sketch, mapped, true) {
                return Ok(json!({"type": "points", "points": points}));
            }
        }
        Ok(sorted_whole(
            json!({"type": "whole_boundaries", "profiles": whole}),
        ))
    }
}

/// Whether a loop of these whole-loop regions (outer or hole) runs along part of a line of the
/// sketch that is written solid.
fn shares_an_edge(regions: &[Value], ctx: &Context, sketch: &str) -> bool {
    let Some(row) = ctx.row("sketches", sketch) else {
        return false;
    };
    let entities = sketch_entities(ctx, sketch);
    let demoted = super::crossing::demoted(row, &entities, ctx.arena("features"));
    let demoted: BTreeSet<String> = demoted.crossing.into_iter().chain(demoted.stray).collect();
    regions.iter().any(|region| {
        let holes = region
            .get("holes")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or_default();
        region
            .get("outer")
            .into_iter()
            .chain(holes)
            .filter_map(|index| source_members(ctx, sketch, index).ok())
            .any(|members| super::crossing::shares_an_edge(members, &entities, &demoted))
    })
}

/// Whether two loops of one whole-loop region (outer or holes) cross each other. SolidCraft
/// then bounds the region by their union, which no single loop of the source is.
fn loops_cross(regions: &[Value], ctx: &Context, sketch: &str) -> bool {
    let entities = sketch_entities(ctx, sketch);
    regions.iter().any(|region| {
        let holes = region
            .get("holes")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or_default();
        let rings: Vec<Vec<P2>> = region
            .get("outer")
            .into_iter()
            .chain(holes)
            .filter_map(|index| source_members(ctx, sketch, index).ok())
            .filter_map(|members| super::crossing::loop_ring(members, &entities))
            .collect();
        rings.iter().enumerate().any(|(i, a)| {
            rings
                .iter()
                .skip(i + 1)
                .any(|b| super::crossing::rings_cross(a, b))
        })
    })
}

/// Regions that use part of a whole circle (or, with `any`, any regions), each named by a point
/// inside it. Two crossing closed curves bound several profiles with the same curves, so curves
/// can't tell them apart; a stored region's boundary can. `None` when no region trims a closed
/// curve and not `any`, or some region's boundary doesn't close into one shape, or leaves no
/// point: those stay selected by their curves, where SolidCraft refuses what stays ambiguous.
fn region_points(
    regions: &[Value],
    ctx: &Context,
    sketch: &str,
    mapped: &Mapped,
    any: bool,
) -> Option<Vec<P2>> {
    let entities = sketch_entities(ctx, sketch);
    fn boundary(region: &Value) -> Option<&[Value]> {
        region
            .get("outer_boundary")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
    }
    if !any
        && !regions.iter().any(|region| {
            boundary(region)
                .is_some_and(|uses| super::crossing::trims_a_closed_curve(uses, &entities))
        })
    {
        return None;
    }
    let boundaries = regions
        .iter()
        .filter(|region| region.get("region").and_then(Value::as_str) == Some("trimmed"))
        .map(|region| {
            let holes = region
                .get("hole_boundaries")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .map(|hole| hole.as_array().map(Vec::as_slice));
            std::iter::once(boundary(region))
                .chain(holes)
                .collect::<Option<Vec<_>>>()
        })
        .collect::<Option<Vec<_>>>()?;
    let boundaries: Vec<&[Value]> = boundaries.into_iter().flatten().collect();
    let mut resolved = if boundaries.is_empty() {
        Vec::new()
    } else {
        super::crossing::trimmed_rings(&boundaries, &entities)?
    }
    .into_iter();
    let mut rings = Vec::with_capacity(regions.len());
    let mut holes = Vec::with_capacity(regions.len());
    for region in regions {
        match region.get("region").and_then(Value::as_str) {
            Some("trimmed") => {
                rings.push(resolved.next()?);
                holes.push(
                    region
                        .get("hole_boundaries")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                        .map(|_| resolved.next())
                        .collect::<Option<Vec<_>>>()?,
                );
            }
            Some("loops") => {
                let members = source_members(ctx, sketch, region.get("outer")?).ok()?;
                rings.push(super::crossing::loop_ring(members, &entities)?);
                holes.push(
                    region
                        .get("holes")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                        .map(|hole| {
                            let members = source_members(ctx, sketch, hole).ok()?;
                            super::crossing::loop_ring(members, &entities)
                        })
                        .collect::<Option<Vec<_>>>()?,
                );
            }
            _ => return None,
        }
    }
    let frame = sketch_to_mapped(ctx.row("sketches", sketch)?, mapped)?;
    let mut points = Vec::with_capacity(rings.len());
    for (ring, holes) in rings.iter().zip(&holes) {
        // Regions can overlap: one stored with no hole can cover a smaller one selected beside
        // it. Its point stays out of the smaller one, which has its own.
        let area = super::crossing::ring_area(ring);
        let mut others: Vec<&[P2]> = holes.iter().map(Vec::as_slice).collect();
        others.extend(
            rings
                .iter()
                .filter(|other| super::crossing::ring_area(other) < area)
                .map(Vec::as_slice),
        );
        let point = super::crossing::interior_point(ring, &others)?;
        points.push(frame(point));
    }
    Some(points)
}

/// Sketch coordinates as the source stores them, in the written sketch's coordinates: the same,
/// unless the written sketch's plane was turned or moved. Without a stored frame, the same.
fn sketch_to_mapped<'a>(sketch: &Value, mapped: &'a Mapped) -> Option<impl Fn(P2) -> P2 + 'a> {
    let frame = sketch.pointer("/placement/frame");
    let v3 = |name: &str| {
        let row = frame?.get(name)?;
        Some([
            row.get("x")?.as_f64()?,
            row.get("y")?.as_f64()?,
            row.get("z")?.as_f64()?,
        ])
    };
    let dot = |a: [f64; 3], b: [f64; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    let source = match (v3("origin"), v3("normal"), v3("u_axis")) {
        (Some(origin), Some(n), Some(u)) => {
            let length = dot(u, u).sqrt();
            let u = [u[0] / length, u[1] / length, u[2] / length];
            let scale = dot(n, n).sqrt();
            let n = [n[0] / scale, n[1] / scale, n[2] / scale];
            let v = [
                n[1] * u[2] - n[2] * u[1],
                n[2] * u[0] - n[0] * u[2],
                n[0] * u[1] - n[1] * u[0],
            ];
            Some((origin, u, v))
        }
        _ => None,
    };
    Some(move |point: P2| {
        let Some((origin, u, v)) = source else {
            return point;
        };
        let world = [
            origin[0] + point[0] * u[0] + point[1] * v[0],
            origin[1] + point[0] * u[1] + point[1] * v[1],
            origin[2] + point[0] * u[2] + point[1] * v[2],
        ];
        let from = [
            world[0] - mapped.origin[0],
            world[1] - mapped.origin[1],
            world[2] - mapped.origin[2],
        ];
        [
            from[0] * mapped.x_dir[0] + from[1] * mapped.x_dir[1] + from[2] * mapped.x_dir[2],
            from[0] * mapped.y_dir[0] + from[1] * mapped.y_dir[1] + from[2] * mapped.y_dir[2],
        ]
    })
}

/// Profiles bounded by the selected curves: each connected group of them is one loop.
fn entities(entities: Option<&Value>, mapped: &Mapped) -> Result<Value, String> {
    let entities = entities
        .and_then(Value::as_array)
        .ok_or("its profile entities are malformed")?;
    let mut groups: Vec<(Vec<usize>, Vec<String>)> = Vec::new();
    for entity in entities {
        let id = entity
            .as_str()
            .ok_or("its profile entities are malformed")?;
        let curve = mapped
            .curve(id)
            .ok_or("its profile uses a curve the sketch does not carry")?;
        let points: Vec<usize> = [curve.start, curve.end].into_iter().flatten().collect();
        // The curve joins the first group it touches; any other group it touches joins that
        // one too. Curves keep their selection order.
        let touching: Vec<usize> = groups
            .iter()
            .enumerate()
            .filter(|(_, (group, _))| points.iter().any(|p| group.contains(p)))
            .map(|(index, _)| index)
            .collect();
        match touching.split_first() {
            None => groups.push((points, curve.ids())),
            Some((&first, others)) => {
                for &other in others.iter().rev() {
                    if other < groups.len() {
                        let (other_points, other_curves) = groups.remove(other);
                        if let Some(group) = groups.get_mut(first) {
                            group.0.extend(other_points);
                            group.1.extend(other_curves);
                        }
                    }
                }
                if let Some(group) = groups.get_mut(first) {
                    group.0.extend(points);
                    group.1.extend(curve.ids());
                }
            }
        }
    }
    curves(groups.into_iter().map(|(_, curves)| curves).collect())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::history::sketch::{Curve, Shape};

    fn line(index: usize, a: usize, b: usize) -> Shape {
        Shape::Curve(Curve {
            pieces: Vec::new(),
            index,
            id: format!("l{}", index + 1),
            kind: "line",
            start: Some(a),
            end: Some(b),
            center: None,
        })
    }

    #[test]
    fn sketch_spline_chain_selection_reverses_piece_order() {
        let mut mapped = mapped();
        let Shape::Curve(mut whole) = line(0, 1, 3) else {
            return;
        };
        let Shape::Curve(a) = line(0, 1, 2) else {
            return;
        };
        let Shape::Curve(b) = line(1, 2, 3) else {
            return;
        };
        whole.kind = "compound_spline";
        whole.pieces = vec![a, b];
        mapped
            .shapes
            .insert("chain".to_owned(), Shape::Curve(whole));
        assert_eq!(
            curve_ids(Some(&json!("chain")), &mapped, false).unwrap(),
            vec!["l1", "l2"]
        );
        assert_eq!(
            curve_ids(Some(&json!("chain")), &mapped, true).unwrap(),
            vec!["l2", "l1"]
        );
        assert_eq!(
            mapped.boundaries(vec![vec!["chain".to_owned()]]).unwrap(),
            vec![vec!["l1", "l2"]]
        );
    }

    /// Two separate triangles (curves l1–l3 and l4–l6) and a circle c1.
    fn mapped() -> Mapped {
        let mut shapes = BTreeMap::new();
        for (i, (a, b)) in [(1, 2), (2, 3), (3, 1), (4, 5), (5, 6), (6, 4)]
            .into_iter()
            .enumerate()
        {
            shapes.insert(format!("e{i}"), line(i, a, b));
        }
        shapes.insert(
            "circle".to_owned(),
            Shape::Curve(Curve {
                pieces: Vec::new(),
                index: 6,
                id: "c1".to_owned(),
                kind: "circle",
                start: None,
                end: None,
                center: Some(7),
            }),
        );
        Mapped {
            feature: 1,
            normal: [0.0, 0.0, 1.0],
            origin: [0.0; 3],
            x_dir: [1.0, 0.0, 0.0],
            y_dir: [0.0, 1.0, 0.0],
            points: Vec::new(),
            shapes,
        }
    }

    fn model() -> Value {
        json!({"sketches": [{"id": "s", "profiles": [
            [{"entity": "e0"}, {"entity": "e1"}, {"entity": "e2"}],
            [{"entity": "circle"}],
        ]}]})
    }

    fn select(model: &Value, profile: Value) -> Result<Value, String> {
        let mut ctx = Context::new(model);
        ctx.sketches.insert("s".to_owned(), mapped());
        super::profile(Some(&profile), &ctx).map(|(_, selection)| selection)
    }

    #[test]
    fn whole_loop_regions_keep_their_holes() {
        let model = model();
        let selection = select(
            &model,
            json!({"kind": "sketch_regions", "value": {"sketch": "s",
            "regions": [{"region": "loops", "outer": 0, "holes": [1]}]}}),
        );
        assert_eq!(
            selection.unwrap(),
            json!({"type": "whole_boundaries", "profiles": [
                {"outer": ["l1", "l2", "l3"], "holes": [["c1"]]}
            ]})
        );
    }

    const BIG: f64 = 20.0;
    const SMALL: f64 = 1.0;
    const PIN: f64 = 7.0;

    /// A large circle and a small one crossing at its top. A stored parameter can run against
    /// the sketch's angle, so taken at face value the small arc would lie outside the large
    /// circle, a bump.
    fn crossing_circles() -> (Value, Mapped) {
        let circle = |id: &str, center: [f64; 2], radius: f64| {
            json!({"id": id, "sketch": "s", "geometry": {"kind": "circle",
                "center": {"u": center[0], "v": center[1]}, "radius": radius}})
        };
        let model = json!({
            "sketches": [{"id": "s", "profiles": [], "placement": {"kind": "resolved", "frame": {
                "origin": {"x": 0.0, "y": 0.0, "z": 0.0}, "normal": {"x": 0.0, "y": 0.0, "z": 1.0},
                "u_axis": {"x": 1.0, "y": 0.0, "z": 0.0}}}}],
            "sketch_entities": [circle("big", [0.0, 0.0], BIG), circle("small", [0.0, BIG], SMALL),
                circle("pin", [0.0, 0.0], PIN)],
        });
        let mut shapes = BTreeMap::new();
        for (index, (entity, id)) in [("big", "c1"), ("small", "c2"), ("pin", "c3")]
            .into_iter()
            .enumerate()
        {
            shapes.insert(
                entity.to_owned(),
                Shape::Curve(Curve {
                    pieces: Vec::new(),
                    index,
                    id: id.to_owned(),
                    kind: "circle",
                    start: None,
                    end: None,
                    center: None,
                }),
            );
        }
        let mut mapped = mapped();
        mapped.shapes = shapes;
        mapped.origin = [0.0, 0.0, 0.0];
        mapped.x_dir = [1.0, 0.0, 0.0];
        mapped.y_dir = [0.0, 1.0, 0.0];
        (model, mapped)
    }

    fn notch_region() -> Value {
        // Where the circles cross, by geometry: the big circle's top is cut away, closed by the
        // small circle's lower half. The parameters are minus the sketch's angle.
        let (big, small, centre) = (BIG, SMALL, BIG);
        let along = (centre * centre + big * big - small * small) / (2.0 * centre);
        let height = (big * big - along * along).sqrt();
        let delta = height.atan2(along);
        let epsilon = (centre - along).atan2(height);
        let half_pi = std::f64::consts::FRAC_PI_2;
        let pi = std::f64::consts::PI;
        json!({"region": "trimmed", "outer_boundary": [
            {"entity": "big", "parameter_range": [delta - half_pi, 3.0 * half_pi - delta]},
            {"entity": "small", "parameter_range": [epsilon, pi - epsilon], "reversed": true}]})
    }

    fn selected(model: &Value, mapped: Mapped, regions: Value) -> Value {
        let mut ctx = Context::new(model);
        ctx.sketches.insert("s".to_owned(), mapped);
        super::profile(
            Some(&json!({"kind": "sketch_regions", "value": {"sketch": "s", "regions": regions}})),
            &ctx,
        )
        .unwrap()
        .1
    }

    #[test]
    fn a_notch_is_named_by_a_point_in_the_notched_region_and_not_the_bump() {
        let (model, mapped) = crossing_circles();
        let selection = selected(&model, mapped, json!([notch_region()]));
        assert_eq!(selection["type"], "points", "{selection}");
        let point = &selection["points"][0];
        let (x, y) = (point[0].as_f64().unwrap(), point[1].as_f64().unwrap());
        assert!(x.hypot(y) < BIG, "inside the big circle: {selection}");
        assert!(
            x.hypot(y - BIG) > SMALL,
            "outside the small one: {selection}"
        );
    }

    #[test]
    fn a_shared_circle_reading_resolves_its_semicircle_but_not_an_unrelated_one() {
        let (mut model, mut mapped) = crossing_circles();
        model["sketch_entities"]
            .as_array_mut()
            .unwrap()
            .push(json!({
                "id": "diameter", "sketch": "s", "geometry": {"kind": "line",
                "start": {"u": -BIG, "v": 0.0}, "end": {"u": BIG, "v": 0.0}}
            }));
        mapped.shapes.insert(
            "diameter".to_owned(),
            Shape::Curve(Curve {
                pieces: Vec::new(),
                index: 3,
                id: "l1".to_owned(),
                kind: "line",
                start: None,
                end: None,
                center: None,
            }),
        );
        let half = json!({"region": "trimmed", "outer_boundary": [
            {"entity": "diameter", "parameter_range": [0.0, 1.0]},
            {"entity": "big", "parameter_range": [0.0, std::f64::consts::PI]}]});
        // The diameter alone closes against either semicircle. The notch fixes the reading
        // of their shared circle, so both selected regions have one resolved shape.
        let picked = selected(&model, mapped, json!([notch_region(), half]));
        assert_eq!(picked["type"], "points", "{picked}");
        assert!(picked["points"][1][1].as_f64().unwrap() < 0.0, "{picked}");

        let (mut model, mut mapped) = crossing_circles();
        model["sketch_entities"].as_array_mut().unwrap().extend([
            json!({"id": "diameter", "sketch": "s", "geometry": {"kind": "line",
                "start": {"u": -PIN, "v": 0.0}, "end": {"u": PIN, "v": 0.0}}}),
        ]);
        mapped.shapes.insert(
            "diameter".to_owned(),
            Shape::Curve(Curve {
                pieces: Vec::new(),
                index: 3,
                id: "l1".to_owned(),
                kind: "line",
                start: None,
                end: None,
                center: None,
            }),
        );
        let unrelated = json!({"region": "trimmed", "outer_boundary": [
            {"entity": "diameter", "parameter_range": [0.0, 1.0]},
            {"entity": "pin", "parameter_range": [0.0, std::f64::consts::PI]}]});
        let picked = selected(&model, mapped, json!([notch_region(), unrelated]));
        assert_eq!(picked["type"], "curves", "{picked}");
    }

    #[test]
    fn overlapping_regions_each_get_a_point_of_their_own() {
        // A region stored with no hole covers the disc selected beside it.
        let (model, mapped) = crossing_circles();
        let half = |from: f64, to: f64| json!({"entity": "pin", "parameter_range": [from, to]});
        let pi = std::f64::consts::PI;
        let selection = selected(
            &model,
            mapped,
            json!([notch_region(), {"region": "trimmed", "outer_boundary": [
                half(0.0, pi), half(pi, 2.0 * pi)]}]),
        );
        let at = |n: usize| {
            let p = &selection["points"][n];
            (p[0].as_f64().unwrap(), p[1].as_f64().unwrap())
        };
        assert!(
            at(0).0.hypot(at(0).1) > PIN,
            "the ring, not the disc: {selection}"
        );
        assert!(at(1).0.hypot(at(1).1) < PIN, "the disc: {selection}");
    }

    #[test]
    fn a_boundary_that_never_closes_stays_selected_by_its_curves() {
        // The arcs don't meet under any reading, so nothing says which region this is.
        let (model, mapped) = crossing_circles();
        let selection = selected(
            &model,
            mapped,
            json!([{"region": "trimmed", "outer_boundary": [
                {"entity": "big", "parameter_range": [0.0, 1.0]},
                {"entity": "small", "parameter_range": [0.0, 1.0]}]}]),
        );
        assert_eq!(selection["type"], "curves", "{selection}");
    }

    #[test]
    fn trimmed_regions_select_by_their_outer_curves() {
        let model = model();
        let boundary = |e: &str| json!({"entity": e, "parameter_range": [0.0, 1.0]});
        let selection = select(
            &model,
            json!({"kind": "sketch_regions", "value": {"sketch": "s",
            "regions": [{"region": "trimmed", "outer_boundary": [
                boundary("e0"), boundary("e1"), boundary("e2"), boundary("e0")]}]}}),
        );
        assert_eq!(
            selection.unwrap(),
            json!({"type": "curves", "loops": [["l1", "l2", "l3"]]})
        );
    }

    #[test]
    fn selected_entities_group_into_connected_loops() {
        let model = model();
        let selection = select(
            &model,
            json!({"kind": "sketch_entities", "value": {"sketch": "s",
            "entities": ["e0", "e3", "e1", "e4", "e2", "e5", "circle"]}}),
        )
        .unwrap();
        assert_eq!(
            selection,
            json!({"type": "curves", "loops": [["l1", "l2", "l3"], ["l4", "l5", "l6"], ["c1"]]})
        );
    }

    #[test]
    fn a_crossed_profile_without_solved_geometry_is_refused() {
        // The first triangle with a curve inside it, which its loop runs along both ways (as a
        // rectangle with its diagonals drawn comes from Fusion), and the circle.
        let model = json!({"sketches": [{"id": "s", "profiles": [
            [{"entity": "e0"}, {"entity": "e3"}, {"entity": "e1"},
             {"entity": "e3", "reversed": true}, {"entity": "e2"}],
            [{"entity": "circle"}],
        ]}]});
        let every = select(
            &model,
            json!({"kind": "sketch_profiles", "value": {"sketch": "s", "profiles": [1, 0]}}),
        );
        assert_eq!(every.unwrap(), json!({"type": "all"}));
        let some = select(
            &model,
            json!({"kind": "sketch_profiles", "value": {"sketch": "s", "profiles": [0]}}),
        );
        assert_eq!(
            some.unwrap_err(),
            "a profile crossed by its own sketch's curves is not mapped yet"
        );
        // A loop that uses each curve once is selected by its curves, as before.
        let plain = select(
            &model,
            json!({"kind": "sketch_profiles", "value": {"sketch": "s", "profiles": [1]}}),
        );
        assert_eq!(plain.unwrap(), json!({"type": "curves", "loops": [["c1"]]}));
    }

    #[test]
    fn a_selection_cadmpeg_keeps_native_is_the_profile_of_a_one_profile_sketch() {
        let one = json!({"sketches": [{"id": "s", "profiles": [
            [{"entity": "e0"}, {"entity": "e1"}, {"entity": "e2"}],
        ]}]});
        let native = json!({"kind": "sketch_selection",
            "value": {"sketch": "s", "selections": ["native#1"]}});
        assert_eq!(
            select(&one, native.clone()).unwrap(),
            json!({"type": "all"})
        );
        assert_eq!(
            select(&model(), native).unwrap_err(),
            "a profile cadmpeg does not resolve, in a sketch of 2 profiles, is not mapped yet"
        );
    }

    #[test]
    fn a_native_selection_in_a_one_profile_sketch_divided_by_a_chain_is_refused() {
        // One line across the loop is written as construction (`crossing`); a chain of two,
        // each ending inside the loop, stays solid and divides it in SolidCraft.
        let line = |id: &str, a: [f64; 2], b: [f64; 2]| {
            json!({"id": id, "sketch": "s", "geometry": {"kind": "line",
                "start": {"u": a[0], "v": a[1]}, "end": {"u": b[0], "v": b[1]}}})
        };
        let corners = [[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]];
        let square = |chain: bool| {
            let mut entities: Vec<Value> = (0..4)
                .map(|i| line(&format!("e{i}"), corners[i], corners[(i + 1) % 4]))
                .collect();
            if chain {
                entities.push(line("e4", [4.0, 0.0], [5.0, 8.0]));
                entities.push(line("e5", [5.0, 8.0], [6.0, 10.0]));
            }
            json!({
                "sketches": [{"id": "s", "profiles": [
                    [{"entity": "e0"}, {"entity": "e1"}, {"entity": "e2"}, {"entity": "e3"}],
                ], "placement": {"kind": "resolved", "frame": {
                    "origin": {"x": 0.0, "y": 0.0, "z": 0.0},
                    "normal": {"x": 0.0, "y": 0.0, "z": 1.0},
                    "u_axis": {"x": 1.0, "y": 0.0, "z": 0.0}}}}],
                "sketch_entities": entities,
            })
        };
        let mut points = corners.to_vec();
        points.extend([[4.0, 0.0], [6.0, 10.0], [5.0, 8.0]]);
        let drawing = || {
            drawn(
                &points,
                &[
                    ("s1", "line", 0, 1, None),
                    ("s2", "line", 1, 2, None),
                    ("s3", "line", 2, 3, None),
                    ("s4", "line", 3, 0, None),
                    ("s5", "line", 4, 6, None),
                    ("s6", "line", 6, 5, None),
                ],
            )
        };
        let pick = |model: &Value| {
            let mut ctx = Context::new(model);
            ctx.sketches.insert("s".to_owned(), drawing());
            let native = json!({"kind": "sketch_selection",
                "value": {"sketch": "s", "selections": ["native#1"]}});
            super::profile(Some(&native), &ctx).map(|(_, selection)| selection)
        };
        assert_eq!(pick(&square(false)).unwrap(), json!({"type": "all"}));
        assert_eq!(
            pick(&square(true)).unwrap_err(),
            "a profile cadmpeg does not resolve, in a sketch whose curves divide its one \
             profile into several regions, is not mapped yet"
        );
    }

    #[test]
    fn unknown_or_missing_references_stop_the_history() {
        let model = model();
        assert!(select(&model, json!({"kind": "native", "value": "x"})).is_err());
        assert!(select(
            &model,
            json!({"kind": "sketch_entities",
            "value": {"sketch": "s", "entities": ["nope"]}})
        )
        .is_err());
        assert!(select(
            &model,
            json!({"kind": "sketch_profiles",
            "value": {"sketch": "s", "profiles": [7]}})
        )
        .is_err());
    }

    /// A mapped sketch from point positions and curves (id, kind, start, end, centre).
    fn drawn(
        points: &[[f64; 2]],
        curves: &[(&str, &'static str, usize, usize, Option<usize>)],
    ) -> Mapped {
        let mut mapped = mapped();
        mapped.points = points
            .iter()
            .enumerate()
            .map(|(i, at)| (format!("p{i}"), *at))
            .collect();
        mapped.shapes = curves
            .iter()
            .enumerate()
            .map(|(index, (id, kind, start, end, center))| {
                (
                    format!("e{index}"),
                    Shape::Curve(Curve {
                        pieces: Vec::new(),
                        index,
                        id: (*id).to_owned(),
                        kind,
                        start: Some(*start),
                        end: Some(*end),
                        center: *center,
                    }),
                )
            })
            .collect();
        mapped
    }

    fn ring(ids: &[&str]) -> Vec<String> {
        ids.iter().map(|id| (*id).to_owned()).collect()
    }

    // Each case is one SolidCraft opens; every other order of the same loop it refuses as a
    // damaged design ("invalid analytic offset topology or boundary reference").
    #[test]
    fn whole_boundaries_run_counter_clockwise_from_the_leftmost_point() {
        // A half disc right of its diameter: the arc runs up from (0, -10).
        let d = drawn(
            &[[0.0, 0.0], [0.0, -10.0], [0.0, 10.0]],
            &[("l1", "line", 1, 2, None), ("a1", "arc", 1, 2, Some(0))],
        );
        assert_eq!(canonical_loop(ring(&["l1", "a1"]), &d), ring(&["a1", "l1"]));
        assert_eq!(canonical_loop(ring(&["a1", "l1"]), &d), ring(&["a1", "l1"]));
        // Mirrored: the arc's own middle is the leftmost point.
        let mirrored = drawn(
            &[[0.0, 0.0], [0.0, -5.0], [0.0, 5.0]],
            &[("l1", "line", 1, 2, None), ("a1", "arc", 2, 1, Some(0))],
        );
        assert_eq!(
            canonical_loop(ring(&["l1", "a1"]), &mirrored),
            ring(&["a1", "l1"])
        );
        // A square, from any curve and in either direction, starts at its lower left corner.
        let square = drawn(
            &[[-20.0, -20.0], [20.0, -20.0], [20.0, 20.0], [-20.0, 20.0]],
            &[
                ("s1", "line", 0, 1, None),
                ("s2", "line", 1, 2, None),
                ("s3", "line", 2, 3, None),
                ("s4", "line", 3, 0, None),
            ],
        );
        for order in [
            ["s3", "s4", "s1", "s2"],
            ["s4", "s3", "s2", "s1"],
            ["s2", "s1", "s4", "s3"],
        ] {
            assert_eq!(
                canonical_loop(ring(&order), &square),
                ring(&["s1", "s2", "s3", "s4"])
            );
        }
        // The leftmost vertex, not the lowest one, starts a triangle.
        let triangle = drawn(
            &[[0.0, 0.0], [10.0, -5.0], [5.0, 10.0]],
            &[
                ("e2", "line", 1, 2, None),
                ("e3", "line", 2, 0, None),
                ("e1", "line", 0, 1, None),
            ],
        );
        assert_eq!(
            canonical_loop(ring(&["e3", "e2", "e1"]), &triangle),
            ring(&["e1", "e2", "e3"])
        );
    }

    #[test]
    fn whole_boundaries_order_their_holes_the_same_way() {
        let mut points = vec![[0.0, 0.0], [0.0, -5.0], [0.0, 5.0]];
        points.extend([[-20.0, -20.0], [20.0, -20.0], [20.0, 20.0], [-20.0, 20.0]]);
        let mut mapped = drawn(
            &points,
            &[
                ("l1", "line", 1, 2, None),
                ("a1", "arc", 1, 2, Some(0)),
                ("s1", "line", 3, 4, None),
                ("s2", "line", 4, 5, None),
                ("s3", "line", 5, 6, None),
                ("s4", "line", 6, 3, None),
            ],
        );
        assert_eq!(
            canonical_loop(ring(&["l1", "a1"]), &mapped),
            ring(&["a1", "l1"])
        );
        // Curves it cannot place, or a loop that does not close, keep their source order.
        mapped.points.clear();
        assert_eq!(
            canonical_loop(ring(&["l1", "a1"]), &mapped),
            ring(&["l1", "a1"])
        );
        let open = drawn(
            &points,
            &[("s1", "line", 3, 4, None), ("s3", "line", 5, 6, None)],
        );
        assert_eq!(
            canonical_loop(ring(&["s3", "s1"]), &open),
            ring(&["s3", "s1"])
        );
    }

    #[test]
    fn the_whole_loop_of_a_one_loop_sketch_is_every_profile() {
        let one_loop = json!({"sketches": [{"id": "s", "profiles": [
            [{"entity": "e0"}, {"entity": "e1"}, {"entity": "e2"}],
        ]}]});
        let whole = json!({"kind": "sketch_regions", "value": {"sketch": "s",
            "regions": [{"region": "loops", "outer": 0, "holes": []}]}});
        assert_eq!(select(&one_loop, whole).unwrap(), json!({"type": "all"}));
        // With two loops in the sketch, the region names its boundary.
        let two = select(
            &model(),
            json!({"kind": "sketch_regions", "value": {"sketch": "s",
                "regions": [{"region": "loops", "outer": 0}]}}),
        );
        assert_eq!(two.unwrap()["type"], "whole_boundaries");
    }

    #[test]
    fn whole_boundaries_break_a_leftmost_tie_by_point_index() {
        // Left corners at the same x: the top one has the lower point index.
        let rectangle = drawn(
            &[[7.5, -2.5], [7.5, -7.5], [12.5, -7.5], [12.5, -2.5]],
            &[
                ("l1", "line", 0, 1, None),
                ("l4", "line", 1, 2, None),
                ("l5", "line", 2, 3, None),
                ("l6", "line", 3, 0, None),
            ],
        );
        assert_eq!(
            canonical_loop(ring(&["l4", "l5", "l6", "l1"]), &rectangle),
            ring(&["l1", "l4", "l5", "l6"])
        );
    }

    /// Concentric circles c1 (radius 3) and c2 (radius 6) as the sketch of a shim.
    fn rings() -> (Value, Mapped) {
        let mut mapped = mapped();
        mapped.shapes.insert(
            "outer".to_owned(),
            Shape::Curve(Curve {
                pieces: Vec::new(),
                index: 7,
                id: "c2".to_owned(),
                kind: "circle",
                start: None,
                end: None,
                center: Some(7),
            }),
        );
        let circle = |id: &str, radius: f64| {
            json!({"id": id, "sketch": "s", "geometry": {"kind": "circle",
                "center": {"u": 0.0, "v": 0.0}, "radius": radius}})
        };
        let model = json!({
            "sketches": [{"id": "s", "profiles": [
                [{"entity": "outer"}], [{"entity": "circle"}],
            ]}],
            "sketch_entities": [circle("outer", 6.0), circle("circle", 3.0)],
        });
        (model, mapped)
    }

    #[test]
    fn concentric_loops_taken_together_are_a_ring() {
        let (model, mapped) = rings();
        let mut ctx = Context::new(&model);
        ctx.sketches.insert("s".to_owned(), mapped);
        let by_index =
            json!({"kind": "sketch_profiles", "value": {"sketch": "s", "profiles": [1, 0]}});
        let whole = super::profile(Some(&by_index), &ctx).unwrap().1;
        assert_eq!(
            whole,
            json!({"type": "whole_boundaries", "profiles": [
                {"outer": ["c2"], "holes": [["c1"]]}
            ]})
        );
        // The whole sketch is the same two loops.
        let sketch = json!({"kind": "sketch", "value": "s"});
        assert_eq!(super::profile(Some(&sketch), &ctx).unwrap().1, whole);
        // The inner loop alone, or loops that do not nest, are not a ring.
        let inner = json!({"kind": "sketch_profiles", "value": {"sketch": "s", "profiles": [1]}});
        assert_eq!(
            super::profile(Some(&inner), &ctx).unwrap().1,
            json!({"type": "curves", "loops": [["c1"]]})
        );
    }
}
