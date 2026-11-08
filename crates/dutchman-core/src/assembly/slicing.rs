//! One shared row index for component extraction. Geometry stays in compact JSON until
//! a component reaches it; only structure and history need an expanded JSON view.

use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::{value::RawValue, Value};

#[derive(Deserialize)]
pub(super) struct Packed<'a> {
    #[serde(borrow)]
    model: BTreeMap<&'a str, Vec<&'a RawValue>>,
    #[serde(borrow)]
    native: Option<&'a RawValue>,
    #[serde(flatten)]
    fields: serde_json::Map<String, Value>,
}

impl<'a> Packed<'a> {
    pub fn read(text: &'a str) -> Result<Self, String> {
        serde_json::from_str(text).map_err(|err| err.to_string())
    }

    /// The assembly algorithms inspect these arenas and native placement records.
    /// Extracted parts retain every header field and omit native records, as before.
    pub fn document(&self) -> Result<Value, String> {
        let mut fields = self.fields.clone();
        let mut model = serde_json::Map::new();
        for name in [
            "bodies",
            "product_definitions",
            "occurrences",
            "assembly_joints",
            "features",
            "parameters",
            "appearance_bindings",
        ] {
            if let Some(rows) = self.model.get(name) {
                let rows: Result<Vec<Value>, _> = rows
                    .iter()
                    .map(|row| serde_json::from_str(row.get()))
                    .collect();
                model.insert(
                    name.to_owned(),
                    Value::Array(rows.map_err(|err| err.to_string())?),
                );
            }
        }
        fields.insert("model".to_owned(), Value::Object(model));
        if let Some(native) = self.native {
            let namespaces: BTreeMap<String, &RawValue> =
                serde_json::from_str(native.get()).map_err(|err| err.to_string())?;
            if let Some(f3d) = namespaces.get("f3d") {
                let arenas: BTreeMap<String, &RawValue> =
                    serde_json::from_str(f3d.get()).map_err(|err| err.to_string())?;
                let mut selected = serde_json::Map::new();
                for name in [
                    "xref_designs",
                    "xref_references",
                    "design_component_occurrences",
                    "display_poses",
                ] {
                    if let Some(rows) = arenas.get(name) {
                        selected.insert(
                            name.to_owned(),
                            serde_json::from_str(rows.get()).map_err(|err| err.to_string())?,
                        );
                    }
                }
                fields.insert("native".to_owned(), serde_json::json!({ "f3d": selected }));
            }
        }
        Ok(Value::Object(fields))
    }
}

#[derive(Clone, Copy)]
pub(super) enum Row<'a> {
    Value(&'a Value),
    Packed(&'a RawValue),
}

impl<'a> Row<'a> {
    pub fn value(self) -> Result<Cow<'a, Value>, String> {
        match self {
            Self::Value(value) => Ok(Cow::Borrowed(value)),
            Self::Packed(value) => serde_json::from_str(value.get())
                .map(Cow::Owned)
                .map_err(|err| err.to_string()),
        }
    }

    pub fn selected(self) -> Selected<'a> {
        match self {
            Self::Value(value) => Selected::Value(Cow::Borrowed(value)),
            Self::Packed(value) => Selected::Packed(value),
        }
    }
}

#[derive(Clone, Copy)]
pub(super) struct Entry<'a> {
    pub arena: &'a str,
    pub ordinal: usize,
    pub row: Row<'a>,
}

pub(super) struct Index<'a> {
    pub document: &'a Value,
    pub rows: BTreeMap<String, Entry<'a>>,
    pub arenas: Vec<&'a str>,
    /// Reverse presentation edges, shared by all component slices. Ids are
    /// complete identities, including instance scope; suffixes are not aliases.
    pub bindings: BTreeMap<&'a str, Vec<&'a Value>>,
    // Decoded models have unique ids. Preserve the old selection behavior for callers
    // passing an in-progress model with duplicate ids: follow the last, emit all.
    pub duplicates: Vec<(String, Entry<'a>)>,
    /// `body_centers` lists that features of more than one definition carry. Such a list
    /// can't be the state of each owning component, so a component's slice leaves it out.
    pub shared_centers: BTreeSet<&'a str>,
}

impl<'a> Index<'a> {
    fn new(document: &'a Value) -> Self {
        let mut bindings: BTreeMap<&str, Vec<&Value>> = BTreeMap::new();
        for binding in document
            .pointer("/model/appearance_bindings")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if let Some(target) = binding.pointer("/target/id").and_then(Value::as_str) {
                bindings.entry(target).or_default().push(binding);
            }
        }
        let mut owners: BTreeMap<&str, BTreeSet<Option<&str>>> = BTreeMap::new();
        for feature in document
            .pointer("/model/features")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if let Some(centers) = feature
                .pointer("/source_properties/body_centers")
                .and_then(Value::as_str)
            {
                owners
                    .entry(centers)
                    .or_default()
                    .insert(feature.get("product_definition").and_then(Value::as_str));
            }
        }
        let shared_centers = owners
            .into_iter()
            .filter(|(_, definitions)| definitions.len() > 1)
            .map(|(centers, _)| centers)
            .collect();
        Self {
            document,
            rows: BTreeMap::new(),
            arenas: Vec::new(),
            bindings,
            duplicates: Vec::new(),
            shared_centers,
        }
    }

    fn insert(&mut self, id: String, entry: Entry<'a>) {
        if let Some(previous) = self.rows.insert(id.clone(), entry) {
            self.duplicates.push((id, previous));
        }
    }

    pub fn packed(source: &Packed<'a>, document: &'a Value) -> Result<Self, String> {
        #[derive(Deserialize)]
        struct Identity {
            id: Option<String>,
        }
        let mut index = Self::new(document);
        for (&arena, rows) in &source.model {
            index.arenas.push(arena);
            for (ordinal, row) in rows.iter().enumerate() {
                let identity: Identity =
                    serde_json::from_str(row.get()).map_err(|err| err.to_string())?;
                if let Some(id) = identity.id {
                    index.insert(
                        id,
                        Entry {
                            arena,
                            ordinal,
                            row: Row::Packed(row),
                        },
                    );
                }
            }
        }
        Ok(index)
    }

    pub fn value(document: &'a Value) -> Result<Self, String> {
        let wire = document
            .get("model")
            .and_then(Value::as_object)
            .ok_or("the model is malformed")?;
        let mut index = Self::new(document);
        for (arena, values) in wire {
            index.arenas.push(arena);
            for (ordinal, row) in values.as_array().into_iter().flatten().enumerate() {
                if let Some(id) = row.get("id").and_then(Value::as_str) {
                    index.insert(
                        id.to_owned(),
                        Entry {
                            arena,
                            ordinal,
                            row: Row::Value(row),
                        },
                    );
                }
            }
        }
        Ok(index)
    }
}

#[derive(Serialize)]
#[serde(untagged)]
pub(super) enum Selected<'a> {
    Packed(&'a RawValue),
    Value(Cow<'a, Value>),
}

#[derive(Serialize)]
pub(super) struct Part<'a> {
    #[serde(flatten)]
    pub fields: serde_json::Map<String, Value>,
    pub model: BTreeMap<&'a str, Vec<Selected<'a>>>,
}
