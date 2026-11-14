//! The model's geometry restated as STEP text, the form a SolidCraft Import keeps.

use cadmpeg_ir::codec::write::target::TargetRequest;
use cadmpeg_ir::codec::write::EncodeInput;
use cadmpeg_registry::{build_encoder, Format};
use dutchman_core::{Error, Model, Report, Severity, Stage};

/// The STEP schema written: AP214, which SolidCraft writes and reads natively.
const SCHEMA: &str = "ap214";

/// The name cadmpeg's STEP writer gives a product it has no name for.
const PLACEHOLDER: &str = "'cadmpeg_model'";

/// cadmpeg's code for procedural surfaces its STEP writer had no form for.
const PROCEDURAL_NOT_WRITTEN: &str = "step/geometry.procedural-definition-not-written";

/// The largest STEP text SolidCraft's reader accepts.
const MAX_STEP_BYTES: usize = 512 << 20;

/// Restates the model as STEP AP214 text, recording what the restatement drops.
///
/// cadmpeg is third-party code run on models decoded from hostile input, so a panic inside it
/// is caught and returned as [`Error::Encode`] instead of taking the process down.
pub(crate) fn restate(model: &Model, report: &mut Report) -> Result<String, Error> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        restate_unguarded(model, report)
    }))
    .unwrap_or_else(|_| {
        Err(Error::Restate(
            "cadmpeg's STEP writer failed unexpectedly on this model".to_owned(),
        ))
    })
}

/// Whether the model has geometry a STEP restatement can carry, found by planning the
/// restatement without writing it; `Err` with what conversion would say otherwise.
pub(crate) fn restatable(model: &Model) -> Result<(), Error> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| plan(model).map(|_| ())))
        .unwrap_or_else(|_| {
            Err(Error::Restate(
                "cadmpeg's STEP writer failed unexpectedly on this model".to_owned(),
            ))
        })
}

/// cadmpeg's plan for restating the model as STEP, refused when it carries no geometry.
fn plan(model: &Model) -> Result<cadmpeg_ir::codec::write::ExportPlan, Error> {
    // With no bodies at all, cadmpeg plans an empty file rather than saying so. Without a face,
    // cadmpeg writes what is left (wire edges, points), none of it a solid or a surface.
    if model.ir.model.bodies.is_empty() || model.ir.model.faces.is_empty() {
        return Err(no_geometry());
    }
    let input = EncodeInput::new(&model.ir, model.fidelity.as_ref());
    let plan = build_encoder(Format::Step)
        .plan(input, TargetRequest::Explicit(SCHEMA))
        .map_err(|err| Error::Restate(format!("cannot restate the model as STEP: {err}")))?;
    let losses = &plan.report().losses;
    if losses
        .iter()
        .any(|loss| loss.code.to_string() == "step/export.no-exportable-solids")
    {
        return Err(no_geometry());
    }
    // cadmpeg keeps what survives of a body whose faces cannot be bounded (its edges as wires,
    // its vertices as points) and says so once per face: when that is every face, nothing it
    // writes is a solid or a surface.
    let unbounded = losses
        .iter()
        .filter(|loss| loss.code.to_string() == "step/topology.face-no-writable-bounds")
        .count();
    if unbounded >= model.ir.model.faces.len() {
        return Err(no_geometry());
    }
    Ok(plan)
}

fn no_geometry() -> Error {
    Error::NoGeometry("the model has no solids or surfaces SolidCraft can import".to_owned())
}

