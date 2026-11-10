//! Resolve an assembly-context sketch against one unambiguous occurrence of its user.

use super::*;

fn xyz(value: &Value) -> Option<[f64; 3]> {
    Some([
        value.get("x")?.as_f64()?,
        value.get("y")?.as_f64()?,
        value.get("z")?.as_f64()?,
    ])
}

fn moved(point: [f64; 3], by: &Placement3x4) -> [f64; 3] {
    by.map(|[a, b, c, t]| {
        let [x, y, z] = point;
        a * x + b * y + c * z + t
    })
}

// Walk only topology: curve supports and surface origins are not body vertices.
type BodyBox = ([f64; 3], [f64; 3]);

fn body_box(slices: &slicing::Index<'_>, bodies: &[&str]) -> Result<Option<BodyBox>, String> {
    let mut queue: Vec<String> = bodies.iter().map(|id| (*id).to_owned()).collect();
    let mut seen = BTreeSet::new();
    let mut low = [f64::INFINITY; 3];
    let mut high = [f64::NEG_INFINITY; 3];
    let mut found = false;
    while let Some(id) = queue.pop() {
        if !seen.insert(id.clone()) {
            continue;
        }
        let Some(entry) = slices.rows.get(&id) else {
            continue;
        };
        let value = entry.row.value()?;
        if entry.arena == "vertices" {
            if let Some(point) = value
                .get("point")
                .and_then(Value::as_str)
                .and_then(|id| slices.rows.get(id))
            {
                let point = point.row.value()?;
                if let Some(position) = point.get("position").and_then(xyz) {
                    for ((lo, hi), p) in low.iter_mut().zip(&mut high).zip(position) {
                        *lo = lo.min(p);
                        *hi = hi.max(p);
                    }
                    found = true;
                }
            }
            continue;
        }
        let mut refs = Vec::new();
        strings(&value, &mut refs);
        queue.extend(
            refs.into_iter()
                .filter(|id| {
                    slices.rows.get(*id).is_some_and(|entry| {
                        matches!(
                            entry.arena,
                            "regions"
                                | "shells"
                                | "faces"
                                | "loops"
                                | "coedges"
                                | "edges"
                                | "vertices"
                        )
                    })
                })
                .map(str::to_owned),
        );
    }
    Ok(found.then_some((low, high)))
}

fn entities(value: &Value, out: &mut BTreeSet<String>) {
    match value {
        Value::Object(fields) => {
            if let Some(id) = fields.get("entity").and_then(Value::as_str) {
                out.insert(id.to_owned());
            }
            for value in fields.values() {
                entities(value, out);
            }
        }
        Value::Array(values) => {
            for value in values {
                entities(value, out);
            }
        }
        _ => {}
    }
}

fn profile(sketch: &Value, index: &Value, out: &mut BTreeSet<String>) -> Option<()> {
    let index = usize::try_from(index.as_u64()?).ok()?;
    entities(sketch.get("profiles")?.get(index)?, out);
    Some(())
}

fn selections(
    value: &Value,
    sketch: &Value,
    all: &[String],
    out: &mut BTreeSet<String>,
) -> Option<()> {
    match value {
        Value::Object(fields) => {
            let selected = fields.get("value");
            let id = sketch.get("id")?.as_str()?;
            if fields.get("kind").and_then(Value::as_str) == Some("sketch")
                && selected?.as_str() == Some(id)
            {
                out.extend(all.iter().cloned());
                return Some(());
            }
            if selected
                .and_then(|v| v.get("sketch"))
                .and_then(Value::as_str)
                == Some(id)
            {
                let selected = selected?;
                match fields.get("kind")?.as_str()? {
                    "sketch_profiles" => {
                        for index in selected.get("profiles")?.as_array()? {
                            profile(sketch, index, out)?;
                        }
                    }
                    "sketch_regions" => {
                        for region in selected.get("regions")?.as_array()? {
                            match region.get("region")?.as_str()? {
                                "loops" => {
                                    profile(sketch, region.get("outer")?, out)?;
                                    for index in region
                                        .get("holes")
                                        .and_then(Value::as_array)
                                        .into_iter()
                                        .flatten()
                                    {
                                        profile(sketch, index, out)?;
                                    }
                                }
                                "trimmed" => entities(region, out),
                                _ => return None,
                            }
                        }
                    }
                    "sketch_entities" => {
                        for id in selected.get("entities")?.as_array()? {
                            out.insert(id.as_str()?.to_owned());
                        }
                    }
                    // A native selection names no curves to test; the other features'
                    // profiles still have to fit, and the history stops at it anyway.
                    "sketch_selection" => {}
                    _ => return None,
                }
                return Some(());
            }
            for value in fields.values() {
                selections(value, sketch, all, out)?;
            }
        }
        Value::Array(values) => {
            for value in values {
                selections(value, sketch, all, out)?;
            }
        }
        _ => {}
    }
    Some(())
}

