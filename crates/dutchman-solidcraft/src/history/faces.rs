//! Body and face appearances of written geometry, as SolidCraft looks.
//!
//! SolidCraft finds a face look's face by its body and a point on the face. A replayed body is
//! built where the source's final body is, so a point inside the source face finds the same
//! face. Analytic picks handle planes and simple round faces; bounded trim charts
//! provide interior picks on other resolved surfaces, while respecting holes.

mod uv;

use dutchman_core::AppearanceLoss;
use serde_json::{json, Value};

use super::sketch::{dot, v3, V3};
use super::Context;

/// A single planar face parallel to the sketch, as a SolidCraft pick: a resolved face on an
/// earlier named body, or a face of `feature`'s own input topology rebuilt from its boundary.
/// Native and generated selections need their own topology resolver.
pub(super) fn pick(
    ctx: &Context,
    feature: &Value,
    selection: Option<&Value>,
    normal: V3,
) -> Result<V3, String> {
    let selection = selection.ok_or("its face selection is missing")?;
    if selection.get("kind").and_then(Value::as_str) == Some("historical") {
        let (point, face_normal) = super::face_profile::planar_face(
            feature,
            selection.get("value").unwrap_or(&Value::Null),
            ctx,
        )?;
        if dot(face_normal, normal).abs() < 1.0 - 1e-9 {
            return Err(
                "an extrude face not parallel to its sketch is not supported yet".to_owned(),
            );
        }
        return Ok(point);
    }
    let ids = match selection.get("kind").and_then(Value::as_str) {
        Some("faces") => selection.get("value"),
        Some("resolved") => selection.pointer("/value/faces"),
        _ => return Err("its face selection is not resolved to model faces".to_owned()),
    }
    .and_then(Value::as_array)
    .ok_or("its face selection is malformed")?;
    if ids.len() != 1 {
        return Err("a selection without exactly one face is not supported yet".to_owned());
    }
    let id = ids
        .first()
        .and_then(Value::as_str)
        .ok_or("its face is missing")?;
    let face = ctx.row("faces", id).ok_or("its selected face is missing")?;
    let surface = face
        .get("surface")
        .and_then(Value::as_str)
        .and_then(|id| ctx.row("surfaces", id))
        .ok_or("its face surface is missing")?;
    let face_normal = v3(surface.pointer("/geometry/normal"))
        .and_then(super::sketch::unit)
        .ok_or("its face normal is missing")?;
    if surface.pointer("/geometry/kind").and_then(Value::as_str) != Some("plane")
        || dot(face_normal, normal).abs() < 1.0 - 1e-9
    {
        return Err("an extrude face not parallel to its sketch is not supported yet".to_owned());
    }
    let earlier = ctx.named_bodies.iter().any(|(body, _)| {
        ctx.row("bodies", body).is_some_and(|body| {
            faces(ctx, body)
                .iter()
                .any(|face| face.get("id").and_then(Value::as_str) == Some(id))
        })
    });
    if !earlier {
        return Err("its selected face is not on an earlier named body".to_owned());
    }
    // The colour picker approximates loops by their vertices. For a feature end,
    // require a straight polygon without holes so the pick is proven inside it.
    let loops = face
        .pointer("/loops/loops")
        .and_then(Value::as_array)
        .filter(|loops| loops.len() == 1)
        .ok_or("a terminating face with holes is not supported yet")?;
    let lp = loops
        .first()
        .and_then(Value::as_str)
        .and_then(|id| ctx.row("loops", id))
        .ok_or("its face loop is missing")?;
    for coedge in lp
        .pointer("/boundary/coedges")
        .and_then(Value::as_array)
        .ok_or("its face boundary is missing")?
    {
        let edge = coedge
            .as_str()
            .and_then(|id| ctx.row("coedges", id))
            .and_then(|row| row.get("edge"))
            .and_then(Value::as_str)
            .and_then(|id| ctx.row("edges", id))
            .ok_or("its face edge is missing")?;
        let curve = edge
            .pointer("/carrier/curve")
            .and_then(Value::as_str)
            .and_then(|id| ctx.row("curves", id))
            .ok_or("its face curve is missing")?;
        if curve.pointer("/geometry/kind").and_then(Value::as_str) != Some("line") {
            return Err("a terminating face with curved edges is not supported yet".to_owned());
        }
    }
    point_on(ctx, face).ok_or_else(|| "its face has no resolved planar interior point".to_owned())
}

