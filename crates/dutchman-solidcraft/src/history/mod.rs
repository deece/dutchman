//! The model's feature history, as SolidCraft parametric features.
//!
//! Mapping stops body-building history at its first blocker, retaining independent sketches.
//! The writer keeps this editable prefix hidden beside an import of the final B-rep.
//!
//! The mapping reads the model in CADIR's documented JSON form (`docs/cad-ir.md` in cadmpeg),
//! the stable contract cadmpeg publishes, rather than its Rust accessors. It knows the wire
//! versions in [`IR_VERSIONS`] and refuses others rather than misread them.

pub(crate) mod base;
mod blend;
mod cap;
mod chain;
mod constraints;
mod crossing;
mod datum;
mod delete_face;
mod dof;
mod extrude;
mod face_profile;
mod faces;
mod helix;
mod helpers;
mod hole;
mod loft;
mod modify;
mod move_face;
pub(crate) mod params;
mod pattern;
mod primitive;
mod profile;
mod revolve;
mod sheet;
mod shell;
mod sketch;
mod spatial;
mod sweep;
mod thread;
mod wire;

use std::cell::OnceCell;
use std::collections::{BTreeMap, BTreeSet, HashMap};

use dutchman_core::{FeatureOutcome, FeatureStatus, Model, Report, Severity};
use serde_json::Value;

/// The CADIR wire versions the mapping reads. Version 7 adds only assembly joint kinds and a
/// sheet-metal thickness side, neither of which the mapping uses. Version 11 adds
/// fragment/reproduction metadata without changing the complete-model feature mapping.
const IR_VERSIONS: [&str; 6] = ["6", "7", "8", "9", "10", "11"];

/// A replayable history: SolidCraft parameters and features in timeline order.
#[derive(Clone, Debug, Default)]
pub(crate) struct History {
    /// Imported names that previously came from STEP rather than CADIR. They
    /// must not reserve slots in the existing explicit-name allocator.
    pub(crate) default_bodies: BTreeMap<String, String>,
    /// Why the editable timeline stops before the source ends.
    pub(crate) stop: Option<String>,
    /// The source row that emitted each feature, including auxiliary features.
    pub(crate) sources: BTreeMap<u64, usize>,
    /// The centres of the bodies the source stores in the state right after a feature, by the
    /// id of the feature it became. Absent when the source doesn't say, which is not "none".
    pub(crate) stored_bodies: BTreeMap<u64, Vec<crate::verify::StoredBody>>,
    pub(crate) params: Vec<Value>,
    pub(crate) features: Vec<Value>,
    /// The looks of the bodies the history makes, by body name.
    pub(crate) looks: BTreeMap<String, Value>,
    /// Looks of faces of those bodies that differ from their body's, as SolidCraft face looks
    /// (a point on the face, in the frame the body is built in: the design's, or its
    /// component's inside an assembly, which SolidCraft applies to every occurrence).
    pub(crate) face_looks: Vec<Value>,
    /// The sheet rules its sheet-metal features use, as SolidCraft stores them.
    pub(crate) sheet_rules: Vec<Value>,
    /// The id the next feature after the history takes.
    pub(crate) next_id: u64,
    /// New pattern instance sets must fall back if an attempted replay cannot finish.
    pub(crate) pattern_replay_required: bool,
    /// Source features that became SolidCraft features or needed none.
    pub(crate) mapped: usize,
    /// Of those, the ones that needed no feature: a component insert, a joint, a default plane.
    pub(crate) not_needed: usize,
    /// Of those, the ones kept that re-apply to a base body already holding their result.
    pub(crate) reapplied: usize,
    /// Active source features.
    pub(crate) total: usize,
    /// Blends whose input edges wait for SolidCraft's replay of the history before them
    /// ([`prove_blends`]); until then each has no body.
    pub(crate) blends: Vec<blend::Pending>,
    /// Ids of sketch features whose source sketch is hidden in the source.
    pub(crate) hidden: Vec<u64>,
    /// Ids of features whose source profile is the whole of a sketch with more than one
    /// profile. Which of them it takes isn't stated, so each is kept only when SolidCraft's
    /// check confirms the replay ([`History::first_unconfirmed`]).
    pub(crate) unconfirmed: BTreeSet<u64>,
}

/// The bodies the source stores right after `feature`, to compare its replay with. A base
/// feature written with its body as the design ends doesn't hold that state (later features
/// changed the body), so there is nothing at that point to compare it with.
fn compared_state(
    outcome: &FeatureOutcome,
    feature: &Value,
) -> Option<Vec<crate::verify::StoredBody>> {
    if outcome.final_body {
        return None;
    }
    stored_body_centers(feature)
}

/// Whether `feature`'s profile is a whole sketch that has more than one profile. cadmpeg
/// doesn't say which of them such a reference takes: a cut that took them all can remove
/// the body Fusion keeps.
fn whole_of_several(feature: &Value, ctx: &Context) -> bool {
    let Some(profile) = feature.pointer("/definition/profile") else {
        return false;
    };
    profile.get("kind").and_then(Value::as_str) == Some("sketch")
        && profile
            .get("value")
            .and_then(Value::as_str)
            .is_some_and(|sketch| profile::profile_count(ctx, sketch) > 1)
}

/// The bodies the source stores in the state right after `feature`: the centres of their
/// boxes, from `id:x,y,z;id:x,y,z` in its `body_centers`, each exact when its id is in
/// `body_centers_exact`. `None` when it has none, which is unknown, when an entry can't be
/// read, or when `body_centers_incomplete` names any body whose centre rests on incomplete
/// edge evidence, even if that body has a centre entry.
fn stored_body_centers(feature: &Value) -> Option<Vec<crate::verify::StoredBody>> {
    let property = |name: &str| {
        feature
            .pointer(&format!("/source_properties/{name}"))
            .and_then(Value::as_str)
    };
    if property("body_centers_incomplete")
        .is_some_and(|ids| ids.split(';').any(|id| !id.trim().is_empty()))
    {
        return None;
    }
    let exact: std::collections::BTreeSet<&str> = property("body_centers_exact")
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .collect();
    let centers = property("body_centers")?
        .split(';')
        .filter(|entry| !entry.is_empty())
        .map(|entry| {
            let (id, point) = entry.split_once(':')?;
            let point: Vec<f64> = point
                .split(',')
                .map(|value| value.trim().parse::<f64>().ok().filter(|v| v.is_finite()))
                .collect::<Option<_>>()?;
            Some(crate::verify::StoredBody {
                center: <[f64; 3]>::try_from(point).ok()?,
                exact: exact.contains(id.trim()),
            })
        })
        .collect::<Option<Vec<_>>>()?;
    (!centers.is_empty()).then_some(centers)
}

impl History {
    /// Where the source stores the bodies at the point this history ends, when its last
    /// feature that isn't a sketch says. An earlier feature's count can't speak for a later one.
    pub(crate) fn stored_bodies_at_end(&self) -> Option<&[crate::verify::StoredBody]> {
        let last = self
            .features
            .iter()
            .rev()
            .find(|feature| feature.get("type").and_then(Value::as_str) != Some("sketch"))?;
        self.stored_bodies
            .get(&last.get("id").and_then(Value::as_u64)?)
            .map(Vec::as_slice)
    }

    /// The source row of the first kept feature whose whole-sketch profile nothing has
    /// confirmed, and why it can't stay unconfirmed.
    pub(crate) fn first_unconfirmed(&self) -> Option<(usize, String)> {
        self.features.iter().find_map(|feature| {
            let id = feature.get("id").and_then(Value::as_u64)?;
            if !self.unconfirmed.contains(&id) {
                return None;
            }
            let name = feature.get("name").and_then(Value::as_str).unwrap_or("?");
            Some((
                *self.sources.get(&id)?,
                format!(
                    "feature \"{name}\": its profile is the whole of a sketch with several \
                     profiles, and SolidCraft's check could not confirm which it takes"
                ),
            ))
        })
    }

    /// Source features that became SolidCraft features, but those that only re-apply to a base
    /// body already holding their result.
    pub(crate) fn rebuilt(&self) -> usize {
        self.mapped
            .saturating_sub(self.not_needed)
            .saturating_sub(self.reapplied)
    }

    /// Source features that need one: all active ones but those that need no feature.
    pub(crate) fn needing(&self) -> usize {
        self.total.saturating_sub(self.not_needed)
    }
}

/// Looks and explicit names for bodies carried by an Import, independent of feature history.
pub(crate) fn imported(
    model: &Model,
    step: &str,
    report: &mut Report,
    name_all: bool,
) -> Result<History, dutchman_core::Error> {
    if model.source_step.is_some() || (!name_all && model.ir.model.appearance_bindings.is_empty()) {
        return Ok(History::default());
    }
    let wire = serde_json::to_value(&model.ir)
        .map_err(|err| dutchman_core::Error::Encode(format!("cannot map appearances: {err}")))?;
    let mut ctx = Context::new(wire.get("model").unwrap_or(&Value::Null));
    ctx.source = Some(model);
    let ids: Vec<_> = model
        .ir
        .model
        .bodies
        .iter()
        .map(|body| body.id.as_str())
        .collect();
    let names = faces::import_names(&mut ctx, &ids, Some(step));
    let face_looks = faces::looks(&mut ctx, report);
    Ok(History {
        features: if names.is_empty() {
            Vec::new()
        } else {
            vec![serde_json::json!({"body_names": names})]
        },
        looks: ctx.looks,
        face_looks,
        default_bodies: ctx.default_bodies,
        ..History::default()
    })
}

/// What one source feature becomes.
enum Mapped {
    /// SolidCraft needs no feature for it (a principal datum plane).
    Nothing,
    /// This SolidCraft feature.
    Feature(Value),
    /// Written with an earlier feature (an attached plate's bend).
    Consumed,
}

impl History {
    /// Names inferred from a STEP default select looks, but must not override
    /// the import's existing naming behaviour or its automatic body numbering.
    pub(crate) fn preserve_import_names(&mut self) {
        for feature in &mut self.features {
            if feature
                .get("type")
                .and_then(Value::as_str)
                .is_some_and(|kind| kind != "import")
            {
                continue;
            }
            let defaults = feature
                .get("body_names")
                .and_then(Value::as_array)
                .is_some_and(|names| {
                    !names.is_empty()
                        && names.iter().all(|name| {
                            name.as_str()
                                .is_some_and(|name| self.default_bodies.contains_key(name))
                        })
                });
            if defaults {
                if let Some(fields) = feature.as_object_mut() {
                    fields.remove("body_names");
                }
            }
        }
    }

