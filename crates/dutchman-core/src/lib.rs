//! Format-neutral conversion core for Dutchman.
//!
//! A conversion is a cadmpeg decode (native file → [`Model`], cadmpeg's neutral CADIR)
//! followed by a [`Writer`] ([`Model`] → target format). The writer never sees the native
//! file and the decoder never sees the target; the [`Report`] carries everything that could
//! not be carried across. See `docs/ARCHITECTURE.md`.

mod assembly;
mod decode;
mod error;
mod format;
mod hidden;
mod input_edges;
mod locate;
mod memory;
mod names;
pub mod parallel;
mod part_positions;
pub mod parts;
pub mod placement;
mod progress;
mod refusal;
pub use part_positions::{PartPositionCounts, PartPositions};
pub use refusal::{
    classify as classify_refusal, clean as clean_refusal, Refusal,
    GENERIC_CATEGORY as GENERIC_REFUSAL,
};
mod report;
pub mod repro;

use std::path::Path;

pub use assembly::{
    frame_aligned, local_assembly, local_assembly_with_positions, Assembly, Joint, JointNote,
    LeftOut, Motion, Placement, MAX_DEPTH, MAX_PARTS, MAX_PLACEMENTS,
};
pub use error::{CheckFacts, Error, FILE_DID_NOT_CONVERT, RELEASES};
pub use format::Format;
pub use locate::{PartSearch, MAX_SEARCHED};
pub use memory::available_memory;
pub use progress::{elapsed, MemberEvent, MemberObserver, Progress, ProgressEvent};

/// The neutral model every conversion passes through.
#[derive(Clone, Debug)]
pub struct Model {
    /// The design as cadmpeg's CADIR: exact B-rep, feature history and product structure, in
    /// millimetres and radians.
    pub ir: cadmpeg_ir::CadIr,
    /// Where each entity came from in the source file, when the decoder recorded it. Writers
    /// that restate geometry through cadmpeg's encoders pass it on.
    pub fidelity: Option<cadmpeg_ir::SourceFidelity>,
    /// The design's name, taken from the input file's name.
    pub name: String,
    /// The input file's name (without its folder), when the model was read from a file.
    pub source_file: Option<String>,
    /// For an assembly: its parts, read from their own files, and where each is placed.
    pub assembly: Option<Assembly>,
    /// The input file's text when it is a STEP file whose geometry is used unchanged: writers
    /// hand it on as it is instead of restating the decoded model as STEP.
    pub source_step: Option<String>,
}

impl Model {
    /// Classifies an assignment no placed component retained, using decoded identities.
    pub fn unplaced_appearance_cause(
        &self,
        binding: &cadmpeg_ir::appearance::AppearanceBinding,
    ) -> AppearanceLoss {
        use cadmpeg_ir::appearance::AppearanceTarget;
        match &binding.target {
            AppearanceTarget::Body(id) => match self.ir.model.bodies.iter().find(|b| b.id == *id) {
                None => AppearanceLoss::TargetAbsent,
                Some(body) if body.kind == cadmpeg_ir::topology::BodyKind::Wire => {
                    AppearanceLoss::Wire
                }
                Some(_) => AppearanceLoss::Other,
            },
            AppearanceTarget::Face(id) if !self.ir.model.faces.iter().any(|f| f.id == *id) => {
                AppearanceLoss::TargetAbsent
            }
            AppearanceTarget::Edge(id)
                if self
                    .ir
                    .model
                    .shells
                    .iter()
                    .any(|s| s.wire_edges().contains(id)) =>
            {
                AppearanceLoss::Wire
            }
            _ => AppearanceLoss::Other,
        }
    }

