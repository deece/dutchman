//! A native file converted end to end, checked against SolidCraft's design container layout
//! and a golden copy of the design document.
//!
//! Set `UPDATE_GOLDEN=1` to rewrite the golden file after an intended change.

// Test helpers outside `#[test]` functions are not covered by clippy.toml's test allowances.
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]

use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};

use dutchman_core::convert;
use dutchman_solidcraft::{SolidCraftWriter, Verification};
use serde_json::Value;
use sha2::{Digest, Sha256};
use zip::{CompressionMethod, ZipArchive};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../dutchman-core/tests/fixtures")
        .join(name)
}

fn golden(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden")
        .join(name)
}

fn entry(archive: &mut ZipArchive<Cursor<Vec<u8>>>, name: &str) -> (Vec<u8>, CompressionMethod) {
    let mut file = archive.by_name(name).unwrap();
    let method = file.compression();
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).unwrap();
    (bytes, method)
}

#[test]
fn fusion_unit_cube_becomes_a_solidcraft_import() {
    converts_to_an_import("generated_unit_cube.f3d");
}

#[test]
fn inventor_unit_cube_becomes_a_solidcraft_import() {
    converts_to_an_import("synthetic_cube.ipt");
}

#[test]
fn solidworks_unit_cube_becomes_a_solidcraft_import() {
    converts_to_an_import("solidworks_cube.sldprt");
}