    pub(crate) fn import_body_names(&self) -> Option<&Value> {
        self.features.first()?.get("body_names").filter(|names| {
            names.as_array().is_some_and(|names| {
                names.iter().all(|name| {
                    name.as_str()
                        .is_some_and(|name| !self.default_bodies.contains_key(name))
                })
            })
        })
    }

    /// Moves the history into component `component` of a design whose features so far end
    /// before id `offset + 1`: every id and sketch reference shifts by `offset`, and each
    /// parameter whose name is in `taken` is renamed (prefixed with `prefix`) everywhere it is
    /// used. The names it ends up with are added to `taken`.
    pub(crate) fn relocate(
        &mut self,
        offset: u64,
        component: u64,
        prefix: &str,
        taken: &mut std::collections::BTreeSet<String>,
    ) {
        let mut renamed = BTreeMap::new();
        for param in &self.params {
            let Some(name) = param.get("name").and_then(Value::as_str) else {
                continue;
            };
            let mut new = name.to_owned();
            let mut n = 1;
            while taken.contains(&new) {
                new = if n == 1 {
                    format!("{prefix}_{name}")
                } else {
                    format!("{prefix}_{name}_{n}")
                };
                n += 1;
            }
            taken.insert(new.clone());
            if new != name {
                renamed.insert(name.to_owned(), new);
            }
        }
        for param in &mut self.params {
            if let Some(fields) = param.as_object_mut() {
                if let Some(name) = fields.get("name").and_then(Value::as_str) {
                    let name = renamed
                        .get(name)
                        .cloned()
                        .unwrap_or_else(|| name.to_owned());
                    fields.insert("name".to_owned(), Value::String(name));
                }
                if let Some(expr) = fields.get("expr").and_then(Value::as_str) {
                    let expr = params::rename_in(expr, &renamed);
                    fields.insert("expr".to_owned(), Value::String(expr));
                }
            }
        }
        for feature in &mut self.features {
            let Some(fields) = feature.as_object_mut() else {
                continue;
            };
            for key in ["id", "sketch", "path_sketch"] {
                if let Some(id) = fields.get(key).and_then(Value::as_u64) {
                    fields.insert(key.to_owned(), Value::from(id.saturating_add(offset)));
                }
            }
            // A loft names the sketch of each section, and a sweep that of each rail.
            for key in ["sections", "rails"] {
                let Some(Value::Array(rows)) = fields.get_mut(key) else {
                    continue;
                };
                for row in rows {
                    if let Some(sketch) = row.get("sketch").and_then(Value::as_u64) {
                        if let Some(row) = row.as_object_mut() {
                            row.insert(
                                "sketch".to_owned(),
                                Value::from(sketch.saturating_add(offset)),
                            );
                        }
                    }
                }
            }
            fields.insert("component".to_owned(), Value::from(component));
            // Feature distances and angles that name a parameter follow its renaming.
            for (parent, key) in [
                ("extent", "distance"),
                ("extent", "distance2"),
                ("", "angle"),
            ] {
                let slot = if parent.is_empty() {
                    fields.get_mut(key)
                } else {
                    fields.get_mut(parent).and_then(|v| v.get_mut(key))
                };
                if let Some(slot) = slot {
                    if let Some(expr) = slot.as_str() {
                        *slot = Value::String(params::rename_in(expr, &renamed));
                    }
                }
            }
            if let Some(constraints) = fields
                .get_mut("sketch")
                .and_then(|sketch| sketch.get_mut("constraints"))
                .and_then(Value::as_array_mut)
            {
                for constraint in constraints {
                    let param = constraint.get("param").and_then(Value::as_str);
                    if let Some(new) = param.and_then(|p| renamed.get(p)).cloned() {
                        if let Some(fields) = constraint.as_object_mut() {
                            fields.insert("param".to_owned(), Value::String(new));
                        }
                    }
                }
            }
        }
        for id in &mut self.hidden {
            *id = id.saturating_add(offset);
        }
        self.next_id = self.next_id.saturating_add(offset);
    }

    /// The sketch features that should start hidden: those the source hides, and those no
    /// feature of the history is built on (kept for reference only).
    pub(crate) fn hidden_sketches(&self) -> Vec<Value> {
        let mut used = std::collections::BTreeSet::new();
        for feature in &self.features {
            for key in ["sketch", "path_sketch"] {
                if let Some(id) = feature.get(key).and_then(Value::as_u64) {
                    used.insert(id);
                }
            }
            for key in ["sections", "rails"] {
                for row in feature
                    .get(key)
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                {
                    if let Some(id) = row.get("sketch").and_then(Value::as_u64) {
                        used.insert(id);
                    }
                }
            }
        }
        self.features
            .iter()
            .filter(|feature| feature.get("type").and_then(Value::as_str) == Some("sketch"))
            .filter_map(|feature| feature.get("id").and_then(Value::as_u64))
            .filter(|id| !used.contains(id) || self.hidden.contains(id))
            .map(Value::from)
            .collect()
    }

    /// Keeps CADIR's component disambiguation and STEP's numbered default names
    /// in separate allocation sets, updating features and looks together.
    pub(crate) fn rename_bodies(&mut self, label: &str, taken: &mut BodyNames) {
        let names: std::collections::BTreeSet<String> = self
            .features
            .iter()
            .filter_map(|feature| feature.get("body_names").and_then(Value::as_array))
            .flatten()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect();
        let mut renamed = BTreeMap::new();
        for name in names
            .iter()
            .filter(|name| !self.default_bodies.contains_key(*name))
        {
            let mut new = name.clone();
            let mut n = 2;
            while taken.explicit.contains(&new) {
                new = if n == 2 {
                    format!("{name} ({label})")
                } else {
                    format!("{name} ({label} {n})")
                };
                n += 1;
            }
            taken.explicit.insert(new.clone());
            if new != *name {
                renamed.insert(name.clone(), new);
            }
        }
        // Default names are allocated in import order, including multi-shape
        // products and repeated product names. They never change the explicit
        // allocator above, which did not previously see these imports.
        let mut seen = BTreeSet::new();
        for name in self
            .features
            .iter()
            .filter_map(|feature| feature.get("body_names").and_then(Value::as_array))
            .flatten()
            .filter_map(Value::as_str)
            .filter(|name| seen.insert(*name))
        {
            if let Some(base) = self.default_bodies.get(name) {
                let mut new = base.clone();
                let mut n = 2;
                while taken.all.contains(&new) {
                    new = format!("{base} ({n})");
                    n += 1;
                }
                taken.all.insert(new.clone());
                if new != name {
                    renamed.insert(name.to_owned(), new);
                }
            } else {
                taken
                    .all
                    .insert(renamed.get(name).map_or(name, String::as_str).to_owned());
            }
        }
        if renamed.is_empty() {
            self.preserve_import_names();
            return;
        }
        let rename = |name: &str| {
            renamed
                .get(name)
                .cloned()
                .unwrap_or_else(|| name.to_owned())
        };
        for feature in &mut self.features {
            for key in ["body_names", "bodies", "tools"] {
                if let Some(names) = feature.get_mut(key).and_then(Value::as_array_mut) {
                    for name in names {
                        if let Some(old) = name.as_str() {
                            *name = Value::String(rename(old));
                        }
                    }
                }
            }
            for key in ["target", "body"] {
                if let Some(value) = feature.get_mut(key) {
                    if let Some(old) = value.as_str() {
                        *value = Value::String(rename(old));
                    }
                }
            }
        }
        self.looks = std::mem::take(&mut self.looks)
            .into_iter()
            .map(|(name, look)| (rename(&name), look))
            .collect();
        for face in &mut self.face_looks {
            if let Some(fields) = face.as_object_mut() {
                if let Some(old) = fields.get("body").and_then(Value::as_str) {
                    let new = rename(old);
                    fields.insert("body".to_owned(), Value::String(new));
                }
            }
        }
        self.default_bodies = std::mem::take(&mut self.default_bodies)
            .into_iter()
            .map(|(name, base)| (rename(&name), base))
            .collect();
        self.preserve_import_names();
    }
}

/// Explicit CADIR names keep their existing component disambiguation; newly
/// matched default imports use the names previously allocated by SolidCraft.
#[derive(Default)]
pub(crate) struct BodyNames {
    explicit: BTreeSet<String>,
    all: BTreeSet<String>,
}

impl BodyNames {
    pub(crate) fn add_defaults(&mut self, names: impl IntoIterator<Item = String>) {
        self.all.extend(names);
    }
}

impl FromIterator<String> for BodyNames {
    fn from_iter<T: IntoIterator<Item = String>>(iter: T) -> Self {
        let explicit: BTreeSet<_> = iter.into_iter().collect();
        Self {
            all: explicit.clone(),
            explicit,
        }
    }
}

type RowIndex<'a> = OnceCell<HashMap<&'a str, &'a Value>>;