    /// Explains an assignment that no placed component retained. Body and face
    /// identities are checked exactly, without conflating different instances.
    pub fn unplaced_appearance_binding(
        &self,
        binding: &cadmpeg_ir::appearance::AppearanceBinding,
    ) -> String {
        use cadmpeg_ir::appearance::AppearanceTarget;
        let (kind, id, exists) = match &binding.target {
            AppearanceTarget::Body(id) => (
                "body",
                id.as_str(),
                self.ir.model.bodies.iter().any(|body| body.id == *id),
            ),
            AppearanceTarget::Face(id) => (
                "face",
                id.as_str(),
                self.ir.model.faces.iter().any(|face| face.id == *id),
            ),
            _ => {
                return format!(
                "Appearance binding {} is left out: its target has no placed component geometry",
                binding.id
            )
            }
        };
        let reason = if exists {
            "its target exists in the decoded model but has no placed component geometry"
        } else {
            "its target is absent from the decoded model"
        };
        format!(
            "Appearance binding {} to {kind} {id} is left out: {reason}",
            binding.id
        )
    }
}
pub use report::{
    AppearanceLoss, Check, Completeness, FeatureOutcome, FeatureStatus, LeftOutGroup, Loss, Mode,
    Outcome, PartHistory, Report, Severity, Stage,
};

/// Whether a source-native CADIR operation identifies a construction binder. Keep native
/// type recognition in the core, so writers need only handle the neutral helper role.
pub fn is_native_binder(kind: &str) -> bool {
    matches!(
        kind,
        "PartDesign::SubShapeBinder" | "PartDesign::ShapeBinder"
    )
}

/// The largest input file Dutchman will open, in bytes. Native CAD files beyond this are
/// rejected up front rather than read into memory.
pub const MAX_INPUT_BYTES: u64 = 1 << 30;

/// Encodes the neutral [`Model`] into a target format. This is the plugin seam: a target
/// (SolidCraft today) implements it and depends on nothing else in Dutchman.
pub trait Writer {
    /// The file extension this writer produces, without the dot.
    fn extension(&self) -> &'static str;

    /// Whether this writer accepts an output suffix, including legacy names.
    fn accepts_extension(&self, extension: &str) -> bool {
        extension.eq_ignore_ascii_case(self.extension())
    }

    /// Writes `model` to `out`, recording anything it drops in `report`.
    fn write(
        &self,
        model: &Model,
        out: &mut dyn std::io::Write,
        report: &mut Report,
    ) -> Result<(), Error>;
}

/// Decodes the file at `input` (already identified as `format`) into the neutral model,
/// recording anything dropped in `report`.
pub fn read(format: Format, input: &Path, report: &mut Report) -> Result<Model, Error> {
    read_in(format, input, &PartSearch::default(), report)
}