/// Body and face bindings override inline colours. Uniform, fully painted bodies need only
/// one body look; mixed bodies retain point-selected face looks in definition coordinates.
pub(crate) fn looks(ctx: &mut Context, report: &mut dutchman_core::Report) -> Vec<Value> {
    use std::collections::{BTreeMap, BTreeSet};

    let mut bound: BTreeMap<&str, Vec<&Value>> = BTreeMap::new();
    for binding in ctx.arena("appearance_bindings") {
        if let Some(id) = binding.pointer("/target/id").and_then(Value::as_str) {
            bound.entry(id).or_default().push(binding);
        }
    }
    let wire_edges: BTreeSet<&str> = ctx
        .arena("shells")
        .iter()
        .filter_map(|shell| shell.get("members").and_then(Value::as_array))
        .flatten()
        .filter(|member| member.get("kind").and_then(Value::as_str) == Some("wire_edge"))
        .filter_map(|member| member.get("id").and_then(Value::as_str))
        .collect();
    let mut applied = BTreeSet::new();
    let mut named = BTreeSet::new();
    let resolve = |row: &Value| -> Option<Value> {
        let id = row.get("id")?.as_str()?;
        if let Some(bindings) = bound.get(id) {
            let first = bindings.first()?.get("appearance")?.as_str()?;
            if bindings
                .iter()
                .any(|b| b.get("appearance").and_then(Value::as_str) != Some(first))
            {
                return None;
            }
            let appearance = ctx.row("appearances", first)?;
            let mut look = super::extrude::look(appearance.get("base_color")?)?;
            if let (Some(name), Some(fields)) = (appearance.get("name"), look.as_object_mut()) {
                fields.insert("name".to_owned(), name.clone());
            }
            Some(look)
        } else {
            row.get("color").and_then(super::extrude::look)
        }
    };
    let mut body_looks = BTreeMap::new();
    let mut out = Vec::new();
    for (body_id, name) in &ctx.named_bodies {
        let Some(body) = ctx.row("bodies", body_id) else {
            continue;
        };
        named.insert(body_id.as_str());
        let body_look = resolve(body);
        if let Some(look) = &body_look {
            body_looks.insert(name.clone(), look.clone());
            applied.insert(body_id.as_str());
        }
        let faces = if let Some(region) = ctx.imported_regions.get(name) {
            faces(ctx, &json!({"regions": [region]}))
        } else {
            faces(ctx, body)
        };
        let painted: Vec<_> = faces.iter().map(|face| resolve(face)).collect();
        named.extend(
            faces
                .iter()
                .filter_map(|face| face.get("id").and_then(Value::as_str)),
        );
        let uniform = painted.first().and_then(Option::as_ref).filter(|first| {
            !bound.contains_key(body_id.as_str())
                && painted.iter().all(|look| look.as_ref() == Some(*first))
        });
        if let Some(look) = uniform {
            body_looks.insert(name.clone(), look.clone());
            for face in &faces {
                if let Some(id) = face.get("id").and_then(Value::as_str) {
                    applied.insert(id);
                }
            }
            continue;
        }
        for (face, look) in faces.iter().zip(painted) {
            let Some(look) = look else { continue };
            let written = if Some(&look) == body_look.as_ref() {
                true
            } else if let Some(point) = appearance_point(ctx, face).and_then(|p| placed(body, p)) {
                out.push(json!({"body": name, "point": point, "look": look}));
                true
            } else {
                if face
                    .get("id")
                    .and_then(Value::as_str)
                    .is_some_and(|id| !bound.contains_key(id))
                {
                    report.push_appearance(
                        AppearanceLoss::FacePickMissed,
                        format!(
                            "Face {} has no resolved interior point; its colour is left out",
                            face.get("id").unwrap_or(&Value::Null)
                        ),
                    );
                }
                false
            };
            if written {
                if let Some(id) = face.get("id").and_then(Value::as_str) {
                    applied.insert(id);
                }
            }
        }
    }
    for binding in ctx.arena("appearance_bindings") {
        let target = binding.pointer("/target/id").and_then(Value::as_str);
        if !target.is_some_and(|id| applied.contains(id)) {
            let kind = binding
                .pointer("/target/kind")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            let appearance = binding
                .get("appearance")
                .and_then(Value::as_str)
                .and_then(|id| ctx.row("appearances", id));
            let owner = target.and_then(|id| {
                if kind == "body" {
                    return Some(id);
                }
                let shell = ctx.row("faces", id)?.get("shell")?.as_str()?;
                let region = ctx.row("shells", shell)?.get("region")?.as_str()?;
                ctx.row("regions", region)?.get("body")?.as_str()
            });
            let wire = owner
                .and_then(|id| ctx.row("bodies", id))
                .is_some_and(|body| body.get("kind").and_then(Value::as_str) == Some("wire"))
                || (kind == "edge" && target.is_some_and(|id| wire_edges.contains(id)));
            let (cause, reason) = if wire {
                (
                    AppearanceLoss::Wire,
                    "its target is a wire body or saved free edge",
                )
            } else if !matches!(kind, "body" | "face") {
                (
                    AppearanceLoss::Other,
                    "its target kind is not supported yet",
                )
            } else if target
                .and_then(|id| ctx.row(if kind == "body" { "bodies" } else { "faces" }, id))
                .is_none()
            {
                (
                    AppearanceLoss::TargetAbsent,
                    "its target is absent from the decoded model",
                )
            } else if appearance.is_none() {
                (AppearanceLoss::Other, "its appearance does not exist")
            } else if appearance
                .and_then(|a| a.get("base_color"))
                .and_then(super::extrude::look)
                .is_none()
            {
                (
                    AppearanceLoss::Other,
                    "its appearance has no resolved base colour",
                )
            } else if target.and_then(|id| bound.get(id)).is_some_and(|bindings| {
                bindings
                    .iter()
                    .any(|b| b.get("appearance") != binding.get("appearance"))
            }) {
                (
                    AppearanceLoss::Other,
                    "its target has conflicting appearance bindings",
                )
            } else if owner.is_some_and(|id| ctx.unwritten_bodies.contains(id)) {
                (
                    AppearanceLoss::UnwritableBody,
                    "its body has no geometry the STEP writer can write",
                )
            } else if !target.is_some_and(|id| named.contains(id)) {
                (AppearanceLoss::Other, "its target has no written body name")
            } else {
                (
                    AppearanceLoss::FacePickMissed,
                    "its face has no resolved interior point",
                )
            };
            report.push_appearance(
                cause,
                format!(
                    "Appearance binding {} to {kind} {} is left out: {reason}",
                    binding
                        .get("id")
                        .and_then(Value::as_str)
                        .unwrap_or("unnamed"),
                    target.unwrap_or("unnamed")
                ),
            );
        }
    }
    ctx.looks.extend(body_looks);
    out
}