fn restate_unguarded(model: &Model, report: &mut Report) -> Result<String, Error> {
    let plan = plan(model)?;
    report.extend_from_cadmpeg_export(&plan.report().losses);
    let reduced = plan
        .report()
        .losses
        .iter()
        .filter(|loss| loss.code.to_string() == "step/topology.region-solid-reduced-to-sheet")
        .count();
    if reduced > 0 {
        let what = if reduced == 1 {
            "solid region converted to an open sheet"
        } else {
            "solid regions converted to open sheets"
        };
        report.push_at(
            Stage::Write,
            Severity::Degraded,
            "geometry",
            format!("{reduced} {what}: solid shells are incomplete or open; check the source geometry and convert again"),
        );
    }
    if plan
        .report()
        .losses
        .iter()
        .any(|loss| loss.code.to_string() == PROCEDURAL_NOT_WRITTEN)
    {
        if let Some(detail) = unwritten_surfaces(model) {
            report.push(Severity::Degraded, "geometry", detail);
        }
    }

    let mut bytes = Vec::new();
    plan.write_to(&mut bytes)
        .map_err(|err| Error::Restate(format!("cannot write STEP: {err}")))?;
    if bytes.len() > MAX_STEP_BYTES {
        return Err(Error::Encode(format!(
            "the STEP restatement is {} MB, over SolidCraft's {} MB import limit",
            bytes.len() >> 20,
            MAX_STEP_BYTES >> 20
        )));
    }
    let text = String::from_utf8(bytes)
        .map_err(|_| Error::Restate("cadmpeg wrote STEP that is not UTF-8".to_owned()))?;
    let step = name_unnamed_product(text, &model.name);
    Ok(name_step_shapes(step, model))
}

/// What the procedural surfaces cadmpeg could not restate are, said plainly: a coil's helix
/// surface is left out of the STEP, so the solid it bounds opens with no volume.
fn unwritten_surfaces(model: &Model) -> Option<String> {
    let wire = serde_json::to_value(&model.ir).ok()?;
    let surfaces = wire.get("model")?.get("procedural_surfaces")?.as_array()?;
    let helices = surfaces
        .iter()
        .filter(|surface| {
            surface
                .get("definition")
                .and_then(|definition| definition.get("kind"))
                .and_then(serde_json::Value::as_str)
                == Some("helix")
        })
        .count();
    match helices {
        0 => None,
        1 => Some(
            "a helix surface (the swept face of a coil or thread) has no form cadmpeg writes \
             to STEP and is left out: the solid it bounds opens without it, with no volume"
                .to_owned(),
        ),
        n => Some(format!(
            "{n} helix surfaces (the swept faces of coils or threads) have no form cadmpeg \
             writes to STEP and are left out: the solids they bound open without them, with \
             no volume"
        )),
    }
}

/// Names the product cadmpeg left unnamed after the design, as SolidCraft does for files it
/// restates through cadmpeg. The name is reduced to plain STEP string characters.
fn name_unnamed_product(step: String, design: &str) -> String {
    let name: String = design
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, ' ' | '-' | '_' | '.'))
        .collect();
    if name.trim().is_empty() || !step.contains(PLACEHOLDER) {
        step
    } else {
        step.replace(PLACEHOLDER, &format!("'{name}'"))
    }
}