fn points<'a>(
    slices: &slicing::Index<'_>,
    sketch: &Value,
    features: impl Iterator<Item = &'a Value>,
) -> Option<Vec<[f64; 3]>> {
    let mut all = Vec::new();
    for (id, entry) in &slices.rows {
        if entry.arena == "sketch_entities" {
            let row = entry.row.value().ok()?;
            if row.get("sketch") == sketch.get("id") {
                all.push(id.clone());
            }
        }
    }
    let mut selected = BTreeSet::new();
    for feature in features {
        selections(feature.get("definition")?, sketch, &all, &mut selected)?;
    }
    let frame = sketch.pointer("/placement/frame")?;
    let origin = xyz(frame.get("origin")?)?;
    let u = xyz(frame.get("u_axis")?)?;
    let [nx, ny, nz] = xyz(frame.get("normal")?)?;
    let [ux, uy, uz] = u;
    let v = [ny * uz - nz * uy, nz * ux - nx * uz, nx * uy - ny * ux];
    let mut points = Vec::new();
    for id in selected {
        let entity = slices.rows.get(&id)?.row.value().ok()?;
        let geometry = entity.get("geometry")?;
        let mut local = Vec::new();
        for key in ["start", "end", "center", "position"] {
            if let Some(point) = geometry.get(key) {
                local.push([point.get("u")?.as_f64()?, point.get("v")?.as_f64()?]);
            }
        }
        match geometry.get("kind")?.as_str()? {
            "circle" | "arc" => {
                let center = geometry.get("center")?;
                let a = center.get("u")?.as_f64()?;
                let b = center.get("v")?.as_f64()?;
                let radius = geometry.get("radius")?.as_f64()?;
                if geometry.get("kind")?.as_str()? == "circle" {
                    local.extend([
                        [a - radius, b],
                        [a + radius, b],
                        [a, b - radius],
                        [a, b + radius],
                    ]);
                } else {
                    for key in ["start_angle", "end_angle"] {
                        let angle = geometry.get(key)?.as_f64()?;
                        local.push([a + radius * angle.cos(), b + radius * angle.sin()]);
                    }
                }
            }
            _ => {}
        }
        if local.is_empty() {
            return None;
        }
        points.extend(local.into_iter().map(|[a, b]| {
            std::array::from_fn(|i| {
                origin.get(i).copied().unwrap_or(0.0)
                    + a * u.get(i).copied().unwrap_or(0.0)
                    + b * v.get(i).copied().unwrap_or(0.0)
            })
        }));
    }
    (!points.is_empty()).then_some(points)
}

/// Follow the stored occurrence path, including a root occurrence when one is present.
fn world<'a>(occurrences: &'a [Value], mut row: &'a Value) -> Option<Placement3x4> {
    let mut world = transform(row).ok()?;
    for _ in 0..MAX_DEPTH {
        let Some(parent) = row.pointer("/parent/occurrence").and_then(Value::as_str) else {
            return Some(world);
        };
        let Some(above) = occurrences
            .iter()
            .find(|row| row.get("id").and_then(Value::as_str) == Some(parent))
        else {
            return Some(world);
        };
        world = compose(&transform(above).ok()?, &world);
        row = above;
    }
    None
}

pub(super) fn choose<'a>(
    slices: &slicing::Index<'_>,
    bodies: &[&str],
    owner: HistoryOwner<'_>,
    occurrences: &[Value],
    there: &Placement3x4,
    sketch: Option<&Value>,
    features: impl Iterator<Item = &'a Value>,
) -> Result<Option<Placement3x4>, String> {
    let HistoryOwner::Definition(definition) = owner else {
        return Ok(None);
    };
    let Some(points) = sketch.and_then(|sketch| points(slices, sketch, features)) else {
        return Ok(None);
    };
    let Some((low, high)) = body_box(slices, bodies)? else {
        return Ok(None);
    };
    let tolerance = low
        .iter()
        .zip(high)
        .map(|(lo, hi)| hi - lo)
        .fold(1.0_f64, f64::max)
        * 1e-6;
    if !tolerance.is_finite() {
        return Ok(None);
    }
    let candidates: Vec<_> = occurrences
        .iter()
        .filter(|row| scope(row) == Some("local") && definition_key(row) == Some(definition))
        .collect();
    if candidates.len() > MAX_PLACEMENTS {
        return Ok(None);
    }
    let mut chosen = None;
    for row in candidates {
        let Some(here) = world(occurrences, row) else {
            return Ok(None);
        };
        let movement = compose(&inverse(&here), there);
        if points.iter().all(|point| {
            moved(*point, &movement)
                .into_iter()
                .zip(low)
                .zip(high)
                .all(|((p, lo), hi)| p.is_finite() && p >= lo - tolerance && p <= hi + tolerance)
        }) {
            if chosen.is_some() {
                return Ok(None);
            }
            // The drawn-against instance puts the profile on the part; replay and the body check still expose a wrong pick as a body mismatch.
            chosen = Some(movement);
        }
    }
    Ok(chosen)
}