/// Converts fixture `file`, which has no history, and checks the result is SolidCraft's
/// import container with a document matching `tests/golden/<stem>.document.json`.
fn converts_to_an_import(file: &str) {
    let stem = file.rsplit_once('.').map_or(file, |(stem, _)| stem);
    let dir = std::env::temp_dir().join(format!("dutchman-e2e-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let output = dir.join(format!("{stem}.solidcraft"));

    let report = convert(
        &fixture(file),
        &output,
        &SolidCraftWriter::new().verification(Verification::Off),
    )
    .unwrap();
    for loss in report.losses() {
        println!("{loss}");
    }

    let bytes = std::fs::read(&output).unwrap();
    assert!(bytes.starts_with(b"PK\x03\x04"), "a zip container");
    let mut archive = ZipArchive::new(Cursor::new(bytes)).unwrap();

    // The layout SolidCraft writes: the STEP payload, then the document, then the stamp.
    let names: Vec<String> = (0..archive.len())
        .map(|index| {
            archive
                .by_index(index)
                .unwrap()
                .name()
                .unwrap()
                .into_owned()
        })
        .collect();
    assert_eq!(names.len(), 3, "{names:?}");
    let blob = names.first().unwrap().clone();
    assert_eq!(names.get(1..).unwrap(), ["document.json", "container.json"]);

    let (container, method) = entry(&mut archive, "container.json");
    assert_eq!(method, CompressionMethod::Stored);
    assert_eq!(container, br#"{"format":"solidcraft-container/1"}"#);

    let (step, method) = entry(&mut archive, &blob);
    assert_eq!(method, CompressionMethod::Deflated);
    let digest: String = Sha256::digest(&step)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    assert_eq!(blob, format!("blobs/{digest}.step"), "content-addressed");
    let step = String::from_utf8(step).unwrap();
    assert!(step.starts_with("ISO-10303-21;"), "STEP text");
    assert!(step.contains("AUTOMOTIVE_DESIGN"), "AP214");
    assert!(step.contains("MANIFOLD_SOLID_BREP"), "the cube is a solid");
    assert_eq!(step.matches("ADVANCED_FACE(").count(), 6, "six cube faces");

    let (document, method) = entry(&mut archive, "document.json");
    assert_eq!(method, CompressionMethod::Deflated);
    let document: Value = serde_json::from_slice(&document).unwrap();
    let import = document["features"].get(0).unwrap();
    assert_eq!(import["type"], "import");
    assert_eq!(import["step"], "");
    assert_eq!(import["step_blob"], blob.as_str());
    assert_eq!(import["file"], file);

    // The STEP blob's name depends on cadmpeg's STEP writer, not on the document.
    let pretty = (serde_json::to_string_pretty(&document).unwrap() + "\n")
        .replace(blob.as_str(), "blobs/<sha256>.step");
    let golden = golden(&format!("{stem}.document.json"));
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        std::fs::write(&golden, &pretty).unwrap();
    }
    let expected = std::fs::read_to_string(&golden).unwrap();
    assert_eq!(
        pretty,
        expected,
        "document.json differs from {}",
        golden.display()
    );
}

#[test]
fn fusion_unit_cube_defaults_to_preserving_source_axes() {
    let dir = std::env::temp_dir().join(format!("dutchman-e2e-default-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let output = dir.join("default.solidcraft");
    convert(
        &fixture("generated_unit_cube.f3d"),
        &output,
        &SolidCraftWriter::new().verification(Verification::Off),
    )
    .unwrap();
    let bytes = std::fs::read(&output).unwrap();
    let mut archive = ZipArchive::new(Cursor::new(bytes)).unwrap();
    let (document, _) = entry(&mut archive, "document.json");
    let document: Value = serde_json::from_slice(&document).unwrap();
    assert_eq!(
        document["occurrences"][0]["transform"],
        serde_json::json!([
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ])
    );
}

#[test]
fn fusion_unit_cube_with_stand_up_rotates_to_z_up() {
    let dir = std::env::temp_dir().join(format!("dutchman-e2e-standup-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let output = dir.join("standup.solidcraft");
    convert(
        &fixture("generated_unit_cube.f3d"),
        &output,
        &SolidCraftWriter::new()
            .stand_up(Some(true))
            .verification(Verification::Off),
    )
    .unwrap();
    let bytes = std::fs::read(&output).unwrap();
    let mut archive = ZipArchive::new(Cursor::new(bytes)).unwrap();
    let (document, _) = entry(&mut archive, "document.json");
    let document: Value = serde_json::from_slice(&document).unwrap();
    assert_eq!(
        document["occurrences"][0]["transform"],
        serde_json::json!([
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, -1.0, 0.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ])
    );
}

#[test]
fn solidcraft_shows_every_face_of_each_cube() {
    let Some(cli) = std::env::var_os(dutchman_solidcraft::CLI_ENV).map(PathBuf::from) else {
        eprintln!("skipped: set SOLIDCRAFT_CLI to open the designs in SolidCraft");
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    for file in [
        "generated_unit_cube.f3d",
        "synthetic_cube.ipt",
        "solidworks_cube.sldprt",
    ] {
        let report = convert(
            &fixture(file),
            &dir.path().join("cube.solidcraft"),
            &SolidCraftWriter::new().verification(Verification::With(cli.clone())),
        )
        .unwrap();
        let geometry: Vec<String> = report
            .losses()
            .iter()
            .filter(|loss| loss.subject == "geometry")
            .map(ToString::to_string)
            .collect();
        assert!(geometry.is_empty(), "{file}: {geometry:?}");
    }
}

#[test]
fn interchange_codecs_convert_generated_geometry_through_step() {
    use cadmpeg_ir::codec::write::{target::TargetRequest, EncodeInput};
    use cadmpeg_registry::{build_encoder, Format};
    let dir = tempfile::tempdir().unwrap();
    let mut ir = cadmpeg_ir::examples::unit_cube().unwrap();
    for face in &mut ir.model.faces {
        face.name = None;
    }
    for (format, extension) in [(Format::Step, "stp"), (Format::Rhino, "3dm")] {
        let encoder = build_encoder(format);
        let (_, target) = encoder.targets().default().unwrap();
        let plan = encoder
            .plan(
                EncodeInput::new(&ir, None),
                TargetRequest::Explicit(target.id.as_str()),
            )
            .unwrap();
        let mut bytes = Vec::new();
        plan.write_to(&mut bytes).unwrap();
        let input = dir.path().join(format!("cube.{extension}"));
        let output = dir.path().join(format!("{extension}.solidcraft"));
        std::fs::write(&input, bytes).unwrap();
        let report = convert(
            &input,
            &output,
            &SolidCraftWriter::new().verification(Verification::Off),
        )
        .unwrap();
        assert!(output.is_file());
        assert!(!report
            .losses()
            .iter()
            .any(|loss| loss.detail.contains("no solids or surfaces")));
    }
}

#[test]
fn a_step_input_is_stored_as_it_is() {
    use cadmpeg_ir::codec::write::{target::TargetRequest, EncodeInput};
    use cadmpeg_registry::{build_encoder, Format};
    use sha2::Digest;
    use std::io::Read;
    let dir = tempfile::tempdir().unwrap();
    let ir = cadmpeg_ir::examples::unit_cube().unwrap();
    let encoder = build_encoder(Format::Step);
    let (_, target) = encoder.targets().default().unwrap();
    let plan = encoder
        .plan(
            EncodeInput::new(&ir, None),
            TargetRequest::Explicit(target.id.as_str()),
        )
        .unwrap();
    let mut bytes = Vec::new();
    plan.write_to(&mut bytes).unwrap();
    // A comment cadmpeg's own STEP writer would never restate.
    let text = String::from_utf8(bytes).unwrap().replacen(
        "DATA;",
        "/* written by another program */\nDATA;",
        1,
    );
    let input = dir.path().join("cube.step");
    let output = dir.path().join("cube.solidcraft");
    std::fs::write(&input, &text).unwrap();
    let report = convert(
        &input,
        &output,
        &SolidCraftWriter::new().verification(Verification::Off),
    )
    .unwrap();
    assert!(report
        .losses()
        .iter()
        .any(|loss| loss.detail.contains("passed to SolidCraft as it is")));
    // Reading the file reports its own losses; restating it would add the writer's.
    assert!(!report.losses().iter().any(|loss| loss
        .code
        .as_deref()
        .is_some_and(|code| code.starts_with("step/") && !code.starts_with("step/decode."))));
    let mut archive = zip::ZipArchive::new(std::fs::File::open(&output).unwrap()).unwrap();
    // Payloads are named by the SHA-256 of their bytes: the input's own name proves it is unchanged.
    let digest = sha2::Sha256::digest(text.as_bytes());
    let name = format!(
        "blobs/{}.step",
        digest
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    );
    let mut stored = String::new();
    archive
        .by_name(&name)
        .unwrap()
        .read_to_string(&mut stored)
        .unwrap();
    assert_eq!(stored, text);
}