/// The model and what has been mapped so far.
struct Context<'a> {
    /// Source for guarded STEP restatement of stored feature bodies.
    source: Option<&'a Model>,
    /// The CADIR `model` object.
    model: &'a Value,
    /// The arenas `model` holds when it is a subset of the whole; reading another is a bug,
    /// which a debug build panics on.
    reading_only: Option<&'static [&'static str]>,
    /// Borrowed rows, indexed once on the first lookup in each arena. Geometry walks
    /// otherwise scan whole arenas for every edge, vertex and point they visit.
    rows: BTreeMap<&'a str, RowIndex<'a>>,
    target_version: crate::SolidCraftVersion,
    next_id: u64,
    /// SolidCraft parameter names by CADIR parameter id.
    params: BTreeMap<String, String>,
    /// Mapped sketches by CADIR sketch id.
    sketches: BTreeMap<String, sketch::Mapped>,
    /// Where the dimensions of each mapped sketch put its points, by CADIR sketch id.
    coordinates: BTreeMap<String, Vec<[Option<chain::Coordinate>; 2]>>,
    /// Mapped 3D sketches by CADIR spatial sketch id.
    spatial: BTreeMap<String, spatial::Mapped>,
    /// SolidCraft looks of the bodies the history makes, by body name.
    looks: BTreeMap<String, Value>,
    /// The timeline name each mapped source feature took, by CADIR feature id.
    feature_names: BTreeMap<String, String>,
    /// Source ids of bends written with their attached plate.
    consumed_bends: std::collections::BTreeSet<String>,
    /// All features making a composite pattern's instance set, including its seeds.
    pattern_features: BTreeMap<String, Vec<String>>,
    pattern_replay_required: bool,
    /// The source bodies the history makes under their own names: (CADIR body id, name).
    named_bodies: Vec<(String, String)>,
    default_bodies: BTreeMap<String, String>,
    /// Region of each explicitly named imported shape.
    imported_regions: BTreeMap<String, String>,
    /// Bodies proven to produce no solid/surface shape in the STEP restatement.
    unwritten_bodies: BTreeSet<String>,
    /// Resolved frames of earlier nonbuilding datum planes.
    planes: BTreeMap<String, datum::Frame>,
    /// Whether a feature that builds geometry has been mapped.
    builds: bool,
    /// The sheet thicknesses (mm) its sheet-metal features use, each with its rule's name.
    sheet_rules: Vec<(f64, String)>,
    /// Blends and face profiles mapped so far, waiting for SolidCraft's replay to prove them.
    blends: Vec<blend::Pending>,
    /// Features the feature being mapped needs before it (a sketch of a face's boundary).
    extra: Vec<Value>,
    /// Whether exactly one feature of the timeline builds geometry, so the final model's
    /// faces at its end are its own.
    sole_builder: bool,
    /// Set by the feature being mapped: its profile region was not proven against the source.
    profile_unverified: bool,
    /// Set by the feature being mapped: its profile region disagrees with the source.
    profile_mismatch: bool,
    /// Source ids of later features that re-apply to a base body already holding their result.
    reapplied: BTreeSet<String>,
    /// Set by a base feature being mapped with its body as the design ends.
    final_body: bool,
}

impl<'a> Context<'a> {
    fn new(model: &'a Value) -> Self {
        Context {
            source: None,
            model,
            reading_only: None,
            rows: model
                .as_object()
                .into_iter()
                .flatten()
                .filter(|(_, rows)| rows.is_array())
                .map(|(name, _)| (name.as_str(), OnceCell::new()))
                .collect(),
            target_version: crate::SolidCraftVersion::default(),
            next_id: 1,
            params: BTreeMap::new(),
            sketches: BTreeMap::new(),
            coordinates: BTreeMap::new(),
            spatial: BTreeMap::new(),
            looks: BTreeMap::new(),
            feature_names: BTreeMap::new(),
            consumed_bends: std::collections::BTreeSet::new(),
            pattern_features: BTreeMap::new(),
            pattern_replay_required: false,
            named_bodies: Vec::new(),
            default_bodies: BTreeMap::new(),
            imported_regions: BTreeMap::new(),
            unwritten_bodies: BTreeSet::new(),
            planes: BTreeMap::new(),
            builds: false,
            sheet_rules: Vec::new(),
            blends: Vec::new(),
            extra: Vec::new(),
            sole_builder: false,
            profile_unverified: false,
            profile_mismatch: false,
            reapplied: BTreeSet::new(),
            final_body: false,
        }
    }

    /// The timeline name `feature` keeps: its own, numbered when an earlier feature has it,
    /// since SolidCraft refers to features by name.
    fn unique_name<'b>(
        &self,
        feature: &Value,
        preceding: impl Iterator<Item = &'b Value> + Clone,
    ) -> String {
        let base = feature
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("Feature")
            .to_owned();
        let taken = |name: &str| {
            self.feature_names.values().any(|known| known == name)
                || preceding
                    .clone()
                    .any(|feature| feature.get("name").and_then(Value::as_str) == Some(name))
        };
        let mut name = base.clone();
        let mut n = 2;
        while taken(&name) {
            name = format!("{base} ({n})");
            n += 1;
        }
        name
    }

    /// A row of a CADIR arena by id.
    fn row(&self, arena: &str, id: &str) -> Option<&'a Value> {
        let rows = self.rows.get(arena)?.get_or_init(|| {
            let mut rows = HashMap::new();
            for row in self.arena(arena) {
                if let Some(id) = row.get("id").and_then(Value::as_str) {
                    // Match the original scan's first row when ids repeat.
                    rows.entry(id).or_insert(row);
                }
            }
            rows
        });
        rows.get(id).copied()
    }

    /// The SolidCraft expression of the one parameter `feature` owns for the source kind
    /// `kind` (`AlongDistance`, `AlongAngle`) whose magnitude is `magnitude` (mm or radians):
    /// the parameter's name, inside `abs()` when the source signs it, since the feature
    /// carries its direction separately. `None` when no parameter, or several, match.
    fn owned_parameter(&self, feature: &Value, kind: &str, magnitude: f64) -> Option<String> {
        let owner = feature.get("id").and_then(Value::as_str)?;
        let value = |row: &Value| row.pointer("/value/value").and_then(Value::as_f64);
        let mut found = self.arena("parameters").iter().filter(|row| {
            row.get("owner").and_then(Value::as_str) == Some(owner)
                && row
                    .pointer("/properties/source_kind")
                    .and_then(Value::as_str)
                    == Some(kind)
                && value(row)
                    .is_some_and(|v| (v.abs() - magnitude).abs() <= 1e-6 * magnitude.max(1.0))
        });
        let row = found.next()?;
        if found.next().is_some() {
            return None;
        }
        let name = self.params.get(row.get("id").and_then(Value::as_str)?)?;
        Some(if value(row)? < 0.0 {
            format!("abs({name})")
        } else {
            name.clone()
        })
    }

    fn id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);
        id
    }

    /// The name of the sheet rule of `thickness` mm, made the first time it is asked for.
    fn sheet_rule(&mut self, thickness: f64) -> String {
        if let Some((_, name)) = self
            .sheet_rules
            .iter()
            .find(|(known, _)| (known - thickness).abs() < 1e-9)
        {
            return name.clone();
        }
        let name = format!("Sheet {thickness} mm");
        self.sheet_rules.push((thickness, name.clone()));
        name
    }

    /// The rows of a CADIR arena.
    fn arena(&self, name: &str) -> &'a [Value] {
        debug_assert!(
            self.reading_only
                .is_none_or(|arenas| arenas.contains(&name)),
            "arena {name:?} is read from a model that holds only {:?}",
            self.reading_only,
        );
        self.model
            .get(name)
            .and_then(Value::as_array)
            .map_or(&[], Vec::as_slice)
    }
}

/// The history as far as it maps, and what became of each feature.
pub(crate) struct Mapping {
    /// The replayable history, or why there is none.
    pub(crate) result: Result<History, String>,
    /// Each source feature's status, in timeline order. Every feature is looked at, also after
    /// one that does not map, so a caller can show everything that stands in the way.
    pub(crate) features: Vec<FeatureOutcome>,
    /// Mapped, resolved sketches retained even when the history cannot replay.
    pub(crate) references: History,
    keep_sketches: bool,
}

/// Helper decisions remain useful even when other features flatten the history.
pub(crate) fn helper_notes(report: &Report) -> Report {
    let mut kept = Report::default();
    for loss in report.losses().iter().filter(|loss| {
        loss.severity == Severity::Info && loss.detail.starts_with("helper not converted:")
    }) {
        kept.push_at(
            loss.stage,
            loss.severity,
            loss.subject.clone(),
            loss.detail.clone(),
        );
    }
    kept
}

/// Maps the model's history, or says why it cannot be replayed. Losses of a history that is
/// written go to `report`.
pub(crate) fn map(
    model: &Model,
    report: &mut Report,
    target_version: crate::SolidCraftVersion,
) -> Mapping {
    map_with_sketches(model, report, target_version, true)
}

pub(crate) fn map_with_sketches(
    model: &Model,
    report: &mut Report,
    target_version: crate::SolidCraftVersion,
    keep_sketches: bool,
) -> Mapping {
    let mut features_out = Vec::new();
    let mut references = History::default();
    let result = map_into(
        model,
        report,
        &mut features_out,
        &mut references,
        target_version,
        keep_sketches,
    );
    let mut mapping = Mapping {
        result,
        features: features_out,
        references,
        keep_sketches,
    };
    discard_reference_sketches(&mut mapping);
    mapping
}

/// Keep only sketches that the retained body-building history references, transitively.
fn discard_reference_sketches(mapping: &mut Mapping) {
    if mapping.keep_sketches {
        return;
    }
    mapping.references.features.clear();
    let Ok(history) = &mut mapping.result else {
        return;
    };
    let sketches: BTreeMap<u64, &Value> = history
        .features
        .iter()
        .filter(|f| f.get("type").and_then(Value::as_str) == Some("sketch"))
        .filter_map(|f| Some((f.get("id")?.as_u64()?, f)))
        .collect();
    let mut pending: Vec<&Value> = history
        .features
        .iter()
        .filter(|f| f.get("type").and_then(Value::as_str) != Some("sketch"))
        .collect();
    let mut needed = std::collections::BTreeSet::new();
    while let Some(value) = pending.pop() {
        match value {
            Value::Object(fields) => {
                for (key, value) in fields {
                    if matches!(key.as_str(), "sketch" | "path_sketch") {
                        if let Some(id) = value.as_u64() {
                            if needed.insert(id) {
                                if let Some(sketch) = sketches.get(&id) {
                                    pending.push(sketch);
                                }
                            }
                        }
                    }
                    if value.is_object() || value.is_array() {
                        pending.push(value);
                    }
                }
            }
            Value::Array(values) => pending.extend(values),
            _ => {}
        }
    }
    let mut removed = Vec::new();
    let mut retained = Vec::new();
    for (index, feature) in std::mem::take(&mut history.features)
        .into_iter()
        .enumerate()
    {
        let id = feature.get("id").and_then(Value::as_u64);
        if feature.get("type").and_then(Value::as_str) == Some("sketch")
            && id.is_none_or(|id| !needed.contains(&id))
        {
            removed.push(index);
            if let Some(outcome) = id
                .and_then(|id| history.sources.get(&id))
                .and_then(|&row| mapping.features.get_mut(row))
            {
                outcome.status = FeatureStatus::LeftOut;
                outcome.reason = Some("reference sketch left out by choice".to_owned());
            }
        } else {
            retained.push(feature);
        }
    }
    history.features = retained;
    for blend in &mut history.blends {
        blend.feature = blend
            .feature
            .saturating_sub(removed.partition_point(|&index| index < blend.feature));
    }
    history.mapped = mapping
        .features
        .iter()
        .filter(|f| matches!(f.status, FeatureStatus::Mapped | FeatureStatus::NotNeeded))
        .count();
    history.not_needed = mapping
        .features
        .iter()
        .filter(|f| f.status == FeatureStatus::NotNeeded)
        .count();
    history.reapplied = mapping
        .features
        .iter()
        .filter(|f| f.status == FeatureStatus::Mapped && f.reapplied)
        .count();
    history.total = mapping
        .features
        .iter()
        .filter(|f| f.status != FeatureStatus::LeftOut)
        .count();
}