/// Prefer the actual trim chart: vertices alone cannot distinguish a long
/// cylindrical arc from its complement, or exclude a hole in a planar face.
fn appearance_point(ctx: &Context, face: &Value) -> Option<V3> {
    let geometry = ctx
        .row("surfaces", face.get("surface")?.as_str()?)?
        .get("geometry")?;
    uv::point(ctx, face, geometry).or_else(|| point_on(ctx, face))
}

fn placed(body: &Value, p: V3) -> Option<V3> {
    let Some(rows) = body.get("transform").and_then(Value::as_array) else {
        return Some(p);
    };
    let mut out = [0.0; 3];
    for (value, row) in out.iter_mut().zip(rows) {
        let row = row.as_array()?;
        *value = row.get(3)?.as_f64()?;
        for (coefficient, coordinate) in row.iter().take(3).zip(p) {
            *value += coefficient.as_f64()? * coordinate;
        }
    }
    out.iter().all(|v| v.is_finite()).then_some(out)
}

/// STEP emits one surface/solid shape per region, in arena order. Only name that
/// sequence when its shape count agrees, so an omitted region cannot shift a look.
pub(super) fn import_names(ctx: &mut Context, ids: &[&str], step: Option<&str>) -> Vec<String> {
    let mut named = Vec::new();
    let mut taken: std::collections::BTreeSet<_> = ctx
        .named_bodies
        .iter()
        .map(|(_, name)| name.clone())
        .collect();
    let mut regions = std::collections::BTreeMap::new();
    let mut defaults = false;
    for region in ctx.arena("regions") {
        let Some(body_id) = region
            .get("body")
            .and_then(Value::as_str)
            .filter(|id| ids.contains(id))
        else {
            continue;
        };
        // STEP emits no shape for a region without shells (including an empty
        // exterior/void region). It must not consume a name in the import order.
        if region
            .get("shells")
            .and_then(Value::as_array)
            .is_some_and(Vec::is_empty)
        {
            continue;
        }
        let Some(body) = ctx.row("bodies", body_id) else {
            continue;
        };
        match body.get("kind").and_then(Value::as_str) {
            // Saved free edges are carried beside solids in STEP, but do not
            // produce SolidCraft solid/surface bodies or consume their names.
            Some("wire") => {
                defaults = true;
                continue;
            }
            Some("solid" | "sheet") => {}
            _ => return Vec::new(),
        }
        let base = body.get("name").and_then(Value::as_str).unwrap_or("Body");
        let mut name = base.to_owned();
        let mut n = 2;
        while !taken.insert(name.clone()) {
            name = format!("{base} ({n})");
            n += 1;
        }
        if let Some(region_id) = region.get("id").and_then(Value::as_str) {
            regions.insert(name.clone(), region_id.to_owned());
        }
        named.push((body_id.to_owned(), name));
    }
    if let Some(step) = step {
        let shapes = shape_count(step);
        if shapes != named.len() {
            defaults = true;
            // A sibling can have topology that STEP cannot write at all. Prove
            // each body's shape count using the same guarded restatement before
            // omitting its name; never shift colours by guessing which vanished.
            let Some(source) = ctx.source else {
                return Vec::new();
            };
            let bodies: std::collections::BTreeSet<_> =
                named.iter().map(|(id, _)| id.as_str()).collect();
            if bodies.len() > 32 {
                return Vec::new();
            }
            let mut omitted = std::collections::BTreeSet::new();
            for body in bodies {
                let Ok(part) = dutchman_core::with_bodies(source, &[body], source.name.clone())
                else {
                    return Vec::new();
                };
                let count = match crate::step::restate(&part, &mut dutchman_core::Report::default())
                {
                    Ok(text) => shape_count(&text),
                    Err(dutchman_core::Error::NoGeometry(_)) => 0,
                    Err(_) => return Vec::new(),
                };
                if count == 0 {
                    omitted.insert(body.to_owned());
                } else if count != named.iter().filter(|(id, _)| id == body).count() {
                    return Vec::new();
                }
            }
            named.retain(|(body, _)| !omitted.contains(body));
            if shapes != named.len() {
                return Vec::new();
            }
            let retained: std::collections::BTreeSet<_> =
                named.iter().map(|(_, name)| name.as_str()).collect();
            regions.retain(|name, _| retained.contains(name.as_str()));
            ctx.unwritten_bodies.extend(omitted);
        }
    }
    if defaults {
        // These imports previously had no explicit CADIR names. Keep the STEP
        // product names SolidCraft already gave their surviving shapes, while
        // still recording the correspondence needed by body and face looks.
        let Some(base) = step.and_then(crate::step::import_name) else {
            return Vec::new();
        };
        let mut old_regions = std::mem::take(&mut regions);
        let mut taken: std::collections::BTreeSet<_> = ctx
            .named_bodies
            .iter()
            .map(|(_, name)| name.clone())
            .collect();
        for (_, name) in &mut named {
            let mut new = base.clone();
            let mut n = 2;
            while !taken.insert(new.clone()) {
                new = format!("{base} ({n})");
                n += 1;
            }
            if let Some(region) = old_regions.remove(name) {
                regions.insert(new.clone(), region);
            }
            *name = new.clone();
            ctx.default_bodies.insert(new, base.clone());
        }
    }
    let names = named.iter().map(|(_, name)| name.clone()).collect();
    ctx.imported_regions.extend(regions);
    ctx.named_bodies.extend(named);
    names
}