/// Resolves an assembly's file references without reading its parts.
/// Uses the same guarded decoder and part search as conversion.
pub fn assembly_references(
    input: &Path,
    search: &PartSearch,
) -> Result<Vec<std::path::PathBuf>, Error> {
    let format = Format::from_path(input).ok_or_else(|| Error::UnknownFormat {
        path: input.to_path_buf(),
    })?;
    if !format.is_assembly() || !format.can_convert() {
        return Ok(Vec::new());
    }
    let model = decode_file(format, input, &mut Report::default(), None)?;
    let value = serde_json::to_value(&model.ir.model.occurrences)
        .map_err(|err| Error::Decode(format!("cannot read assembly references: {err}")))?;
    let finder = locate::Finder::new(search);
    let folder = input.parent().unwrap_or(Path::new("."));
    let mut references: std::collections::BTreeSet<_> = value
        .as_array()
        .into_iter()
        .flatten()
        .take(MAX_PLACEMENTS)
        // SolidWorks keeps external references below local assembly holders. Inventor
        // reads nested placements from each sub-assembly's own file instead.
        .filter(|row| {
            format == Format::SolidWorksAssembly
                || row
                    .pointer("/parent/kind")
                    .and_then(serde_json::Value::as_str)
                    == Some("root")
        })
        .filter_map(|row| {
            row.pointer("/prototype/document/path")
                .and_then(serde_json::Value::as_str)
        })
        .filter(|stored| {
            let name = stored.rsplit(['/', '\\']).next().unwrap_or_default();
            matches!(
                Format::from_path(Path::new(name)),
                Some(
                    Format::InventorPart
                        | Format::InventorAssembly
                        | Format::SolidWorksPart
                        | Format::SolidWorksAssembly
                )
            )
        })
        .filter_map(|stored| {
            let found = if format == Format::SolidWorksAssembly {
                finder.sibling_first(stored, folder)
            } else {
                finder.locate(stored, folder)
            };
            found.ok().map(|found| found.path)
        })
        .collect();
    if format == Format::SolidWorksAssembly {
        // Local sub-assembly holders retain the source document's stem rather than an
        // external path. Match their neutral source names to assembly files too, so a
        // saved copy of a placed sub-assembly is not converted again as a project root.
        let definitions: std::collections::BTreeMap<_, _> = model
            .ir
            .model
            .product_definitions
            .iter()
            .map(|definition| (definition.id.as_str(), definition))
            .collect();
        for row in value.as_array().into_iter().flatten().take(MAX_PLACEMENTS) {
            if row
                .pointer("/parent/kind")
                .and_then(serde_json::Value::as_str)
                == Some("root")
            {
                continue;
            }
            let definition = row
                .pointer("/prototype/definition")
                .and_then(serde_json::Value::as_str)
                .and_then(|id| definitions.get(id))
                .filter(|definition| definition.bodies.is_empty());
            let Some(name) = definition.and_then(|definition| definition.source_name.as_deref())
            else {
                continue;
            };
            if let Ok(found) = finder.sibling_first(&format!("{name}.sldasm"), folder) {
                references.insert(found.path);
            }
        }
    }
    Ok(references.into_iter().collect())
}

/// Reads the full drawing graph without requiring a 3D body or filtering hidden bodies.
pub fn read_drawings(input: &Path, report: &mut Report) -> Result<Model, Error> {
    let format = Format::from_path(input).ok_or_else(|| Error::UnknownFormat {
        path: input.to_path_buf(),
    })?;
    if !format.can_convert() {
        return Err(Error::NotSupportedYet { format });
    }
    check_input(input)?;
    let bytes = std::fs::read(input).map_err(|source| Error::Io {
        path: input.to_path_buf(),
        source,
    })?;
    decode::drawings(format, input, &bytes, report)
}

/// [`read`], finding the files an assembly places as `search` says, when they are not at
/// their stored paths.
pub fn read_in(
    format: Format,
    input: &Path,
    search: &PartSearch,
    report: &mut Report,
) -> Result<Model, Error> {
    read_with_progress(format, input, search, report, None)
}

/// Reads with coarse progress, including each separate assembly part file.
pub fn read_with_progress(
    format: Format,
    input: &Path,
    search: &PartSearch,
    report: &mut Report,
    progress: Option<&Progress<'_>>,
) -> Result<Model, Error> {
    if !format.is_assembly() && format != Format::FusionMultiDocument {
        if let Some(progress) = progress {
            progress.designs_total(1);
        }
    }
    let read = std::sync::Mutex::new(std::collections::BTreeSet::new());
    let finder = locate::Finder::new(search);
    let mut model = assembly::read(
        format,
        input,
        report,
        &|format, path, report| {
            let model = decode_file(format, path, report, progress)?;
            // The codec owns Fusion archive members; do not count the whole archive as one.
            if format != Format::FusionMultiDocument {
                if let Some(progress) = progress {
                    let first = read.lock().is_ok_and(|mut read| {
                        read.insert(path.canonicalize().unwrap_or_else(|_| path.to_path_buf()))
                    });
                    if first {
                        progress.design_read();
                    }
                    if format.is_assembly() {
                        progress.stage(Stage::Input);
                    }
                }
            }
            Ok(model)
        },
        &finder,
        &mut Vec::new(),
    )?;
    for path in finder.paths() {
        names::saw_stored(&path, report);
    }
    if let Some(searched) = finder.searched_note() {
        report.push_at(Stage::Input, Severity::Info, "parts search", searched);
    }
    local_components(&mut model, finder.threads, search.part_positions, report);
    report.note_part_positions(search.part_positions);
    Ok(model)
}

