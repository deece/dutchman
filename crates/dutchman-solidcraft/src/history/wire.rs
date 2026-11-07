//! The part of the neutral model's wire form that sketch mapping reads.
//!
//! Turning the whole neutral model into a JSON value costs several times the size of the
//! model on a large assembly, and the reference sketches read a handful of its arenas. The
//! value built here holds those arenas only, with the rows the model's own serialisation
//! writes for them, so a lookup finds exactly what it would find in the full value.

use serde::ser::{Error as _, Impossible, Serialize, SerializeStruct, Serializer};
use serde_json::{Map, Value};

/// Every arena the reference sketch path reads, and where. This is the whole list: a read of
/// any other arena from that path is a bug, and fails a debug build (see
/// `Context::reading_only`).
///
/// - `sketches`: `sketch::map` (the sketch row, its plane), `sketches_where` (which sketches
///   to map).
/// - `sketch_entities`: `sketch::map` (the entities of the sketch), reached again through
///   `follow_plane`.
/// - `sketch_constraints`: `sketch::map` (the ties to the origin) and `constraints::map` (the
///   constraints it turns into SolidCraft ones).
/// - `features`: `sketch::map`, through `crossing::region_lines`, `crossing::stray_curves` and
///   `crossing::trimmed_region_curves`, which look at the features that use the sketch.
/// - `parameters`: `Mapper::distance_value` and `Mapper::angle_value` in `constraints`
///   (`Context::row("parameters", ..)`) and `params::unsigned_dimensions`.
///
/// `params::map` and `params::needed` read the typed model and `Context::params` only, not
/// the wire value; `Context::row` goes through `Context::arena`, so the rows it finds are
/// those of an arena in this list as well.
pub(super) const SKETCH_ARENAS: &[&str] = &[
    "features",
    "parameters",
    "sketch_constraints",
    "sketch_entities",
    "sketches",
];

/// The wire form of `model`, with only the arenas in `keep`.
pub(super) fn only_arenas<T: Serialize>(
    model: &T,
    keep: &[&str],
) -> Result<Value, serde_json::Error> {
    model.serialize(Filter { keep })
}

/// A serializer for a struct that keeps the named fields as a JSON object.
struct Filter<'k> {
    keep: &'k [&'k str],
}

struct Fields<'k> {
    keep: &'k [&'k str],
    kept: Map<String, Value>,
}

impl SerializeStruct for Fields<'_> {
    type Ok = Value;
    type Error = serde_json::Error;

    fn serialize_field<T: ?Sized + Serialize>(
        &mut self,
        key: &'static str,
        value: &T,
    ) -> Result<(), Self::Error> {
        if self.keep.contains(&key) {
            self.kept
                .insert(key.to_owned(), serde_json::to_value(value)?);
        }
        Ok(())
    }

    fn end(self) -> Result<Value, Self::Error> {
        Ok(Value::Object(self.kept))
    }
}

macro_rules! refuse {
    ($($method:ident($($arg:ty),*);)*) => {
        $(fn $method(self, $(_: $arg),*) -> Result<Value, Self::Error> {
            Err(serde_json::Error::custom("the model is not a struct"))
        })*
    };
}

impl<'k> Serializer for Filter<'k> {
    type Ok = Value;
    type Error = serde_json::Error;
    type SerializeSeq = Impossible<Value, serde_json::Error>;
    type SerializeTuple = Impossible<Value, serde_json::Error>;
    type SerializeTupleStruct = Impossible<Value, serde_json::Error>;
    type SerializeTupleVariant = Impossible<Value, serde_json::Error>;
    type SerializeMap = Impossible<Value, serde_json::Error>;
    type SerializeStruct = Fields<'k>;
    type SerializeStructVariant = Impossible<Value, serde_json::Error>;

    fn serialize_struct(
        self,
        _name: &'static str,
        len: usize,
    ) -> Result<Self::SerializeStruct, Self::Error> {
        Ok(Fields {
            keep: self.keep,
            kept: Map::with_capacity(len.min(self.keep.len())),
        })
    }

    refuse! {
        serialize_bool(bool);
        serialize_i8(i8);
        serialize_i16(i16);
        serialize_i32(i32);
        serialize_i64(i64);
        serialize_u8(u8);
        serialize_u16(u16);
        serialize_u32(u32);
        serialize_u64(u64);
        serialize_f32(f32);
        serialize_f64(f64);
        serialize_char(char);
        serialize_str(&str);
        serialize_bytes(&[u8]);
        serialize_none();
        serialize_unit();
        serialize_unit_struct(&'static str);
        serialize_unit_variant(&'static str, u32, &'static str);
    }

    fn serialize_some<T: ?Sized + Serialize>(self, _: &T) -> Result<Value, Self::Error> {
        Err(serde_json::Error::custom("the model is not a struct"))
    }
    fn serialize_newtype_struct<T: ?Sized + Serialize>(
        self,
        _: &'static str,
        _: &T,
    ) -> Result<Value, Self::Error> {
        Err(serde_json::Error::custom("the model is not a struct"))
    }
    fn serialize_newtype_variant<T: ?Sized + Serialize>(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        _: &T,
    ) -> Result<Value, Self::Error> {
        Err(serde_json::Error::custom("the model is not a struct"))
    }
    fn serialize_seq(self, _: Option<usize>) -> Result<Self::SerializeSeq, Self::Error> {
        Err(serde_json::Error::custom("the model is not a struct"))
    }
    fn serialize_tuple(self, _: usize) -> Result<Self::SerializeTuple, Self::Error> {
        Err(serde_json::Error::custom("the model is not a struct"))
    }
    fn serialize_tuple_struct(
        self,
        _: &'static str,
        _: usize,
    ) -> Result<Self::SerializeTupleStruct, Self::Error> {
        Err(serde_json::Error::custom("the model is not a struct"))
    }
    fn serialize_tuple_variant(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        _: usize,
    ) -> Result<Self::SerializeTupleVariant, Self::Error> {
        Err(serde_json::Error::custom("the model is not a struct"))
    }
    fn serialize_map(self, _: Option<usize>) -> Result<Self::SerializeMap, Self::Error> {
        Err(serde_json::Error::custom("the model is not a struct"))
    }
    fn serialize_struct_variant(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        _: usize,
    ) -> Result<Self::SerializeStructVariant, Self::Error> {
        Err(serde_json::Error::custom("the model is not a struct"))
    }
}