pub(super) fn shape_count(step: &str) -> usize {
    step.lines()
        .filter_map(|line| line.split_once('='))
        .filter(|(id, definition)| {
            id.trim()
                .strip_prefix('#')
                .is_some_and(|id| !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit()))
                && [
                    "MANIFOLD_SOLID_BREP(",
                    "BREP_WITH_VOIDS(",
                    "SHELL_BASED_SURFACE_MODEL(",
                ]
                .iter()
                .any(|kind| definition.trim_start().starts_with(kind))
        })
        .count()
}

/// The faces of `body`.
fn faces<'a>(ctx: &Context<'a>, body: &Value) -> Vec<&'a Value> {
    fn ids<'a>(row: &'a Value, key: &str) -> Vec<&'a str> {
        row.get(key)
            .and_then(Value::as_array)
            .map(|ids| ids.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default()
    }
    let mut out = Vec::new();
    for region in ids(body, "regions") {
        let Some(region) = ctx.row("regions", region) else {
            continue;
        };
        for shell in ids(region, "shells") {
            let Some(shell) = ctx.row("shells", shell) else {
                continue;
            };
            for member in shell
                .get("members")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                if member.get("kind").and_then(Value::as_str) != Some("face") {
                    continue;
                }
                if let Some(face) = member
                    .get("id")
                    .and_then(Value::as_str)
                    .and_then(|id| ctx.row("faces", id))
                {
                    out.push(face);
                }
            }
        }
    }
    out
}