/// Gathers the components a single file places itself into `model.assembly`, and records the
/// assembly's completeness.
fn local_components(
    model: &mut Model,
    threads: usize,
    positions: PartPositions,
    report: &mut Report,
) {
    if model.assembly.is_none() {
        match assembly::local(model, threads, positions, report) {
            assembly::Components::None => {}
            // The writer reports the assembly's joint notes: what it leaves out depends on
            // what it can build.
            // A STEP file's own product structure around one part placed once is no assembly to
            // build: the file goes to SolidCraft as it is.
            assembly::Components::Assembly(components)
                if model.source_step.is_some()
                    && components.parts.len() == 1
                    && components.placements.len() == 1
                    && components.joints.is_empty()
                    && components.left_out.is_empty() => {}
            assembly::Components::Assembly(components) => {
                model.assembly = Some(*components);
            }
            assembly::Components::Unplaceable(reason) => report.push(
                Severity::Degraded,
                "components",
                format!(
                    "the design places components, but {reason}; it converts as stored, so \
                     placements beyond the stored geometry are not shown"
                ),
            ),
        }
    }
    if let Some(assembly) = &model.assembly {
        report.set_completeness(assembly.completeness());
    }
}

/// The part of `model` holding only `bodies` (CADIR body ids) and what they are made of, named
/// `name`; why it cannot stand alone, else.
pub fn with_bodies(model: &Model, bodies: &[&str], name: String) -> Result<Model, String> {
    let document = serde_json::to_value(&model.ir).map_err(|err| err.to_string())?;
    assembly::geometry_only(model, &document, bodies, name)
}

/// Reads a design from its contents alone, without the file system: for a sandboxed
/// importer (such as a SolidCraft plugin) that is handed a file's bytes and nothing else.
/// `name` is the file's name; only its stem and extension are used. Parts an assembly keeps in
/// other files are not read: only what the file itself holds converts.
pub fn read_bytes(
    format: Format,
    name: &Path,
    bytes: &[u8],
    report: &mut Report,
) -> Result<Model, Error> {
    if !format.can_convert() {
        return Err(Error::NotSupportedYet { format });
    }
    if bytes.len() as u64 > MAX_INPUT_BYTES {
        return Err(Error::TooLarge {
            path: name.to_path_buf(),
            bytes: bytes.len() as u64,
            limit: MAX_INPUT_BYTES,
        });
    }
    report.saw(&name.display().to_string());
    names::saw_stored(&name.display().to_string(), report);
    let mut model = decode_bytes(format, name, bytes, report, None)?;
    local_components(&mut model, 1, PartPositions::Smart, report);
    report.note_part_positions(PartPositions::Smart);
    Ok(model)
}

/// Decodes one file, without reading the parts an assembly places.
fn decode_file(
    format: Format,
    input: &Path,
    report: &mut Report,
    progress: Option<&Progress<'_>>,
) -> Result<Model, Error> {
    if !format.can_convert() {
        return Err(Error::NotSupportedYet { format });
    }
    if let Some(progress) = progress {
        progress.stage(Stage::Input);
    }
    names::saw_path(input, report);
    check_input(input)?;
    let bytes = std::fs::read(input).map_err(|source| Error::Io {
        path: input.to_path_buf(),
        source,
    })?;
    decode_bytes(format, input, &bytes, report, progress)
}

