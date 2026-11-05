//! Planar faces moved along their normal, as SolidCraft's offset-face feature.
//!
//! CADIR gives the faces as faces of the feature's own input topology, known by the geometry of
//! the edges around them ([`super::face_profile`]). SolidCraft picks a face by a point on it, on
//! the body as it rebuilds it before the feature, so each face is written as a point inside it.
//! Its offset is a distance along the outward normal, which is the motion CADIR calls an offset.
//! A translation along a direction would need the outward side of the face, which the input
//! topology does not give, and a rotation has no SolidCraft feature; both are refused.

use serde_json::{json, Map, Value};

use super::sketch::number;
use super::{face_profile, name, Context};

pub(super) fn map(feature: &Value, definition: &Value, ctx: &mut Context) -> Result<Value, String> {
    let motion = definition
        .get("motion")
        .ok_or("the motion of its faces is not given")?;
    let distance = match motion.get("kind").and_then(Value::as_str) {
        Some("offset") => {
            number(motion.get("distance")).ok_or("the distance its faces move is not resolved")?
        }
        Some("translate") => {
            return Err(
                "translating faces along a direction is not supported yet; only a move along \
                 the face normal is"
                    .to_owned(),
            )
        }
        Some("rotate") => return Err("rotating faces is not supported yet".to_owned()),
        _ => return Err("the motion of its faces is not resolved".to_owned()),
    };
    let selection = definition.get("faces").ok_or("its faces are not given")?;
    let points = match selection.get("kind").and_then(Value::as_str) {
        Some("historical") => face_profile::planar_points(
            feature,
            selection.get("value").unwrap_or(&Value::Null),
            ctx,
        )?,
        found @ Some("native" | "unresolved") => {
            return Err(super::unsupported_selection("face", found))
        }
        Some("historical_partial") => {
            return Err("some of its faces are not resolved in its input topology".to_owned())
        }
        _ => return Err("moved faces must come from the feature's own input topology".to_owned()),
    };
    let id = ctx.id();
    let mut out = Map::new();
    out.insert("type".to_owned(), json!("offset_face"));
    out.insert("suppressed".to_owned(), json!(false));
    out.insert("faces".to_owned(), json!(points));
    out.insert("distance".to_owned(), json!(format!("{distance} mm")));
    out.insert("id".to_owned(), json!(id));
    out.insert(
        "name".to_owned(),
        json!(name(feature)
            .map(str::to_owned)
            .unwrap_or_else(|| format!("MoveFace{id}"))),
    );
    Ok(Value::Object(out))
}