/// A point inside a planar face, from its outer loop.
fn point_on(ctx: &Context, face: &Value) -> Option<V3> {
    let surface = ctx.row("surfaces", face.get("surface")?.as_str()?)?;
    let geometry = surface.get("geometry")?;
    if matches!(geometry.get("kind")?.as_str()?, "cylinder" | "cone") {
        return cylinder_point(ctx, face, geometry);
    }
    if geometry.get("kind")?.as_str()? != "plane" {
        return None;
    }
    let normal = v3(geometry.get("normal"))?;
    let u = v3(geometry.get("u_axis"))?;
    let v = [
        normal[1] * u[2] - normal[2] * u[1],
        normal[2] * u[0] - normal[0] * u[2],
        normal[0] * u[1] - normal[1] * u[0],
    ];
    let outer = face
        .pointer("/loops/loops")?
        .as_array()?
        .first()?
        .as_str()?;
    let lp = ctx.row("loops", outer)?;
    let mut corners: Vec<V3> = Vec::new();
    for coedge in lp.pointer("/boundary/coedges")?.as_array()? {
        let coedge = ctx.row("coedges", coedge.as_str()?)?;
        let edge = ctx.row("edges", coedge.get("edge")?.as_str()?)?;
        let reversed = coedge.get("sense").and_then(Value::as_str) == Some("reversed");
        let vertex = edge.get(if reversed { "end" } else { "start" })?.as_str()?;
        let point = ctx.row(
            "points",
            ctx.row("vertices", vertex)?.get("point")?.as_str()?,
        )?;
        corners.push(v3(point.get("position"))?);
    }
    if corners.len() < 3 {
        return None;
    }
    let flat: Vec<[f64; 2]> = corners.iter().map(|p| [dot(*p, u), dot(*p, v)]).collect();
    let n = flat.len();
    (0..n)
        .find_map(|i| {
            let a = *corners.get((i + n - 1) % n)?;
            let b = *corners.get(i)?;
            let c = *corners.get((i + 1) % n)?;
            let centre = [
                (a[0] + b[0] + c[0]) / 3.0,
                (a[1] + b[1] + c[1]) / 3.0,
                (a[2] + b[2] + c[2]) / 3.0,
            ];
            let degenerate = {
                let ab = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
                let ac = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
                let cross = [
                    ab[1] * ac[2] - ab[2] * ac[1],
                    ab[2] * ac[0] - ab[0] * ac[2],
                    ab[0] * ac[1] - ab[1] * ac[0],
                ];
                dot(cross, cross) < 1e-18
            };
            (!degenerate && inside(&flat, [dot(centre, u), dot(centre, v)])).then_some(centre)
        })
        .filter(|p| p.iter().all(|c| c.is_finite()))
}