fn map_into(
    model: &Model,
    report: &mut Report,
    out: &mut Vec<FeatureOutcome>,
    references: &mut History,
    target_version: crate::SolidCraftVersion,
    keep_sketches: bool,
) -> Result<History, String> {
    let document =
        serde_json::to_value(&model.ir).map_err(|err| format!("cannot read the model: {err}"))?;
    check_version(&document)?;
    let wire = document.get("model").cloned().unwrap_or(Value::Null);
    let (params, param_names) = params::map(model, report);
    references.params = params.clone();
    let mut ctx = Context::new(&wire);
    ctx.source = Some(model);
    ctx.target_version = target_version;
    ctx.params = param_names;
    let mut features: Vec<&Value> = ctx.arena("features").iter().collect();
    let no_timeline = features.is_empty();
    features.sort_by_key(|feature| feature.get("ordinal").and_then(Value::as_u64));
    // Suppressed features count too: a caller can suppress a suffix of the timeline (the
    // FreeCAD partial prefix does) while the final model still holds what that suffix built.
    ctx.sole_builder = features
        .iter()
        .filter(|feature| {
            let kind = kind(feature);
            !(kind == "sketch"
                || kind.starts_with("datum")
                || matches!(kind.as_str(), "tree_node" | "reference_image")
                || is_assembly_feature(feature)
                || helpers::is_helper(feature))
        })
        .count()
        == 1;

    let mut history = History::default();
    let mut blocked: Option<String> = None;
    for feature in features {
        let outcome = |status, reason: Option<String>| FeatureOutcome {
            name: name(feature).map(str::to_owned),
            kind: kind(feature),
            status,
            reason,
            ordinal: feature.get("ordinal").and_then(Value::as_u64),
            variant: variant(feature),
            profile_unverified: false,
            reapplied: false,
            final_body: false,
        };
        if feature.get("suppressed").and_then(Value::as_bool) == Some(true) {
            report.push(
                Severity::Info,
                describe(feature),
                "suppressed in the source; left out",
            );
            out.push(outcome(
                FeatureStatus::LeftOut,
                Some("suppressed in the source".to_owned()),
            ));
            continue;
        }
        if !keep_sketches && blocked.is_some() && kind(feature) == "sketch" {
            out.push(outcome(
                FeatureStatus::LeftOut,
                Some("reference sketch left out by choice".to_owned()),
            ));
            continue;
        }
        history.total += 1;
        let definition = operation(feature);
        let structural = matches!(
            definition.get("definition").and_then(Value::as_str),
            Some("datum_principal_plane" | "tree_node" | "reference_image")
        ) || is_assembly_feature(feature)
            || helpers::is_helper(feature);
        if blocked.is_some() && (kind(feature) != "sketch" || !keep_sketches) && !structural {
            out.push(outcome(
                FeatureStatus::Blocked,
                Some(
                    "after the first blocking feature; kept in the imported final shape".to_owned(),
                ),
            ));
            continue;
        }
        let builds = ctx.builds;
        let before = history.features.len();
        let blends = ctx.blends.len();
        match map_feature(feature, &mut ctx, report, &mut history.features) {
            Ok(Mapped::Nothing) => {
                history.mapped += 1;
                history.not_needed += 1;
                out.push(outcome(FeatureStatus::NotNeeded, None));
            }
            Ok(Mapped::Consumed) => {
                history.mapped += 1;
                history.not_needed += 1;
                out.push(outcome(
                    FeatureStatus::NotNeeded,
                    Some("mapped with its plate".to_owned()),
                ));
            }
            Ok(Mapped::Feature(mut mapped)) => {
                history.mapped += 1;
                let mut extras = Vec::new();
                for mut extra in std::mem::take(&mut ctx.extra) {
                    let name = ctx.unique_name(&extra, history.features.iter());
                    if let Some(fields) = extra.as_object_mut() {
                        fields.insert("name".to_owned(), Value::String(name));
                    }
                    history.features.push(extra);
                    extras.push(history.features.len().saturating_sub(1));
                }
                if mapped.get("type").and_then(Value::as_str) == Some("sketch") {
                    references.features.push(mapped.clone());
                    let source = operation(feature)
                        .pointer("/sketch/sketch")
                        .and_then(Value::as_str)
                        .and_then(|id| ctx.row("sketches", id));
                    if let (Some(source), Some(id)) =
                        (source, mapped.get("id").and_then(Value::as_u64))
                    {
                        if source.get("visible").and_then(Value::as_bool) == Some(false) {
                            history.hidden.push(id);
                        }
                    }
                }
                let name = ctx.unique_name(&mapped, history.features.iter());
                if let Some(fields) = mapped.as_object_mut() {
                    fields.insert("name".to_owned(), Value::String(name.clone()));
                }
                if let Some(id) = feature.get("id").and_then(Value::as_str) {
                    ctx.feature_names.insert(id.to_owned(), name);
                }
                history.features.push(mapped);
                let mut kept = outcome(FeatureStatus::Mapped, None);
                kept.profile_unverified = std::mem::take(&mut ctx.profile_unverified);
                kept.final_body = std::mem::take(&mut ctx.final_body);
                kept.reapplied = feature
                    .get("id")
                    .and_then(Value::as_str)
                    .is_some_and(|id| ctx.reapplied.contains(id));
                out.push(kept);
                let main = history.features.len().saturating_sub(1);
                for pending in ctx.blends.iter_mut().skip(blends) {
                    pending.feature = pending
                        .extra
                        .and_then(|index| extras.get(index).copied())
                        .unwrap_or(main);
                    pending.outcome = out.len().saturating_sub(1);
                    pending.subject = describe(feature);
                }
            }
            Err(reason) => {
                ctx.builds = builds;
                ctx.profile_unverified = false;
                ctx.final_body = false;
                if std::mem::take(&mut ctx.profile_mismatch) {
                    report.push_coded(
                        Severity::Degraded,
                        describe(feature),
                        "history/profile-region-mismatch",
                        &reason,
                    );
                }
                ctx.extra.clear();
                history.features.truncate(before);
                ctx.blends.truncate(blends);
                blocked.get_or_insert_with(|| format!("{}: {reason}", describe(feature)));
                out.push(outcome(FeatureStatus::Blocked, Some(reason)));
            }
        }
        for emitted in history.features.iter().skip(before) {
            if let Some(id) = emitted.get("id").and_then(Value::as_u64) {
                history.sources.insert(id, out.len().saturating_sub(1));
                if let Some(centers) = out
                    .last()
                    .and_then(|outcome| compared_state(outcome, feature))
                {
                    history.stored_bodies.insert(id, centers);
                }
                if whole_of_several(feature, &ctx) {
                    history.unconfirmed.insert(id);
                }
            }
        }
    }
    history.reapplied = out
        .iter()
        .filter(|f| f.status == FeatureStatus::Mapped && f.reapplied)
        .count();
    // Some decoders supply sketches without timeline rows. Their resolved frames are
    // independent of replayed topology; unresolved attachments are refused by sketch::map.
    let bound: std::collections::BTreeSet<&str> = ctx
        .arena("features")
        .iter()
        .filter_map(|feature| {
            operation(feature)
                .pointer("/sketch/sketch")
                .and_then(Value::as_str)
        })
        .collect();
    for sketch in ctx.arena("sketches") {
        let Some(id) = sketch.get("id").and_then(Value::as_str) else {
            continue;
        };
        if !keep_sketches || bound.contains(id) {
            continue;
        }
        let feature = serde_json::json!({"name": sketch.get("name")});
        let definition = serde_json::json!({"sketch": {"space": "planar", "sketch": id}});
        match sketch::map(&feature, &definition, &mut ctx, report) {
            Ok(mut mapped) => {
                let name = ctx.unique_name(&mapped, history.features.iter());
                if let Some(fields) = mapped.as_object_mut() {
                    fields.insert("name".to_owned(), Value::String(name));
                }
                references.features.push(mapped.clone());
                if sketch.get("visible").and_then(Value::as_bool) == Some(false) {
                    if let Some(id) = mapped.get("id").and_then(Value::as_u64) {
                        history.hidden.push(id);
                    }
                }
                history.features.push(mapped);
            }
            Err(why) => report.push(
                Severity::Degraded,
                describe(&feature),
                format!("sketch left out: {why}"),
            ),
        }
    }
    params::unsigned_dimensions(references, &ctx);
    references.next_id = ctx.next_id;
    if !ctx.builds {
        return Err(blocked.unwrap_or_else(|| {
            if no_timeline {
                "the design carries no feature history".to_owned()
            } else {
                "no feature in the history builds geometry".to_owned()
            }
        }));
    }
    history.stop = blocked;
    history.params = params;
    params::unsigned_dimensions(&mut history, &ctx);
    let mut appearance_report = Report::default();
    history.face_looks = faces::looks(&mut ctx, &mut appearance_report);
    // A partial prefix becomes a hidden editable copy. Its unnamed final
    // bodies are mapped by the visible original's Import instead.
    if history.stop.is_none() {
        report.append(appearance_report);
    }
    history.looks = std::mem::take(&mut ctx.looks);
    history.default_bodies = std::mem::take(&mut ctx.default_bodies);
    history.sheet_rules = ctx
        .sheet_rules
        .iter()
        .map(|(thickness, name)| sheet::rule(name, *thickness))
        .collect();
    history.next_id = ctx.next_id;
    history.pattern_replay_required = ctx.pattern_replay_required;
    history.blends = std::mem::take(&mut ctx.blends);
    Ok(history)
}

/// The design's own sketches, each mapped as a SolidCraft sketch, for a design whose sketches
/// no feature history carries (an assembly's): every planar sketch that maps, with the
/// parameters their dimensions use, and how many sketches did not map.
pub(crate) fn sketches(model: &Model, report: &mut Report) -> (History, usize) {
    sketches_where(model, &OnceCell::new(), report, &|_| true)
}

/// The arenas the reference sketches read, in the wire form of the model. Built the first
/// time it is asked for, so a conversion that maps several groups of sketches builds it once.
pub(crate) type SketchWire = OnceCell<Option<Value>>;

