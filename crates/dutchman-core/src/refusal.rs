//! Failures SolidCraft itself reports when it replays a feature, told apart from Dutchman's
//! own mapping limits.

/// A reason that came from SolidCraft refusing a replayed feature.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Refusal {
    /// The reason as shown to a user: names SolidCraft, without its internal site number.
    pub text: String,
    /// A stable, name-free category.
    pub category: &'static str,
    /// The upstream issue that tracks the refusal, when one is known.
    pub upstream: Option<&'static str>,
}

/// The category of a SolidCraft refusal no known pattern matches.
pub const GENERIC_CATEGORY: &str = "solidcraft-refused";

/// One known refusal: SolidCraft's message contains `pattern`.
struct Known {
    pattern: &'static str,
    category: &'static str,
    upstream: &'static str,
}

/// Known refusals, first match wins. Add a row to recognise another one.
const KNOWN: &[Known] = &[Known {
    pattern: "the blend is larger than the neighbouring faces",
    category: "fillet-overflows-neighbouring-face",
    upstream: "https://github.com/tangency-labs/solidcraft/issues/4782",
}];

/// What the SolidCraft CLI said about a feature, when `reason` is such a failure
/// (`feature "X" failed: ...`, or `failed there:`, possibly inside a longer reason).
pub fn classify(reason: &str) -> Option<Refusal> {
    let start = reason.find("feature \"")?;
    let rest = reason.get(start + "feature \"".len()..)?;
    let close = rest.find('"')?;
    let name = rest.get(..close)?;
    let after = rest.get(close + 1..)?;
    let message = after
        .strip_prefix(" failed: ")
        .or_else(|| after.strip_prefix(" failed there: "))?;
    let message = strip_site(message.trim());
    let message = message
        .strip_prefix("not supported yet: ")
        .unwrap_or(message);
    let known = KNOWN.iter().find(|k| message.contains(k.pattern));
    Some(Refusal {
        text: format!("SolidCraft refused feature \"{name}\": {message}"),
        category: known.map_or(GENERIC_CATEGORY, |k| k.category),
        upstream: known.map(|k| k.upstream),
    })
}

/// `text` without a trailing ` (site N)`.
fn strip_site(text: &str) -> &str {
    let Some(open) = text.rfind(" (site ") else {
        return text;
    };
    let tail = text.get(open + " (site ".len()..).unwrap_or("");
    match tail.strip_suffix(')') {
        Some(digits) if !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()) => {
            text.get(..open).unwrap_or(text)
        }
        _ => text,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_known_refusal_gets_its_category_link_and_loses_the_site() {
        let r = classify(
            "feature \"Fillet1\" failed: not supported yet: the blend is larger than the \
             neighbouring faces (site 387)",
        )
        .unwrap();
        assert_eq!(
            r.text,
            "SolidCraft refused feature \"Fillet1\": the blend is larger than the neighbouring faces"
        );
        assert_eq!(r.category, "fillet-overflows-neighbouring-face");
        assert_eq!(
            r.upstream,
            Some("https://github.com/tangency-labs/solidcraft/issues/4782")
        );
    }

    #[test]
    fn an_unknown_refusal_is_generic() {
        let r = classify("feature \"Loft2\" failed there: no profile (site 9)").unwrap();
        assert_eq!(r.text, "SolidCraft refused feature \"Loft2\": no profile");
        assert_eq!(r.category, GENERIC_CATEGORY);
        assert_eq!(r.upstream, None);
    }

    #[test]
    fn a_dutchman_reason_is_not_a_refusal() {
        assert_eq!(classify("sweep with a guide rail is not mapped"), None);
        assert_eq!(classify("a sketch was left out: not resolved"), None);
    }
}