/// Decodes one file's `bytes` (named `input`) into the neutral model, the way
/// [`decode_file`] does once it has read them.
fn decode_bytes(
    format: Format,
    input: &Path,
    bytes: &[u8],
    report: &mut Report,
    progress: Option<&Progress<'_>>,
) -> Result<Model, Error> {
    if let Some(progress) = progress {
        progress.stage(Stage::Decode);
    }
    let mut model = decode::decode_with_progress(format, input, bytes, report, progress)?;
    if matches!(format, Format::SolidWorksPart | Format::SolidWorksAssembly) {
        assembly::release_solidworks_display(&mut model, report);
    }
    names::saw_model(&model, report);
    if matches!(format, Format::FusionArchive | Format::FusionMultiDocument) {
        return Ok(hidden::fusion(model, report));
    }
    if format != Format::FreeCadDocument {
        return Ok(model);
    }
    match hidden::shown(&model) {
        Ok(None) => Ok(model),
        Ok(Some((shown, hidden))) => {
            if hidden > 0 {
                let body = if hidden == 1 { "body" } else { "bodies" };
                report.push(
                    Severity::Info,
                    "bodies",
                    format!("{hidden} hidden {body} not converted: FreeCAD hides them or a container above them"),
                );
            }
            Ok(shown)
        }
        Err(why) => {
            report.push(
                Severity::Degraded,
                "bodies",
                format!("hidden bodies converted too, as {why}"),
            );
            Ok(model)
        }
    }
}

/// Converts the file at `input` into `output` through `writer`, replacing an existing output.
///
/// The output file is only written once the whole conversion has succeeded.
pub fn convert(input: &Path, output: &Path, writer: &dyn Writer) -> Result<Report, Error> {
    convert_to(input, output, writer, true)
}

/// [`convert`], replacing an existing `output` only when `replace` is set; otherwise an
/// existing output is refused as [`Error::OutputExists`], before anything is read, and again
/// when writing if one appeared meanwhile.
pub fn convert_to(
    input: &Path,
    output: &Path,
    writer: &dyn Writer,
    replace: bool,
) -> Result<Report, Error> {
    convert_traced(
        input,
        output,
        writer,
        replace,
        &PartSearch::default(),
        &mut |_| {},
    )
}

/// [`convert_to`], finding an assembly's files as `search` says (see [`read_in`]), and telling
/// `on_stage` as each stage begins: decoding (with projection, which cadmpeg does as it
/// reads), then mapping and writing, then writing the file.
pub fn convert_traced(
    input: &Path,
    output: &Path,
    writer: &dyn Writer,
    replace: bool,
    search: &PartSearch,
    on_stage: &mut dyn FnMut(Stage),
) -> Result<Report, Error> {
    convert_impl(input, output, writer, replace, search, on_stage, None, None)
}

/// Which part of a design to convert alone (see [`parts`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PartSelection {
    /// The part's number, from 1.
    pub number: usize,
    /// For a sub-assembly: also the parts inside it.
    pub children: bool,
    /// Cut the part's history after this feature (see [`parts::stop_at_feature`]).
    pub stop_at_feature: Option<usize>,
}

/// [`convert_with_progress`] for one part of the design only, by its number. The whole
/// design is still read; only the part is mapped and written.
pub fn convert_part_with_progress(
    input: &Path,
    output: &Path,
    writer: &dyn Writer,
    replace: bool,
    search: &PartSearch,
    part: PartSelection,
    progress: &Progress<'_>,
) -> Result<Report, Error> {
    progress.run(|| {
        convert_impl(
            input,
            output,
            writer,
            replace,
            search,
            &mut |_| {},
            Some(progress),
            Some(part),
        )
    })
}

/// Converts with stage, document, and elapsed progress and a five-second heartbeat.
pub fn convert_with_progress(
    input: &Path,
    output: &Path,
    writer: &dyn Writer,
    replace: bool,
    search: &PartSearch,
    progress: &Progress<'_>,
) -> Result<Report, Error> {
    progress.run(|| {
        convert_impl(
            input,
            output,
            writer,
            replace,
            search,
            &mut |_| {},
            Some(progress),
            None,
        )
    })
}