/// [`sketches`] for the sketches whose ids `wanted` accepts; the count left out is of those.
/// `wire` is shared between calls on the same model.
pub(crate) fn sketches_where(
    model: &Model,
    wire: &SketchWire,
    report: &mut Report,
    wanted: &dyn Fn(&str) -> bool,
) -> (History, usize) {
    let mut kept = History::default();
    let count = model
        .ir
        .model
        .sketches
        .iter()
        .filter(|sketch| wanted(sketch.id.as_str()))
        .count();
    if count == 0 {
        return (kept, 0);
    }
    if model.ir.check_decode_depth().is_err() || !IR_VERSIONS.contains(&model.ir.ir_version()) {
        return (kept, count);
    }
    // Reference sketches read only the neutral arenas in `wire::SKETCH_ARENAS`. Keep those
    // expanded once, without expanding native records or the rest of the model.
    let Some(wire) = wire
        .get_or_init(|| wire::only_arenas(&model.ir.model, wire::SKETCH_ARENAS).ok())
        .as_ref()
    else {
        return (kept, count);
    };
    let mut param_notes = Report::default();
    let (params, param_names) = params::map(model, &mut param_notes);
    let mut ctx = Context::new(wire);
    ctx.reading_only = Some(wire::SKETCH_ARENAS);
    ctx.params = param_names;
    let mut left_out = 0;
    for sketch in ctx.arena("sketches") {
        if !sketch.get("id").and_then(Value::as_str).is_some_and(wanted) {
            continue;
        }
        let feature = serde_json::json!({ "name": sketch.get("name") });
        let definition = serde_json::json!({
            "sketch": { "space": "planar", "sketch": sketch.get("id") },
        });
        // A sketch that does not map is counted, and leaves no notes of its own behind.
        let mut notes = Report::default();
        match sketch::map(&feature, &definition, &mut ctx, &mut notes) {
            Ok(mut mapped) => {
                report.append(notes);
                let name = ctx.unique_name(&mapped, kept.features.iter());
                if let Some(fields) = mapped.as_object_mut() {
                    fields.insert("name".to_owned(), Value::String(name));
                }
                kept.features.push(mapped);
            }
            Err(_) => left_out += 1,
        }
    }
    kept.params = params;
    params::unsigned_dimensions(&mut kept, &ctx);
    let used = kept
        .features
        .iter()
        .filter_map(|feature| feature.pointer("/sketch/constraints"))
        .filter_map(Value::as_array)
        .flatten()
        .filter_map(|constraint| constraint.get("param").and_then(Value::as_str))
        .map(str::to_owned)
        .collect();
    kept.params = params::needed(kept.params, used);
    // Only the notes about parameters the sketches keep.
    let names: Vec<&str> = kept
        .params
        .iter()
        .filter_map(|param| param.get("name").and_then(Value::as_str))
        .collect();
    for loss in param_notes.losses() {
        let about = |name: &&str| {
            loss.subject == format!("parameter {name}")
                || loss.detail.starts_with(&format!("renamed to {name}:"))
        };
        if names.iter().any(about) {
            report.push_at(loss.stage, loss.severity, &loss.subject, &loss.detail);
        }
    }
    kept.next_id = ctx.next_id;
    (kept, left_out)
}

/// What SolidCraft rebuilt before each blend, as [`crate::verify::replay_edges`] reports it.
pub(crate) type Replayed = Result<Vec<Result<crate::verify::Replay, String>>, String>;

/// What proving a pending feature settled.
enum Proven {
    /// The body a blend's edges are on.
    Body(String),
    /// A face profile's sketch rewritten on the other side of its plane, if it had to be.
    Sketch(Option<Value>),
}

/// Proves the input edges of every blend of a history that maps, on the bodies `replay`
/// gives for the history before each blend (`replay(history, after)` with the ids of the
/// features each blend follows), and names its body. A blend that cannot be proven blocks
/// the history with the reason; without a replay (`replay` returning `Err`) none can be.
pub(crate) fn prove_blends(
    mapping: &mut Mapping,
    replay: impl FnOnce(&History, &[u64], bool) -> Replayed,
) {
    let Ok(history) = &mut mapping.result else {
        return;
    };
    if history.blends.is_empty() {
        return;
    }
    let after: Vec<Option<u64>> = history
        .blends
        .iter()
        .map(|pending| {
            pending
                .feature
                .checked_sub(1)
                .and_then(|before| history.features.get(before))
                .and_then(|feature| feature.get("id"))
                .and_then(Value::as_u64)
        })
        .collect();
    let steps: Vec<u64> = after.iter().flatten().copied().collect();
    let replayed = if steps.is_empty() {
        Ok(Vec::new())
    } else {
        let faces = history.blends.iter().any(|pending| pending.face.is_some());
        replay(history, &steps, faces)
    };
    let blends = std::mem::take(&mut history.blends);
    let mut step = 0usize;
    let mut first = None;
    for (pending, after) in blends.iter().zip(&after) {
        let proven = match (after, &replayed) {
            (None, _) => Err("no feature before it builds a body to blend".to_owned()),
            (Some(_), Err(why)) => Err(format!(
                "its input edges are found only on SolidCraft's replay of the history before \
                 it, and there is none: {why}"
            )),
            (Some(_), Ok(bodies)) => {
                let at = bodies.get(step);
                step = step.saturating_add(1);
                match at {
                    Some(Ok(replay)) => match &pending.face {
                        None => blend::prove(pending, &replay.bodies).map(Proven::Body),
                        Some(face) => face_profile::prove(face, replay).and_then(|mirrored| {
                            if mirrored {
                                face.sketch(true)
                                    .map(|(sketch, _)| Proven::Sketch(Some(sketch)))
                            } else {
                                Ok(Proven::Sketch(None))
                            }
                        }),
                    },
                    Some(Err(why)) => Err(format!(
                        "SolidCraft cannot replay the history before it: {why}"
                    )),
                    None => {
                        Err("SolidCraft's replay of the history before it is missing".to_owned())
                    }
                }
            }
        };
        match proven {
            Ok(Proven::Body(body)) => {
                if let Some(fields) = history
                    .features
                    .get_mut(pending.feature)
                    .and_then(Value::as_object_mut)
                {
                    fields.insert("body".to_owned(), Value::String(body));
                }
            }
            // The face's sketch on the other side, which runs along the face's outward normal.
            Ok(Proven::Sketch(mirrored)) => {
                let id = pending.face.as_ref().map(|face| face.sketch);
                let target = history
                    .features
                    .iter_mut()
                    .find(|feature| feature.get("id").and_then(Value::as_u64) == id);
                if let (Some(mirrored), Some(target)) = (mirrored, target) {
                    for key in ["plane", "sketch"] {
                        if let (Some(value), Some(fields)) =
                            (mirrored.get(key), target.as_object_mut())
                        {
                            fields.insert(key.to_owned(), value.clone());
                        }
                    }
                }
            }
            Err(reason) => {
                if let Some(outcome) = mapping.features.get_mut(pending.outcome) {
                    outcome.status = FeatureStatus::Blocked;
                    outcome.reason = Some(reason.clone());
                }
                first.get_or_insert_with(|| {
                    (pending.outcome, format!("{}: {reason}", pending.subject))
                });
            }
        }
    }
    if let Some((source, reason)) = first {
        shorten(mapping, source, reason);
    }
}

/// Keeps the prefix before a failed source feature, plus independently placed sketches.
pub(crate) fn shorten(mapping: &mut Mapping, source: usize, reason: String) {
    let first_blocker = mapping
        .features
        .iter()
        .position(|f| f.status == FeatureStatus::Blocked);
    let Ok(history) = &mut mapping.result else {
        return;
    };
    history.features.retain(|feature| {
        feature.get("type").and_then(Value::as_str) == Some("sketch")
            || feature
                .get("id")
                .and_then(Value::as_u64)
                .and_then(|id| history.sources.get(&id))
                .is_some_and(|row| *row < source)
    });
    for feature in mapping.features.iter_mut().skip(source) {
        if feature.status == FeatureStatus::Mapped && feature.kind != "sketch" {
            feature.status = FeatureStatus::Blocked;
            feature.reason = Some(reason.clone());
        }
    }
    history.mapped = mapping
        .features
        .iter()
        .filter(|feature| {
            matches!(
                feature.status,
                FeatureStatus::Mapped | FeatureStatus::NotNeeded
            )
        })
        .count();
    history.not_needed = mapping
        .features
        .iter()
        .filter(|feature| feature.status == FeatureStatus::NotNeeded)
        .count();
    history.reapplied = mapping
        .features
        .iter()
        .filter(|feature| feature.status == FeatureStatus::Mapped && feature.reapplied)
        .count();
    if first_blocker.is_none_or(|first| source <= first) {
        history.stop = Some(reason.clone());
    }
    history.blends.clear();
    if history
        .features
        .iter()
        .all(|f| f.get("type").and_then(Value::as_str) == Some("sketch"))
    {
        mapping.result = Err(reason);
    }
    discard_reference_sketches(mapping);
}

/// Refuses a CADIR wire version the mapping does not know.
fn check_version(document: &Value) -> Result<(), String> {
    let version = document
        .get("ir_version")
        .map(|v| v.as_str().map_or_else(|| v.to_string(), str::to_owned))
        .unwrap_or_default();
    if IR_VERSIONS.contains(&version.as_str()) {
        Ok(())
    } else {
        Err(format!(
            "unsupported CADIR version {version:?} (Dutchman reads {})",
            IR_VERSIONS.join(", ")
        ))
    }
}