/// A point on a cylindrical face, from where its boundary meets the cylinder: at the middle of
/// its height, and at the angle opposite the seam of a face that goes all the way round, or the
/// middle of the shorter way between its two sides. Faces whose corners leave that unclear
/// (a hole in the cylinder, sides half a turn apart, corners all round) give none.
fn cylinder_point(ctx: &Context, face: &Value, geometry: &Value) -> Option<V3> {
    let origin = v3(geometry.get("origin"))?;
    let axis = super::sketch::unit(v3(geometry.get("axis"))?)?;
    let reference = v3(geometry.get("ref_direction"))?;
    let cone = geometry.get("kind").and_then(Value::as_str) == Some("cone");
    let radius = geometry.get("radius")?.as_f64().filter(|r| *r >= 0.0)?;
    let growth = if cone {
        geometry.get("half_angle")?.as_f64()?.tan()
    } else {
        0.0
    };
    let ratio = if cone {
        geometry.get("ratio")?.as_f64()?
    } else {
        1.0
    };
    if !growth.is_finite() || !ratio.is_finite() || ratio <= 0.0 {
        return None;
    }
    let side = super::sketch::unit(super::sketch::cross(axis, reference))?;
    let reference = super::sketch::unit(super::sketch::cross(side, axis))?;
    let loops = face.pointer("/loops/loops")?.as_array()?;
    if loops.is_empty() || loops.len() > 2 {
        return None;
    }
    let mut heights = Vec::new();
    let mut angles: Vec<f64> = Vec::new();
    for lp in loops {
        let lp = ctx.row("loops", lp.as_str()?)?;
        for coedge in lp.pointer("/boundary/coedges")?.as_array()? {
            let coedge = ctx.row("coedges", coedge.as_str()?)?;
            let edge = ctx.row("edges", coedge.get("edge")?.as_str()?)?;
            let vertex = edge.get("start")?.as_str()?;
            let point = ctx.row(
                "points",
                ctx.row("vertices", vertex)?.get("point")?.as_str()?,
            )?;
            let p = v3(point.get("position"))?;
            let d = [p[0] - origin[0], p[1] - origin[1], p[2] - origin[2]];
            let height = dot(d, axis);
            let radial = [
                d[0] - height * axis[0],
                d[1] - height * axis[1],
                d[2] - height * axis[2],
            ];
            heights.push(height);
            if dot(radial, radial) > 1e-16 {
                angles.push((dot(radial, side) / ratio).atan2(dot(radial, reference)));
            }
        }
    }
    let low = heights.iter().copied().fold(f64::INFINITY, f64::min);
    let high = heights.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    if !(high - low).is_finite() || high - low <= 1e-6 {
        return None;
    }
    let tau = std::f64::consts::TAU;
    angles.iter_mut().for_each(|a| *a = a.rem_euclid(tau));
    angles.sort_by(f64::total_cmp);
    angles.dedup_by(|a, b| (*a - *b).abs() < 1e-6);
    // The empty stretches between neighbouring corner angles, going round.
    let n = angles.len();
    let mut widest = (0.0, 0.0);
    for i in 0..n {
        let from = *angles.get(i)?;
        let to = *angles.get((i + 1) % n)?;
        let gap = if n == 1 {
            tau
        } else {
            (to - from).rem_euclid(tau)
        };
        if gap > widest.0 {
            widest = (gap, from);
        }
    }
    let (gap, from) = widest;
    let middle = if n == 1 {
        from + std::f64::consts::PI
    } else if gap > std::f64::consts::PI + 1e-6 {
        // The face is the shorter way round, opposite the widest empty stretch.
        from + gap + (tau - gap) / 2.0
    } else {
        return None;
    };
    let mid_height = (low + high) / 2.0;
    let radius = radius + mid_height * growth;
    if !radius.is_finite() || radius <= 0.0 {
        return None;
    }
    let p = [0, 1, 2].map(|i| {
        origin.get(i).copied().unwrap_or(0.0)
            + mid_height * axis.get(i).copied().unwrap_or(0.0)
            + radius
                * (middle.cos() * reference.get(i).copied().unwrap_or(0.0)
                    + ratio * middle.sin() * side.get(i).copied().unwrap_or(0.0))
    });
    p.iter().all(|c| c.is_finite()).then_some(p)
}