#[allow(clippy::too_many_arguments)]
fn convert_impl(
    input: &Path,
    output: &Path,
    writer: &dyn Writer,
    replace: bool,
    search: &PartSearch,
    on_stage: &mut dyn FnMut(Stage),
    progress: Option<&Progress<'_>>,
    part: Option<PartSelection>,
) -> Result<Report, Error> {
    let mut stage = |stage| {
        on_stage(stage);
        if let Some(progress) = progress {
            progress.stage(stage);
        }
    };
    stage(Stage::Input);
    check_output(output, writer)?;
    if !replace && output.exists() {
        return Err(Error::OutputExists {
            path: output.to_path_buf(),
        });
    }
    let format = Format::from_path(input).ok_or_else(|| Error::UnknownFormat {
        path: input.to_path_buf(),
    })?;

    let mut report = Report::default();
    if progress.is_none() {
        stage(Stage::Decode);
    }
    let mut model = read_with_progress(format, input, search, &mut report, progress)?;
    if let Some(part) = part {
        let total = parts::catalog(&model).len();
        model = parts::select(&model, part.number, part.children)
            .map_err(|why| Error::Assembly(why.to_string()))?;
        if let Some(number) = part.stop_at_feature {
            let features = parts::history_length(&model);
            model = parts::stop_at_feature(&model, number)
                .map_err(|why| Error::Assembly(why.to_string()))?;
            report.push(
                Severity::Info,
                "part",
                format!("its history stops at feature {number} of {features}"),
            );
        }
        report.push(
            Severity::Info,
            "part",
            format!(
                "only part {} of {total} was converted ({})",
                part.number,
                if part.children {
                    "with the parts inside it"
                } else {
                    "alone"
                }
            ),
        );
    }

    if let Some(sink) = progress.and_then(|p| p.members.as_ref()) {
        sink.model(&model);
    }

    // Encode in memory so a failed conversion never leaves a partial file behind.
    let mut bytes = Vec::new();
    stage(Stage::Map);
    writer.write(&model, &mut bytes, &mut report)?;
    stage(Stage::Write);
    write_output(output, &bytes, replace)?;
    Ok(report)
}

/// Writes `bytes` to `output`; when `replace` is not set, only if no file is there.
fn write_output(output: &Path, bytes: &[u8], replace: bool) -> Result<(), Error> {
    use std::io::Write as _;
    let io = |source| Error::Io {
        path: output.to_path_buf(),
        source,
    };
    if replace {
        return std::fs::write(output, bytes).map_err(io);
    }
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output)
        .map_err(|source| match source.kind() {
            std::io::ErrorKind::AlreadyExists => Error::OutputExists {
                path: output.to_path_buf(),
            },
            _ => io(source),
        })?;
    file.write_all(bytes).map_err(io)
}

fn check_output(output: &Path, writer: &dyn Writer) -> Result<(), Error> {
    let expected = writer.extension();
    let matches = output
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| writer.accepts_extension(ext));
    if matches {
        Ok(())
    } else {
        Err(Error::WrongOutputExtension {
            path: output.to_path_buf(),
            expected,
        })
    }
}

fn check_input(input: &Path) -> Result<(), Error> {
    let meta = std::fs::metadata(input).map_err(|source| Error::Io {
        path: input.to_path_buf(),
        source,
    })?;
    if !meta.is_file() {
        return Err(Error::NotAFile {
            path: input.to_path_buf(),
        });
    }
    if meta.len() > MAX_INPUT_BYTES {
        return Err(Error::TooLarge {
            path: input.to_path_buf(),
            bytes: meta.len(),
            limit: MAX_INPUT_BYTES,
        });
    }
    Ok(())
}