/// One source feature as a SolidCraft feature, or why it cannot be.
fn map_feature(
    feature: &Value,
    ctx: &mut Context,
    report: &mut Report,
    preceding: &mut Vec<Value>,
) -> Result<Mapped, String> {
    let definition = operation(feature);
    if matches!(
        definition.get("definition").and_then(Value::as_str),
        Some("datum_principal_plane" | "datum_plane" | "datum_offset_plane")
    ) {
        return datum::map(feature, ctx).map(|()| Mapped::Nothing);
    }
    if is_assembly_feature(feature) {
        return Ok(Mapped::Nothing);
    }
    if helpers::is_helper(feature) {
        return helpers::map(feature, ctx, report);
    }
    match definition.get("definition").and_then(Value::as_str) {
        // A folder, a PartDesign Body or another node that orders the tree and builds nothing.
        Some("tree_node") => Ok(Mapped::Nothing),
        // A raster image placed for reference (a Fusion Canvas): it builds no geometry.
        Some("reference_image") => Ok(Mapped::Nothing),
        Some("sketch") => sketch::map(feature, definition, ctx, report).map(Mapped::Feature),
        Some("spatial_sketch") => {
            spatial::map(feature, definition, ctx, report).map(Mapped::Feature)
        }
        Some("extrude") => extrude::map(feature, definition, ctx).map(Mapped::Feature),
        Some("revolve") => revolve::map(feature, definition, ctx).map(Mapped::Feature),
        Some("loft") => loft::map(feature, definition, ctx, preceding).map(Mapped::Feature),
        Some("sweep") => sweep::map(feature, definition, ctx).map(Mapped::Feature),
        Some("sphere") => primitive::sphere(feature, definition, ctx).map(Mapped::Feature),
        Some("torus") => primitive::torus(feature, definition, ctx).map(Mapped::Feature),
        Some("coil") => primitive::coil(feature, definition, ctx).map(Mapped::Feature),
        Some("helical_sweep") => helix::map(feature, definition, ctx, report).map(Mapped::Feature),
        Some("primitive") => primitive::placed(feature, definition, ctx).map(Mapped::Feature),
        Some(kind @ "block") => Err(primitive::unplaced(kind)),
        Some("shell") => shell::map(feature, definition, ctx).map(Mapped::Feature),
        // Draft still needs the geometry of its own input faces.
        Some("draft") => Err("draft waits on input face geometry".to_owned()),
        Some("combine") => modify::combine(feature, definition, ctx).map(Mapped::Feature),
        Some("split_body") => modify::split(feature, definition, ctx).map(Mapped::Feature),
        Some("mirror_shape") => Err(
            "replacing a source shape with its mirror is not supported yet; SolidCraft mirrors keep the original body".to_owned(),
        ),
        Some("unresolved")
            if matches!(
                definition.get("family").and_then(Value::as_str),
                Some("thread" | "detailed_thread")
            ) =>
        {
            Err("threads whose construction is unresolved are not supported yet".to_owned())
        }
        Some("cosmetic_thread") => thread::map(feature, definition, ctx, report).map(Mapped::Feature),
        Some("pattern")
            if definition.pointer("/pattern/kind").and_then(Value::as_str) == Some("mirror")
                && definition
                    .get("seeds")
                    .and_then(Value::as_array)
                    .is_some_and(|seeds| {
                        seeds.iter().any(|seed| {
                            seed.get("kind").and_then(Value::as_str) == Some("bodies")
                        })
                    }) =>
        {
            modify::mirror(feature, definition, ctx).map(Mapped::Feature)
        }
        Some("pattern") => pattern::map(feature, definition, ctx, preceding).map(Mapped::Feature),
        Some("fillet" | "chamfer") => {
            blend::map(feature, definition, ctx, report).map(Mapped::Feature)
        }
        Some("move_face") => move_face::map(feature, definition, ctx).map(Mapped::Feature),
        Some("delete_face") => delete_face::map(feature, definition, ctx).map(Mapped::Feature),
        Some("hole") => hole::map(feature, definition, ctx, report).map(Mapped::Feature),
        Some("sheet_metal_base_flange") => {
            sheet::base_flange(feature, definition, ctx).map(Mapped::Feature)
        }
        Some("sheet_metal_edge_flange") => {
            sheet::edge_flange(feature, definition, ctx, preceding).map(Mapped::Feature)
        }
        Some("sheet_metal_hem") => Err(sheet::hem_refusal(definition)),
        Some("sheet_metal_attached_plate") => {
            sheet::attached_plate(feature, definition, ctx, preceding).map(Mapped::Feature)
        }
        Some("sheet_metal_bend")
            if feature
                .get("id")
                .and_then(Value::as_str)
                .is_some_and(|id| ctx.consumed_bends.contains(id)) =>
        {
            Ok(Mapped::Consumed)
        }
        Some("sheet_metal_bend") => Err(sheet::bend_refusal()),
        Some("stored_geometry") => base::stored(feature, ctx, report).map(Mapped::Feature),
        Some("base_feature") if base::holds_none(definition) => Ok(Mapped::Nothing),
        Some("base_feature") => base::map(feature, definition, ctx).map(Mapped::Feature),
        _ => Err(format!("{} is not mapped yet", kind(feature))),
    }
}

/// Assembly timeline rows build no part geometry; occurrences and joints carry them.
fn is_assembly_feature(feature: &Value) -> bool {
    match operation(feature).get("definition").and_then(Value::as_str) {
        Some("insert_component" | "assembly_joint") => true,
        // cadmpeg retains unresolved As-built joints as native operations with a stable
        // source tag. Display names may be edited and must never identify an operation.
        Some("native") => matches!(
            feature.get("source_tag").and_then(Value::as_str),
            Some("As-built" | "As-built Joint")
        ),
        _ => false,
    }
}

/// The feature's operation, looking through a post-processing wrapper.
fn operation(feature: &Value) -> &Value {
    let definition = feature.get("definition").unwrap_or(&Value::Null);
    match definition.get("definition").and_then(Value::as_str) {
        Some("post_process") => definition.get("operation").unwrap_or(&Value::Null),
        _ => definition,
    }
}

/// The CADIR operation name of a feature (`extrude`, `fillet`, …).
fn kind(feature: &Value) -> String {
    operation(feature)
        .get("definition")
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .to_owned()
}

/// The message for an input selection of a form that is not mapped, naming that form:
/// `noun` is `face`, `edge` or `body`, `kind` the CADIR selection kind that was found.
pub(crate) fn unsupported_selection(noun: &str, kind: Option<&str>) -> String {
    match kind {
        Some("historical") => {
            format!("a {noun} from an earlier history state is not supported yet")
        }
        Some("historical_set") => {
            format!("{noun}s from an earlier history state are not supported yet")
        }
        Some("historical_partial") => format!(
            "a {noun} selection from an earlier history state with some {noun}s unresolved is \
             not supported yet"
        ),
        Some("native" | "native_set") => format!(
            "a native {noun} reference is not supported yet: cadmpeg does not resolve it to a \
             model {noun}"
        ),
        Some("unresolved") => format!(
            "an unresolved {noun} reference is not supported yet: cadmpeg did not resolve it \
             to a model {noun}"
        ),
        Some("generated") => {
            format!("a {noun} generated by an earlier feature is not supported yet")
        }
        Some("local") => {
            format!("a {noun} local to an earlier feature is not supported yet")
        }
        Some("faces" | "edges" | "bodies") => {
            format!("a {noun} selection in this form is not supported yet")
        }
        Some(other) => format!("a {noun} selection of kind {other} is not supported yet"),
        None => format!("a {noun} selection with no kind is not supported yet"),
    }
}

/// A short description of a feature's form, from its extent, start, hole form or group
/// count. It holds no names and no sizes.
pub(crate) fn variant(feature: &Value) -> Option<String> {
    let op = operation(feature);
    let kind = kind(feature);
    let text = |pointer: &str| op.pointer(pointer).and_then(Value::as_str);
    let end = |termination: Option<&str>| match termination {
        Some("blind") => "blind",
        Some("through_all") => "through all",
        Some("to_face") => "to a face",
        Some("to_plane") => "to a plane",
        Some("to_vertex") => "to a vertex",
        Some("to_first") => "to the first face",
        Some("to_last") => "to the last face",
        Some("to_shape") => "to a shape",
        _ => "other end",
    };
    let mut parts = Vec::new();
    match kind.as_str() {
        "extrude" => {
            parts.push(match text("/extent/kind") {
                Some("symmetric") => "symmetric".to_owned(),
                Some("two_sided") => format!(
                    "two sides, {} and {}",
                    end(text("/extent/first/termination/kind")),
                    end(text("/extent/second/termination/kind"))
                ),
                _ => end(text("/extent/side/termination/kind")).to_owned(),
            });
            match text("/start/kind") {
                Some("from_face") => parts.push("start from a face".to_owned()),
                Some("from_plane") => parts.push("start from a plane".to_owned()),
                Some("offset_profile_plane") => parts.push("offset start".to_owned()),
                _ => {}
            }
        }
        "hole" => {
            let form = op
                .pointer("/shape/construction/kind/kind")
                .and_then(Value::as_str)
                .unwrap_or("other");
            parts.push(form.replace('_', " "));
            parts.push(end(text("/extent/kind")).to_owned());
        }
        "fillet" | "chamfer" => {
            let groups = op
                .get("groups")
                .and_then(Value::as_array)
                .map_or(0, Vec::len);
            if groups > 0 {
                parts.push(format!(
                    "{groups} {} group{}",
                    if kind == "fillet" {
                        "radius"
                    } else {
                        "distance"
                    },
                    if groups == 1 { "" } else { "s" }
                ));
            }
        }
        _ => return None,
    }
    Some(format!("{kind}, {}", parts.join(", ")))
}

/// A feature's source name, else its kind, for the report and the timeline.
fn name(feature: &Value) -> Option<&str> {
    feature
        .get("name")
        .and_then(Value::as_str)
        .filter(|name| !name.trim().is_empty())
}