/// Names the solid and surface shapes cadmpeg left unnamed in the STEP text,
/// using the corresponding body names from the model in shape arena order.
fn name_step_shapes(step: String, model: &Model) -> String {
    let names: Vec<String> = model
        .ir
        .model
        .bodies
        .iter()
        .filter(|b| {
            matches!(
                b.kind,
                cadmpeg_ir::topology::BodyKind::Solid | cadmpeg_ir::topology::BodyKind::Sheet
            )
        })
        .filter_map(|b| b.name.as_ref())
        .map(|name| {
            name.chars()
                .filter(|c| c.is_ascii_alphanumeric() || matches!(c, ' ' | '-' | '_' | '.'))
                .collect::<String>()
        })
        .collect();
    if names.is_empty() {
        return step;
    }
    let mut out = String::with_capacity(step.len() + names.len() * 32);
    let mut name_idx = 0;
    for line in step.lines() {
        let trimmed = line.trim_start();
        if let Some((id, def)) = trimmed.split_once('=') {
            let def_trimmed = def.trim_start();
            let is_shape = [
                "MANIFOLD_SOLID_BREP('',",
                "BREP_WITH_VOIDS('',",
                "SHELL_BASED_SURFACE_MODEL('',",
            ]
            .iter()
            .find(|prefix| def_trimmed.starts_with(*prefix));
            if let Some(prefix) = is_shape {
                if let Some(name) = names.get(name_idx) {
                    if !name.trim().is_empty() {
                        let new_prefix = prefix.replace("''", &format!("'{name}'"));
                        let remainder = &def_trimmed[prefix.len()..];
                        out.push_str(id);
                        out.push('=');
                        out.push_str(&new_prefix);
                        out.push_str(remainder);
                        out.push('\n');
                        name_idx += 1;
                        continue;
                    }
                }
                name_idx += 1;
            }
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

/// The product name of our single-part STEP restatement, used by SolidCraft
/// when an Import has no explicit body names. STEP escapes a quote by doubling it.
pub(crate) fn import_name(step: &str) -> Option<String> {
    let product = step.lines().find_map(|line| {
        let (_, definition) = line.split_once('=')?;
        definition.trim_start().strip_prefix("PRODUCT(")
    })?;
    let mut chars = product.trim_start().strip_prefix('\'')?.chars().peekable();
    let mut name = String::new();
    while let Some(c) = chars.next() {
        if c == '\'' {
            if chars.peek() == Some(&'\'') {
                chars.next();
            } else {
                return Some(name);
            }
        }
        name.push(c);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_import_names_read_the_written_product_and_escaped_quotes() {
        assert_eq!(
            import_name("#1 = PRODUCT('Bracket','id','',());"),
            Some("Bracket".into())
        );
        assert_eq!(
            import_name("#1=PRODUCT('Ray''s bracket','id','',());"),
            Some("Ray's bracket".into())
        );
        assert_eq!(import_name("#1=PRODUCT('unfinished"), None);
        assert_eq!(import_name("#1=MANIFOLD_SOLID_BREP('',#2);"), None);
    }

    #[test]
    fn a_solid_reduced_to_a_sheet_gets_a_counted_degraded_loss() {
        let mut ir = cadmpeg_ir::examples::unit_cube().unwrap();
        let removed = ir.model.edges[0].curve().unwrap().clone();
        ir.model.edges[0].carrier = cadmpeg_ir::topology::EdgeCarrier::Free;
        ir.model.curves.retain(|curve| curve.id != removed);
        let model = Model {
            ir,
            fidelity: None,
            name: "Synthetic open cube".into(),
            source_file: None,
            source_step: None,
            assembly: None,
        };
        let mut report = Report::default();
        let step = restate(&model, &mut report).unwrap();
        assert!(step.contains("SHELL_BASED_SURFACE_MODEL"));
        assert!(report
            .losses()
            .iter()
            .any(|loss| loss.severity == Severity::Degraded
                && loss.stage == Stage::Write
                && loss.subject == "geometry"
                && loss
                    .detail
                    .starts_with("1 solid region converted to an open sheet")));
        let mut sheet = model;
        sheet.ir.model.bodies[0].kind = cadmpeg_ir::topology::BodyKind::Sheet;
        let mut report = Report::default();
        restate(&sheet, &mut report).unwrap();
        assert!(!report
            .losses()
            .iter()
            .any(|loss| loss.detail.contains("solid region converted")));
    }

    #[test]
    fn names_the_placeholder_product_after_the_design() {
        let step =
            "FILE_NAME('cadmpeg_model');\n#1=PRODUCT('cadmpeg_model','cadmpeg_model','',(#2));";
        assert_eq!(
            name_unnamed_product(step.to_owned(), "Bracket v2"),
            "FILE_NAME('Bracket v2');\n#1=PRODUCT('Bracket v2','Bracket v2','',(#2));"
        );
    }

    #[test]
    fn keeps_the_placeholder_when_the_name_has_no_plain_characters() {
        let step = "PRODUCT('cadmpeg_model')";
        assert_eq!(name_unnamed_product(step.to_owned(), "'''"), step);
    }

    #[test]
    fn strips_quotes_from_the_design_name() {
        let step = "PRODUCT('cadmpeg_model')";
        assert_eq!(
            name_unnamed_product(step.to_owned(), "it's"),
            "PRODUCT('its')"
        );
    }

    #[test]
    fn step_shapes_are_named_from_model_bodies() {
        let mut model = crate::testing::cube();
        model.ir.model.bodies[0].name = Some("Bottom".to_owned());
        let step = "#10 = MANIFOLD_SOLID_BREP('',#11);\n#20 = BREP_WITH_VOIDS('',#21,#22);\n";
        let named = name_step_shapes(step.to_owned(), &model);
        assert!(named.contains("MANIFOLD_SOLID_BREP('Bottom',#11);"));
        assert!(named.contains("BREP_WITH_VOIDS('',#21,#22);"));
    }
}