/// Whether `p` lies inside the polygon `poly` (even-odd rule).
fn inside(poly: &[[f64; 2]], p: [f64; 2]) -> bool {
    let mut inside = false;
    let n = poly.len();
    for i in 0..n {
        let (Some(a), Some(b)) = (poly.get(i), poly.get((i + n - 1) % n)) else {
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

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]

    use super::*;

    /// A model whose feature `f` has an input topology holding one rectangular face
    /// `x0..x1` by `y0..y1` on the plane `z`, given only by its sampled boundary.
    fn input_face(corners: [[f64; 3]; 4]) -> Value {
        let p = |c: [f64; 3]| json!({"x": c[0], "y": c[1], "z": c[2]});
        let rows: Vec<Value> = (0..4)
            .map(|k| {
                let (a, b) = (corners[k], corners[(k + 1) % 4]);
                let mid = [
                    (a[0] + b[0]) / 2.0,
                    (a[1] + b[1]) / 2.0,
                    (a[2] + b[2]) / 2.0,
                ];
                let tangent = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
                json!({"edge": format!("e{k}"), "start": p(a), "end": p(b),
                    "interior": p(mid), "tangent": p(tangent), "faces": ["face"]})
            })
            .collect();
        json!({"feature_input_topologies": [
            {"id": "st", "input_of": "f", "faces": ["face"], "edge_geometry": rows}]})
    }

    fn historical() -> Value {
        json!({"kind": "historical", "value": {"state": "st", "faces": ["face"], "native": "n"}})
    }

    #[test]
    fn an_extrude_starts_from_a_face_of_its_own_input_topology() {
        let z = -30.0;
        let model = input_face([
            [10.0, -14.0, z],
            [14.0, -14.0, z],
            [14.0, 14.0, z],
            [10.0, 14.0, z],
        ]);
        let ctx = Context::new(&model);
        let feature = json!({"id": "f"});
        let point = pick(&ctx, &feature, Some(&historical()), [0.0, 0.0, 1.0]).unwrap();
        assert!((point[2] - z).abs() < 1e-9, "{point:?}");
        assert!(
            point[0] > 10.0 && point[0] < 14.0 && point[1].abs() < 14.0,
            "{point:?}"
        );

        let wall = input_face([
            [-14.0, -14.0, -30.0],
            [-14.0, 14.0, -30.0],
            [-14.0, 14.0, 2.0],
            [-14.0, -14.0, 2.0],
        ]);
        let ctx = Context::new(&wall);
        assert_eq!(
            pick(&ctx, &feature, Some(&historical()), [0.0, 0.0, 1.0]).unwrap_err(),
            "an extrude face not parallel to its sketch is not supported yet"
        );
        let other = json!({"id": "g"});
        assert!(pick(&ctx, &other, Some(&historical()), [0.0, 0.0, 1.0]).is_err());
    }

    #[test]
    fn points_inside_a_concave_outline() {
        // An L shape: its corner-triangle at (2, 2) is a notch; others lie inside.
        let l = [
            [0.0, 0.0],
            [4.0, 0.0],
            [4.0, 2.0],
            [2.0, 2.0],
            [2.0, 4.0],
            [0.0, 4.0],
        ];
        assert!(inside(&l, [1.0, 1.0]));
        assert!(!inside(&l, [3.0, 3.0]));
    }

    /// A cylinder face (radius 5 about z, from z 0 to 10) whose loops start at `corners`; each
    /// loop is the next `per_loop` corners.
    fn cylinder_face(corners: &[[f64; 3]], per_loop: usize) -> (Value, Value) {
        let mut points = Vec::new();
        let mut vertices = Vec::new();
        let mut edges = Vec::new();
        let mut coedges = Vec::new();
        for (k, corner) in corners.iter().enumerate() {
            points.push(json!({"id": format!("p{k}"),
                "position": {"x": corner[0], "y": corner[1], "z": corner[2]}}));
            vertices.push(json!({"id": format!("v{k}"), "point": format!("p{k}")}));
            edges.push(json!({"id": format!("e{k}"), "start": format!("v{k}")}));
            coedges.push(json!({"id": format!("c{k}"), "edge": format!("e{k}")}));
        }
        let loops: Vec<Value> = corners
            .chunks(per_loop)
            .enumerate()
            .map(|(l, chunk)| {
                let ids: Vec<String> = (0..chunk.len())
                    .map(|k| format!("c{}", l * per_loop + k))
                    .collect();
                json!({"id": format!("l{l}"), "boundary": {"coedges": ids}})
            })
            .collect();
        let ids: Vec<String> = (0..loops.len()).map(|l| format!("l{l}")).collect();
        let model = json!({
            "surfaces": [{"id": "s", "geometry": {"kind": "cylinder",
                "origin": {"x": 0.0, "y": 0.0, "z": 0.0}, "axis": {"x": 0.0, "y": 0.0, "z": 1.0},
                "ref_direction": {"x": 1.0, "y": 0.0, "z": 0.0}, "radius": 5.0}}],
            "faces": [{"id": "f", "surface": "s", "loops": {"loops": ids}}],
            "loops": loops, "coedges": coedges, "edges": edges,
            "vertices": vertices, "points": points,
        });
        let face = model["faces"][0].clone();
        (model, face)
    }

    #[test]
    fn a_full_cylinder_gets_a_point_opposite_its_seam() {
        let (model, face) = cylinder_face(&[[5.0, 0.0, 0.0], [5.0, 0.0, 10.0]], 1);
        let ctx = Context::new(&model);
        let p = point_on(&ctx, &face).unwrap();
        assert!((p[0] + 5.0).abs() < 1e-9 && p[1].abs() < 1e-9 && (p[2] - 5.0).abs() < 1e-9);
    }

    #[test]
    fn a_conical_appearance_uses_the_radius_at_the_picked_height() {
        let (mut model, face) = cylinder_face(&[[5.0, 0.0, 0.0], [10.0, 0.0, 10.0]], 1);
        model["surfaces"][0]["geometry"]["kind"] = json!("cone");
        model["surfaces"][0]["geometry"]["half_angle"] = json!(0.5_f64.atan());
        model["surfaces"][0]["geometry"]["ratio"] = json!(1.0);
        let ctx = Context::new(&model);
        let p = point_on(&ctx, &face).unwrap();
        assert!((p[0] + 7.5).abs() < 1e-9 && p[1].abs() < 1e-9 && (p[2] - 5.0).abs() < 1e-9);
    }

    #[test]
    fn a_quarter_cylinder_gets_a_point_in_the_middle_of_its_arc() {
        let corners = [
            [5.0, 0.0, 0.0],
            [0.0, 5.0, 0.0],
            [0.0, 5.0, 10.0],
            [5.0, 0.0, 10.0],
        ];
        let (model, face) = cylinder_face(&corners, 4);
        let ctx = Context::new(&model);
        let p = point_on(&ctx, &face).unwrap();
        let half = 5.0 * std::f64::consts::FRAC_1_SQRT_2;
        assert!((p[0] - half).abs() < 1e-9 && (p[1] - half).abs() < 1e-9);
        assert!((p[2] - 5.0).abs() < 1e-9);
    }
}

#[cfg(test)]
mod import_tests {
    use super::*;

    #[test]
    fn wire_regions_do_not_consume_or_discard_solid_names() {
        let model = json!({
            "bodies": [{"id": "wire", "name": "Body", "kind": "wire"},
                       {"id": "a", "name": "Body", "kind": "solid"},
                       {"id": "b", "name": "Body", "kind": "sheet"}],
            "regions": [{"id": "rw", "body": "wire", "shells": ["sw"]},
                        {"id": "ra", "body": "a", "shells": ["sa"]},
                        {"id": "rw2", "body": "wire", "shells": ["sw2"]},
                        {"id": "rb", "body": "b", "shells": ["sb"]}]
        });
        let mut ctx = Context::new(&model);
        let step = "#1=MANIFOLD_SOLID_BREP('',#2);\n#3=SHELL_BASED_SURFACE_MODEL('',(#4));\n#5=PRODUCT('Block','Block','',());";
        assert_eq!(
            import_names(&mut ctx, &["wire", "a", "b"], Some(step)),
            ["Block", "Block (2)"]
        );
        assert_eq!(
            ctx.named_bodies,
            [
                ("a".into(), "Block".into()),
                ("b".into(), "Block (2)".into())
            ]
        );
    }

    #[test]
    fn import_names_validate_before_committing_and_skip_empty_regions() {
        let model = json!({
            "bodies": [{"id": "a", "name": "Body", "kind": "solid"},
                       {"id": "b", "name": "Body", "kind": "solid"}],
            "regions": [{"id": "empty", "body": "a", "shells": []},
                        {"id": "ra", "body": "a", "shells": ["sa"]},
                        {"id": "rb", "body": "b", "shells": ["sb"]}]
        });
        let mut ctx = Context::new(&model);
        let one = "#1=MANIFOLD_SOLID_BREP('a',#2);";
        assert!(import_names(&mut ctx, &["a", "b"], Some(one)).is_empty());
        assert!(ctx.named_bodies.is_empty());
        assert!(ctx.imported_regions.is_empty());
        let two = format!("{one}\n#3=MANIFOLD_SOLID_BREP('b',#4);");
        assert_eq!(
            import_names(&mut ctx, &["a", "b"], Some(&two)),
            ["Body", "Body (2)"]
        );
        assert_eq!(
            ctx.imported_regions.get("Body").map(String::as_str),
            Some("ra")
        );
        assert_eq!(
            ctx.imported_regions.get("Body (2)").map(String::as_str),
            Some("rb")
        );
    }
}