/// A feature as the report names it.
fn describe(feature: &Value) -> String {
    match name(feature) {
        Some(name) => format!("feature \"{name}\""),
        None => format!("{} feature", kind(feature)),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::testing::{cube, feature, with};

    /// The reference sketches map the same from the arenas they read as from the whole
    /// model, and read no other arena (a debug build panics on one).
    #[test]
    fn sketches_read_the_same_from_the_arenas_they_need() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../dutchman-core/tests/fixtures");
        let mut mapped = 0;
        for name in [
            "design_sketch_constraints.f3d",
            "design_copy_paste.f3d",
            "generated_unit_cube.f3d",
            "container_metadata_only.f3d",
        ] {
            let Ok(model) = dutchman_core::read(
                dutchman_core::Format::FusionArchive,
                &root.join(name),
                &mut Report::default(),
            ) else {
                continue;
            };
            let whole = SketchWire::new();
            whole
                .set(Some(serde_json::to_value(&model.ir.model).unwrap()))
                .unwrap();
            let subset = SketchWire::new();
            let (mut a_notes, mut b_notes) = (Report::default(), Report::default());
            let a = sketches_where(&model, &whole, &mut a_notes, &|_| true);
            let b = sketches_where(&model, &subset, &mut b_notes, &|_| true);
            let keys = |wire: &SketchWire| -> Vec<String> {
                wire.get()
                    .and_then(Option::as_ref)
                    .and_then(Value::as_object)
                    .map(|rows| rows.keys().cloned().collect())
                    .unwrap_or_default()
            };
            if let Some(held) = subset.get().and_then(Option::as_ref) {
                assert!(keys(&subset)
                    .iter()
                    .all(|k| wire::SKETCH_ARENAS.contains(&k.as_str())));
                let full = whole.get().and_then(Option::as_ref).unwrap();
                for arena in wire::SKETCH_ARENAS {
                    assert_eq!(held.get(arena), full.get(arena), "{name}: {arena}");
                }
            }
            assert_eq!(a.1, b.1, "{name}");
            assert_eq!(a.0.features, b.0.features, "{name}");
            assert_eq!(a.0.params, b.0.params, "{name}");
            assert_eq!(a.0.next_id, b.0.next_id, "{name}");
            assert_eq!(format!("{a_notes:?}"), format!("{b_notes:?}"), "{name}");
            mapped += a.0.features.len();
        }
        assert!(mapped > 0, "no fixture sketch mapped");
    }

    #[test]
    fn unsupported_selections_name_their_form() {
        assert_eq!(
            unsupported_selection("face", Some("historical")),
            "a face from an earlier history state is not supported yet"
        );
        assert!(unsupported_selection("edge", Some("native")).contains("a native edge reference"));
        assert!(unsupported_selection("face", Some("unresolved"))
            .starts_with("an unresolved face reference"));
        assert!(unsupported_selection("face", Some("generated"))
            .starts_with("a face generated by an earlier feature"));
        assert!(
            unsupported_selection("body", Some("local")).contains("local to an earlier feature")
        );
        assert!(unsupported_selection("edge", Some("weird")).contains("of kind weird"));
        assert!(unsupported_selection("edge", None).contains("with no kind"));
    }

    #[test]
    fn variants_describe_the_form_without_sizes() {
        let extrude = json!({"definition": {"definition": "extrude",
            "start": {"kind": "from_face"},
            "extent": {"kind": "one_sided", "side": {"termination": {"kind": "to_plane"}}}}});
        assert_eq!(
            variant(&extrude).as_deref(),
            Some("extrude, to a plane, start from a face")
        );
        let hole = json!({"definition": {"definition": "hole",
            "shape": {"construction": {"kind": {"kind": "counterbore"}}, "diameter": 5.0},
            "extent": {"kind": "through_all"}}});
        assert_eq!(
            variant(&hole).as_deref(),
            Some("hole, counterbore, through all")
        );
        let fillet = json!({"definition": {"definition": "fillet",
            "groups": [{"radius": 1.0}, {"radius": 2.0}]}});
        assert_eq!(variant(&fillet).as_deref(), Some("fillet, 2 radius groups"));
        assert_eq!(
            variant(&json!({"definition": {"definition": "native"}})),
            None
        );
    }

    #[test]
    fn row_lookups_reuse_one_borrowed_index_per_arena() {
        let faces: Vec<_> = (0..16_384)
            .map(|i| json!({"id": format!("synthetic:face:{i}"), "ordinal": i}))
            .collect();
        let model = json!({"faces": faces, "points": [{"id": "point"}]});
        let ctx = Context::new(&model);
        assert!(ctx.rows.values().all(|index| index.get().is_none()));
        for row in ctx.arena("faces").iter().rev() {
            let id = row["id"].as_str().unwrap();
            assert!(std::ptr::eq(ctx.row("faces", id).unwrap(), row));
        }
        let index = ctx.rows["faces"].get().unwrap();
        assert_eq!(index.len(), 16_384);
        // Repeated misses also reuse the index; unrelated arenas stay unindexed.
        for _ in 0..16_384 {
            assert!(ctx.row("faces", "missing").is_none());
        }
        assert!(std::ptr::eq(ctx.rows["faces"].get().unwrap(), index));
        assert!(ctx.rows["points"].get().is_none());
        assert!(ctx.row("unknown", "point").is_none());
    }

    #[test]
    fn row_index_keeps_first_duplicate_and_arena_scope() {
        let model = json!({
            "faces": [null, {"id": 7}, {"id": "same", "value": 1},
                      {"id": "same", "value": 2}],
            "points": [{"id": "same", "value": 3}],
            "metadata": {"id": "same"}
        });
        let ctx = Context::new(&model);
        assert_eq!(ctx.row("faces", "same").unwrap()["value"], 1);
        assert_eq!(ctx.row("points", "same").unwrap()["value"], 3);
        assert!(ctx.row("metadata", "same").is_none());
        assert!(ctx.row("faces", "7").is_none());
        assert!(Context::new(&Value::Null).row("faces", "same").is_none());
    }

    fn datum(key: u32, ordinal: u64) -> Value {
        feature(
            key,
            ordinal,
            "XY Plane",
            json!({"definition": "datum_principal_plane", "plane": "top"}),
        )
    }

    fn opaque(key: u32, ordinal: u64, name: &str) -> Value {
        feature(
            key,
            ordinal,
            name,
            json!({"definition": "native", "kind": "Mystery", "parameters": {}}),
        )
    }

    #[test]
    fn body_renaming_updates_modify_operands() {
        let mut history = History {
            features: vec![
                json!({"body_names": ["Part"]}),
                json!({"target": "Part", "tools": ["Part"]}),
                json!({"bodies": ["Part"]}),
                json!({"body": "Part"}),
            ],
            ..History::default()
        };
        let mut taken = ["Part".to_owned()].into_iter().collect();
        history.rename_bodies("Second", &mut taken);
        assert_eq!(history.features[0]["body_names"], json!(["Part (Second)"]));
        assert_eq!(history.features[1]["target"], "Part (Second)");
        assert_eq!(history.features[1]["tools"], json!(["Part (Second)"]));
        assert_eq!(history.features[2]["bodies"], json!(["Part (Second)"]));
        assert_eq!(history.features[3]["body"], "Part (Second)");
    }

    #[test]
    fn default_import_names_follow_shape_order_without_reserving_cadir_names() {
        let mut taken: BodyNames = ["Block".to_owned()].into_iter().collect();
        let mut history = History {
            features: vec![json!({"body_names": ["Block", "Block (2)"]})],
            default_bodies: BTreeMap::from([
                ("Block".to_owned(), "Block".to_owned()),
                ("Block (2)".to_owned(), "Block".to_owned()),
            ]),
            looks: BTreeMap::from([("Block".to_owned(), json!({"color": [255, 0, 0]}))]),
            face_looks: vec![json!({"body": "Block (2)", "color": [0, 0, 255]})],
            ..History::default()
        };
        history.rename_bodies("Component", &mut taken);
        assert!(history.features[0].get("body_names").is_none());
        assert_eq!(
            history.default_bodies.keys().cloned().collect::<Vec<_>>(),
            ["Block (2)", "Block (3)"]
        );
        assert_eq!(history.looks["Block (2)"]["color"], json!([255, 0, 0]));
        assert_eq!(history.face_looks[0]["body"], "Block (3)");
        assert_eq!(taken.explicit, BTreeSet::from(["Block".to_owned()]));
        let mut next = History {
            features: vec![json!({"body_names": ["Block (2)"]})],
            ..History::default()
        };
        next.rename_bodies("Later", &mut taken);
        assert_eq!(next.features[0]["body_names"], json!(["Block (2)"]));
    }

    #[test]
    fn known_cadir_versions_are_read_and_others_refused() {
        assert!(check_version(&json!({"ir_version": "6"})).is_ok());
        for version in ["7", "8", "9", "10", "11"] {
            assert!(check_version(&json!({"ir_version": version})).is_ok());
        }
        assert_eq!(
            check_version(&json!({"ir_version": "12"})).unwrap_err(),
            "unsupported CADIR version \"12\" (Dutchman reads 6, 7, 8, 9, 10, 11)"
        );
        assert!(check_version(&json!({})).is_err());
    }

    #[test]
    fn sketches_nothing_is_built_on_or_the_source_hides_start_hidden() {
        let history = History {
            features: vec![
                json!({"id": 1, "type": "sketch"}),
                json!({"id": 2, "type": "sketch"}),
                json!({"id": 3, "type": "sketch"}),
                json!({"id": 4, "type": "sketch"}),
                json!({"id": 5, "type": "extrude", "sketch": 1}),
                json!({"id": 6, "type": "loft", "sections": [{"sketch": 3}]}),
            ],
            hidden: vec![3],
            ..History::default()
        };
        // 1 is extruded and shown; 2 is built on by nothing; 3 is lofted but hidden in the
        // source; 4 is built on by nothing.
        assert_eq!(history.hidden_sketches(), [json!(2), json!(3), json!(4)]);
    }

    #[test]
    fn a_model_without_history_is_flattened() {
        let err = map(
            &cube(),
            &mut Report::default(),
            crate::SolidCraftVersion::default(),
        )
        .result
        .unwrap_err();
        assert_eq!(err, "the design carries no feature history");
    }

    #[test]
    fn datum_planes_alone_build_no_geometry() {
        let model = with(cube(), json!([datum(1, 0), datum(2, 1)]), json!([]));
        let err = map(
            &model,
            &mut Report::default(),
            crate::SolidCraftVersion::default(),
        )
        .result
        .unwrap_err();
        assert_eq!(err, "no feature in the history builds geometry");
    }

    #[test]
    fn a_reference_image_builds_nothing() {
        let model = json!({"features": []});
        let mut ctx = Context::new(&model);
        let canvas = json!({"id": "f3d:model:feature#7", "ordinal": 7, "name": "Canvas 1",
            "definition": {"definition": "reference_image", "asset": "f3d:model:asset#1"}});
        let mut features = Vec::new();
        let mapped = map_feature(&canvas, &mut ctx, &mut Report::default(), &mut features);
        assert!(matches!(mapped, Ok(Mapped::Nothing)));
        assert!(features.is_empty());
    }

    #[test]
    fn tree_nodes_build_nothing_and_block_nothing() {
        let body = feature(
            2,
            1,
            "Body",
            json!({"definition": "tree_node", "role": "history"}),
        );
        let model = with(cube(), json!([datum(1, 0), body]), json!([]));
        let mapping = map(
            &model,
            &mut Report::default(),
            crate::SolidCraftVersion::default(),
        );
        // Nothing else builds geometry here; the node itself does not stop the history.
        assert_eq!(
            mapping.result.unwrap_err(),
            "no feature in the history builds geometry"
        );
        let statuses: Vec<FeatureStatus> = mapping.features.iter().map(|f| f.status).collect();
        assert_eq!(
            statuses,
            [FeatureStatus::NotNeeded, FeatureStatus::NotNeeded]
        );
    }

    #[test]
    fn components_inserted_and_joints_made_in_the_timeline_are_not_needed() {
        let insert = feature(
            2,
            1,
            "Component Insert 1",
            json!({"definition": "insert_component", "occurrence": "f3d:model:occurrence#1"}),
        );
        let joint = feature(
            3,
            2,
            "Joint 1",
            json!({"definition": "assembly_joint", "joint": "f3d:model:joint#1"}),
        );
        let model = with(cube(), json!([datum(1, 0), insert, joint]), json!([]));
        let mapping = map(
            &model,
            &mut Report::default(),
            crate::SolidCraftVersion::default(),
        );
        assert_eq!(
            mapping.result.unwrap_err(),
            "no feature in the history builds geometry"
        );
        let statuses: Vec<FeatureStatus> = mapping.features.iter().map(|f| f.status).collect();
        assert_eq!(statuses, [FeatureStatus::NotNeeded; 3]);
    }

    #[test]
    fn a_placed_datum_plane_is_not_needed() {
        let frame = json!({"origin": {"x": 0.0, "y": 0.0, "z": 15.0},
            "normal": {"x": 0.0, "y": 0.0, "z": 1.0}, "u_axis": {"x": 1.0, "y": 0.0, "z": 0.0}});
        let placed = feature(
            2,
            1,
            "DatumPlane",
            json!({"definition": "datum_plane", "frame": frame}),
        );
        let model = with(cube(), json!([datum(1, 0), placed]), json!([]));
        let mapping = map(
            &model,
            &mut Report::default(),
            crate::SolidCraftVersion::default(),
        );
        assert_eq!(
            mapping.result.unwrap_err(),
            "no feature in the history builds geometry"
        );
        let statuses: Vec<FeatureStatus> = mapping.features.iter().map(|f| f.status).collect();
        assert_eq!(
            statuses,
            [FeatureStatus::NotNeeded, FeatureStatus::NotNeeded]
        );
    }

    #[test]
    fn the_first_unmapped_feature_names_the_reason() {
        let model = with(
            cube(),
            json!([datum(1, 0), opaque(2, 2, "Later"), opaque(3, 1, "Earlier")]),
            json!([]),
        );
        let err = map(
            &model,
            &mut Report::default(),
            crate::SolidCraftVersion::default(),
        )
        .result
        .unwrap_err();
        assert_eq!(err, "feature \"Earlier\": native is not mapped yet");
    }

    /// A stored-geometry feature imports exactly the bodies it outputs, solids and sheets.
    #[allow(clippy::unwrap_used, clippy::indexing_slicing)]
    fn stored_outputs(outputs: Value) -> Result<Value, String> {
        let mut source = cube();
        source.ir = cadmpeg_ir::examples::unit_cube().unwrap();
        source.fidelity = None;
        let mut wire = serde_json::to_value(&source.ir).unwrap();
        let template = wire["model"].clone();
        for (prefix, label) in [("skin", "Skin"), ("other", "Other")] {
            let text = serde_json::to_string(&template)
                .unwrap()
                .replace("synthetic:cube:", &format!("synthetic:{prefix}:"));
            let mut copy: Value = serde_json::from_str(&text).unwrap();
            copy["bodies"][0]["name"] = json!(label);
            copy["bodies"][0]["kind"] = json!("sheet");
            for arena in [
                "points", "vertices", "curves", "edges", "coedges", "loops", "surfaces", "faces",
                "shells", "regions", "bodies",
            ] {
                wire["model"][arena]
                    .as_array_mut()
                    .unwrap()
                    .extend(copy[arena].as_array().unwrap().iter().cloned());
            }
        }
        wire["model"]["bodies"][0]["name"] = json!("Solid");
        source.ir = cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap();
        let model = &wire["model"];
        let mut ctx = Context::new(model);
        ctx.source = Some(&source);
        let outputs: Value = serde_json::from_str(
            &outputs
                .to_string()
                .replace("b#1", "synthetic:cube:body#0")
                .replace("b#2", "synthetic:skin:body#0")
                .replace("b#3", "synthetic:other:body#0"),
        )
        .unwrap();
        let stored = json!({
            "id": "f#1", "ordinal": 0, "name": "Composite1", "suppressed": false,
            "outputs": outputs,
            "definition": {"definition": "stored_geometry"},
        });
        match map_feature(&stored, &mut ctx, &mut Report::default(), &mut Vec::new())? {
            Mapped::Feature(mapped) => Ok(mapped),
            Mapped::Nothing | Mapped::Consumed => Err("mapped to nothing".to_owned()),
        }
    }

    #[test]
    fn stored_geometry_imports_its_solid_and_sheet_outputs_only() {
        let mapped = stored_outputs(json!(["b#1", "b#2"])).unwrap();
        assert_eq!(mapped["type"], "import");
        assert_eq!(mapped["name"], "Composite1");
        assert_eq!(
            mapped["_bodies"],
            json!(["synthetic:cube:body#0", "synthetic:skin:body#0"])
        );
        assert_eq!(mapped["body_names"], json!(["Solid", "Skin"]));
        let sheets = stored_outputs(json!(["b#2", "b#3"])).unwrap();
        assert_eq!(
            sheets["_bodies"],
            json!(["synthetic:skin:body#0", "synthetic:other:body#0"])
        );
        assert_eq!(sheets["body_names"], json!(["Skin", "Other"]));
    }

    #[test]
    fn stored_geometry_without_outputs_is_reported_not_guessed() {
        assert_eq!(stored_outputs(json!([])).unwrap_err(), "it holds no bodies");
    }

    #[test]
    fn suppressed_features_are_left_out_and_reported() {
        let mut hidden = opaque(2, 1, "Hidden");
        hidden["suppressed"] = json!(true);
        let model = with(cube(), json!([datum(1, 0), hidden]), json!([]));
        let mut report = Report::default();
        let err = map(&model, &mut report, crate::SolidCraftVersion::default())
            .result
            .unwrap_err();
        assert_eq!(err, "no feature in the history builds geometry");
        let loss = report.losses().first().unwrap();
        assert_eq!(loss.subject, "feature \"Hidden\"");
    }

    #[test]
    fn a_whole_sketch_of_several_profiles_waits_for_confirmation() {
        let model = json!({"sketches": [
            {"id": "two", "profiles": [[{"entity": "a"}], [{"entity": "b"}]]},
            {"id": "one", "profiles": [[{"entity": "a"}]]},
        ]});
        let ctx = Context::new(&model);
        let whole =
            |sketch: &str| json!({"definition": {"profile": {"kind": "sketch", "value": sketch}}});
        assert!(whole_of_several(&whole("two"), &ctx));
        assert!(!whole_of_several(&whole("one"), &ctx));
        let picked = json!({"definition": {"profile": {"kind": "sketch_profiles",
            "value": {"sketch": "two", "profiles": [0]}}}});
        assert!(!whole_of_several(&picked, &ctx));

        let mut history = History {
            features: vec![
                json!({"id": 1, "name": "Sketch 1", "type": "sketch"}),
                json!({"id": 2, "name": "Extrude 1", "type": "extrude"}),
                json!({"id": 3, "name": "Extrude 4", "type": "extrude"}),
            ],
            sources: BTreeMap::from([(1, 0), (2, 1), (3, 2)]),
            ..History::default()
        };
        assert_eq!(history.first_unconfirmed(), None);
        history.unconfirmed.insert(3);
        let (source, why) = history.first_unconfirmed().unwrap();
        assert_eq!(source, 2);
        assert!(
            why.starts_with("feature \"Extrude 4\": its profile is the whole of a sketch"),
            "{why}"
        );
    }

    #[test]
    fn body_centers_with_incomplete_bodies_are_unknown() {
        let feature = json!({"source_properties": {
            "body_centers": "1:0,0,10",
            "body_centers_exact": "1",
            "body_centers_incomplete": "2;3",
        }});
        assert!(stored_body_centers(&feature).is_none());
    }

    #[test]
    fn body_centers_with_incomplete_evidence_are_unknown_even_with_all_centers() {
        let feature = json!({"source_properties": {
            "body_centers": "1:0,0,10;2:5,0,20",
            "body_centers_exact": "1,2",
            "body_centers_incomplete": "2",
        }});
        assert!(stored_body_centers(&feature).is_none());
    }

    #[test]
    fn body_centers_with_empty_incomplete_list_are_usable() {
        let feature = json!({"source_properties": {
            "body_centers": "1:0,0,10;2:5,0,20",
            "body_centers_exact": "1",
            "body_centers_incomplete": "",
        }});
        assert_eq!(
            stored_body_centers(&feature),
            Some(vec![
                crate::verify::StoredBody {
                    center: [0.0, 0.0, 10.0],
                    exact: true,
                },
                crate::verify::StoredBody {
                    center: [5.0, 0.0, 20.0],
                    exact: false,
                },
            ])
        );
    }

    #[test]
    fn body_centers_without_incomplete_property_are_usable() {
        let feature = json!({"source_properties": {
            "body_centers": "1:0,0,10;2:5,0,20",
            "body_centers_exact": "1",
        }});
        assert_eq!(
            stored_body_centers(&feature),
            Some(vec![
                crate::verify::StoredBody {
                    center: [0.0, 0.0, 10.0],
                    exact: true,
                },
                crate::verify::StoredBody {
                    center: [5.0, 0.0, 20.0],
                    exact: false,
                },
            ])
        );
    }

    #[test]
    fn a_base_feature_holding_its_final_body_has_no_stored_state_to_compare() {
        let feature = json!({"name": "Base Feature 1",
            "source_properties": {"body_centers": "1:0,0,10"}});
        let outcome = |final_body: bool| FeatureOutcome {
            name: Some("Base Feature 1".to_owned()),
            kind: "base_feature".to_owned(),
            status: FeatureStatus::Mapped,
            reason: None,
            ordinal: Some(0),
            variant: None,
            profile_unverified: false,
            reapplied: false,
            final_body,
        };
        assert_eq!(
            compared_state(&outcome(false), &feature).map(|c| c.len()),
            Some(1)
        );
        assert!(compared_state(&outcome(true), &feature).is_none());
    }
}
