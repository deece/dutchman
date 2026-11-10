//! Assemblies: the parts and sub-assemblies an assembly places, read from their own files, and
//! their placements.
//!
//! cadmpeg reads an assembly's occurrences and leaves each external part unresolved, with the
//! path the assembly stored for it. That path is usually absolute on the machine that saved
//! it, so a part is looked for at that path, then by its file name next to the assembly. Each
//! part file is read once, however often it is placed.

mod freecad;
mod poses;
mod position_bounds;
mod position_skeleton;
mod position_smart;
mod sketch_instance;
mod slicing;
mod solidworks;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::locate::Finder;
#[cfg(test)]
use crate::locate::PartSearch;
use crate::placement::{compose, inverse, Placement3x4, IDENTITY};
use crate::{Completeness, Error, Format, LeftOutGroup, Model, Report, Severity, Stage};

/// The most part files one assembly may bring in.
pub const MAX_PARTS: usize = 1_000;

/// The most placements one assembly may hold.
pub const MAX_PLACEMENTS: usize = 100_000;

/// How deep sub-assemblies may nest.
pub const MAX_DEPTH: usize = 32;

/// How far from 1 an occurrence's scale may be and still be rigid.
const RIGID: f64 = 1e-9;

/// An assembly's parts and where they are placed.
#[derive(Clone, Debug)]
pub struct Assembly {
    /// The part holding the design's own loose bodies, when it has any, by index in `parts`.
    pub own_part: Option<usize>,
    /// Each part, read from its own file, in the order first placed. A sub-assembly is a part
    /// whose model has an assembly of its own.
    pub parts: Vec<Model>,
    /// The component definition (CADIR product definition id) each part was built from, by
    /// index in `parts`, when it names one; may be shorter than `parts`.
    pub definitions: Vec<Option<String>>,
    /// What each part is, by index in `parts`, when that can be told; may be shorter than
    /// `parts`. Two parts with the same identity, at any level of the assembly, come from one
    /// source with the same layout below it, so a writer may write them as one definition.
    pub identities: Vec<Option<String>>,
    /// Each placed part, in the assembly's order.
    pub placements: Vec<Placement>,
    /// Joints between its placements.
    pub joints: Vec<Joint>,
    /// The placements its file records that are not placed, and why.
    pub left_out: Vec<LeftOut>,
    /// The placements (paths, as [`Joint::a`]) held where they are in what holds them: a
    /// design's own root component, which Fusion holds to its document, or to the occurrence
    /// that inserts the design.
    pub held: Vec<Vec<usize>>,
    /// Why each joint the file records was left out or changed, for the writer to report: a
    /// writer that builds that joint some other way leaves its note out.
    pub joint_notes: Vec<JointNote>,
    /// Selected source placements before joint solving; empty when not captured.
    pub stored_placements: Vec<Placement3x4>,
}

/// Why a joint was left out or changed in [`Assembly::joints`].
#[derive(Clone, Debug, PartialEq)]
pub struct JointNote {
    /// What the report says (`joint 3: cylindrical joints are not converted yet`).
    pub text: String,
    /// The joint's kind in the file (`fixed`, `revolute`, `cylindrical`, `grounded`…), when
    /// the note is about one joint's kind.
    pub family: Option<String>,
}

/// A placement an assembly's file records that is not placed.
#[derive(Clone, Debug, PartialEq)]
pub struct LeftOut {
    /// The occurrence's name, when it has one.
    pub name: Option<String>,
    /// The file it places, by name, when it names one.
    pub file: Option<String>,
    /// Why it is left out.
    pub reason: String,
    /// The stage it was lost at.
    pub stage: Stage,
}

impl Assembly {
    fn remember_placements(&mut self) {
        if self.stored_placements.is_empty() {
            self.stored_placements = self.placements.iter().map(|p| p.transform).collect();
        }
        for part in &mut self.parts {
            if let Some(assembly) = &mut part.assembly {
                assembly.remember_placements();
            }
        }
    }

    /// Restore the source placements when a writer leaves joints out by choice.
    pub fn restore_stored_placements(&mut self) {
        for (placement, stored) in self.placements.iter_mut().zip(&self.stored_placements) {
            placement.transform = *stored;
        }
        for part in &mut self.parts {
            if let Some(assembly) = &mut part.assembly {
                assembly.restore_stored_placements();
            }
        }
    }

    /// How much of the assembly is placed: its own placements and, for each placed
    /// sub-assembly, the sub-assembly's, as often as it is placed.
    pub fn completeness(&self) -> Completeness {
        let mut groups: BTreeMap<(Option<String>, String, Stage), u64> = BTreeMap::new();
        let mut placed_empty = 0;
        let placed = self.count(1, &mut groups, &mut placed_empty, 0);
        let mut left_out: Vec<LeftOutGroup> = groups
            .into_iter()
            .map(|((file, reason, stage), count)| LeftOutGroup {
                file,
                reason,
                stage,
                count,
            })
            .collect();
        left_out.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.file.cmp(&b.file)));
        Completeness {
            placed,
            placed_empty,
            left_out,
        }
    }

    /// The placements placed in this assembly placed `times` times, adding those left out to
    /// `groups`.
    fn count(
        &self,
        times: u64,
        groups: &mut BTreeMap<(Option<String>, String, Stage), u64>,
        placed_empty: &mut u64,
        depth: usize,
    ) -> u64 {
        for left in &self.left_out {
            let count = groups
                .entry((left.file.clone(), left.reason.clone(), left.stage))
                .or_default();
            *count = count.saturating_add(times);
        }
        let mut placed = 0u64;
        let mut uses: BTreeMap<usize, u64> = BTreeMap::new();
        for (index, placement) in self.placements.iter().enumerate() {
            if let Err(kind) = crate::placement::rigid(&placement.transform) {
                let file = self
                    .parts
                    .get(placement.part)
                    .and_then(|p| p.source_file.clone());
                let reason = format!("Occurrence index {index} is left out: {kind}");
                let count = groups.entry((file, reason, Stage::Write)).or_default();
                *count = count.saturating_add(times);
                continue;
            }
            if self
                .parts
                .get(placement.part)
                .is_some_and(|part| part.assembly.is_none() && part.ir.model.bodies.is_empty())
            {
                *placed_empty = placed_empty.saturating_add(times);
            } else {
                placed = placed.saturating_add(times);
            }
            *uses.entry(placement.part).or_default() += 1;
        }
        if depth >= MAX_DEPTH {
            return placed;
        }
        for (part, used) in uses {
            if let Some(sub) = self.parts.get(part).and_then(|part| part.assembly.as_ref()) {
                placed = placed.saturating_add(sub.count(
                    times.saturating_mul(used),
                    groups,
                    placed_empty,
                    depth + 1,
                ));
            }
        }
        placed
    }
}

/// A joint between two placed parts. Its frames meet face to face, as two mated faces do:
/// their origins coincide and their z axes oppose, the second frame half a turn about the
/// first's x axis. Its parts are placed where the joint puts them,
/// `A · frame_a · motion · half turn about x = B · frame_b`, with A and B the sides'
/// placements in the world.
#[derive(Clone, Debug, PartialEq)]
pub struct Joint {
    /// The first side: the indices of the placements leading to it, from the top assembly's
    /// [`Assembly::placements`] down through each sub-assembly's; empty for the top assembly
    /// itself, which stays fixed.
    pub a: Vec<usize>,
    /// The second side, the same way.
    pub b: Vec<usize>,
    /// The joint's frame on the first side, in that side's own part.
    pub frame_a: crate::placement::Placement3x4,
    /// The joint's frame on the second side, in that side's own part.
    pub frame_b: crate::placement::Placement3x4,
    /// Whether the source suppresses it.
    pub suppressed: bool,
    /// How the second side moves against the first, and where it is now.
    pub motion: Motion,
    /// Whether the two frames meet with their z axes the same way (the source's frame sense
    /// `aligned`), rather than face to face as two snapped faces do.
    pub flip: bool,
}

/// How a joint lets its second side move, and where it is now.
#[derive(Clone, Debug, PartialEq)]
pub enum Motion {
    /// Not at all: the frames meet.
    Rigid,
    /// Turning about the first frame's z axis, now `angle` radians from where the frames meet,
    /// within `limits` (radians), when it has them. The source's angular offset is in the
    /// second frame; the angle is where the placements put the joint.
    Revolute {
        angle: f64,
        limits: Option<(f64, f64)>,
    },
    /// Sliding along the first frame's z axis, now `distance` mm from where the frames meet.
    Slider { distance: f64 },
}

impl Motion {
    /// The placement of the second frame in the first, where the joint is now.
    pub fn transform(&self) -> crate::placement::Placement3x4 {
        match *self {
            Motion::Rigid => IDENTITY,
            Motion::Revolute { angle, .. } => turn(angle),
            Motion::Slider { distance } => {
                let mut out = IDENTITY;
                out[2][3] = distance;
                out
            }
        }
    }
}

/// Whether a CADIR joint row's connector frames point the same way (its operands'
/// `frame_sense: "aligned"`); absent or `opposed`, they meet face to face.
pub fn frame_aligned(row: &Value) -> bool {
    row.pointer("/operands/frame_sense").and_then(Value::as_str) == Some("aligned")
}

/// Half a turn about x: how the second frame of a joint meets the first.
const FACING: Placement3x4 = [
    [1.0, 0.0, 0.0, 0.0],
    [0.0, -1.0, 0.0, 0.0],
    [0.0, 0.0, -1.0, 0.0],
];

/// How the second frame of a joint meets the first: face to face, or, flipped, with their z
/// axes the same way.
fn facing(flip: bool) -> Placement3x4 {
    if flip {
        IDENTITY
    } else {
        FACING
    }
}

impl Joint {
    /// The placement of the second frame in the first, where the joint is now.
    pub fn meeting(&self) -> Placement3x4 {
        compose(&self.motion.transform(), &facing(self.flip))
    }
}

/// A turn of `angle` radians about z.
fn turn(angle: f64) -> Placement3x4 {
    let (s, c) = angle.sin_cos();
    [[c, -s, 0.0, 0.0], [s, c, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0]]
}

/// One part placed in an assembly.
#[derive(Clone, Debug, PartialEq)]
pub struct Placement {
    /// The occurrence's name in the source, when it has one.
    pub name: Option<String>,
    /// Which of [`Assembly::parts`] it places.
    pub part: usize,
    /// Its placement in the assembly: a rigid transform, row-major 3 × 4, in millimetres.
    pub transform: [[f64; 4]; 3],
    /// Whether the source hides it: the occurrence is hidden, or every body of the part it
    /// places is. It is placed all the same, and the writer keeps it hidden.
    pub hidden: bool,
}

/// Whether every body of a part is one its source hides: a part of hidden bodies alone is
/// placed hidden. A part with any body shown is shown whole.
fn all_hidden(part: &Model) -> bool {
    let bodies = &part.ir.model.bodies;
    part.assembly.is_none()
        && !bodies.is_empty()
        && bodies.iter().all(|body| body.visible == Some(false))
}

/// Decodes one file, without reading the parts an assembly places.
pub(crate) type Decode<'a> = dyn Fn(Format, &Path, &mut Report) -> Result<Model, Error> + 'a;

/// Reads `input` with `decode` and, for an assembly, the parts and sub-assemblies it places,
/// found by `finder`, inside the assemblies whose files are in `open` (outermost first), so
/// an assembly that places itself, directly or not, is refused instead of read forever.
pub(crate) fn read(
    format: Format,
    input: &Path,
    report: &mut Report,
    decode: &Decode,
    finder: &Finder,
    open: &mut Vec<PathBuf>,
) -> Result<Model, Error> {
    read_cached(
        format,
        input,
        report,
        decode,
        finder,
        open,
        &mut SolidWorksReads::new(),
    )
}

/// Release saved display meshes after decode validation, before any conversion-side
/// serialization. SolidWorks conversion uses exact bodies and history instead.
pub(crate) fn release_solidworks_display(model: &mut Model, report: &mut Report) {
    // Preserve the names used to redact anonymous loss reports, without expanding
    // vertex/triangle arrays into a JSON value just to visit their metadata.
    for mesh in &model.ir.model.tessellations {
        if let Some(source) = &mesh.source_object {
            if let Some(name) = &source.name {
                crate::names::saw_stored(name, report);
            }
            for path in &source.instance_path {
                crate::names::saw_stored(path, report);
            }
        }
        for coordinates in mesh.texture_coordinates() {
            crate::names::saw_stored(coordinates.name.as_str(), report);
        }
    }
    model.ir.model.tessellations = Vec::new();
    model.ir.model.appearance_bindings.retain(|binding| {
        !matches!(
            binding.target,
            cadmpeg_ir::appearance::AppearanceTarget::Tessellation(_)
                | cadmpeg_ir::appearance::AppearanceTarget::TriangleGroup { .. }
        )
    });
}

// The SolidWorks reader supplies only the saved active configuration. All requests
// that fall back to it share this cache, across sibling assemblies too. Cache
// decoded models, not resolved trees: cycle and depth checks depend on the ancestors.
type SolidWorksReads = BTreeMap<PathBuf, Result<std::rc::Rc<Model>, (Stage, String)>>;

fn read_cached(
    format: Format,
    input: &Path,
    report: &mut Report,
    decode: &Decode,
    finder: &Finder,
    open: &mut Vec<PathBuf>,
    reads: &mut SolidWorksReads,
) -> Result<Model, Error> {
    let key = input.canonicalize().unwrap_or_else(|_| input.to_path_buf());
    let mut model = match reads.get(&key) {
        Some(Ok(source)) => source.as_ref().clone(),
        _ => decode(format, input, report)?,
    };
    if matches!(format, Format::SolidWorksPart | Format::SolidWorksAssembly) {
        release_solidworks_display(&mut model, report);
        reads
            .entry(key.clone())
            .or_insert_with(|| Ok(std::rc::Rc::new(model.clone())));
    }
    if format.is_assembly() || format == Format::FreeCadDocument {
        if open.contains(&key) {
            return Err(Error::Assembly(
                "the assembly places itself through its sub-assemblies".to_owned(),
            ));
        }
        if open.len() >= MAX_DEPTH {
            return Err(Error::Assembly(format!(
                "sub-assemblies nest deeper than Dutchman's limit of {MAX_DEPTH}"
            )));
        }
        open.push(key);
        let resolved = if format == Format::FreeCadDocument {
            freecad::resolve(&model, input, report, decode, finder, open)
        } else if format == Format::SolidWorksAssembly {
            solidworks::resolve(&model, input, report, decode, finder, open, reads)
        } else {
            resolve(&model, input, report, decode, finder, open).map(Some)
        };
        open.pop();
        model.assembly = resolved?;
    }
    Ok(model)
}

/// Reads the parts the assembly `model`, read from `input`, places, found by `finder`. A
/// placement whose part cannot be found or read is left out and reported; an assembly with
/// none left is an error.
pub(crate) fn resolve(
    model: &Model,
    input: &Path,
    report: &mut Report,
    decode: &Decode,
    finder: &Finder,
    open: &mut Vec<PathBuf>,
) -> Result<Assembly, Error> {
    let wire = serde_json::to_value(&model.ir.model)
        .map_err(|err| Error::Decode(format!("cannot read the assembly: {err}")))?;
    let mut occurrences: Vec<&Value> = wire
        .get("occurrences")
        .and_then(Value::as_array)
        .map(|rows| rows.iter().collect())
        .unwrap_or_default();
    if occurrences.len() > MAX_PLACEMENTS {
        return Err(Error::Decode(format!(
            "the assembly places {} parts, over Dutchman's limit of {MAX_PLACEMENTS}",
            occurrences.len()
        )));
    }
    occurrences.sort_by_key(|row| row.get("ordinal").and_then(Value::as_u64));

    let folder = input
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut parts: Vec<Model> = Vec::new();
    let mut identities = Vec::new();
    // Each file read, once: its part, or the stage and reason it could not be read.
    let mut by_file: BTreeMap<PathBuf, Result<usize, (Stage, String)>> = BTreeMap::new();
    let mut placements = Vec::new();
    let mut left_out: Vec<LeftOut> = Vec::new();
    let mut nested = 0usize;
    for occurrence in &occurrences {
        let name = occurrence
            .get("name")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let subject = format!("occurrence \"{}\"", name.as_deref().unwrap_or("unnamed"));
        if occurrence.pointer("/parent/kind").and_then(Value::as_str) != Some("root") {
            nested += 1;
            continue;
        }
        let Some(stored) = occurrence
            .pointer("/prototype/document/path")
            .and_then(Value::as_str)
        else {
            leave_out(
                report,
                &mut left_out,
                (&subject, &name, None),
                Stage::Input,
                "its part is not in a separate file Dutchman can find".to_owned(),
            );
            continue;
        };
        let file = file_name(stored);
        let format = match Format::from_path(Path::new(file)) {
            Some(format @ (Format::InventorPart | Format::InventorAssembly)) => format,
            _ => {
                leave_out(
                    report,
                    &mut left_out,
                    (&subject, &name, Some(file)),
                    Stage::Input,
                    format!("{file}: not an Inventor part or assembly"),
                );
                continue;
            }
        };
        let path = match finder.locate(stored, folder) {
            Ok(found) => {
                crate::names::saw_path(&found.path, report);
                if let Some((count, used)) = found.ambiguous {
                    crate::names::saw_stored(&used, report);
                    report.push_at(
                        Stage::Input,
                        Severity::Info,
                        file,
                        format!("{count} files in the project have this name; used \"{used}\""),
                    );
                }
                found.path
            }
            Err(missing) => {
                leave_out(
                    report,
                    &mut left_out,
                    (&subject, &name, Some(file)),
                    Stage::Input,
                    format!("{file}: {missing}"),
                );
                continue;
            }
        };
        let part = match by_file.get(&path) {
            Some(known) => known.clone(),
            None => {
                let read = if parts.len() >= MAX_PARTS {
                    Err((
                        Stage::Input,
                        format!("over Dutchman's limit of {MAX_PARTS} part files"),
                    ))
                } else {
                    let mut part_report = Report::default();
                    let result = read(format, &path, &mut part_report, decode, finder, open)
                        .map_err(|err| (err.stage(), err.to_string()));
                    report.append_prefixed(part_report, file);
                    result.map(|part| {
                        parts.push(part);
                        identities.push(Some(format!("file {}", path.display())));
                        parts.len() - 1
                    })
                };
                if let Err((stage, why)) = &read {
                    report.push_at(*stage, Severity::Dropped, file, why.clone());
                }
                by_file.insert(path, read.clone());
                read
            }
        };
        let part = match part {
            Ok(part) => part,
            Err((stage, why)) => {
                // The file's own loss says why; this one which placement it costs.
                report.push_at(
                    stage,
                    Severity::Dropped,
                    subject.clone(),
                    format!("{file} could not be read"),
                );
                left_out.push(LeftOut {
                    name: name.clone(),
                    file: Some(file.to_owned()),
                    reason: format!("{file} could not be read: {why}"),
                    stage,
                });
                continue;
            }
        };
        match transform(occurrence) {
            Ok(transform) => placements.push(Placement {
                hidden: false,
                name,
                part,
                transform,
            }),
            Err(why) => leave_out(
                report,
                &mut left_out,
                (&subject, &name, Some(file)),
                Stage::Map,
                why,
            ),
        }
    }
    if nested > 0 {
        report.push(
            Severity::Info,
            "assembly",
            format!(
                "{nested} occurrences inside sub-assemblies are read from the sub-assembly files"
            ),
        );
    }
    // An assembly that places nothing, as Inventor's own templates do, is empty: there is
    // nothing to search for, so it converts as an empty design.
    if occurrences.is_empty() {
        report.push(
            Severity::Info,
            "assembly",
            "the assembly places no parts: it is empty, as Inventor's assembly templates are",
        );
    } else if placements.is_empty() {
        return Err(Error::Assembly(
            "none of the assembly's parts could be found or read; if they are elsewhere, give \
             the project folder that holds them"
                .to_owned(),
        ));
    }
    Ok(Assembly {
        own_part: None,
        parts,
        placements,
        joints: Vec::new(),
        left_out,
        held: Vec::new(),
        joint_notes: Vec::new(),
        stored_placements: Vec::new(),
        identities,
        definitions: Vec::new(),
    })
}

/// Reports the placement `who` (its loss subject, its name and the file it places) left out
/// at `stage` for `reason`, and keeps it for the completeness count.
fn leave_out(
    report: &mut Report,
    left_out: &mut Vec<LeftOut>,
    (subject, name, file): (&str, &Option<String>, Option<&str>),
    stage: Stage,
    reason: String,
) {
    report.push_at(stage, Severity::Dropped, subject, reason.clone());
    left_out.push(LeftOut {
        name: name.clone(),
        file: file.map(str::to_owned),
        reason,
        stage,
    });
}

/// Arenas a component's own geometry never reaches into: other components' bodies are kept
/// out by not following `bodies`, and these hold structure and history, not geometry.
pub(crate) const NOT_GEOMETRY: [&str; 6] = [
    "bodies",
    "product_definitions",
    "occurrences",
    "assembly_joints",
    "features",
    "configurations",
];

/// What became of a design's own components.
pub(crate) enum Components {
    /// The design places no components of its own.
    None,
    /// They can be written as an assembly, which carries why each joint was left out.
    Assembly(Box<Assembly>),
    /// They cannot, for this reason; the design converts as stored.
    Unplaceable(String),
}

/// The components a design defines and places itself (as Fusion designs do), as an assembly
/// of part models each holding only its component's bodies, plus the design's loose bodies.
pub(crate) fn local(
    model: &Model,
    threads: usize,
    positions: crate::PartPositions,
    report: &mut Report,
) -> Components {
    match local_components_with_threads(model, &BTreeMap::new(), threads, positions, report) {
        Ok(Some((assembly, _))) => Components::Assembly(Box::new(assembly)),
        Ok(None) => Components::None,
        Err(reason) => Components::Unplaceable(reason),
    }
}

/// The components a design defines and places itself, as an assembly of part models each
/// holding only its component's bodies, plus the design's loose bodies; `None` when it places
/// no components of its own, `Err` with the reason when they cannot be placed.
pub fn local_assembly(model: &Model) -> Result<Option<Assembly>, String> {
    local_assembly_with_positions(model, crate::PartPositions::Smart)
}

/// Build a decoded model at a selected position policy, reusing its source CADIR.
pub fn local_assembly_with_positions(
    model: &Model,
    positions: crate::PartPositions,
) -> Result<Option<Assembly>, String> {
    Ok(local_components_with_threads(
        model,
        &BTreeMap::new(),
        1,
        positions,
        &mut Report::default(),
    )?
    .map(|(assembly, _)| assembly))
}

/// [`local_assembly`], with why each joint left out was.
#[cfg(test)]
fn local_components(model: &Model) -> Result<Option<(Assembly, Vec<String>)>, String> {
    local_components_with(model, &BTreeMap::new())
}

type ExternalParts = BTreeMap<String, Result<(usize, std::rc::Rc<Model>), LeftOut>>;

#[cfg(test)]
fn local_components_with(
    model: &Model,
    external: &ExternalParts,
) -> Result<Option<(Assembly, Vec<String>)>, String> {
    local_components_with_threads(
        model,
        external,
        1,
        crate::PartPositions::LastShown,
        &mut Report::default(),
    )
}

fn local_components_with_threads(
    model: &Model,
    external: &ExternalParts,
    threads: usize,
    positions: crate::PartPositions,
    report: &mut Report,
) -> Result<Option<(Assembly, Vec<String>)>, String> {
    let packed = serde_json::to_string(&model.ir).map_err(|err| err.to_string())?;
    let source = slicing::Packed::read(&packed)?;
    let mut document = source.document()?;
    let displayed = poses::select(model, &mut document, positions, report)?;
    let wire = document.get("model").unwrap_or(&Value::Null);
    let arena = |name: &str| {
        wire.get(name)
            .and_then(Value::as_array)
            .map_or(&[][..], Vec::as_slice)
    };
    // A design inserted into this one whose contents the file carries (as an .f3z carries
    // its other designs) is an external occurrence holding local ones: it is placed as a
    // sub-assembly of them.
    let all = arena("occurrences");
    let holds_local = |row: &Value| {
        let id = row.get("id").and_then(Value::as_str);
        id.is_some()
            && all.iter().any(|child| {
                scope(child) == Some("local")
                    && child.pointer("/parent/occurrence").and_then(Value::as_str) == id
            })
    };
    let mut occurrences: Vec<&Value> = all
        .iter()
        .filter(|row| match scope(row) {
            Some("local") => true,
            Some("external") => {
                holds_local(row)
                    || row
                        .get("id")
                        .and_then(Value::as_str)
                        .is_some_and(|id| external.contains_key(id))
            }
            _ => false,
        })
        .collect();
    if occurrences.is_empty() {
        return Ok(None);
    }
    if occurrences.len() > MAX_PLACEMENTS {
        return Err(format!("it places over {MAX_PLACEMENTS} components"));
    }
    occurrences.sort_by_key(|row| row.get("ordinal").and_then(Value::as_u64));

    let slices = slicing::Index::packed(&source, &document)?;

    let definitions: BTreeMap<&str, &Value> = arena("product_definitions")
        .iter()
        .filter_map(|row| Some((row.get("id")?.as_str()?, row)))
        .collect();
    let owned: Vec<&str> = definitions
        .values()
        .filter_map(|row| row.get("bodies").and_then(Value::as_array))
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    // Each occurrence's children, by the id of the occurrence they are placed in.
    let mut children: BTreeMap<&str, Vec<&Value>> = BTreeMap::new();
    let mut roots = Vec::new();
    for occurrence in &occurrences {
        match occurrence.pointer("/parent/kind").and_then(Value::as_str) {
            Some("root") => roots.push(*occurrence),
            _ => {
                let parent = occurrence
                    .pointer("/parent/occurrence")
                    .and_then(Value::as_str)
                    .ok_or("a component placement names no parent")?;
                children.entry(parent).or_default().push(*occurrence);
            }
        }
    }

    // Bodies no component owns are the design's own, placed where they are.
    let loose: Vec<&str> = arena("bodies")
        .iter()
        .filter_map(|row| row.get("id")?.as_str())
        .filter(|id| !owned.contains(id))
        .collect();
    let prepared = prepare_slices(
        model,
        &slices,
        &definitions,
        &occurrences,
        &loose,
        threads,
        report,
    );
    let mut layouts = Layouts::default();
    for occurrence in &occurrences {
        layouts.classify(occurrence, &children, external, 0);
    }
    let mut tree = Tree {
        prepared,
        external,
        external_built: BTreeMap::new(),
        left_out: Vec::new(),
        all_owned: loose.is_empty(),
        model,
        slices: &slices,
        definitions,
        children,
        layouts,
        variants: BTreeMap::new(),
        own_bodies: BTreeMap::new(),
        built: BTreeMap::new(),
        parts: Vec::new(),
        part_definitions: Vec::new(),
        part_identities: Vec::new(),
        placed: 0,
        open: Vec::new(),
        path: Vec::new(),
        paths: BTreeMap::new(),
    };
    let mut placements = tree.placements(&roots, None)?;
    let split: Vec<usize> = tree
        .variants
        .values()
        .map(Vec::len)
        .filter(|layouts| *layouts > 1)
        .collect();
    if !split.is_empty() {
        report.push(
            Severity::Info,
            "assembly",
            format!(
                "{} components written as {} definitions because their instances hold different layouts",
                split.len(),
                split.iter().sum::<usize>()
            ),
        );
    }
    if tree.placed != occurrences.len() {
        return Err(
            "some components are placed inside components the design does not place".to_owned(),
        );
    }
    let mut parts = std::mem::take(&mut tree.parts);
    let mut definitions = std::mem::take(&mut tree.part_definitions);
    let mut identities = std::mem::take(&mut tree.part_identities);
    let worlds = world_placements(&occurrences)?;
    let inserted = inserted_designs(&occurrences);
    let (joints, mut left_out) = joints(arena("assembly_joints"), &tree.paths, &worlds, &inserted);

    let own_part = if !loose.is_empty() {
        parts.push(tree.sliced(Slice::Loose, &loose, model.name.clone())?);
        definitions.push(None);
        identities.push(None);
        placements.push(Placement {
            hidden: parts.last().is_some_and(all_hidden),
            name: None,
            part: parts.len() - 1,
            transform: IDENTITY,
        });
        Some(parts.len() - 1)
    } else {
        None
    };
    // A design's root component is held by what holds the design: the document, or the
    // occurrence that inserts it. cadmpeg names its occurrence `root-…`.
    let mut held: Vec<Vec<usize>> = tree
        .paths
        .iter()
        .filter(|(id, _)| id.contains("occurrence#root-"))
        .map(|(_, located)| located.path.clone())
        .collect();
    held.sort();
    held.dedup();
    // Occurrences the file places at the pose it shows them in: their joints are checked
    // where they are, not solved again.
    let mut posed_ids = source_posed(&document);
    posed_ids.extend(displayed);
    let posed: BTreeSet<Vec<usize>> = tree
        .paths
        .iter()
        .filter(|(id, _)| posed_ids.contains(**id))
        .map(|(_, located)| located.path.clone())
        .collect();
    let mut assembly = Assembly {
        own_part,
        parts,
        placements,
        joints,
        left_out: tree.left_out,
        held,
        joint_notes: Vec::new(),
        stored_placements: Vec::new(),
        identities,
        definitions,
    };
    fn bindings(assembly: &Assembly, kept: &mut BTreeSet<String>) {
        for part in &assembly.parts {
            kept.extend(
                part.ir
                    .model
                    .appearance_bindings
                    .iter()
                    .map(|b| b.id.to_string()),
            );
            if let Some(sub) = &part.assembly {
                bindings(sub, kept);
            }
        }
    }
    let mut kept = BTreeSet::new();
    bindings(&assembly, &mut kept);
    for binding in &model.ir.model.appearance_bindings {
        if !kept.contains(binding.id.as_str()) {
            report.push_appearance(
                model.unplaced_appearance_cause(binding),
                model.unplaced_appearance_binding(binding),
            );
        }
    }
    assembly.remember_placements();
    left_out.extend(
        place_by_joints(&mut assembly, &posed)
            .into_iter()
            .map(|text| JointNote { text, family: None }),
    );
    let texts = left_out.iter().map(|note| note.text.clone()).collect();
    assembly.joint_notes = left_out;
    Ok(Some((assembly, texts)))
}

/// The occurrences placed at a source pose, which a Fusion design
/// stores for some occurrences beside their placements: an external occurrence with a
/// current pose or with poses of the occurrences inside it, and a local occurrence with a
/// current pose. When an inserted design has no current pose of its own, contained poses
/// also protect the child occurrence they name. Read from cadmpeg's Fusion records by the
/// occurrences' native references.
fn source_posed(document: &Value) -> BTreeSet<String> {
    let records = |name: &str| {
        document
            .pointer(&format!("/native/f3d/{name}"))
            .and_then(Value::as_array)
            .map_or(&[][..], Vec::as_slice)
    };
    let posed_refs: BTreeSet<&str> = records("xref_references")
        .iter()
        .chain(records("design_component_occurrences"))
        .filter(|row| {
            row.get("current_pose").is_some_and(|pose| !pose.is_null())
                || row
                    .get("contained_poses")
                    .and_then(Value::as_array)
                    .is_some_and(|poses| !poses.is_empty())
        })
        .filter_map(|row| row.get("id")?.as_str())
        .collect();
    let occurrences = document
        .pointer("/model/occurrences")
        .and_then(Value::as_array)
        .map_or(&[][..], Vec::as_slice);
    let mut posed: BTreeSet<String> = occurrences
        .iter()
        .filter(|row| {
            row.get("native_ref")
                .and_then(Value::as_str)
                .is_some_and(|native| posed_refs.contains(native))
        })
        .filter_map(|row| Some(row.get("id")?.as_str()?.to_owned()))
        .collect();
    // A contained pose names a child record in the referenced design, not just the
    // external occurrence that holds it. Preserve that child's decoded placement too.
    let carriers: BTreeMap<&str, u64> = records("design_component_occurrences")
        .iter()
        .filter_map(|row| Some((row.get("id")?.as_str()?, row.get("record_index")?.as_u64()?)))
        .collect();
    let references: BTreeMap<&str, &Value> = records("xref_references")
        .iter()
        .filter_map(|row| Some((row.get("id")?.as_str()?, row)))
        .collect();
    let by_id: BTreeMap<&str, &Value> = occurrences
        .iter()
        .filter_map(|row| Some((row.get("id")?.as_str()?, row)))
        .collect();
    let parent = |row: &Value| {
        row.pointer("/parent/occurrence")
            .and_then(Value::as_str)
            .and_then(|id| by_id.get(id).copied())
    };
    // Record numbers are local to a member. Resolve them only below the nearest
    // inserted occurrence, and refuse ambiguous targets rather than protecting a sibling.
    let mut targets: BTreeMap<(&str, u64), Vec<&Value>> = BTreeMap::new();
    for row in occurrences {
        let Some(record) = row
            .get("native_ref")
            .and_then(Value::as_str)
            .and_then(|id| carriers.get(id))
        else {
            continue;
        };
        let mut up = parent(row);
        for _ in 0..MAX_DEPTH {
            let Some(outer) = up else { break };
            if outer
                .get("native_ref")
                .and_then(Value::as_str)
                .is_some_and(|id| references.contains_key(id))
            {
                if let Some(id) = outer.get("id").and_then(Value::as_str) {
                    targets.entry((id, *record)).or_default().push(row);
                }
                break;
            }
            up = parent(outer);
        }
    }
    for ((instance, record), targets) in targets {
        let [target] = targets.as_slice() else {
            continue;
        };
        let Some(outer) = by_id.get(instance) else {
            continue;
        };
        let Some(reference) = outer
            .get("native_ref")
            .and_then(Value::as_str)
            .and_then(|id| references.get(id))
        else {
            continue;
        };
        // An explicit current pose places the inserted occurrence itself. Its
        // contained records do not independently pose the children; their own
        // current poses, if present, were collected above.
        if reference
            .get("current_pose")
            .is_some_and(|pose| !pose.is_null())
        {
            continue;
        }
        // A component-path pose applies only in the owner chain it names, from
        // the innermost owner outwards. Missing owner indices mean every instance.
        let mut owners = Vec::new();
        let mut up = parent(outer);
        for _ in 0..MAX_DEPTH {
            let Some(owner) = up else { break };
            let Some(index) = owner
                .get("native_ref")
                .and_then(Value::as_str)
                .filter(|id| !id.starts_with("f3d:xref/"))
                .and_then(|id| carriers.get(id))
            else {
                break;
            };
            owners.push(*index);
            up = parent(owner);
        }
        let matches = reference
            .get("contained_poses")
            .and_then(Value::as_array)
            .is_some_and(|poses| {
                poses.iter().any(|pose| {
                    pose.get("contained_record_index").and_then(Value::as_u64) == Some(record)
                        && pose
                            .get("owner_record_indices")
                            .and_then(Value::as_array)
                            .is_none_or(|path| {
                                path.len() <= owners.len()
                                    && path
                                        .iter()
                                        .zip(&owners)
                                        .all(|(a, b)| a.as_u64() == Some(*b))
                            })
                })
            });
        if matches {
            if let Some(id) = target.get("id").and_then(Value::as_str) {
                posed.insert(id.to_owned());
            }
        }
    }
    posed
}

/// Place joined parts from stored poses and constraints. Closing joints anchor
/// their parts in their common component before open joints solve. When a joint
/// group reaches no anchor, prefer a non-identity stored world pose over an
/// identity partner, then the outermost occurrence, with B first on a tie.
///
/// Identity organisers carry only connected children. A rejected organiser can
/// contain a smaller holder that should travel as a whole; an origin body can
/// also identify a holder when no outside joint independently places its children.
/// Internal joints solve before outer joints, and hold their parts relative to
/// their component so it can still travel. Explicit holder poses stay fixed;
/// isolated leaf pose markers within a posed component yield to closing groups.
/// Conflicting joints remain suppressed so the target solver preserves the result.
fn place_by_joints(assembly: &mut Assembly, posed: &BTreeSet<Vec<usize>>) -> Vec<String> {
    let mut joints = assembly.joints.clone();
    // An identity holder may need its placement from joints. A joint crossing its boundary
    // places the holder, not only the exact child named by the connector. Joint islands
    // must distinguish that holder from parts placed relative to it by internal joints.
    for joint in &mut joints {
        let common = joint
            .a
            .iter()
            .zip(&joint.b)
            .take_while(|(a, b)| a == b)
            .count();
        for path in [&mut joint.a, &mut joint.b] {
            let original = path.clone();
            for depth in common + 1..original.len() {
                let Some(candidate) = original.get(..depth) else {
                    continue;
                };
                let Some((_, parent)) = candidate.split_last() else {
                    continue;
                };
                let (Some(at), Some(above)) =
                    (world_at(assembly, candidate), world_at(assembly, parent))
                else {
                    continue;
                };
                if same(&compose(&inverse(&above), &at), &IDENTITY) {
                    *path = candidate.to_vec();
                    break;
                }
            }
        }
    }
    keep_holders_the_joints_agree_on(assembly, posed, &mut joints);
    let mut notes = Vec::new();
    // A placement is held relative to the common ancestor of the joint that placed
    // it. Internal joints therefore do not anchor their holder in the outer design.
    let mut fixed: BTreeMap<Vec<usize>, Vec<usize>> = BTreeMap::new();
    fixed.insert(Vec::new(), Vec::new());
    fixed.extend(posed.iter().cloned().map(|path| (path, Vec::new())));
    // A joint between two components holding parts the file poses, which already holds as the
    // file places them, shows both placed in the design: no other joint moves them.
    for (index, row) in assembly.joints.iter().enumerate() {
        let poses_inside = |path: &[usize]| posed.iter().any(|pose| pose.starts_with(path));
        if row.a.starts_with(&row.b)
            || row.b.starts_with(&row.a)
            || !poses_inside(&row.a)
            || !poses_inside(&row.b)
        {
            continue;
        }
        let (Some(a), Some(b)) = (world_at(assembly, &row.a), world_at(assembly, &row.b)) else {
            continue;
        };
        if as_built(&a, &b, row).is_some() {
            let holders = joints.get(index).map(|joint| [&joint.a, &joint.b]);
            for path in [&row.a, &row.b]
                .into_iter()
                .chain(holders.into_iter().flatten())
            {
                hold_in(&mut fixed, path.clone(), &[]);
            }
        }
    }
    let scopes: Vec<Vec<usize>> = joints
        .iter()
        .map(|joint| {
            joint
                .a
                .iter()
                .zip(&joint.b)
                .take_while(|(a, b)| a == b)
                .map(|(a, _)| *a)
                .collect()
        })
        .collect();
    let mut closing = BTreeMap::new();
    let mut pose_only = posed.clone();
    // A closing joint is placement evidence before any open joint can disturb it.
    // Keep that evidence in its local scope so an outer joint can still carry it.
    for ((exact, joint), scope) in assembly.joints.iter().zip(&joints).zip(&scopes) {
        if exact.suppressed {
            continue;
        }
        if let (Some(a), Some(b)) = (world_at(assembly, &exact.a), world_at(assembly, &exact.b)) {
            if as_built(&a, &b, exact).is_some() {
                for path in [&exact.a, &exact.b, &joint.a, &joint.b] {
                    hold_in(&mut fixed, path.clone(), scope);
                    hold_in(&mut closing, path.clone(), scope);
                    pose_only.remove(path);
                }
            }
        }
    }
    let prefer_stored: BTreeSet<Vec<usize>> = joints
        .iter()
        .flat_map(|joint| [(&joint.a, &joint.b), (&joint.b, &joint.a)])
        .filter_map(|(side, other)| {
            if side.is_empty() || other.is_empty() {
                return None;
            }
            let (at, partner) = (world_at(assembly, side)?, world_at(assembly, other)?);
            (!same(&at, &IDENTITY) && same(&partner, &IDENTITY)).then(|| side.clone())
        })
        .collect();
    let mut pending: Vec<usize> = (0..joints.len()).collect();
    while !pending.is_empty() {
        // Solve the component's joints before using an inner connector to place its
        // holder. Its connector must be measured after those internal joints solve.
        let deepest = pending
            .iter()
            .filter_map(|&i| scopes.get(i))
            .map(Vec::len)
            .max()
            .unwrap_or(0);
        let scope = pending
            .iter()
            .filter_map(|&i| scopes.get(i))
            .find(|scope| scope.len() == deepest)
            .cloned()
            .unwrap_or_default();
        let reaching = |fixed: &BTreeMap<Vec<usize>, Vec<usize>>| {
            pending.iter().position(|&i| {
                scopes.get(i) == Some(&scope)
                    && joints.get(i).is_some_and(|j| {
                        placed_in(fixed, &j.a, &scope) || placed_in(fixed, &j.b, &scope)
                    })
            })
        };
        if reaching(&fixed).is_none() {
            let outermost = pending
                .iter()
                .filter(|&&i| scopes.get(i) == Some(&scope))
                .filter_map(|&i| joints.get(i))
                .flat_map(|j| [&j.b, &j.a])
                .min_by_key(|path| (!prefer_stored.contains(path.as_slice()), path.len()));
            if let Some(outermost) = outermost {
                hold_in(&mut fixed, outermost.clone(), &scope);
            }
        }
        let next = reaching(&fixed).unwrap_or(0);
        let index = pending.remove(next);
        let Some(mut joint) = joints.get(index).cloned() else {
            continue;
        };
        // Internal joints may have refined the part's pose since island selection.
        // Resolve its connector in the holder's current coordinates before placing it.
        if let Some(exact) = assembly.joints.get(index) {
            for (path, frame, original, original_frame) in [
                (&joint.a, &mut joint.frame_a, &exact.a, &exact.frame_a),
                (&joint.b, &mut joint.frame_b, &exact.b, &exact.frame_b),
            ] {
                if let (Some(holder), Some(part)) =
                    (world_at(assembly, path), world_at(assembly, original))
                {
                    *frame = compose(&compose(&inverse(&holder), &part), original_frame);
                }
            }
        }
        let (Some(a), Some(b)) = (world_at(assembly, &joint.a), world_at(assembly, &joint.b))
        else {
            continue;
        };
        if let Some((motion, frame_b, note)) = as_built(&a, &b, &joint) {
            // The solver may use a holder; the exported joint still names the exact
            // part, so return the adjusted frame to that part's coordinates.
            let frame_b = assembly
                .joints
                .get(index)
                .and_then(|row| world_at(assembly, &row.b))
                .map_or(frame_b, |exact| {
                    compose(&compose(&inverse(&exact), &b), &frame_b)
                });
            if let Some(row) = assembly.joints.get_mut(index) {
                row.motion = motion;
                row.frame_b = frame_b;
            }
            if let Some(note) = note {
                notes.push(format!("joint {}: {note}", index + 1));
            }
            hold_in(&mut fixed, joint.a.clone(), &scope);
            hold_in(&mut fixed, joint.b.clone(), &scope);
            pose_only.remove(&joint.a);
            pose_only.remove(&joint.b);
            if let Some(exact) = assembly.joints.get(index) {
                hold_in(&mut fixed, exact.a.clone(), &scope);
                hold_in(&mut fixed, exact.b.clone(), &scope);
                pose_only.remove(&exact.a);
                pose_only.remove(&exact.b);
            }
            continue;
        }
        // A lone child pose marker is weaker evidence than a group whose joints
        // already close. Keep explicit holder poses, and poses confirmed by another
        // solved joint; only the isolated leaf may move to the closing group.
        for (side, partner) in [(&joint.a, &joint.b), (&joint.b, &joint.a)] {
            if partner != &scope
                && pose_only.contains(side)
                && posed
                    .iter()
                    .any(|outer| outer.len() < side.len() && side.starts_with(outer))
                && assembly_at(assembly, side).is_none()
                && closing
                    .get(partner)
                    .is_some_and(|anchor| scope.starts_with(anchor))
                && !placed_in(&closing, side, &scope)
            {
                fixed.remove(side);
                pose_only.remove(side);
            }
        }
        let reach = compose(&compose(&a, &joint.frame_a), &joint.meeting());
        // An occurrence holds the ones inside it, so moving it to meet one of them moves that
        // one too: of a joint between an occurrence and one inside it, the inner one moves.
        let inside = |outer: &[usize], inner: &[usize]| {
            outer.len() < inner.len() && inner.starts_with(outer)
        };
        let (a_holds_b, b_holds_a) = (inside(&joint.a, &joint.b), inside(&joint.b, &joint.a));
        if a_holds_b && !placed_in(&fixed, &joint.b, &scope) {
            hold_in(&mut fixed, joint.a.clone(), &scope);
        }
        let (a_fixed, b_fixed) = (
            placed_in(&fixed, &joint.a, &scope),
            placed_in(&fixed, &joint.b, &scope),
        );
        // Never move an occurrence that holds a placed one: check the joint instead.
        let (a_fixed, b_fixed) =
            if (a_fixed && !b_fixed && b_holds_a) || (b_fixed && !a_fixed && a_holds_b) {
                (true, true)
            } else {
                (a_fixed, b_fixed)
            };
        // A pending outer joint may place, from something placed already (a part joined to
        // the origin, say), the part this joint would move or a part inside it. That joint
        // places the part where it goes: this joint then only checks a part it would move, and
        // moves what holds the others without them.
        let moving = match (a_fixed, b_fixed) {
            (true, false) => Some(&joint.b),
            (false, _) => Some(&joint.a),
            (true, true) => None,
        };
        let placed_by_outer: Vec<Vec<usize>> = moving.map_or_else(Vec::new, |moving| {
            pending
                .iter()
                .filter_map(|&k| Some((joints.get(k)?, scopes.get(k)?)))
                .filter(|(_, outer)| outer.len() < scope.len())
                .flat_map(|(other, outer)| {
                    [(&other.a, &other.b), (&other.b, &other.a)]
                        .into_iter()
                        .filter(|(from, to)| {
                            placed_in(&fixed, from, outer)
                                && to.starts_with(moving)
                                && (posed.contains(to.as_slice()) || !placed_in(&fixed, to, outer))
                        })
                        .map(|(_, to)| to.clone())
                        .collect::<Vec<_>>()
                })
                .collect()
        });
        let (a_fixed, b_fixed) = if moving.is_some_and(|moving| placed_by_outer.contains(moving)) {
            (true, true)
        } else {
            (a_fixed, b_fixed)
        };
        match (a_fixed, b_fixed) {
            (true, true) => {
                let b_frame = compose(&b, &joint.frame_b);
                // SolidCraft solves the joints when it opens a design, so an active joint that
                // cannot hold its parts would move them away from where the file places them.
                // It is kept, suppressed, for the user to fix or turn back on.
                if !same(&b_frame, &reach) && !joint.suppressed {
                    if let Some(row) = assembly.joints.get_mut(index) {
                        row.suppressed = true;
                    }
                    notes.push(format!(
                        "joint {}: its parts are placed by other joints where it cannot hold them; \
                         it is kept suppressed, so they stay where the file places them",
                        index + 1
                    ));
                }
            }
            (true, false) => {
                move_keeping_placed(
                    assembly,
                    &fixed,
                    posed,
                    &placed_by_outer,
                    &joint.b,
                    &compose(&reach, &inverse(&joint.frame_b)),
                );
                hold_in(&mut fixed, joint.b.clone(), &scope);
            }
            (false, _) => {
                // A goes where B's frame, taken back through the joint, meets A's frame.
                let at = compose(
                    &compose(&compose(&b, &joint.frame_b), &inverse(&joint.meeting())),
                    &inverse(&joint.frame_a),
                );
                move_keeping_placed(assembly, &fixed, posed, &placed_by_outer, &joint.a, &at);
                hold_in(&mut fixed, joint.a.clone(), &scope);
                hold_in(&mut fixed, joint.b.clone(), &scope);
            }
        }
        // An outer joint also holds its exact connector part. Later internal joints
        // must solve from that part rather than move it away from the outer connector.
        if let Some(exact) = assembly.joints.get(index).filter(|row| !row.suppressed) {
            hold_in(&mut fixed, exact.a.clone(), &scope);
            hold_in(&mut fixed, exact.b.clone(), &scope);
            pose_only.remove(&exact.a);
            pose_only.remove(&exact.b);
        }
    }
    // A later outer joint can move a holder while preserving one of its children.
    // Recheck earlier internal joints at the final poses too: an active conflict would
    // otherwise move those parts again when the target solver opens the design.
    let conflicts: Vec<usize> = assembly
        .joints
        .iter()
        .enumerate()
        .filter_map(|(index, joint)| {
            if joint.suppressed {
                return None;
            }
            let (a, b) = (world_at(assembly, &joint.a)?, world_at(assembly, &joint.b)?);
            let reach = compose(&compose(&a, &joint.frame_a), &joint.meeting());
            (!same(&reach, &compose(&b, &joint.frame_b))).then_some(index)
        })
        .collect();
    for index in conflicts {
        if let Some(joint) = assembly.joints.get_mut(index) {
            joint.suppressed = true;
        }
        notes.push(format!(
            "joint {}: its parts are placed by other joints where it cannot hold them; \
             it is kept suppressed, so they stay where the file places them",
            index + 1
        ));
    }

    notes
}

/// Puts back the exact part a joint names where moving its identity holder would not be right.
/// A holder is placed as one piece, which carries the rest of what it holds with the part the
/// joint names, so it is moved only when internal joints or file poses bind that rest to the part, and when
/// the joints that reach into it agree on where it goes. Parts nothing binds stay where the
/// file puts them; and when each of many joints (a part to the origin, say) asks for a place
/// of its own, the holder is only an organiser of parts that each joint places, and moving it
/// for the first would carry the rest away from their joints.
fn keep_holders_the_joints_agree_on(
    assembly: &Assembly,
    posed: &BTreeSet<Vec<usize>>,
    joints: &mut [Joint],
) {
    let proposed = joints.to_vec();
    // For each holder, the places the joints that reach into it ask for it, per joint side.
    let mut asked: BTreeMap<Vec<usize>, Vec<(usize, bool, Placement3x4)>> = BTreeMap::new();
    let mut reverted: Vec<(usize, bool)> = Vec::new();
    for (index, joint) in joints.iter().enumerate() {
        let Some(exact) = assembly.joints.get(index) else {
            continue;
        };
        let (Some(wa), Some(wb)) = (world_at(assembly, &exact.a), world_at(assembly, &exact.b))
        else {
            continue;
        };
        for is_a in [true, false] {
            let (holder, part, frame) = if is_a {
                (&joint.a, &exact.a, &exact.frame_a)
            } else {
                (&joint.b, &exact.b, &exact.frame_b)
            };
            if holder == part {
                continue;
            }
            // Nothing binds the rest of a holder to the part the joint names, so the rest has
            // no reason to follow it: the joint places that part alone.
            // The file places both parts a joint names: it checks them and moves neither.
            if posed.contains(&exact.a) && posed.contains(&exact.b) {
                reverted.push((index, is_a));
                continue;
            }
            let bound = posed.iter().any(|path| path.starts_with(holder))
                || holder_is_bound(assembly, posed, holder, part);
            if !bound {
                reverted.push((index, is_a));
                continue;
            }
            let (Some(at), Some(held)) = (world_at(assembly, holder), world_at(assembly, part))
            else {
                continue;
            };
            let meet = if is_a {
                compose(
                    &compose(&compose(&wb, &exact.frame_b), &inverse(&exact.meeting())),
                    &inverse(frame),
                )
            } else {
                compose(
                    &compose(&compose(&wa, &exact.frame_a), &exact.meeting()),
                    &inverse(frame),
                )
            };
            let place = compose(&meet, &inverse(&compose(&inverse(&at), &held)));
            asked
                .entry(holder.clone())
                .or_default()
                .push((index, is_a, place));
        }
    }
    for requests in asked.values() {
        let Some((_, _, first)) = requests.first() else {
            continue;
        };
        if !requests.iter().all(|(_, _, place)| same(place, first)) {
            reverted.extend(requests.iter().map(|(index, is_a, _)| (*index, *is_a)));
        }
    }
    for (index, is_a) in reverted {
        let (Some(exact), Some(joint)) = (assembly.joints.get(index), joints.get_mut(index)) else {
            continue;
        };
        if is_a {
            joint.a.clone_from(&exact.a);
        } else {
            joint.b.clone_from(&exact.b);
        }
    }
    // A rejected outer organiser may still contain a smaller connected assembly.
    // Also admit a non-identity holder when its origin body is the only connector
    // through which outside joints reach it.
    for (index, joint) in joints.iter_mut().enumerate() {
        let (Some(exact), Some(proposed)) = (assembly.joints.get(index), proposed.get(index))
        else {
            continue;
        };
        if posed.contains(&exact.a) && posed.contains(&exact.b) {
            continue;
        }
        let common = exact
            .a
            .iter()
            .zip(&exact.b)
            .take_while(|(a, b)| a == b)
            .count();
        for (path, part, rejected) in [
            (&mut joint.a, &exact.a, &proposed.a),
            (&mut joint.b, &exact.b, &proposed.b),
        ] {
            if path != part {
                continue;
            }
            for depth in (common + 1..part.len()).rev() {
                let Some(holder) = part.get(..depth) else {
                    continue;
                };
                if holder == rejected
                    || assembly.joints.iter().any(|other| {
                        !other.suppressed
                            && ((other.a == holder && !other.b.starts_with(holder))
                                || (other.b == holder && !other.a.starts_with(holder)))
                    })
                {
                    continue;
                }
                if (holder_is_bound(assembly, posed, holder, part)
                    && holder_requests_agree(assembly, holder))
                    || holder_has_only_its_origin_connector(assembly, holder, part)
                {
                    *path = holder.to_vec();
                    break;
                }
            }
        }
    }
}

/// Rejected outer holders do not make a smaller holder safe automatically:
/// independently placed children must also ask for one common holder pose.
fn holder_requests_agree(assembly: &Assembly, holder: &[usize]) -> bool {
    let Some(at) = world_at(assembly, holder) else {
        return false;
    };
    let mut first = None;
    for joint in assembly.joints.iter().filter(|joint| !joint.suppressed) {
        let is_a = match (joint.a.starts_with(holder), joint.b.starts_with(holder)) {
            (true, false) => true,
            (false, true) => false,
            _ => continue,
        };
        let (Some(a), Some(b)) = (world_at(assembly, &joint.a), world_at(assembly, &joint.b))
        else {
            return false;
        };
        let (held, meet) = if is_a {
            (
                a,
                compose(
                    &compose(&compose(&b, &joint.frame_b), &inverse(&joint.meeting())),
                    &inverse(&joint.frame_a),
                ),
            )
        } else {
            (
                b,
                compose(
                    &compose(&compose(&a, &joint.frame_a), &joint.meeting()),
                    &inverse(&joint.frame_b),
                ),
            )
        };
        let place = compose(&meet, &inverse(&compose(&inverse(&at), &held)));
        let reference = first.get_or_insert(place);
        if !same(reference, &place) {
            return false;
        }
    }
    true
}

/// A holder's children must all belong to the joined group. An unrelated internal
/// joint does not bind the rest of an organiser to the connector part.
fn holder_is_bound(
    assembly: &Assembly,
    posed: &BTreeSet<Vec<usize>>,
    holder: &[usize],
    part: &[usize],
) -> bool {
    let Some(inner) = assembly_at(assembly, holder) else {
        return false;
    };
    let Some(&start) = part.get(holder.len()) else {
        return false;
    };
    let mut links: BTreeMap<usize, BTreeSet<usize>> = BTreeMap::new();
    for joint in assembly.joints.iter().filter(|joint| !joint.suppressed) {
        if !joint.a.starts_with(holder) || !joint.b.starts_with(holder) {
            continue;
        }
        let a = joint.a.get(holder.len()).copied().unwrap_or(usize::MAX);
        let b = joint.b.get(holder.len()).copied().unwrap_or(usize::MAX);
        links.entry(a).or_default().insert(b);
        links.entry(b).or_default().insert(a);
    }
    // Explicit child poses describe a layout relative to this holder.
    let posed_children: BTreeSet<usize> = posed
        .iter()
        .filter(|path| path.starts_with(holder))
        .filter_map(|path| path.get(holder.len()).copied())
        .collect();
    for child in posed_children {
        links.entry(child).or_default().insert(usize::MAX);
        links.entry(usize::MAX).or_default().insert(child);
    }
    if links.is_empty() {
        return false;
    }
    let mut reached = BTreeSet::from([start]);
    let mut pending = vec![start];
    while let Some(child) = pending.pop() {
        for &next in links.get(&child).into_iter().flatten() {
            if reached.insert(next) {
                pending.push(next);
            }
        }
    }
    (0..inner.placements.len()).all(|child| reached.contains(&child))
}

/// Bodies at the holder's origin represent its physical reference part. Carry
/// their holder only when other joints do not independently place its children.
fn holder_has_only_its_origin_connector(
    assembly: &Assembly,
    holder: &[usize],
    part: &[usize],
) -> bool {
    let Some((&child, parent)) = part.split_last() else {
        return false;
    };
    if parent != holder {
        return false;
    }
    let Some(inner) = assembly_at(assembly, holder) else {
        return false;
    };
    let Some(placement) = inner.placements.get(child) else {
        return false;
    };
    if !same(&placement.transform, &IDENTITY)
        || !inner
            .parts
            .get(placement.part)
            .is_some_and(|part| !part.ir.model.bodies.is_empty())
    {
        return false;
    }
    assembly
        .joints
        .iter()
        .filter(|joint| !joint.suppressed)
        .all(
            |joint| match (joint.a.starts_with(holder), joint.b.starts_with(holder)) {
                (true, false) => joint.a == part,
                (false, true) => joint.b == part,
                _ => true,
            },
        )
}

fn assembly_at<'a>(assembly: &'a Assembly, path: &[usize]) -> Option<&'a Assembly> {
    let mut level = assembly;
    for &index in path {
        level = level
            .parts
            .get(level.placements.get(index)?.part)?
            .assembly
            .as_ref()?;
    }
    Some(level)
}

/// Whether a part is held in this joint's common ancestor, rather than only
/// inside a component that the outer joint can still move.
fn placed_in(fixed: &BTreeMap<Vec<usize>, Vec<usize>>, path: &[usize], scope: &[usize]) -> bool {
    path == scope
        || fixed
            .get(path)
            .is_some_and(|anchor| scope.starts_with(anchor))
}

/// Keep the outermost frame in which a part has already been placed.
fn hold_in(fixed: &mut BTreeMap<Vec<usize>, Vec<usize>>, path: Vec<usize>, scope: &[usize]) {
    fixed
        .entry(path)
        .and_modify(|anchor| {
            if scope.len() < anchor.len() {
                *anchor = scope.to_vec();
            }
        })
        .or_insert_with(|| scope.to_vec());
}

/// A joint as it holds its parts where they are placed (`a` and `b`, in the world), when they
/// already turn or slide on its axis there: its frames' z axes parallel or opposed, and their
/// origins apart only along them (to a micrometre); for a fixed joint, its frames' origins
/// and x axes together. Fusion mates a joint's frames opposed or, flipped, aligned, and its
/// files do not say which, so placements that hold a joint either way keep it. Its value is measured
/// from the placements (a revolute's within a half turn either way), and B's frame
/// is taken from where A's meets it, so what else lies between the frames (a revolute's shift
/// along its axis, a slider's turn about it) is kept as built. A revolute's limits are left
/// out, with the reason, when the measured angle lies outside them.
fn as_built(
    a: &Placement3x4,
    b: &Placement3x4,
    joint: &Joint,
) -> Option<(Motion, Placement3x4, Option<String>)> {
    let wa = compose(a, &joint.frame_a);
    // B's frame in A's, taken back through the half turn: on the axis, a turn about z and a
    // shift along it, with z itself kept or reversed.
    let between = compose(
        &compose(&inverse(&wa), &compose(b, &joint.frame_b)),
        &inverse(&facing(joint.flip)),
    );
    let [[r00, _, _, x], [r10, _, _, y], [_, _, r22, z]] = between;
    if (r22.abs() - 1.0).abs() > 1e-9 || x.hypot(y) > 1e-3 {
        return None;
    }
    let mut note = None;
    let motion = match joint.motion {
        Motion::Rigid => {
            if (r00 - 1.0).abs() > 1e-9 || z.abs() > 1e-3 {
                return None;
            }
            Motion::Rigid
        }
        Motion::Revolute { limits, .. } => {
            let angle = r10.atan2(r00);
            let limits = match limits {
                Some((min, max)) if angle < min - 1e-9 || angle > max + 1e-9 => {
                    note = Some("its limits are left out: its angle lies outside them".to_owned());
                    None
                }
                limits => limits,
            };
            Motion::Revolute { angle, limits }
        }
        Motion::Slider { .. } => Motion::Slider { distance: z },
    };
    let meeting = compose(&motion.transform(), &facing(joint.flip));
    let frame_b = compose(&inverse(b), &compose(&wa, &meeting));
    Some((motion, frame_b, note))
}

/// Whether two placements agree, to a micrometre and a nanoradian.
fn same(a: &Placement3x4, b: &Placement3x4) -> bool {
    a.iter().zip(b).all(|(ra, rb)| {
        ra.iter()
            .zip(rb)
            .enumerate()
            .all(|(c, (x, y))| (x - y).abs() <= if c == 3 { 1e-3 } else { 1e-9 })
    })
}

/// The placement in the world of the occurrence a placement path leads to.
fn world_at(assembly: &Assembly, path: &[usize]) -> Option<Placement3x4> {
    let mut world = IDENTITY;
    let mut level = assembly;
    for (depth, index) in path.iter().enumerate() {
        let placement = level.placements.get(*index)?;
        world = compose(&world, &placement.transform);
        if depth + 1 < path.len() {
            level = level.parts.get(placement.part)?.assembly.as_ref()?;
        }
    }
    Some(world)
}

/// Moves the occurrence `path` leads to so that it is at `world`, keeping each occurrence
/// inside it that an outside joint placed already where that joint put it. Parts held by
/// internal joints travel with the component, and so do the ones inside it that the file
/// poses (`posed`): their stored pose is relative to the component that holds them, so it
/// moves them with it rather than leaving them where the component was.
fn move_keeping_placed(
    assembly: &mut Assembly,
    fixed: &BTreeMap<Vec<usize>, Vec<usize>>,
    posed: &BTreeSet<Vec<usize>>,
    stay: &[Vec<usize>],
    path: &[usize],
    world: &Placement3x4,
) {
    // Outer before inner, as the set orders a path before those that extend it.
    let inside: Vec<(&Vec<usize>, Placement3x4)> = fixed
        .iter()
        .filter(|(placed, scope)| {
            placed.len() > path.len()
                && placed.starts_with(path)
                && scope.len() < path.len()
                && !posed.contains(*placed)
        })
        .filter_map(|(placed, _)| Some((placed, world_at(assembly, placed)?)))
        .collect();
    let mut staying: Vec<(&Vec<usize>, Placement3x4)> = stay
        .iter()
        .filter(|held| held.len() > path.len() && held.starts_with(path))
        .filter_map(|held| Some((held, world_at(assembly, held)?)))
        .collect();
    // Outer before inner, so an inner part is put back relative to its outer part's new place.
    staying.extend(inside);
    staying.sort_by_key(|(held, _)| held.len());
    set_world_at(assembly, path, world);
    for (held, at) in staying {
        set_world_at(assembly, held, &at);
    }
}

/// Moves the occurrence a placement path leads to so that it is at `world`.
fn set_world_at(assembly: &mut Assembly, path: &[usize], world: &Placement3x4) {
    let Some((last, outer)) = path.split_last() else {
        return;
    };
    let Some(parent) = world_at(assembly, outer) else {
        return;
    };
    let mut level = assembly;
    for index in outer {
        let Some(part) = level.placements.get(*index).map(|p| p.part) else {
            return;
        };
        let Some(next) = level
            .parts
            .get_mut(part)
            .and_then(|part| part.assembly.as_mut())
        else {
            return;
        };
        level = next;
    }
    if let Some(placement) = level.placements.get_mut(*last) {
        placement.transform = compose(&inverse(&parent), world);
    }
}

/// Each occurrence's placement in the world: its own, after those of the occurrences it is
/// placed in.
fn world_placements<'a>(
    occurrences: &[&'a Value],
) -> Result<BTreeMap<&'a str, Placement3x4>, String> {
    let by_id: BTreeMap<&str, &Value> = occurrences
        .iter()
        .filter_map(|row| Some((row.get("id")?.as_str()?, *row)))
        .collect();
    let mut out = BTreeMap::new();
    for (id, row) in &by_id {
        let mut world = transform(row)?;
        let mut parent = row.pointer("/parent/occurrence").and_then(Value::as_str);
        let mut depth = 0;
        while let Some(up) = parent {
            depth += 1;
            let row = by_id.get(up).filter(|_| depth <= MAX_DEPTH);
            let Some(row) = row else {
                break;
            };
            world = compose(&transform(row)?, &world);
            parent = row.pointer("/parent/occurrence").and_then(Value::as_str);
        }
        out.insert(*id, world);
    }
    Ok(out)
}

/// The joints that can be written between placed parts, and a note for each that cannot.
/// `paths` gives where each occurrence is placed; `worlds`, its placement in the world as
/// cadmpeg stores it.
fn joints(
    rows: &[Value],
    paths: &BTreeMap<&str, Located>,
    worlds: &BTreeMap<&str, Placement3x4>,
    inserted: &BTreeMap<&str, &str>,
) -> (Vec<Joint>, Vec<JointNote>) {
    let mut joints: Vec<Joint> = Vec::new();
    // Whether each joint is in a copy of a component placed more than once, other than the
    // first.
    let mut copied: Vec<bool> = Vec::new();
    let mut notes = Vec::new();
    let resolved: Vec<_> = rows
        .iter()
        .map(|row| joint(row, paths, worlds, inserted))
        .collect();
    let lifted = |changed: &Option<String>| {
        changed
            .as_deref()
            .is_some_and(|text| text.starts_with(WHOLE_COPY))
    };
    // The joints that join two occurrences as they are, not through a whole copy.
    let direct: Vec<(usize, Vec<usize>, Vec<usize>)> = resolved
        .iter()
        .enumerate()
        .filter_map(|(index, result)| match result {
            Ok((joint, changed, _)) if !lifted(changed) => {
                Some((index, joint.a.clone(), joint.b.clone()))
            }
            _ => None,
        })
        .collect();
    let joined_directly = |joint: &Joint| {
        direct
            .iter()
            .find(|(_, a, b)| (*a == joint.a && *b == joint.b) || (*a == joint.b && *b == joint.a))
    };
    for (index, (row, result)) in rows.iter().zip(resolved).enumerate() {
        let family = family(row);
        let mut note = |text: String| {
            notes.push(JointNote {
                text: format!("joint {}: {text}", index + 1),
                family: family.clone(),
            });
        };
        match result {
            // A joint from a part inside a copy would hold the whole copy; when another joint
            // already joins that copy itself to the same occurrence, it holds the copy, and
            // the part is where the copy puts it.
            Ok((joint, changed, _)) if lifted(&changed) && joined_directly(&joint).is_some() => {
                let holder = joined_directly(&joint).map_or(0, |(other, _, _)| other + 1);
                note(format!(
                    "it is left out: the copy of the component placed more than once that its \
                     part is in is held by joint {holder} itself"
                ));
            }
            Ok((joint, changed, copy)) => {
                // A component's own joint is stored again in each copy of the component; the
                // copies hold one component, so they are one joint of it.
                let again = joints.iter().zip(&copied).any(|(other, other_copy)| {
                    (copy || *other_copy) && other.a == joint.a && other.b == joint.b
                });
                if again {
                    continue;
                }
                if let Some(changed) = changed {
                    note(changed);
                }
                joints.push(joint);
                copied.push(copy);
            }
            Err(reason) => note(reason),
        }
    }
    (joints, notes)
}

/// A joint row's kind: `grounded` for a joint to the ground, else its pair kind.
fn family(row: &Value) -> Option<String> {
    let operands = row.get("operands")?;
    match operands.get("arity").and_then(Value::as_str) {
        Some("grounded") => Some("grounded".to_owned()),
        _ => operands
            .pointer("/kind/kind")
            .and_then(Value::as_str)
            .map(str::to_owned),
    }
}

/// What a joint holds when it joins a part inside one copy of a component placed more than
/// once to something outside that copy.
const WHOLE_COPY: &str =
    "it holds the whole copy of the component placed more than once that its part is in";

/// One joint row as a joint, with a note when it is changed, and whether it is in a copy of a
/// component placed more than once other than the first.
fn joint(
    row: &Value,
    paths: &BTreeMap<&str, Located>,
    worlds: &BTreeMap<&str, Placement3x4>,
    inserted: &BTreeMap<&str, &str>,
) -> Result<(Joint, Option<String>, bool), String> {
    let operands = row.get("operands").unwrap_or(&Value::Null);
    if operands.get("arity").and_then(Value::as_str) == Some("grounded") {
        let operand = operands
            .pointer("/connector/operand")
            .ok_or("it is malformed")?;
        let (located, _, world) = side_path(operand, paths, worlds, inserted)?;
        if located.path.is_empty() {
            return Err("it joins a component to itself".to_owned());
        }
        // Grounding one copy would ground every copy of the component that holds it.
        if located.shared.is_some() {
            return Err("it grounds a component inside one placed more than once".to_owned());
        }
        // Grounding fixes the occurrence where it is stored. Use the same rigid joint to
        // the design as a paired joint to the root, with frames that preserve its placement.
        return Ok((
            Joint {
                a: Vec::new(),
                b: located.path,
                frame_a: world,
                frame_b: FACING,
                flip: false,
                suppressed: row
                    .get("suppressed")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                motion: Motion::Rigid,
            },
            None,
            false,
        ));
    }
    if operands.get("arity").and_then(Value::as_str) != Some("pair") {
        return Err("its operands are not supported yet".to_owned());
    }
    let kind = operands.get("kind").unwrap_or(&Value::Null);
    let side = |index: usize| -> Result<(Located, Placement3x4, Placement3x4), String> {
        let connector = operands
            .pointer(&format!("/connectors/{index}"))
            .ok_or("it is malformed")?;
        let frame = connector
            .get("frame")
            .and_then(matrix)
            .ok_or("its connector frames are malformed")?;
        let (located, container, side) = side_path(
            connector.get("operand").unwrap_or(&Value::Null),
            paths,
            worlds,
            inserted,
        )?;
        // The frame is in its operand's own container; the side is the deepest occurrence
        // the operand names, inside that container. Both are kept in the world.
        Ok((located, compose(&container, &frame), side))
    };
    let ((mut a, a_frame, mut a_at), (mut b, b_frame, mut b_at)) = (side(0)?, side(1)?);
    // A joint inside one copy of a component placed more than once is that component's own,
    // written through its first copy. A joint from inside a copy to something outside it
    // cannot hold that part alone, as every copy holds it: it holds the whole copy, which
    // its part is fixed in, where the part is.
    let mut lifted = false;
    let whole_copy = |side: &Located, other: &Located| {
        let depth = side.shared?;
        if side.own.get(..depth) == other.own.get(..depth) {
            return None;
        }
        let own = side.own.get(..depth)?;
        paths
            .iter()
            .find(|(_, holder)| holder.own == own)
            .map(|(id, holder)| (holder.clone(), worlds.get(id).copied().unwrap_or(IDENTITY)))
    };
    if let Some((holder, at)) = whole_copy(&a, &b) {
        (a, a_at, lifted) = (holder, at, true);
    }
    if let Some((holder, at)) = whole_copy(&b, &a) {
        (b, b_at, lifted) = (holder, at, true);
    }
    // A joint between a copy and a part inside it is the component's own, as its part is:
    // the copy is written through the first copy too.
    let holder = |copy: &Located, part: &Located| {
        let depth = part.shared?;
        (copy.own.len() == depth && part.own.starts_with(&copy.own))
            .then(|| part.path.get(..depth).map(<[usize]>::to_vec))
            .flatten()
    };
    if let Some(path) = holder(&a, &b) {
        a.path = path;
    } else if let Some(path) = holder(&b, &a) {
        b.path = path;
    }
    for depth in [a.shared, b.shared].into_iter().flatten() {
        if a.own.get(..depth) != b.own.get(..depth) {
            return Err(
                "it joins a component inside one placed more than once to one outside that copy"
                    .to_owned(),
            );
        }
    }
    let (frame_a, frame_b) = (
        compose(&inverse(&a_at), &a_frame),
        compose(&inverse(&b_at), &b_frame),
    );
    let copy = a.own != a.path || b.own != b.path;
    let (a, b) = (a.path, b.path);
    if a == b {
        return Err("it joins a component to itself".to_owned());
    }
    let number = |key: &str| kind.get(key).and_then(Value::as_f64).unwrap_or(0.0);
    let mut note = None;
    let flip = frame_aligned(row);
    let translation = kind
        .get("translation_offset")
        .and_then(Value::as_array)
        .map(|t| [0, 1, 2].map(|i| t.get(i).and_then(Value::as_f64).unwrap_or(0.0)))
        .unwrap_or([0.0; 3]);
    let offset = |angle| {
        let mut offset = turn(angle);
        for (row, value) in offset.iter_mut().zip(translation) {
            row[3] = value;
        }
        compose(&frame_b, &offset)
    };
    // CADIR closes C0 = C1 * T(offset) * Rz(angle) * facing. SolidCraft
    // closes A * motion * facing = B, so keep A and fold the source offset into B.
    // Placement and export consume these same frames.
    let (motion, frame_b) = match kind
        .get("kind")
        .and_then(Value::as_str)
        .unwrap_or("unknown")
    {
        "fixed" => (Motion::Rigid, offset(number("angle"))),
        "revolute" => {
            let limits = match kind.get("angular_limits") {
                None | Some(Value::Null) => None,
                Some(limits) => match limits.get("bounds").and_then(Value::as_str) {
                    Some("range") => {
                        let bound = |key: &str| limits.get(key).and_then(Value::as_f64);
                        match (bound("minimum"), bound("maximum")) {
                            (Some(min), Some(max)) => Some((min, max)),
                            _ => return Err("its limits are malformed".to_owned()),
                        }
                    }
                    _ => {
                        note = Some("its limit on one side only is left out".to_owned());
                        None
                    }
                },
            };
            // The source's angle aligns the frames by turning B's frame about z.
            // Further motion is measured from the placements once the parts are placed.
            (
                Motion::Revolute { angle: 0.0, limits },
                compose(&frame_b, &turn(number("angle"))),
            )
        }
        "slider" => (
            Motion::Slider {
                // C0 = C1 * Tz(d) * facing. Express B's motion along A's z:
                // the half turn reverses z for opposed frames, but aligned frames do not.
                distance: if flip {
                    -number("distance")
                } else {
                    number("distance")
                },
            },
            offset(0.0),
        ),
        other => return Err(format!("{other} joints are not converted yet")),
    };
    Ok((
        Joint {
            a,
            b,
            frame_a,
            frame_b,
            suppressed: row
                .get("suppressed")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            motion,
            flip,
        },
        match (lifted, note) {
            (true, Some(note)) => Some(format!("{WHOLE_COPY}; {note}")),
            (true, None) => Some(WHOLE_COPY.to_owned()),
            (false, note) => note,
        },
        copy,
    ))
}

/// A placement stored as rows, `[[r00, r01, r02, tx], …]`.
fn matrix(rows: &Value) -> Option<Placement3x4> {
    let rows = rows.as_array().filter(|rows| rows.len() == 3)?;
    let mut out = [[0.0; 4]; 3];
    for (row, values) in out.iter_mut().zip(rows) {
        let values = values.as_array().filter(|values| values.len() == 4)?;
        for (cell, value) in row.iter_mut().zip(values) {
            *cell = value.as_f64().filter(|v| v.is_finite())?;
        }
    }
    Some(out)
}

/// The values of a list, or none.
fn listed(value: Option<&Value>) -> &[Value] {
    value
        .and_then(Value::as_array)
        .map_or(&[][..], Vec::as_slice)
}

/// The placement path of one side of a joint (the design itself for its root, else the
/// deepest occurrence the operand names), with the world placements, as stored, of the
/// operand's container and of that occurrence.
fn side_path(
    operand: &Value,
    paths: &BTreeMap<&str, Located>,
    worlds: &BTreeMap<&str, Placement3x4>,
    inserted: &BTreeMap<&str, &str>,
) -> Result<(Located, Placement3x4, Placement3x4), String> {
    let container = operand.get("container").unwrap_or(&Value::Null);
    match container.get("container").and_then(Value::as_str) {
        Some("root") => Ok((Located::default(), IDENTITY, IDENTITY)),
        Some("occurrence") => {
            let id = container
                .get("occurrence")
                .and_then(Value::as_str)
                .ok_or("it is malformed")?;
            // Occurrences nested inside the named one are listed by their keys, outermost
            // first (as `occurrence_path`, or as the object's subelements): the deepest one
            // placed wins. Their ids are built the same way as its, or, inside a design
            // inserted into it, end in their key.
            let object = operand.get("object").and_then(Value::as_str).unwrap_or("");
            let prefix = id.strip_suffix(object).filter(|_| !object.is_empty());
            let below = paths.get(id).map(|located| located.own.as_slice());
            let inside = |key: &str| {
                let below = below?;
                let mut found = paths.iter().filter(|(nested, located)| {
                    nested.rsplit(['#', ':']).next() == Some(key)
                        && located.own.len() > below.len()
                        && located.own.starts_with(below)
                });
                match (found.next(), found.next()) {
                    (Some((nested, _)), None) => Some(*nested),
                    _ => None,
                }
            };
            let nested = listed(container.get("occurrence_path"))
                .iter()
                .chain(listed(operand.get("subelements")))
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .filter_map(Value::as_str)
                .find_map(|key| {
                    prefix
                        .and_then(|prefix| {
                            let nested = format!("{prefix}{key}");
                            paths.get_key_value(nested.as_str()).map(|(id, _)| *id)
                        })
                        .or_else(|| inside(key))
                });
            let named = nested.unwrap_or(id);
            let located = paths
                .get(named)
                .ok_or("it joins a component the design does not place")?;
            let world = |id: &str| worlds.get(id).copied().unwrap_or(IDENTITY);
            Ok((located.clone(), world(id), world(named)))
        }
        Some("external") => {
            // A component inside a design inserted into this one, whose contents the file
            // carries: the operand names the occurrences inside it by their keys, outermost
            // first. Its frame is in the deepest of them, and the joint holds the occurrence
            // that inserts the design: the inserted design's own joints are not carried, so
            // its parts move as one.
            let named = listed(operand.get("subelements"))
                .iter()
                .rev()
                .filter_map(Value::as_str)
                .find_map(|key| {
                    let mut found = inserted
                        .keys()
                        .filter(|id| id.rsplit_once('#').map(|(_, own)| own) == Some(key));
                    match (found.next(), found.next()) {
                        (Some(id), None) => Some(*id),
                        _ => None,
                    }
                })
                .ok_or("it joins a component of another design")?;
            let holder = inserted.get(named).copied().unwrap_or(named);
            let located = paths
                .get(holder)
                .ok_or("it joins a component the design does not place")?;
            let world = |id: &str| worlds.get(id).copied().unwrap_or(IDENTITY);
            Ok((located.clone(), world(named), world(holder)))
        }
        _ => Err("it is malformed".to_owned()),
    }
}

/// A definition's layout built into a part: the part, and the placement path of the first
/// occurrence it was built for. Every other occurrence at that level with the same layout
/// (see [`Layouts`]) is a copy of it.
type Built = (usize, Vec<usize>);

/// Two local transforms closer than this in every cell are the same placement: it absorbs the
/// noise of composing nested poses without merging placements anyone could tell apart.
const LAYOUT_TOLERANCE: f64 = 1e-7;

/// What a layout holds: the layout and local placement of each child.
type Held = Vec<(usize, [[f64; 4]; 3])>;

/// The layouts occurrences of one component hold. Two occurrences of a component share a layout
/// when they hold the same components (compared deepest first, so a difference anywhere below
/// separates them) at the same local placements, within [`LAYOUT_TOLERANCE`].
#[derive(Default)]
struct Layouts<'a> {
    /// Each layout: its component and what it holds, as the layout and placement of each child.
    table: Vec<(String, Held)>,
    /// The layouts of each component, by component.
    of_component: BTreeMap<String, Vec<usize>>,
    /// The layout of each occurrence, by its id.
    of: BTreeMap<&'a str, usize>,
}

impl<'a> Layouts<'a> {
    fn same(a: &[(usize, [[f64; 4]; 3])], b: &[(usize, [[f64; 4]; 3])]) -> bool {
        a.len() == b.len()
            && a.iter().zip(b).all(|((layout, t), (other, u))| {
                layout == other
                    && t.iter()
                        .flatten()
                        .zip(u.iter().flatten())
                        .all(|(x, y)| (x - y).abs() <= LAYOUT_TOLERANCE)
            })
    }

    /// The layout of `occurrence`, and of everything below it.
    fn classify(
        &mut self,
        occurrence: &'a Value,
        children: &BTreeMap<&'a str, Vec<&'a Value>>,
        external: &ExternalParts,
        depth: usize,
    ) -> usize {
        let id = occurrence.get("id").and_then(Value::as_str);
        if let Some(layout) = id.and_then(|id| self.of.get(id)) {
            return *layout;
        }
        let component = match id.and_then(|id| external.get(id)) {
            Some(Ok((key, _))) => format!("external:{key}"),
            Some(Err(_)) => definition_key(occurrence).unwrap_or("").to_owned(),
            None => occurrence
                .pointer("/prototype/definition")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned(),
        };
        let held: Held = if depth > MAX_DEPTH {
            // Too deep to compare: a layout of its own, which the build refuses.
            vec![(usize::MAX, [[f64::NAN; 4]; 3])]
        } else {
            id.and_then(|id| children.get(id))
                .into_iter()
                .flatten()
                .map(|child| {
                    (
                        self.classify(child, children, external, depth + 1),
                        // A malformed placement never matches; the build reports it.
                        transform(child).unwrap_or([[f64::NAN; 4]; 3]),
                    )
                })
                .collect()
        };
        let known = self.of_component.get(&component).and_then(|layouts| {
            layouts.iter().copied().find(|layout| {
                self.table
                    .get(*layout)
                    .is_some_and(|(_, other)| Self::same(other, &held))
            })
        });
        let layout = known.unwrap_or_else(|| {
            self.table.push((component.clone(), held));
            let layout = self.table.len() - 1;
            self.of_component.entry(component).or_default().push(layout);
            layout
        });
        if let Some(id) = id {
            self.of.insert(id, layout);
        }
        layout
    }
}

/// Where an occurrence is placed.
#[derive(Clone, Debug, Default, PartialEq)]
struct Located {
    /// Its placement path, through the first copy of each component placed more than once that
    /// holds it: the copy a joint inside such a component is written through.
    path: Vec<usize>,
    /// Its own placement path.
    own: Vec<usize>,
    /// The length of the path to the outermost component placed more than once that holds it,
    /// when one does.
    shared: Option<usize>,
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Slice<'a> {
    Empty,
    Definition(&'a str),
    Loose,
}

impl<'a> Slice<'a> {
    fn owner(self) -> HistoryOwner<'a> {
        match self {
            Self::Definition(id) => HistoryOwner::Definition(id),
            Self::Loose => HistoryOwner::Root,
            Self::Empty => HistoryOwner::Unknown,
        }
    }
}

/// The history scope of a component slice; an unknown scope uses body and sketch links only.
#[derive(Clone, Copy)]
pub(crate) enum HistoryOwner<'a> {
    Definition(&'a str),
    Root,
    Unknown,
}

fn prepare_slices<'a>(
    model: &Model,
    slices: &slicing::Index<'_>,
    definitions: &BTreeMap<&'a str, &'a Value>,
    occurrences: &[&'a Value],
    loose: &[&'a str],
    threads: usize,
    report: &mut Report,
) -> BTreeMap<Slice<'a>, Model> {
    if threads == 1 {
        report.set_threads(threads, 1);
        return BTreeMap::new();
    }
    // Empty holders share one empty IR. Only Model.name differs, assigned by the tree.
    let mut jobs = vec![(Slice::Empty, Vec::new())];
    let mut seen = BTreeSet::new();
    for &occurrence in occurrences {
        if scope(occurrence) != Some("local") {
            continue;
        }
        let Some(key) = definition_key(occurrence) else {
            continue;
        };
        if !seen.insert(key) {
            continue;
        }
        let Some(&row) = definitions.get(key) else {
            continue;
        };
        let bodies: Vec<&str> = row
            .get("bodies")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect();
        if !bodies.is_empty() {
            jobs.push((Slice::Definition(key), bodies));
        }
    }
    if !loose.is_empty() {
        jobs.push((Slice::Loose, loose.to_vec()));
    }
    let largest = jobs
        .iter()
        .map(|(_, bodies)| bodies.len())
        .max()
        .unwrap_or(0);
    // Slicing clones geometry and history arenas; a single dense body can dominate.
    let entities = crate::parts::entity_count(model);
    let entities_per_body = entities.div_ceil(model.ir.model.bodies.len().max(1) as u64);
    // The row index and compact source are shared; workers allocate only their slice.
    let estimate =
        crate::parallel::part_peak_bytes(entities_per_body.saturating_mul(largest as u64), largest);
    // The shared empty template is a helper, not another part worker slot.
    let workers = crate::parallel::workers(threads, jobs.len().saturating_sub(1), estimate);
    report.set_threads(threads, workers);
    if workers == 1 {
        return BTreeMap::new();
    }
    let prepared = crate::parallel::prepare_ordered(&jobs, workers, |(key, bodies)| {
        only_indexed(model, slices, bodies, key.owner(), String::new())
    });
    jobs.into_iter()
        .zip(prepared)
        .filter_map(|((key, _), part)| part.map(|part| (key, part)))
        .collect()
}

/// A design's occurrence tree, turned into parts and sub-assemblies one definition at a time.
struct Tree<'a> {
    prepared: BTreeMap<Slice<'a>, Model>,
    external: &'a ExternalParts,
    external_built: BTreeMap<usize, usize>,
    left_out: Vec<LeftOut>,
    model: &'a Model,
    slices: &'a slicing::Index<'a>,
    definitions: BTreeMap<&'a str, &'a Value>,
    /// Occurrences placed inside each occurrence, by its id.
    children: BTreeMap<&'a str, Vec<&'a Value>>,
    /// The layouts of the occurrences, to build one definition per layout of a component.
    layouts: Layouts<'a>,
    /// The layouts of each component built so far, in the order first built: a layout's place
    /// in it numbers its definition.
    variants: BTreeMap<&'a str, Vec<usize>>,
    /// The bodies of components with several layouts, sliced once for every definition of them.
    own_bodies: BTreeMap<&'a str, Model>,
    /// The definitions built so far at the level being built, by component and layout.
    built: BTreeMap<(&'a str, usize), Built>,
    /// The parts of the level being built.
    parts: Vec<Model>,
    /// The definition of each of `parts`.
    part_definitions: Vec<Option<String>>,
    /// The identity of each of `parts`, for [`Assembly::identities`].
    part_identities: Vec<Option<String>>,
    /// How many occurrences have been placed, to find those placed in nothing the design places.
    placed: usize,
    /// The occurrences being built, outermost first, to refuse a placement inside itself.
    open: Vec<&'a str>,
    /// The placement indices leading to the occurrence being built.
    path: Vec<usize>,
    /// Where each occurrence is placed.
    paths: BTreeMap<&'a str, Located>,
    /// Whether every body of the design belongs to a component, so that a component naming
    /// no bodies and holding none is empty rather than one cadmpeg could not attribute.
    all_owned: bool,
}

impl<'a> Tree<'a> {
    fn sliced(&mut self, key: Slice<'a>, bodies: &[&str], name: String) -> Result<Model, String> {
        let prepared = if bodies.is_empty() {
            self.prepared.get(&Slice::Empty).cloned()
        } else {
            self.prepared.remove(&key)
        };
        match prepared {
            Some(mut part) => {
                part.name = name;
                Ok(part)
            }
            None => only_indexed(self.model, self.slices, bodies, key.owner(), name),
        }
    }

    /// The placements of `occurrences` among `self.parts`; `shared`, the length of the path to
    /// the outermost component placed more than once that holds them, when one does.
    fn placements(
        &mut self,
        occurrences: &[&'a Value],
        shared: Option<usize>,
    ) -> Result<Vec<Placement>, String> {
        let definition = |row: &Value| definition_key(row).map(str::to_owned);
        let mut placements = Vec::with_capacity(occurrences.len());
        for occurrence in occurrences {
            let id = occurrence.get("id").and_then(Value::as_str).unwrap_or("");
            if let Some(Err(left)) = self.external.get(id) {
                self.left_out.push(left.clone());
                self.placed += 1;
                continue;
            }
            let index = placements.len();
            let layout = |row: &Value| {
                row.get("id")
                    .and_then(Value::as_str)
                    .and_then(|id| self.layouts.of.get(id))
                    .copied()
            };
            let copies = occurrences
                .iter()
                .filter(|other| {
                    definition(other) == definition(occurrence)
                        && layout(other) == layout(occurrence)
                })
                .count();
            self.path.push(index);
            if let Some(id) = occurrence.get("id").and_then(Value::as_str) {
                let located = Located {
                    path: self.path.clone(),
                    own: self.path.clone(),
                    shared,
                };
                self.paths.insert(id, located);
            }
            // A placement is its own occurrence however many copies sit beside it; what it
            // holds is shared between those copies.
            let inner = if copies > 1 {
                shared.or(Some(self.path.len()))
            } else {
                shared
            };
            let part = self.part(occurrence, inner);
            self.path.pop();
            self.placed += 1;
            let hidden = occurrence.get("visible").and_then(Value::as_bool) == Some(false)
                || part
                    .as_ref()
                    .is_ok_and(|part| self.parts.get(*part).is_some_and(all_hidden));
            placements.push(Placement {
                hidden,
                name: occurrence
                    .get("name")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                part: part?,
                transform: transform(occurrence)?,
            });
        }
        Ok(placements)
    }

    /// The identity of what occurrence `id` places: its layout, the same for every occurrence
    /// of one component that holds the same below it, at any level.
    fn layout_identity(&self, id: &str) -> Option<String> {
        self.layouts
            .of
            .get(id)
            .map(|layout| format!("layout {layout}"))
    }

    /// Counts and records the occurrences inside `id`, an occurrence of a definition already
    /// built at this level, whose own placements are that definition's: `first` is the path of
    /// the copy it matches, and `shared` as for [`Tree::placements`].
    fn reuse_inside(&mut self, id: &str, first: &[usize], shared: Option<usize>) {
        let inner: Vec<&'a Value> = self.children.get(id).cloned().unwrap_or_default();
        for (index, occurrence) in inner.iter().enumerate() {
            self.placed += 1;
            self.path.push(index);
            let mut path = first.to_vec();
            path.push(index);
            if let Some(id) = occurrence.get("id").and_then(Value::as_str) {
                let located = Located {
                    path: path.clone(),
                    own: self.path.clone(),
                    shared,
                };
                self.paths.insert(id, located);
                if self.path.len() <= MAX_DEPTH {
                    self.reuse_inside(id, &path, shared);
                }
            }
            self.path.pop();
        }
    }

    /// The part an occurrence places: its definition's bodies, and, when components are
    /// placed inside it, those as a sub-assembly beside them. `shared` is for the components
    /// inside it, as for [`Tree::placements`].
    fn part(&mut self, occurrence: &'a Value, shared: Option<usize>) -> Result<usize, String> {
        let id = occurrence.get("id").and_then(Value::as_str).unwrap_or("");
        if let Some(Ok((key, model))) = self.external.get(id) {
            if let Some(part) = self.external_built.get(key) {
                return Ok(*part);
            }
            self.parts.push(model.as_ref().clone());
            self.part_definitions.push(None);
            self.part_identities.push(self.layout_identity(id));
            let part = self.parts.len() - 1;
            self.external_built.insert(*key, part);
            return Ok(part);
        }
        let definition =
            definition_key(occurrence).ok_or("a component placement names no component")?;
        let id = occurrence.get("id").and_then(Value::as_str).unwrap_or("");
        let external = scope(occurrence) == Some("external");
        let inner: Vec<&'a Value> = self.children.get(id).cloned().unwrap_or_default();
        for child in &inner {
            transform(child)?;
        }
        let layout = self.layouts.of.get(id).copied().unwrap_or(usize::MAX);
        if let Some((part, first)) = self.built.get(&(definition, layout)) {
            let (part, first) = (*part, first.clone());
            self.reuse_inside(id, &first, shared);
            return Ok(part);
        }
        if self.open.contains(&id) || self.open.len() >= MAX_DEPTH {
            return Err("its components nest too deep or inside themselves".to_owned());
        }
        let empty = Value::Null;
        let row = if external {
            &empty
        } else {
            self.definitions
                .get(definition)
                .ok_or("a component placement names a component the design lacks")?
        };
        let bodies: Vec<&str> = row
            .get("bodies")
            .and_then(Value::as_array)
            .map(|bodies| bodies.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default();
        let inserted_name = if external {
            inserted_design_name(self.slices.document, occurrence)
        } else {
            None
        };
        let name = inserted_name
            .or_else(|| name_of(row))
            .map(str::to_owned)
            .unwrap_or_else(|| format!("{} component {}", self.model.name, self.parts.len() + 1));
        // A component whose occurrences hold different layouts is one definition per layout,
        // the first as the component is named and the others numbered.
        let seen = self.variants.entry(definition).or_default();
        let number = match seen.iter().position(|seen| *seen == layout) {
            Some(at) => at + 1,
            None => {
                seen.push(layout);
                seen.len()
            }
        };
        let name = if number > 1 {
            format!("{name} ({number})")
        } else {
            name
        };
        let model = if inner.is_empty() {
            if bodies.is_empty() && self.all_owned {
                self.sliced(Slice::Empty, &[], name)?
            } else if bodies.is_empty() {
                return Err(format!(
                    "cadmpeg does not say which bodies belong to component {}",
                    name_of(row).unwrap_or(definition)
                ));
            } else {
                self.sliced(Slice::Definition(definition), &bodies, name)?
            }
        } else {
            // A sub-assembly: the components inside it, and its own bodies at its origin.
            let outer_left_out = std::mem::take(&mut self.left_out);
            let outer = std::mem::take(&mut self.parts);
            let outer_definitions = std::mem::take(&mut self.part_definitions);
            let outer_identities = std::mem::take(&mut self.part_identities);
            let outer_built = std::mem::take(&mut self.built);
            let outer_external = std::mem::take(&mut self.external_built);
            self.open.push(id);
            let placed = self.placements(&inner, shared);
            self.open.pop();
            let mut placements = placed?;
            let own_part = if !bodies.is_empty() {
                // The component's own bodies are the same in every layout: sliced once.
                let shared_bodies = self
                    .layouts
                    .of_component
                    .get(definition)
                    .is_some_and(|layouts| layouts.len() > 1);
                let own = match self.own_bodies.get(definition) {
                    Some(cached) => {
                        let mut own = cached.clone();
                        own.name = format!("{name} bodies");
                        own
                    }
                    None => {
                        let own = self.sliced(
                            Slice::Definition(definition),
                            &bodies,
                            format!("{name} bodies"),
                        )?;
                        if shared_bodies {
                            self.own_bodies.insert(definition, own.clone());
                        }
                        own
                    }
                };
                let hidden = all_hidden(&own);
                self.parts.push(own);
                self.part_definitions.push(Some(definition.to_owned()));
                self.part_identities
                    .push(Some(format!("bodies of {definition}")));
                placements.push(Placement {
                    hidden,
                    name: None,
                    part: self.parts.len() - 1,
                    transform: IDENTITY,
                });
                Some(self.parts.len() - 1)
            } else {
                None
            };
            let parts = std::mem::replace(&mut self.parts, outer);
            let inner_definitions =
                std::mem::replace(&mut self.part_definitions, outer_definitions);
            let inner_identities = std::mem::replace(&mut self.part_identities, outer_identities);
            self.built = outer_built;
            self.external_built = outer_external;
            let mut holder = self.sliced(Slice::Empty, &[], name)?;
            holder.assembly = Some(Assembly {
                own_part,
                parts,
                placements,
                joints: Vec::new(),
                left_out: std::mem::replace(&mut self.left_out, outer_left_out),
                held: Vec::new(),
                joint_notes: Vec::new(),
                stored_placements: Vec::new(),
                identities: inner_identities,
                definitions: inner_definitions,
            });
            holder
        };
        self.parts.push(model);
        self.part_definitions.push(Some(definition.to_owned()));
        self.part_identities.push(self.layout_identity(id));
        let index = self.parts.len() - 1;
        self.built
            .insert((definition, layout), (index, self.path.clone()));
        Ok(index)
    }
}

/// Whether an occurrence places a component of this design (`local`) or another design
/// (`external`).
fn scope(occurrence: &Value) -> Option<&str> {
    occurrence
        .pointer("/prototype/scope")
        .and_then(Value::as_str)
}

/// What makes two placements copies of each other: the component they place, or for a
/// design inserted into this one, the occurrence itself.
fn definition_key(occurrence: &Value) -> Option<&str> {
    match scope(occurrence) {
        Some("external") => occurrence.get("id").and_then(Value::as_str),
        _ => occurrence
            .pointer("/prototype/definition")
            .and_then(Value::as_str),
    }
}

/// Each occurrence inside a design inserted into this one, with the occurrence that inserts
/// that design (the nearest external occurrence above it).
fn inserted_designs<'a>(occurrences: &[&'a Value]) -> BTreeMap<&'a str, &'a str> {
    let by_id: BTreeMap<&str, &Value> = occurrences
        .iter()
        .filter_map(|row| Some((row.get("id")?.as_str()?, *row)))
        .collect();
    let mut out = BTreeMap::new();
    for (id, row) in &by_id {
        let mut parent = row.pointer("/parent/occurrence").and_then(Value::as_str);
        let mut depth = 0;
        while let Some(up) = parent {
            depth += 1;
            let Some(row) = by_id.get(up).filter(|_| depth <= MAX_DEPTH) else {
                break;
            };
            if scope(row) == Some("external") {
                out.insert(*id, up);
                break;
            }
            parent = row.pointer("/parent/occurrence").and_then(Value::as_str);
        }
    }
    out
}

/// The name of the design an external occurrence inserts, as the file stores it with the
/// inserted design's file name.
fn inserted_design_name<'v>(document: &'v Value, occurrence: &Value) -> Option<&'v str> {
    let path = occurrence
        .pointer("/prototype/document/path")
        .and_then(Value::as_str)?;
    let file = file_name(path);
    document
        .pointer("/native/f3d/xref_designs")
        .and_then(Value::as_array)?
        .iter()
        .find(|design| design.get("target_file_name").and_then(Value::as_str) == Some(file))
        .and_then(|design| design.get("display_name").and_then(Value::as_str))
        .filter(|name| !name.trim().is_empty())
}

fn name_of(definition: &Value) -> Option<&str> {
    ["label", "source_name"]
        .into_iter()
        .find_map(|key| definition.get(key).and_then(Value::as_str))
        .filter(|name| !name.trim().is_empty())
}

/// `model` reduced to `bodies`, the geometry they reach and the history that builds them,
/// named `name`. When that history does not stand alone without the rest of the design, the
/// part keeps only its geometry.
pub(crate) fn only(
    model: &Model,
    document: &Value,
    bodies: &[&str],
    owner: HistoryOwner<'_>,
    name: String,
) -> Result<Model, String> {
    let slices = slicing::Index::value(document)?;
    only_indexed(model, &slices, bodies, owner, name)
}

fn only_indexed(
    model: &Model,
    slices: &slicing::Index<'_>,
    bodies: &[&str],
    owner: HistoryOwner<'_>,
    name: String,
) -> Result<Model, String> {
    match slice(model, slices, bodies, owner, name.clone(), true) {
        Ok(part) => Ok(part),
        Err(_) => slice(model, slices, bodies, owner, name, false),
    }
}

/// `model` reduced to `bodies` and the geometry they reach, named `name`.
pub(crate) fn geometry_only(
    model: &Model,
    document: &Value,
    bodies: &[&str],
    name: String,
) -> Result<Model, String> {
    let slices = slicing::Index::value(document)?;
    slice(model, &slices, bodies, HistoryOwner::Unknown, name, false)
}

/// [`only`] with its `history` or [`geometry_only`] without. A feature is the part's when
/// its definition owns it, it makes one of `bodies`, or it builds a sketch the component
/// owns. Referenced features come along, except those owned by another definition or making
/// only other bodies. Timeline dependencies do not establish component membership.
fn slice(
    model: &Model,
    slices: &slicing::Index<'_>,
    bodies: &[&str],
    owner: HistoryOwner<'_>,
    name: String,
    history: bool,
) -> Result<Model, String> {
    let document = slices.document;
    let wire = document
        .get("model")
        .and_then(Value::as_object)
        .ok_or("the model is malformed")?;
    let rows = &slices.rows;
    let mut keep: BTreeMap<&str, ()> = BTreeMap::new();
    let mut queue: Vec<&str> = Vec::new();
    for body in bodies {
        if rows.contains_key(*body) && keep.insert(body, ()).is_none() {
            queue.push(body);
        }
    }
    let ours = |ids: Option<&Value>| {
        ids.and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .any(|id| bodies.contains(&id))
    };
    fn product_definition(row: &Value) -> Option<&str> {
        row.get("product_definition").and_then(Value::as_str)
    }
    let definition = match owner {
        HistoryOwner::Definition(id) => Some(id),
        HistoryOwner::Root | HistoryOwner::Unknown => None,
    };
    let owned = |row: &Value| match owner {
        HistoryOwner::Definition(id) => product_definition(row) == Some(id),
        HistoryOwner::Root => product_definition(row).is_none(),
        HistoryOwner::Unknown => false,
    };
    let here_world = match owner {
        HistoryOwner::Definition(id) => single_world(
            wire.get("occurrences")
                .and_then(Value::as_array)
                .map_or(&[][..], Vec::as_slice),
            Some(id),
        ),
        HistoryOwner::Root => Some(IDENTITY),
        HistoryOwner::Unknown => None,
    };
    // Explicit ownership takes precedence over output links from older files.
    let foreign = |row: &Value| {
        if let Some(owner) = product_definition(row) {
            return Some(owner) != definition;
        }
        // A root sketch has no ownership field: repeated parts must resolve its instance
        // before following its binder, just as for a sketch with an explicit foreign owner.
        if definition.is_some() && here_world.is_none() && sketched(row).is_some() {
            return true;
        }
        row.get("outputs")
            .and_then(Value::as_array)
            .is_some_and(|ids| !ids.is_empty())
            && !ours(row.get("outputs"))
    };
    // Each sketch's features: a profile reaches its sketch, not the feature that builds it.
    let mut binders: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    if history && !bodies.is_empty() {
        let features = wire.get("features").and_then(Value::as_array);
        for row in features.map_or(&[][..], Vec::as_slice) {
            let Some(id) = row.get("id").and_then(Value::as_str) else {
                continue;
            };
            if let Some(sketch) = sketched(row) {
                binders.entry(sketch).or_default().push(id);
            }
            if (owned(row) || (ours(row.get("outputs")) && !foreign(row)))
                && keep.insert(id, ()).is_none()
            {
                queue.push(id);
            }
        }
        let parameters = wire.get("parameters").and_then(Value::as_array);
        for row in parameters.map_or(&[][..], Vec::as_slice) {
            if owned(row) {
                if let Some(id) = row.get("id").and_then(Value::as_str) {
                    if keep.insert(id, ()).is_none() {
                        queue.push(id);
                    }
                }
            }
        }
        let definitions = wire.get("product_definitions").and_then(Value::as_array);
        for component in definitions.map_or(&[][..], Vec::as_slice) {
            let selected = match definition {
                Some(id) => component.get("id").and_then(Value::as_str) == Some(id),
                None => ours(component.get("bodies")),
            };
            if !selected {
                continue;
            }
            let sketches = component.get("sketches").and_then(Value::as_array);
            for sketch in sketches.map_or(&[][..], Vec::as_slice) {
                if let Some(id) = sketch.as_str() {
                    if rows.contains_key(id) && keep.insert(id, ()).is_none() {
                        queue.push(id);
                    }
                }
            }
        }
    }
    while let Some(id) = queue.pop() {
        let Some(entry) = rows.get(id) else {
            continue;
        };
        let arena = entry.arena;
        let value = entry.row.value()?;
        let row = value.as_ref();
        let mut refs = Vec::new();
        if arena == "features" {
            for (key, value) in row.as_object().into_iter().flatten() {
                if key != "dependencies" && key != "regeneration_parent" {
                    strings(value, &mut refs);
                }
            }
        } else {
            strings(row, &mut refs);
        }
        if history && arena == "sketches" {
            refs.extend(binders.get(id).into_iter().flatten().copied());
        }
        for reference in refs {
            if let Some((id, entry)) = rows.get_key_value(reference) {
                let follow = !NOT_GEOMETRY.contains(&entry.arena)
                    || (history
                        && !bodies.is_empty()
                        && entry.arena == "features"
                        && !foreign(entry.row.value()?.as_ref()));
                if follow && keep.insert(id.as_str(), ()).is_none() {
                    queue.push(id.as_str());
                }
            }
        }
    }
    // A sketch another component builds that a feature of this one uses (designing in the
    // context of an assembly): carry its sketch feature in, with the sketch's plane moved from
    // that component's coordinates into this one's. Repeated using components need a unique
    // instance whose selected profile fits the stored part.
    let mut carried: BTreeMap<String, Placement3x4> = BTreeMap::new();
    let mut carried_features: BTreeSet<String> = BTreeSet::new();
    if history && !bodies.is_empty() {
        let occurrences = wire
            .get("occurrences")
            .and_then(Value::as_array)
            .map_or(&[][..], Vec::as_slice);
        let here = here_world;
        // Sketches this component's own features use (a profile, a path), not every sketch
        // its rows happen to reach.
        let mut used: BTreeSet<String> = BTreeSet::new();
        for id in keep.keys() {
            let Some(entry) = rows.get(*id).filter(|entry| entry.arena == "features") else {
                continue;
            };
            let value = entry.row.value()?;
            let row = value.as_ref();
            if !owned(row) || sketched(row).is_some() {
                continue;
            }
            let mut refs = Vec::new();
            strings(row.get("definition").unwrap_or(&Value::Null), &mut refs);
            used.extend(
                refs.into_iter()
                    .filter(|id| rows.get(*id).is_some_and(|entry| entry.arena == "sketches"))
                    .map(str::to_owned),
            );
        }
        for sketch in used.iter().map(String::as_str) {
            let Some(builders) = binders.get(sketch) else {
                continue;
            };
            if builders.iter().any(|id| keep.contains_key(id)) {
                continue;
            }
            let mut owners = BTreeSet::new();
            for id in builders {
                if let Some(entry) = rows.get(*id) {
                    owners
                        .insert(product_definition(entry.row.value()?.as_ref()).map(str::to_owned));
                }
            }
            let owners: Vec<Option<String>> = owners.into_iter().collect();
            let [there] = owners.as_slice() else {
                continue;
            };
            let Some(there) = single_world(occurrences, there.as_deref()) else {
                continue;
            };
            let resolved = rows
                .get(sketch)
                .map(|entry| entry.row.value())
                .transpose()?;
            let movable = resolved.as_ref().is_some_and(|row| {
                row.pointer("/placement/kind").and_then(Value::as_str) == Some("resolved")
                    && row.pointer("/placement/frame").is_some()
            });
            if !movable {
                continue;
            }
            let movement = match here {
                Some(here) => Some(compose(&inverse(&here), &there)),
                None => sketch_instance::choose(
                    slices,
                    bodies,
                    owner,
                    occurrences,
                    &there,
                    resolved.as_ref().map(|row| row.as_ref()),
                    keep.keys()
                        .filter_map(|id| rows.get(*id))
                        .filter(|entry| entry.arena == "features")
                        .map(|entry| entry.row.value())
                        .collect::<Result<Vec<_>, _>>()?
                        .iter()
                        .map(|row| row.as_ref())
                        .filter(|row| owned(row)),
                )?,
            };
            let Some(movement) = movement else { continue };
            carried.insert(sketch.to_owned(), movement);
            for id in builders {
                keep.insert(id, ());
                carried_features.insert((*id).to_owned());
            }
        }
    }
    // Bindings point to topology, rather than topology pointing to its bindings.
    // Keep that reverse edge and the appearance asset in each definition's slice.
    let bindings: Vec<_> = keep
        .keys()
        .filter_map(|id| slices.bindings.get(id))
        .flatten()
        .copied()
        .collect();
    for binding in bindings {
        for id in [binding.get("id"), binding.get("appearance")]
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
        {
            keep.insert(id, ());
        }
    }
    // Visit just the reached rows, then restore their original order within each arena.
    // No component rebuilds a global index or scans the complete geometry again.
    let mut selected: BTreeMap<&str, Vec<(usize, slicing::Selected<'_>)>> = slices
        .arenas
        .iter()
        .map(|arena| (*arena, Vec::new()))
        .collect();
    for entry in keep.keys().filter_map(|id| rows.get(*id)).chain(
        slices
            .duplicates
            .iter()
            .filter_map(|(id, entry)| keep.contains_key(id.as_str()).then_some(entry)),
    ) {
        let row = if entry.arena == "features" {
            let mut row = entry.row.value()?.into_owned();
            if let Some(fields) = row.as_object_mut() {
                // A sketch feature carried in from another component is this one's now.
                if fields
                    .get("id")
                    .and_then(Value::as_str)
                    .is_some_and(|id| carried_features.contains(id))
                {
                    match definition {
                        Some(id) => {
                            fields.insert("product_definition".to_owned(), Value::from(id));
                        }
                        None => {
                            fields.remove("product_definition");
                        }
                    }
                }
                // A predecessor outside this slice is not part of its timeline.
                if fields
                    .get("regeneration_parent")
                    .and_then(Value::as_str)
                    .is_some_and(|id| !keep.contains_key(id))
                {
                    fields.remove("regeneration_parent");
                }
                if let Some(ids) = fields.get_mut("dependencies").and_then(Value::as_array_mut) {
                    ids.retain(|id| id.as_str().is_some_and(|id| keep.contains_key(id)));
                }
                if let Some(properties) = fields
                    .get_mut("source_properties")
                    .and_then(Value::as_object_mut)
                {
                    if properties
                        .get("body_centers")
                        .and_then(Value::as_str)
                        .is_some_and(|centers| slices.shared_centers.contains(centers))
                    {
                        properties.remove("body_centers");
                    }
                }
            }
            slicing::Selected::Value(std::borrow::Cow::Owned(row))
        } else if entry.arena == "sketches" && !carried.is_empty() {
            let mut row = entry.row.value()?.into_owned();
            let moved = row
                .get("id")
                .and_then(Value::as_str)
                .and_then(|id| carried.get(id))
                .copied();
            if let Some(by) = moved {
                if let Some(frame) = row.pointer_mut("/placement/frame") {
                    move_frame(frame, &by).ok_or("a carried sketch's plane is malformed")?;
                }
            }
            slicing::Selected::Value(std::borrow::Cow::Owned(row))
        } else {
            entry.row.selected()
        };
        selected
            .entry(entry.arena)
            .or_default()
            .push((entry.ordinal, row));
    }
    let reduced = selected
        .into_iter()
        .map(|(arena, mut rows)| {
            rows.sort_by_key(|(ordinal, _)| *ordinal);
            (arena, rows.into_iter().map(|(_, row)| row).collect())
        })
        .collect();
    // The document's other fields, without copying its model or native records: those are
    // the bulk of a large archive, and each part would otherwise copy them once.
    let fields: serde_json::Map<String, Value> = document
        .as_object()
        .into_iter()
        .flatten()
        .filter(|(key, _)| key.as_str() != "model" && key.as_str() != "native")
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    let part = slicing::Part {
        fields,
        model: reduced,
    };
    // cadmpeg is third-party code; a panic in it is a reason, not a crash.
    let text = serde_json::to_string(&part).map_err(|err| err.to_string())?;
    let ir = std::panic::catch_unwind(|| cadmpeg_ir::CadIr::from_json(&text))
        .map_err(|_| "cadmpeg failed unexpectedly on a component's geometry".to_owned())?
        .map_err(|err| format!("a component's geometry does not stand alone: {err}"))?;
    Ok(Model {
        ir,
        fidelity: None,
        name,
        source_file: model.source_file.clone(),
        assembly: None,
        source_step: None,
    })
}

/// The sketch a feature builds, when it builds one.
fn sketched(row: &Value) -> Option<&str> {
    row.pointer("/definition/sketch/sketch")
        .and_then(Value::as_str)
}

/// Every string inside `value`.
pub(crate) fn strings<'v>(value: &'v Value, out: &mut Vec<&'v str>) {
    match value {
        Value::String(s) => out.push(s),
        Value::Array(values) => values.iter().for_each(|v| strings(v, out)),
        Value::Object(fields) => fields.values().for_each(|v| strings(v, out)),
        _ => {}
    }
}

/// Where the one instance of component `definition` sits in the world, when the design has
/// exactly one: one local occurrence places it, and one places each component around it.
/// `None` for a component placed more than once (or not at all), whose instances differ, or
/// for a placement that is not rigid. The design's root (`None`) is the world itself.
fn single_world(occurrences: &[Value], definition: Option<&str>) -> Option<Placement3x4> {
    let Some(definition) = definition else {
        return Some(IDENTITY);
    };
    let local = || occurrences.iter().filter(|row| scope(row) == Some("local"));
    let placed = |definition: &str| {
        local()
            .filter(|row| definition_key(row) == Some(definition))
            .count()
    };
    let mut row = {
        let mut rows = local().filter(|row| definition_key(row) == Some(definition));
        let row = rows.next()?;
        rows.next().is_none().then_some(row)?
    };
    let mut world = transform(row).ok()?;
    for _ in 0..MAX_DEPTH {
        let Some(parent) = row.pointer("/parent/occurrence").and_then(Value::as_str) else {
            return Some(world);
        };
        // The design's own root occurrence, when the file stores one, places nothing else.
        let Some(above) = occurrences
            .iter()
            .find(|row| row.get("id").and_then(Value::as_str) == Some(parent))
        else {
            return Some(world);
        };
        if let Some(key) = definition_key(above) {
            if placed(key) != 1 {
                return None;
            }
        }
        world = compose(&transform(above).ok()?, &world);
        row = above;
    }
    None
}

/// `frame` (a sketch placement's `origin`, `normal` and `u_axis`) moved by `by`.
fn move_frame(frame: &mut Value, by: &Placement3x4) -> Option<()> {
    let read = |value: &Value| -> Option<[f64; 3]> {
        Some([
            value.get("x")?.as_f64()?,
            value.get("y")?.as_f64()?,
            value.get("z")?.as_f64()?,
        ])
    };
    let apply = |[x, y, z]: [f64; 3], translate: bool| -> Value {
        let at = |[a, b, c, t]: [f64; 4]| a * x + b * y + c * z + if translate { t } else { 0.0 };
        let [first, second, third] = *by;
        serde_json::json!({"x": at(first), "y": at(second), "z": at(third)})
    };
    let origin = read(frame.get("origin")?)?;
    let normal = read(frame.get("normal")?)?;
    let u_axis = read(frame.get("u_axis")?)?;
    let fields = frame.as_object_mut()?;
    fields.insert("origin".to_owned(), apply(origin, true));
    fields.insert("normal".to_owned(), apply(normal, false));
    fields.insert("u_axis".to_owned(), apply(u_axis, false));
    Some(())
}

/// The file name in a stored path, whichever separators the saving machine used.
fn file_name(stored: &str) -> &str {
    stored.rsplit(['/', '\\']).next().unwrap_or(stored)
}

/// An occurrence's rigid placement, or why it is not one.
fn transform(occurrence: &Value) -> Result<[[f64; 4]; 3], String> {
    let malformed = || "its placement is malformed".to_owned();
    let rows = occurrence
        .get("transform")
        .and_then(Value::as_array)
        .ok_or_else(malformed)?;
    let mut out = [[0.0; 4]; 3];
    for (row, values) in out.iter_mut().zip(rows) {
        let values = values.as_array().ok_or_else(malformed)?;
        for (cell, value) in row.iter_mut().zip(values) {
            *cell = value
                .as_f64()
                .filter(|v| v.is_finite())
                .ok_or_else(malformed)?;
        }
    }
    if rows.len() != 3
        || rows
            .iter()
            .any(|row| row.as_array().map(Vec::len) != Some(4))
    {
        return Err(malformed());
    }
    let scaled = occurrence
        .get("scale")
        .and_then(Value::as_array)
        .is_some_and(|scale| {
            scale
                .iter()
                .any(|s| s.as_f64().is_none_or(|s| (s - 1.0).abs() > RIGID))
        });
    if scaled {
        return Err("scaled placements are not carried".to_owned());
    }
    Ok(out)
}

// Test helpers outside `#[test]` functions are not covered by clippy.toml's test allowances.
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use serde_json::json;

    use super::*;

    fn fixture(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name)
    }

    fn occurrence(key: usize, path: &str, parent: Value, x: f64) -> Value {
        json!({
            "id": format!("inventor:assembly:occurrence#{key}"),
            "prototype": {"scope": "external", "document": {"resolution": "path", "path": path}},
            "parent": parent,
            "ordinal": key,
            "transform": [[1.0, 0.0, 0.0, x], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0]],
            "scale": [1.0, 1.0, 1.0],
            "name": format!("Occurrence:{key}"),
        })
    }

    fn root() -> Value {
        json!({"kind": "root"})
    }

    /// A folder of placeholder files and a decoder that gives each its model: the synthetic
    /// Inventor cube for `cube.ipt`, and for an assembly the occurrences `assemblies` lists
    /// for its file name (as `(stored path, x)`).
    struct Project {
        dir: tempfile::TempDir,
        cube: Model,
        assemblies: BTreeMap<String, Vec<Value>>,
    }

    impl Project {
        fn new(assemblies: &[(&str, Vec<Value>)]) -> Project {
            let dir = tempfile::tempdir().unwrap();
            let cube = crate::read(
                Format::InventorPart,
                &fixture("synthetic_cube.ipt"),
                &mut Report::default(),
            )
            .unwrap();
            std::fs::write(dir.path().join("Cube.IPT"), b"").unwrap();
            for (name, _) in assemblies {
                std::fs::write(dir.path().join(name), b"").unwrap();
            }
            let assemblies = assemblies
                .iter()
                .map(|(name, rows)| ((*name).to_owned(), rows.clone()))
                .collect();
            Project {
                dir,
                cube,
                assemblies,
            }
        }

        fn decode(&self, _format: Format, path: &Path, _: &mut Report) -> Result<Model, Error> {
            let name = path.file_name().unwrap().to_string_lossy().to_lowercase();
            if name == "cube.ipt" {
                return Ok(self.cube.clone());
            }
            let rows = self.assemblies.get(&name).cloned().unwrap_or_default();
            let mut wire = serde_json::to_value(&self.cube.ir).unwrap();
            wire["model"]["occurrences"] = Value::Array(rows);
            Ok(Model {
                ir: cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap(),
                fidelity: None,
                name: name.trim_end_matches(".iam").to_owned(),
                source_file: Some(name),
                source_step: None,
                assembly: None,
            })
        }

        fn read(&self, name: &str, report: &mut Report) -> Result<Model, Error> {
            let decode = |format: Format, path: &Path, report: &mut Report| {
                self.decode(format, path, report)
            };
            read(
                Format::InventorAssembly,
                &self.dir.path().join(name),
                report,
                &decode,
                &Finder::new(&PartSearch::project(self.dir.path())),
                &mut Vec::new(),
            )
        }
    }

    fn dropped(report: &Report) -> Vec<String> {
        report
            .losses()
            .iter()
            .filter(|loss| loss.severity == Severity::Dropped)
            .map(|loss| format!("{}: {}", loss.subject, loss.detail))
            .collect()
    }

    #[test]
    fn parts_and_sub_assemblies_are_read_from_their_files() {
        let nested = json!({"kind": "occurrence", "occurrence": "inventor:assembly:occurrence#2"});
        let project = Project::new(&[
            (
                "top.iam",
                vec![
                    occurrence(0, r"C:\Work\Parts\cube.ipt", root(), 0.0),
                    occurrence(1, r"C:\Work\Parts\cube.ipt", root(), 30.0),
                    occurrence(2, r"C:\Work\sub.iam", root(), 100.0),
                    occurrence(3, r"C:\Work\Parts\gone.ipt", root(), 0.0),
                    occurrence(4, r"C:\Work\Parts\cube.ipt", nested, 0.0),
                ],
            ),
            (
                "sub.iam",
                vec![
                    occurrence(0, "cube.ipt", root(), 0.0),
                    occurrence(1, "cube.ipt", root(), 15.0),
                ],
            ),
        ]);
        let mut report = Report::default();
        let top = project.read("top.iam", &mut report).unwrap();
        let assembly = top.assembly.unwrap();
        assert_eq!(
            assembly.parts.len(),
            2,
            "the cube once, and the sub-assembly"
        );
        assert_eq!(assembly.placements.len(), 3);
        assert_eq!(assembly.placements[2].transform[0][3], 100.0);
        let sub = assembly.parts[1].assembly.as_ref().unwrap();
        assert_eq!(sub.parts.len(), 1);
        assert_eq!(sub.placements.len(), 2);
        let completeness = Assembly::completeness(&Assembly {
            own_part: None,
            parts: assembly.parts.clone(),
            placements: assembly.placements.clone(),
            joints: Vec::new(),
            left_out: assembly.left_out.clone(),
            held: Vec::new(),
            joint_notes: Vec::new(),
            stored_placements: Vec::new(),
            identities: Vec::new(),
            definitions: Vec::new(),
        });
        assert_eq!((completeness.placed, completeness.placements()), (5, 6));
        assert_eq!(
            dropped(&report),
            [format!(
                "occurrence \"Occurrence:3\": gone.ipt: not found at its stored path or in the \
                 project folder {}",
                crate::locate::tests::absolute_for_tests(project.dir.path()).display()
            )]
        );
    }

    #[test]
    fn completeness_counts_through_each_placed_sub_assembly() {
        let project = Project::new(&[
            (
                "top.iam",
                vec![
                    occurrence(0, "cube.ipt", root(), 0.0),
                    occurrence(1, "sub.iam", root(), 30.0),
                    occurrence(2, "sub.iam", root(), 60.0),
                    occurrence(3, "gone.ipt", root(), 0.0),
                    occurrence(4, "notes.txt", root(), 0.0),
                ],
            ),
            (
                "sub.iam",
                vec![
                    occurrence(0, "cube.ipt", root(), 0.0),
                    occurrence(1, "cube.ipt", root(), 15.0),
                    occurrence(2, "lost.ipt", root(), 0.0),
                ],
            ),
        ]);
        let top = project.read("top.iam", &mut Report::default()).unwrap();
        let completeness = top.assembly.unwrap().completeness();
        // The cube and the sub-assembly twice at the top; two cubes in each sub-assembly.
        assert_eq!(completeness.placed, 7);
        assert_eq!(completeness.placements(), 11);
        assert_eq!(completeness.left_out_count(), 4);
        let groups: Vec<(Option<&str>, u64, Stage)> = completeness
            .left_out
            .iter()
            .map(|g| (g.file.as_deref(), g.count, g.stage))
            .collect();
        assert_eq!(
            groups,
            [
                (Some("lost.ipt"), 2, Stage::Input),
                (Some("gone.ipt"), 1, Stage::Input),
                (Some("notes.txt"), 1, Stage::Input),
            ]
        );
        assert!(completeness.left_out[0]
            .reason
            .starts_with("lost.ipt: not found"));
    }

    #[test]
    fn empty_leaves_count_for_each_use_but_child_holders_are_placed() {
        let project = Project::new(&[
            (
                "top.iam",
                vec![
                    occurrence(0, "sub.iam", root(), 0.0),
                    occurrence(1, "sub.iam", root(), 30.0),
                ],
            ),
            (
                "sub.iam",
                vec![
                    occurrence(0, "cube.ipt", root(), 0.0),
                    occurrence(1, "cube.ipt", root(), 15.0),
                    occurrence(2, "gone.ipt", root(), 0.0),
                ],
            ),
        ]);
        let mut top = project.read("top.iam", &mut Report::default()).unwrap();
        let assembly = top.assembly.as_mut().unwrap();
        let holder = &mut assembly.parts[0];
        holder.ir.model.bodies.clear();
        let leaf = &mut holder.assembly.as_mut().unwrap().parts[0];
        leaf.ir.model.bodies.clear();
        let counts = assembly.completeness();
        assert_eq!((counts.placed, counts.placed_empty), (2, 4));
        assert_eq!(counts.left_out_count(), 2);
        assert_eq!(counts.placements(), 8);
        assert!(!counts.is_complete());
    }

    #[test]
    fn an_assembly_that_places_itself_is_refused() {
        let project = Project::new(&[
            (
                "a.iam",
                vec![
                    occurrence(0, "b.iam", root(), 0.0),
                    occurrence(1, "cube.ipt", root(), 0.0),
                ],
            ),
            ("b.iam", vec![occurrence(0, "a.iam", root(), 0.0)]),
        ]);
        let mut report = Report::default();
        let a = project.read("a.iam", &mut report).unwrap();
        assert_eq!(
            a.assembly.unwrap().placements.len(),
            1,
            "the cube is still placed"
        );
        assert!(
            dropped(&report)
                .iter()
                .any(|loss| loss.contains("places itself through its sub-assemblies")),
            "{:?}",
            dropped(&report)
        );
    }

    #[test]
    fn sub_assemblies_nest_only_so_deep() {
        let chain: Vec<(String, Vec<Value>)> = (0..=MAX_DEPTH + 2)
            .map(|n| {
                (
                    format!("d{n}.iam"),
                    vec![
                        occurrence(0, &format!("d{}.iam", n + 1), root(), 0.0),
                        occurrence(1, "cube.ipt", root(), 0.0),
                    ],
                )
            })
            .collect();
        let named: Vec<(&str, Vec<Value>)> = chain
            .iter()
            .map(|(name, rows)| (name.as_str(), rows.clone()))
            .collect();
        let project = Project::new(&named);
        let mut report = Report::default();
        project.read("d0.iam", &mut report).unwrap();
        assert!(
            dropped(&report)
                .iter()
                .any(|loss| loss.contains("nest deeper")),
            "{:?}",
            dropped(&report)
        );
    }

    #[test]
    fn an_assembly_with_no_readable_part_is_an_error() {
        let project = Project::new(&[("lonely.iam", vec![occurrence(0, "gone.ipt", root(), 0.0)])]);
        let err = project
            .read("lonely.iam", &mut Report::default())
            .unwrap_err();
        assert!(
            err.to_string()
                .contains("give the project folder that holds them"),
            "{err}"
        );
    }

    #[test]
    fn an_assembly_that_places_nothing_is_empty_rather_than_missing_its_parts() {
        let project = Project::new(&[("empty.iam", vec![])]);
        let mut report = Report::default();
        let model = project.read("empty.iam", &mut report).unwrap();
        let assembly = model.assembly.unwrap();
        assert!(assembly.parts.is_empty() && assembly.placements.is_empty());
        assert!(
            report
                .losses()
                .iter()
                .any(|loss| loss.severity == Severity::Info
                    && loss.detail.contains("the assembly places no parts")),
            "{report:?}"
        );
    }

    #[test]
    fn parts_in_the_folders_of_an_inventor_project_are_found() {
        // Robot/Robot.ipj, Robot/Assemblies/top.iam, Robot/Parts/cube.ipt and
        // Robot/SubAssembly/sub.iam, stored with the paths of the machine that saved them.
        let project = Project::new(&[
            (
                "top.iam",
                vec![
                    occurrence(0, r"C:\Users\someone\Robot\Parts\cube.ipt", root(), 0.0),
                    occurrence(
                        1,
                        r"C:\Users\someone\Robot\SubAssembly\sub.iam",
                        root(),
                        50.0,
                    ),
                ],
            ),
            (
                "sub.iam",
                vec![occurrence(0, r"\\server\old\cube.ipt", root(), 0.0)],
            ),
        ]);
        let top = project.dir.path().join("Robot");
        for folder in ["Assemblies", "Parts", "SubAssembly"] {
            std::fs::create_dir_all(top.join(folder)).unwrap();
        }
        std::fs::write(top.join("Robot.ipj"), b"").unwrap();
        for (from, to) in [
            ("top.iam", "Assemblies/top.iam"),
            ("Cube.IPT", "Parts/cube.ipt"),
            ("sub.iam", "SubAssembly/sub.iam"),
        ] {
            std::fs::rename(project.dir.path().join(from), top.join(to)).unwrap();
        }
        let decode =
            |format: Format, path: &Path, report: &mut Report| project.decode(format, path, report);
        let mut report = Report::default();
        let model = read(
            Format::InventorAssembly,
            &top.join("Assemblies/top.iam"),
            &mut report,
            &decode,
            &Finder::new(&PartSearch::default()),
            &mut Vec::new(),
        )
        .unwrap();
        assert_eq!(dropped(&report), Vec::<String>::new());
        let assembly = model.assembly.unwrap();
        assert_eq!(assembly.placements.len(), 2);
        let sub = assembly.parts[1].assembly.as_ref().unwrap();
        assert_eq!(
            sub.placements.len(),
            1,
            "the sub-assembly's cube, found by searching"
        );
        // Each part found, and the folders it was found in, are names the report met: by
        // name, as every system spells the path to them its own way (`\\`, `/private/var`).
        for name in ["Robot", "Parts", "SubAssembly", "cube.ipt", "cube"] {
            assert!(
                report.names().contains(name),
                "{name}: {:?}",
                report.names()
            );
        }
        assert!(
            report
                .names()
                .iter()
                .any(|name| name.ends_with("cube.ipt") && name.contains("Parts")),
            "the whole path found: {:?}",
            report.names()
        );
    }

    /// cadmpeg's example 10 mm cube in a design whose component `Block` owns it, placed at
    /// the origin and 30 mm along X; with `owned` false the component does not say what it owns.
    fn with_components(owned: bool) -> Model {
        let part = crate::read(
            Format::InventorPart,
            &fixture("synthetic_cube.ipt"),
            &mut Report::default(),
        )
        .unwrap();
        let mut wire = serde_json::to_value(&part.ir).unwrap();
        let body = wire["model"]["bodies"][0]["id"]
            .as_str()
            .unwrap()
            .to_owned();
        let bodies = if owned { json!([body]) } else { json!([]) };
        wire["model"]["product_definitions"] = json!([{
            "id": "f3d:model:component#block", "kind": "part", "label": "Block", "bodies": bodies,
        }]);
        let placed = |key: u32, x: f64, parent: Value| {
            json!({
                "id": format!("f3d:model:occurrence#{key}"),
                "prototype": {"scope": "local", "definition": "f3d:model:component#block"},
                "parent": parent,
                "ordinal": key,
                "transform": [[1.0, 0.0, 0.0, x], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0]],
                "scale": [1.0, 1.0, 1.0],
            })
        };
        wire["model"]["occurrences"] = json!([placed(0, 0.0, root()), placed(1, 30.0, root())]);
        Model {
            ir: cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap(),
            ..part
        }
    }

    #[test]
    fn component_slicing_reports_an_appearance_binding_without_a_target() {
        let mut model = with_components(true);
        let mut wire = serde_json::to_value(&model.ir).unwrap();
        wire["model"]["appearances"] = json!([{
            "id": "synthetic:test:appearance#red", "base_color": {"r": 1.0, "g": 0.0, "b": 0.0, "a": 1.0}
        }]);
        wire["model"]["appearance_bindings"] = json!([{
            "id": "synthetic:test:appearance-binding#missing",
            "target": {"kind": "face", "id": "synthetic:test:face#missing"},
            "appearance": "synthetic:test:appearance#red"
        }]);
        model.ir = cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap();
        let mut report = Report::default();
        assert!(matches!(
            local(&model, 1, crate::PartPositions::LastShown, &mut report),
            Components::Assembly(_)
        ));
        let losses: Vec<_> = report
            .losses()
            .iter()
            .filter(|loss| loss.subject == "appearance")
            .collect();
        assert_eq!(losses.len(), 1);
        assert_eq!(losses[0].severity, Severity::Dropped);
        assert_eq!(
            losses[0].code.as_deref(),
            Some("dutchman/appearance.target-absent")
        );
        assert!(losses[0]
            .detail
            .contains("synthetic:test:appearance-binding#missing"));
        assert!(losses[0]
            .detail
            .contains("to face synthetic:test:face#missing"));
        assert!(losses[0]
            .detail
            .ends_with("its target is absent from the decoded model"));
    }

    #[test]
    fn an_unplaced_appearance_target_is_distinguished_from_a_missing_target() {
        let mut model = with_components(true);
        let mut wire = serde_json::to_value(&model.ir).unwrap();
        let body = wire["model"]["bodies"][0]["id"].clone();
        wire["model"]["product_definitions"]
            .as_array_mut()
            .unwrap()
            .push(json!({
                "id": "synthetic:test:component#empty", "kind": "part", "bodies": []
            }));
        for occurrence in wire["model"]["occurrences"].as_array_mut().unwrap() {
            occurrence["prototype"]["definition"] = json!("synthetic:test:component#empty");
        }
        wire["model"]["appearances"] = json!([{
            "id": "synthetic:test:appearance#red", "base_color": {"r": 1.0, "g": 0.0, "b": 0.0, "a": 1.0}
        }]);
        wire["model"]["appearance_bindings"] = json!([{
            "id": "synthetic:test:appearance-binding#unplaced", "target": {"kind": "body", "id": body},
            "appearance": "synthetic:test:appearance#red"
        }]);
        model.ir = cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap();
        let mut report = Report::default();
        assert!(matches!(
            local(&model, 1, crate::PartPositions::LastShown, &mut report),
            Components::Assembly(_)
        ));
        let losses: Vec<_> = report
            .losses()
            .iter()
            .filter(|loss| loss.subject == "appearance")
            .collect();
        assert_eq!(losses.len(), 1);
        assert_eq!(losses[0].code.as_deref(), Some("dutchman/appearance.other"));
        assert!(losses[0].detail.ends_with(
            "its target exists in the decoded model but has no placed component geometry"
        ));
    }

    #[test]
    fn shared_slices_keep_complete_instance_scopes_and_only_their_appearances() {
        let model = with_components(true);
        let original = serde_json::to_value(&model.ir).unwrap();
        let mut wire = original.clone();
        let geometry = wire["model"].as_object_mut().unwrap();
        geometry.remove("product_definitions");
        geometry.remove("occurrences");
        let ids: BTreeSet<String> = geometry
            .values()
            .filter_map(Value::as_array)
            .flatten()
            .filter_map(|row| row["id"].as_str().map(str::to_owned))
            .collect();
        fn scoped(value: &Value, ids: &BTreeSet<String>, scope: usize) -> Value {
            match value {
                Value::String(id) if ids.contains(id) => json!(format!(
                    "f3d:xref/role-test/reference-0/occurrence-{scope}/{}",
                    id.split_once(':').map_or(id.as_str(), |(_, tail)| tail)
                )),
                Value::Array(rows) => rows.iter().map(|row| scoped(row, ids, scope)).collect(),
                Value::Object(fields) => fields
                    .iter()
                    .map(|(key, value)| (key.clone(), scoped(value, ids, scope)))
                    .collect(),
                _ => value.clone(),
            }
        }
        let a = scoped(&Value::Object(geometry.clone()), &ids, 0);
        let b = scoped(&Value::Object(geometry.clone()), &ids, 1);
        for (arena, rows) in geometry.iter_mut() {
            *rows = json!(a[arena]
                .as_array()
                .unwrap()
                .iter()
                .chain(b[arena].as_array().unwrap())
                .cloned()
                .collect::<Vec<_>>());
        }
        let body_a = a["bodies"][0]["id"].clone();
        let body_b = b["bodies"][0]["id"].clone();
        geometry.insert("appearances".into(), json!([
            {"id": "synthetic:test:appearance#red", "base_color": {"r": 1.0, "g": 0.0, "b": 0.0, "a": 1.0}},
            {"id": "synthetic:test:appearance#blue", "base_color": {"r": 0.0, "g": 0.0, "b": 1.0, "a": 1.0}}
        ]));
        geometry.insert("appearance_bindings".into(), json!([
            {"id": "synthetic:test:binding#red", "target": {"kind": "body", "id": body_a}, "appearance": "synthetic:test:appearance#red"},
            {"id": "synthetic:test:binding#blue", "target": {"kind": "body", "id": body_b}, "appearance": "synthetic:test:appearance#blue"}
        ]));
        geometry.insert(
            "product_definitions".into(),
            json!([
                {"id": "synthetic:test:component#a", "kind": "part", "bodies": [body_a]},
                {"id": "synthetic:test:component#b", "kind": "part", "bodies": [body_b]}
            ]),
        );
        let rows: Vec<Value> = (0..4).map(|key| json!({
            "id": format!("synthetic:test:occurrence#{key}"), "ordinal": key, "parent": {"kind": "root"},
            "prototype": {"scope": "local", "definition": if key < 2 {"synthetic:test:component#a"} else {"synthetic:test:component#b"}},
            "transform": [[1,0,0,key * 30],[0,1,0,0],[0,0,1,0]], "scale": [1,1,1]
        })).collect();
        geometry.insert("occurrences".into(), json!(rows));
        let model = Model {
            ir: cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap(),
            ..model
        };
        for threads in [1, 4] {
            let (assembly, _) = local_components_with_threads(
                &model,
                &BTreeMap::new(),
                threads,
                crate::PartPositions::LastShown,
                &mut Report::default(),
            )
            .unwrap()
            .unwrap();
            assert_eq!(assembly.parts.len(), 2);
            assert_eq!(
                assembly
                    .placements
                    .iter()
                    .map(|p| p.part)
                    .collect::<Vec<_>>(),
                [0, 0, 1, 1]
            );
            for (part, color) in assembly.parts.iter().zip(["red", "blue"]) {
                assert_eq!(part.ir.model.appearance_bindings.len(), 1);
                assert_eq!(part.ir.model.appearances.len(), 1);
                assert_eq!(
                    part.ir.model.appearance_bindings[0].appearance.as_str(),
                    format!("synthetic:test:appearance#{color}")
                );
            }
        }
    }

    #[test]
    fn cadir_cannot_receive_component_or_occurrence_appearance_targets_yet() {
        for kind in ["component", "occurrence"] {
            let mut wire = serde_json::to_value(&with_components(true).ir).unwrap();
            wire["model"]["appearance_bindings"] = json!([{
                "id": "synthetic:test:appearance-binding#unsupported",
                "target": {"kind": kind, "id": "f3d:model:component#block"},
                "appearance": "synthetic:test:appearance#red"
            }]);
            let error = cadmpeg_ir::CadIr::from_json(&wire.to_string())
                .unwrap_err()
                .to_string();
            assert!(error.contains(kind), "{error}");
        }
    }

    #[test]
    fn components_that_own_their_bodies_become_an_assembly() {
        let assembly = local_assembly(&with_components(true)).unwrap().unwrap();
        assert_eq!(assembly.own_part, None);
        assert_eq!(assembly.parts.len(), 1);
        let part = &assembly.parts[0];
        assert_eq!(part.name, "Block");
        assert_eq!(part.ir.model.bodies.len(), 1);
        assert_eq!(part.ir.model.faces.len(), 6);
        assert!(part.ir.model.occurrences.is_empty());
        assert_eq!(assembly.placements.len(), 2);
        assert_eq!(assembly.placements[1].transform[0][3], 30.0);
    }

    #[test]
    fn the_designs_own_loose_bodies_keep_their_part_identity() {
        let model = with_components(true);
        let mut wire = serde_json::to_value(&model.ir).unwrap();
        let mut loose = wire["model"]["bodies"][0].clone();
        loose["id"] = json!("synthetic:cube:body#loose");
        wire["model"]["bodies"].as_array_mut().unwrap().push(loose);
        let model = Model {
            ir: cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap(),
            ..model
        };
        let assembly = local_assembly(&model).unwrap().unwrap();
        assert_eq!(assembly.own_part, Some(1));
        assert_eq!(assembly.parts[1].name, model.name);
        assert_eq!(assembly.parts[1].ir.model.bodies.len(), 1);
        assert_eq!(assembly.placements[2].part, 1);
        assert_eq!(assembly.placements[2].transform, IDENTITY);
    }

    #[test]
    fn components_that_do_not_say_what_they_own_convert_as_stored() {
        let err = local_assembly(&with_components(false)).unwrap_err();
        assert_eq!(
            err,
            "cadmpeg does not say which bodies belong to component Block"
        );
    }

    /// A Fusion fixture's sketch, its sketch feature and its body, split between component
    /// `Block` (the body, and the sketch when `owns_sketch`) and `Other` (a copy of the body),
    /// with a feature on that sketch that makes only the copy.
    fn with_history(owns_sketch: bool) -> Model {
        let part = crate::read(
            Format::FusionArchive,
            &fixture("design_sketch_constraints.f3d"),
            &mut Report::default(),
        )
        .unwrap();
        let mut wire = serde_json::to_value(&part.ir).unwrap();
        let body = wire["model"]["bodies"][0]["id"].clone();
        let sketch = wire["model"]["sketches"][0]["id"].clone();
        let mut copy = wire["model"]["bodies"][0].clone();
        copy["id"] = json!("synthetic:copy:body#copy");
        wire["model"]["bodies"].as_array_mut().unwrap().push(copy);
        let mut other = wire["model"]["features"][0].clone();
        other["id"] = json!("synthetic:copy:feature#other");
        other["outputs"] = json!(["synthetic:copy:body#copy"]);
        wire["model"]["features"]
            .as_array_mut()
            .unwrap()
            .push(other);
        let sketches = if owns_sketch {
            json!([sketch])
        } else {
            json!([])
        };
        wire["model"]["product_definitions"] = json!([
            {"id": "f3d:model:component#block", "kind": "part", "label": "Block", "bodies": [body], "sketches": sketches},
            {"id": "f3d:model:component#other", "kind": "part", "label": "Other", "bodies": ["synthetic:copy:body#copy"]},
        ]);
        wire["model"]["occurrences"] = json!([
            placed(0, "block", root(), 0.0),
            placed(1, "other", root(), 30.0),
        ]);
        Model {
            ir: cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap(),
            ..part
        }
    }

    fn feature_ids(part: &Model) -> Vec<String> {
        let wire = serde_json::to_value(&part.ir).unwrap();
        wire["model"]["features"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|row| row["id"].as_str().map(str::to_owned))
            .collect()
    }

    #[test]
    fn a_component_keeps_the_history_that_builds_it() {
        let assembly = local_assembly(&with_history(true)).unwrap().unwrap();
        let block = &assembly.parts[0];
        assert_eq!(block.name, "Block");
        // Its sketch, and the feature that builds it, though no row of the part points at
        // that feature; not the feature that makes the other component's body.
        assert_eq!(block.ir.model.sketches.len(), 1);
        let features = feature_ids(block);
        assert_eq!(features.len(), 1);
        assert!(!features[0].starts_with("synthetic:"));
        assert_eq!(block.ir.model.bodies.len(), 1);
        // The other component's feature is built on Block's sketch, so that sketch and the
        // feature building it come along; Block's body does not.
        let other = &assembly.parts[1];
        assert_eq!(
            feature_ids(other),
            [features[0].as_str(), "synthetic:copy:feature#other"]
        );
        assert_eq!(other.ir.model.sketches.len(), 1);
        assert_eq!(other.ir.model.bodies.len(), 1);
    }

    /// The fixture's history split between component `Layout`, which builds the sketch, and
    /// `Block`, which owns the body and every other feature (designing in the context of an
    /// assembly). Block is at the origin; Layout is placed at x = 30, then at x = 60 for each
    /// of its `extra` further instances.
    fn with_layout_sketch(extra: usize) -> (Model, String) {
        let part = crate::read(
            Format::FusionArchive,
            &fixture("design_sketch_constraints.f3d"),
            &mut Report::default(),
        )
        .unwrap();
        let mut wire = serde_json::to_value(&part.ir).unwrap();
        let body = wire["model"]["bodies"][0]["id"].clone();
        let mut builder = String::new();
        for feature in wire["model"]["features"].as_array_mut().unwrap() {
            let sketch = feature["definition"]["definition"] == "sketch";
            if sketch {
                builder = feature["id"].as_str().unwrap().to_owned();
            }
            let owner = if sketch { "layout" } else { "block" };
            feature["product_definition"] = json!(format!("f3d:model:component#{owner}"));
            feature["outputs"] = json!([]);
        }
        assert!(!builder.is_empty());
        // Block's feature that uses Layout's sketch.
        let sketch = wire["model"]["sketches"][0]["id"].clone();
        wire["model"]["features"]
            .as_array_mut()
            .unwrap()
            .push(json!({
                "id": "synthetic:block:feature#extrude",
                "ordinal": 100,
                "name": "Extrude",
                "suppressed": false,
                "product_definition": "f3d:model:component#block",
                "definition": {
                    "definition": "extrude",
                    "profile": {"kind": "sketch", "value": sketch},
                    "direction": {"kind": "profile_normal"},
                    "start": {"kind": "profile_plane"},
                    "extent": {"kind": "one_sided",
                        "side": {"termination": {"kind": "blind", "length": 5.0}}},
                    "op": "new_body",
                },
            }));
        wire["model"]["product_definitions"] = json!([
            {"id": "f3d:model:component#block", "kind": "part", "label": "Block", "bodies": [body]},
            {"id": "f3d:model:component#layout", "kind": "part", "label": "Layout", "bodies": []},
        ]);
        let mut occurrences = vec![
            placed(0, "block", root(), 0.0),
            placed(1, "layout", root(), 30.0),
        ];
        for key in 0..extra {
            occurrences.push(placed(2 + key as u32, "layout", root(), 60.0));
        }
        wire["model"]["occurrences"] = json!(occurrences);
        let model = Model {
            ir: cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap(),
            ..part
        };
        (model, builder)
    }

    #[test]
    fn a_sketch_another_component_builds_is_carried_in_at_its_place() {
        let source = |model: &Model| {
            serde_json::to_value(&model.ir).unwrap()["model"]["sketches"][0]["placement"]["frame"]
                .clone()
        };
        let (model, builder) = with_layout_sketch(0);
        let before = source(&model);
        let assembly = local_assembly(&model).unwrap().unwrap();
        let block = assembly
            .parts
            .iter()
            .find(|part| part.name == "Block")
            .unwrap();
        // The sketch feature comes along as Block's own, so Block's history has its sketch.
        let wire = serde_json::to_value(&block.ir).unwrap();
        let carried = wire["model"]["features"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["id"] == builder.as_str())
            .expect("the sketch feature is carried in");
        assert_eq!(carried["product_definition"], "f3d:model:component#block");
        // Its plane is where Layout puts it, in Block's coordinates: 30 along x.
        let after = &wire["model"]["sketches"][0]["placement"]["frame"];
        let x = |frame: &Value| frame["origin"]["x"].as_f64().unwrap();
        assert!(
            (x(after) - x(&before) - 30.0).abs() < 1e-9,
            "{before} {after}"
        );
        assert_eq!(after["normal"], before["normal"]);
        assert_eq!(after["u_axis"], before["u_axis"]);

        // Placed twice, Layout has no one place relative to Block: the sketch feature stays
        // out, and the history stops at the feature that uses it, as before.
        let (model, builder) = with_layout_sketch(1);
        let assembly = local_assembly(&model).unwrap().unwrap();
        let block = assembly
            .parts
            .iter()
            .find(|part| part.name == "Block")
            .unwrap();
        assert!(!feature_ids(block).contains(&builder));
    }

    /// Hand-written CADIR: a root rectangle at x = 10 and a vertex-bounded part.
    /// A second, unused profile lies far away, so checking the whole sketch would fail.
    fn repeated_sketch_part(instances: [f64; 2], selection: Value) -> Model {
        let mut wire = serde_json::to_value(cadmpeg_ir::CadIr::empty()).unwrap();
        let empty_arenas = wire["model"].clone();
        let id = |kind: &str, key: &str| format!("synthetic:model:{kind}#{key}");
        let body = id("body", "part");
        let region = id("region", "part");
        let shell = id("shell", "part");
        let sketch = id("sketch", "layout");
        let builder = id("feature", "sketch");
        let mut points = Vec::new();
        let mut vertices = Vec::new();
        let mut members = Vec::new();
        for (i, [x, y, z]) in [[-3.0, -2.0, -8.0], [13.0, 2.0, 8.0]]
            .into_iter()
            .enumerate()
        {
            let point = id("point", &i.to_string());
            let vertex = id("vertex", &i.to_string());
            points.push(json!({"id": point, "position": {"x": x, "y": y, "z": z}}));
            vertices.push(json!({"id": vertex, "point": point}));
            members.push(json!({"kind": "free_vertex", "id": vertex}));
        }
        let mut entities = Vec::new();
        let corners = [[-1.5, -7.1], [1.5, -7.1], [1.5, 7.1], [-1.5, 7.1]];
        let mut profile = Vec::new();
        for (i, [u, v]) in corners.into_iter().enumerate() {
            let [a, b] = corners[(i + 1) % 4];
            let entity = id("sketch-entity", &i.to_string());
            profile.push(json!({"entity": entity, "reversed": false}));
            entities.push(
                json!({"id": entity, "sketch": sketch, "construction": false,
                "geometry": {"kind": "line", "start": {"u": u, "v": v}, "end": {"u": a, "v": b}}}),
            );
        }
        let far = id("sketch-entity", "unused");
        entities.push(json!({"id": far, "sketch": sketch, "construction": false,
            "geometry": {"kind": "circle", "center": {"u": 100.0, "v": 100.0}, "radius": 1.0}}));
        wire["model"] = json!({
            "bodies": [{"id": body, "kind": "general", "regions": [region]}],
            "regions": [{"id": region, "body": body, "shells": [shell]}],
            "shells": [{"id": shell, "region": region, "members": members}],
            "vertices": vertices, "points": points,
            "sketches": [{"id": sketch, "placement": {"kind": "resolved", "frame": {
                "origin": {"x": 10.0, "y": 0.0, "z": 0.0},
                "normal": {"x": 1.0, "y": 0.0, "z": 0.0},
                "u_axis": {"x": 0.0, "y": 1.0, "z": 0.0}}},
                "profiles": [profile, [{"entity": far, "reversed": false}]]}],
            "sketch_entities": entities,
            "features": [
                {"id": builder, "ordinal": 0, "suppressed": false, "definition": {"definition": "sketch", "sketch": {"space": "planar", "sketch": sketch}}},
                {"id": id("feature", "extrude"), "ordinal": 1, "suppressed": false, "product_definition": "f3d:model:component#block",
                 "definition": {"definition": "extrude", "profile": selection,
                    "direction": {"kind": "profile_normal"}, "start": {"kind": "profile_plane"},
                    "extent": {"kind": "one_sided", "side": {"termination": {"kind": "blind", "length": 3.0}}},
                    "op": "new_body"}}
            ],
            "product_definitions": [{"id": "f3d:model:component#block", "kind": "part", "bodies": [body]}],
            "occurrences": [placed(0, "block", root(), instances[0]), placed(1, "block", root(), instances[1])]
        });
        for (arena, rows) in empty_arenas.as_object().unwrap() {
            wire["model"]
                .as_object_mut()
                .unwrap()
                .entry(arena.clone())
                .or_insert_with(|| rows.clone());
        }
        Model {
            ir: cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap(),
            fidelity: None,
            name: "Repeated part".to_owned(),
            source_file: None,
            assembly: None,
            source_step: None,
        }
    }

    fn selected_layout_profile() -> Value {
        json!({"kind": "sketch_profiles", "value": {"sketch": "synthetic:model:sketch#layout", "profiles": [0]}})
    }

    fn slice_repeated_sketch(model: &Model) -> Model {
        let wire = serde_json::to_value(&model.ir).unwrap();
        only(
            model,
            &wire,
            &["synthetic:model:body#part"],
            HistoryOwner::Definition("f3d:model:component#block"),
            "Part".to_owned(),
        )
        .unwrap()
    }

    #[test]
    fn a_repeated_component_carries_a_sketch_at_the_only_fitting_instance() {
        let model = repeated_sketch_part([100.0, 10.0], selected_layout_profile());
        let part = slice_repeated_sketch(&model);
        assert!(feature_ids(&part).contains(&"synthetic:model:feature#sketch".to_owned()));
        let wire = serde_json::to_value(&part.ir).unwrap();
        let frame = &wire["model"]["sketches"][0]["placement"]["frame"];
        assert_eq!(frame["origin"], json!({"x": 0.0, "y": 0.0, "z": 0.0}));
        assert_eq!(frame["u_axis"], json!({"x": 0.0, "y": 1.0, "z": 0.0}));
        assert_eq!(
            wire["model"]["features"][0]["product_definition"],
            "f3d:model:component#block"
        );
    }

    #[test]
    fn a_repeated_component_leaves_out_a_sketch_when_both_instances_fit() {
        let part = slice_repeated_sketch(&repeated_sketch_part(
            [0.0, 10.0],
            selected_layout_profile(),
        ));
        assert!(!feature_ids(&part).contains(&"synthetic:model:feature#sketch".to_owned()));
    }

    #[test]
    fn a_repeated_component_leaves_out_a_sketch_when_neither_instance_fits() {
        let part = slice_repeated_sketch(&repeated_sketch_part(
            [100.0, 200.0],
            selected_layout_profile(),
        ));
        assert!(!feature_ids(&part).contains(&"synthetic:model:feature#sketch".to_owned()));
    }

    #[test]
    fn a_repeated_component_uses_region_boundaries_to_place_a_sketch() {
        for regions in [
            json!([{"region": "loops", "outer": 0, "holes": []}]),
            json!([{"region": "trimmed", "outer_boundary": [
                {"entity": "synthetic:model:sketch-entity#0", "parameter_range": [0.0, 1.0]},
                {"entity": "synthetic:model:sketch-entity#1", "parameter_range": [0.0, 1.0]},
                {"entity": "synthetic:model:sketch-entity#2", "parameter_range": [0.0, 1.0]},
                {"entity": "synthetic:model:sketch-entity#3", "parameter_range": [0.0, 1.0]}]}]),
        ] {
            let selection = json!({"kind": "sketch_regions", "value": {
                "sketch": "synthetic:model:sketch#layout", "regions": regions}});
            let part = slice_repeated_sketch(&repeated_sketch_part([100.0, 10.0], selection));
            assert!(feature_ids(&part).contains(&"synthetic:model:feature#sketch".to_owned()));
        }
    }

    #[test]
    fn a_repeated_component_checks_every_curve_for_a_whole_sketch_selection() {
        let selection = json!({"kind": "sketch", "value": "synthetic:model:sketch#layout"});
        let model = repeated_sketch_part([100.0, 10.0], selection);
        let part = slice_repeated_sketch(&model);
        assert!(!feature_ids(&part).contains(&"synthetic:model:feature#sketch".to_owned()));
        let mut wire = serde_json::to_value(&model.ir).unwrap();
        wire["model"]["sketch_entities"]
            .as_array_mut()
            .unwrap()
            .pop();
        wire["model"]["sketches"][0]["profiles"]
            .as_array_mut()
            .unwrap()
            .pop();
        let model = Model {
            ir: cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap(),
            ..model
        };
        let part = slice_repeated_sketch(&model);
        assert!(feature_ids(&part).contains(&"synthetic:model:feature#sketch".to_owned()));
    }

    #[test]
    fn a_native_selection_of_the_same_sketch_does_not_stop_the_instance_check() {
        let model = repeated_sketch_part([100.0, 10.0], selected_layout_profile());
        let mut wire = serde_json::to_value(&model.ir).unwrap();
        wire["model"]["features"].as_array_mut().unwrap().push(json!({
            "id": "synthetic:model:feature#native", "ordinal": 2, "suppressed": false,
            "product_definition": "f3d:model:component#block",
            "definition": {"definition": "extrude", "profile": {"kind": "sketch_selection",
                "value": {"sketch": "synthetic:model:sketch#layout", "selections": ["native"]}},
                "direction": {"kind": "profile_normal"}, "start": {"kind": "profile_plane"},
                "extent": {"kind": "one_sided", "side": {"termination": {"kind": "blind", "length": 1.0}}},
                "op": "join"}
        }));
        let model = Model {
            ir: cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap(),
            ..model
        };
        let part = slice_repeated_sketch(&model);
        assert!(feature_ids(&part).contains(&"synthetic:model:feature#sketch".to_owned()));
    }

    #[test]
    fn a_component_without_its_sketch_keeps_no_history_of_it() {
        let assembly = local_assembly(&with_history(false)).unwrap().unwrap();
        assert!(assembly.parts[0].ir.model.sketches.is_empty());
        assert!(feature_ids(&assembly.parts[0]).is_empty());
    }

    fn with_owned_history() -> Model {
        let model = with_history(false);
        let mut wire = serde_json::to_value(&model.ir).unwrap();
        wire["model"]["features"][0]["product_definition"] = json!("f3d:model:component#block");
        wire["model"]["features"][0]["outputs"] = json!([]);
        wire["model"]["features"][1]["product_definition"] = json!("f3d:model:component#other");
        wire["model"]["parameters"] = json!([
            {"id": "synthetic:model:parameter#block", "name": "block_length", "expression": "5 mm",
             "product_definition": "f3d:model:component#block"},
            {"id": "synthetic:model:parameter#other", "name": "other_length", "expression": "6 mm",
             "product_definition": "f3d:model:component#other"},
            {"id": "synthetic:model:parameter#root", "name": "root_length", "expression": "7 mm"},
        ]);
        Model {
            ir: cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap(),
            ..model
        }
    }

    #[test]
    fn an_owned_feature_without_outputs_and_parameters_come_with_the_component() {
        let model = with_owned_history();
        for threads in [1, 4] {
            let (assembly, _) = local_components_with_threads(
                &model,
                &BTreeMap::new(),
                threads,
                crate::PartPositions::LastShown,
                &mut Report::default(),
            )
            .unwrap()
            .unwrap();
            let block = &assembly.parts[0];
            assert_eq!(feature_ids(block), [feature_ids(&model)[0].clone()]);
            let wire = serde_json::to_value(&block.ir).unwrap();
            assert_eq!(wire["model"]["parameters"].as_array().unwrap().len(), 1);
            assert_eq!(wire["model"]["parameters"][0]["name"], "block_length");
            // Other consumes Block's sketch, but cannot pull in its owned sketch feature.
            assert_eq!(
                feature_ids(&assembly.parts[1]),
                ["synthetic:copy:feature#other"]
            );
        }
    }

    #[test]
    fn a_body_centers_list_two_components_share_is_left_out_of_both() {
        let model = with_owned_history();
        let mut wire = serde_json::to_value(&model.ir).unwrap();
        let shared = "1:0,0,1;2:0,0,2";
        wire["model"]["features"][0]["source_properties"] = json!({"body_centers": shared});
        wire["model"]["features"][1]["source_properties"] = json!({"body_centers": shared});
        let model = Model {
            ir: cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap(),
            ..model
        };
        let centers = |model: &Model| -> Vec<Value> {
            let wire = serde_json::to_value(&model.ir).unwrap();
            wire["model"]["features"]
                .as_array()
                .unwrap()
                .iter()
                .map(|f| {
                    f.pointer("/source_properties/body_centers")
                        .cloned()
                        .unwrap_or(Value::Null)
                })
                .collect()
        };
        let (assembly, _) = local_components_with_threads(
            &model,
            &BTreeMap::new(),
            1,
            crate::PartPositions::LastShown,
            &mut Report::default(),
        )
        .unwrap()
        .unwrap();
        for part in &assembly.parts {
            assert!(
                centers(part).iter().all(Value::is_null),
                "{:?}",
                centers(part)
            );
        }

        // A list only one component carries is that component's, and stays.
        wire["model"]["features"][1]["source_properties"] = json!({"body_centers": "1:0,0,3"});
        let model = Model {
            ir: cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap(),
            ..model
        };
        let (assembly, _) = local_components_with_threads(
            &model,
            &BTreeMap::new(),
            1,
            crate::PartPositions::LastShown,
            &mut Report::default(),
        )
        .unwrap()
        .unwrap();
        let kept: Vec<Value> = assembly.parts.iter().flat_map(centers).collect();
        assert!(kept.contains(&json!("1:0,0,1;2:0,0,2")), "{kept:?}");
        assert!(kept.contains(&json!("1:0,0,3")), "{kept:?}");
    }

    #[test]
    fn another_components_feature_is_not_seeded_by_outputs_or_followed_from_a_sketch() {
        let model = with_owned_history();
        let mut wire = serde_json::to_value(&model.ir).unwrap();
        let sketch = wire["model"]["sketches"][0]["id"].clone();
        wire["model"]["product_definitions"][0]["sketches"] = json!([sketch]);
        wire["model"]["features"][0]["product_definition"] = json!("f3d:model:component#other");
        wire["model"]["features"][0]["outputs"] = json!([wire["model"]["bodies"][0]["id"]]);
        let model = Model {
            ir: cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap(),
            ..model
        };
        let assembly = local_assembly(&model).unwrap().unwrap();
        assert!(feature_ids(&assembly.parts[0]).is_empty());
        assert_eq!(assembly.parts[0].ir.model.sketches.len(), 1);
    }

    fn history_sphere() -> Value {
        json!({"id": "synthetic:model:feature#sphere", "ordinal": 100, "suppressed": false,
            "definition": {"definition": "sphere", "center": {"x": 0.0, "y": 0.0, "z": 0.0},
                "radius": 1.0, "op": "new_body"}})
    }

    #[test]
    fn timeline_predecessors_do_not_bring_another_parts_history() {
        let model = with_owned_history();
        let mut wire = serde_json::to_value(&model.ir).unwrap();
        let mut predecessor = history_sphere();
        predecessor["ordinal"] = json!(0);
        wire["model"]["features"][0]["ordinal"] = json!(101);
        wire["model"]["features"][0]["dependencies"] = json!([predecessor["id"]]);
        wire["model"]["features"][0]["regeneration_parent"] = predecessor["id"].clone();
        wire["model"]["features"]
            .as_array_mut()
            .unwrap()
            .push(predecessor);
        let model = Model {
            ir: cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap(),
            ..model
        };
        let assembly = local_assembly(&model).unwrap().unwrap();
        assert_eq!(
            feature_ids(&assembly.parts[0]),
            [feature_ids(&model)[0].clone()]
        );
    }

    #[test]
    fn root_owned_history_goes_only_to_the_loose_body_part_when_it_exists() {
        for loose in [false, true] {
            let model = with_owned_history();
            let mut wire = serde_json::to_value(&model.ir).unwrap();
            let feature = history_sphere();
            wire["model"]["features"]
                .as_array_mut()
                .unwrap()
                .push(feature.clone());
            if loose {
                let mut body = wire["model"]["bodies"][0].clone();
                body["id"] = json!("synthetic:model:body#loose");
                wire["model"]["bodies"].as_array_mut().unwrap().push(body);
            }
            let model = Model {
                ir: cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap(),
                ..model
            };
            for threads in [1, 4] {
                let (assembly, _) = local_components_with_threads(
                    &model,
                    &BTreeMap::new(),
                    threads,
                    crate::PartPositions::LastShown,
                    &mut Report::default(),
                )
                .unwrap()
                .unwrap();
                assert!(assembly
                    .parts
                    .iter()
                    .take(2)
                    .all(|part| !feature_ids(part)
                        .contains(&feature["id"].as_str().unwrap().to_owned())));
                assert_eq!(assembly.own_part.is_some(), loose);
                if let Some(own) = assembly.own_part {
                    assert_eq!(
                        feature_ids(&assembly.parts[own]),
                        [feature["id"].as_str().unwrap()]
                    );
                    let wire = serde_json::to_value(&assembly.parts[own].ir).unwrap();
                    assert_eq!(wire["model"]["parameters"].as_array().unwrap().len(), 1);
                    assert_eq!(wire["model"]["parameters"][0]["name"], "root_length");
                }
            }
        }
    }

    #[test]
    fn a_sub_assemblys_own_bodies_keep_its_owned_history() {
        let model = nested(true, None);
        let mut wire = serde_json::to_value(&model.ir).unwrap();
        let mut feature = history_sphere();
        feature["product_definition"] = json!("f3d:model:component#frame");
        wire["model"]["features"] = json!([feature]);
        let model = Model {
            ir: cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap(),
            ..model
        };
        for threads in [1, 4] {
            let (assembly, _) = local_components_with_threads(
                &model,
                &BTreeMap::new(),
                threads,
                crate::PartPositions::LastShown,
                &mut Report::default(),
            )
            .unwrap()
            .unwrap();
            let inner = assembly.parts[0].assembly.as_ref().unwrap();
            assert!(feature_ids(&inner.parts[0]).is_empty());
            assert_eq!(
                feature_ids(&inner.parts[inner.own_part.unwrap()]),
                ["synthetic:model:feature#sphere"]
            );
        }
    }

    #[test]
    fn an_unknown_scope_does_not_seed_root_owned_history() {
        let model = with_history(false);
        let document = serde_json::to_value(&model.ir).unwrap();
        let body = model.ir.model.bodies[0].id.to_string();
        let part = only(
            &model,
            &document,
            &[&body],
            HistoryOwner::Unknown,
            "Block".to_owned(),
        )
        .unwrap();
        assert!(feature_ids(&part).is_empty());
    }

    #[test]
    fn a_body_restated_alone_keeps_no_history() {
        let model = with_history(true);
        let body = model.ir.model.bodies[0].id.to_string();
        let part = crate::with_bodies(&model, &[body.as_str()], "Block".to_owned()).unwrap();
        assert!(feature_ids(&part).is_empty());
        assert!(part.ir.model.sketches.is_empty());
    }

    #[test]
    fn a_component_that_is_empty_in_the_source_is_placed_empty() {
        // Every body belongs to `Block`, so `Carriage`, naming none and holding none, is
        // empty rather than unattributed.
        let model = with_components(true);
        let mut wire = serde_json::to_value(&model.ir).unwrap();
        wire["model"]["product_definitions"]
            .as_array_mut()
            .unwrap()
            .push(
                json!({"id": "f3d:model:component#carriage", "kind": "part", "label": "Carriage"}),
            );
        wire["model"]["occurrences"]
            .as_array_mut()
            .unwrap()
            .push(json!({
                "id": "f3d:model:occurrence#2",
                "prototype": {"scope": "local", "definition": "f3d:model:component#carriage"},
                "parent": root(), "ordinal": 2,
                "transform": [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0]],
                "scale": [1.0, 1.0, 1.0],
            }));
        let model = Model {
            ir: cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap(),
            ..model
        };
        let assembly = local_assembly(&model).unwrap().unwrap();
        assert_eq!(assembly.parts.len(), 2);
        assert_eq!(assembly.parts[1].name, "Carriage");
        assert!(assembly.parts[1].ir.model.bodies.is_empty());
        assert_eq!(assembly.placements.len(), 3);
        let counts = assembly.completeness();
        assert_eq!(
            (counts.placed, counts.placed_empty, counts.placements()),
            (2, 1, 3)
        );
        assert!(!counts.is_complete());
    }

    /// cadmpeg's example cube, owned by component `Block`, placed twice inside a `Frame`
    /// placed 100 mm along X. `frame_owns` gives the frame the cube too; `rows` replaces the
    /// occurrences.
    fn nested(frame_owns: bool, rows: Option<Value>) -> Model {
        let part = crate::read(
            Format::InventorPart,
            &fixture("synthetic_cube.ipt"),
            &mut Report::default(),
        )
        .unwrap();
        let mut wire = serde_json::to_value(&part.ir).unwrap();
        let body = wire["model"]["bodies"][0]["id"].clone();
        let frame_bodies = if frame_owns { json!([body]) } else { json!([]) };
        wire["model"]["product_definitions"] = json!([
            {"id": "f3d:model:component#frame", "kind": "part", "label": "Frame", "bodies": frame_bodies},
            {"id": "f3d:model:component#block", "kind": "part", "label": "Block", "bodies": [body]},
        ]);
        wire["model"]["occurrences"] = rows.unwrap_or_else(|| {
            json!([
                placed(0, "frame", root(), 100.0),
                placed(1, "block", inside(0), 0.0),
                placed(2, "block", inside(0), 30.0),
            ])
        });
        Model {
            ir: cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap(),
            ..part
        }
    }

    /// `model` with these assembly joints.
    fn with_joints(model: Model, joints: Value) -> Model {
        let mut wire = serde_json::to_value(&model.ir).unwrap();
        wire["model"]["assembly_joints"] = joints;
        Model {
            ir: cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap(),
            ..model
        }
    }

    /// One side of a joint: the design's root, or an occurrence (with nested occurrence keys).
    fn connector(occurrence: Option<(u32, &[&str])>) -> Value {
        let operand = match occurrence {
            None => json!({"container": {"container": "root"}, "object": "root:origin"}),
            Some((key, nested)) => json!({
                "container": {"container": "occurrence", "occurrence": format!("f3d:model:occurrence#{key}")},
                "object": key.to_string(),
                "subelements": nested,
            }),
        };
        json!({"operand": operand, "frame": IDENTITY, "detached": false})
    }

    /// A connector on occurrence `key` (with nested occurrence keys) with its frame `frame`.
    fn connector_at(key: u32, nested: &[&str], frame: Placement3x4) -> Value {
        let mut out = connector(Some((key, nested)));
        out["frame"] = json!(frame);
        out
    }

    /// A placement at `origin`, turned by `turn` radians about z.
    fn at(origin: [f64; 3], turn: f64) -> Placement3x4 {
        let (s, c) = turn.sin_cos();
        [
            [c, -s, 0.0, origin[0]],
            [s, c, 0.0, origin[1]],
            [0.0, 0.0, 1.0, origin[2]],
        ]
    }

    /// `frame` turned half a turn about its x axis, as a joint's frame faces the other.
    fn facing(frame: Placement3x4) -> Placement3x4 {
        compose(&frame, &FACING)
    }

    /// Block 2's placement in the world (and the joint, and the notes) once one joint `kind`
    /// joins block 2 (stored at x = 130) to block 1 (placed at x = 100 in the world) with
    /// these connectors, block 2's first.
    fn joined(kind: Value, a: Value, b: Value) -> (Placement3x4, Joint, Vec<String>) {
        let model = with_joints(nested(false, None), json!([joint(1, kind, a, b)]));
        let (assembly, notes) = local_components(&model).unwrap().unwrap();
        let world = world_at(&assembly, &[0, 1]).unwrap();
        (world, assembly.joints[0].clone(), notes)
    }

    fn near(a: &Placement3x4, b: &Placement3x4) -> bool {
        same(a, b)
    }

    #[test]
    fn a_joint_from_an_occurrence_to_one_inside_it_moves_the_inner_one() {
        // The frame (stored at x = 100) joined rigidly to block 2 inside it, the frame's side
        // first, as a rigid group holds its members to its first: the frame keeps its stored
        // placement and block 2 moves to meet it. Moving the frame would move block 2 with it.
        let row = joint(
            1,
            json!({"kind": "fixed"}),
            connector_at(0, &[], at([5.0, 0.0, 0.0], 0.0)),
            connector_at(2, &[], at([0.0, 0.0, 0.0], 0.0)),
        );
        let model = with_joints(nested(false, None), json!([row]));
        let (assembly, _) = local_components(&model).unwrap().unwrap();
        let joint = &assembly.joints[0];
        assert_eq!((joint.a.clone(), joint.b.clone()), (vec![0], vec![0, 1]));
        let frame = world_at(&assembly, &[0]).unwrap();
        assert!(near(&frame, &at([100.0, 0.0, 0.0], 0.0)), "{frame:?}");
        let block = world_at(&assembly, &[0, 1]).unwrap();
        assert!(near(
            &compose(&block, &joint.frame_b),
            &compose(&compose(&frame, &joint.frame_a), &joint.meeting())
        ));
    }

    #[test]
    fn a_copy_held_by_its_own_joint_is_not_held_again_through_a_part_inside_it() {
        // A rigid group holding a frame placed twice, and a block inside that frame, to the
        // design: the frame's own joint holds it, and the block's joint, which could only hold
        // the whole frame, is left out instead of holding it a second time.
        let model = nested(
            false,
            Some(json!([
                placed(0, "frame", root(), 0.0),
                placed(1, "frame", root(), 100.0),
                placed(2, "block", inside(0), 30.0),
                placed(3, "block", inside(1), 30.0),
            ])),
        );
        let fixed = json!({"kind": "fixed"});
        let model = with_joints(
            model,
            json!([
                joint(1, fixed.clone(), connector(None), connector(Some((3, &[])))),
                joint(2, fixed, connector(None), connector(Some((1, &[])))),
            ]),
        );
        let (assembly, left_out) = local_components(&model).unwrap().unwrap();
        let sides: Vec<_> = assembly
            .joints
            .iter()
            .map(|j| (j.a.clone(), j.b.clone()))
            .collect();
        assert_eq!(sides, [(vec![], vec![1])]);
        assert_eq!(
            left_out,
            [
                "joint 1: it is left out: the copy of the component placed more than once that \
              its part is in is held by joint 2 itself"
            ]
        );
    }

    #[test]
    fn an_aligned_joint_meets_its_frames_z_to_z() {
        // The same rigid joint with both frames pointing up: face to face, block 2 is turned
        // over to meet block 1; aligned, it is not turned.
        let placed_with = |flip: bool| {
            let row = joint(
                1,
                json!({"kind": "fixed"}),
                connector_at(2, &[], at([0.0, 0.0, 0.0], 0.0)),
                connector_at(1, &[], at([10.0, 0.0, 0.0], 0.0)),
            );
            // The joint as the file gives it, then the parts placed by it with each sense.
            let model = with_joints(nested(false, None), json!([row]));
            let mut joint = local_components(&model).unwrap().unwrap().0.joints[0].clone();
            joint.flip = flip;
            let mut fresh = local_components(&nested(false, None)).unwrap().unwrap().0;
            fresh.joints = vec![joint];
            let notes = place_by_joints(&mut fresh, &BTreeSet::new());
            assert!(notes.is_empty(), "{notes:?}");
            world_at(&fresh, &[0, 1]).unwrap()
        };
        assert!(near(&placed_with(true), &at([110.0, 0.0, 0.0], 0.0)));
        assert!(near(
            &placed_with(false),
            &compose(&at([110.0, 0.0, 0.0], 0.0), &FACING)
        ));
    }

    #[test]
    fn a_joint_takes_its_flip_from_its_operands_frame_sense() {
        let flip = |sense: Option<&str>| {
            let mut row = joint(
                1,
                json!({"kind": "fixed"}),
                connector_at(2, &[], at([0.0, 0.0, 0.0], 0.0)),
                connector_at(1, &[], at([10.0, 0.0, 0.0], 0.0)),
            );
            if let Some(sense) = sense {
                row["operands"]["frame_sense"] = json!(sense);
            }
            let model = with_joints(nested(false, None), json!([row]));
            local_components(&model).unwrap().unwrap().0.joints[0].flip
        };
        assert!(flip(Some("aligned")));
        assert!(!flip(Some("opposed")));
        assert!(!flip(None));
    }

    #[test]
    fn a_joint_left_out_is_noted_with_its_kind_for_the_writer() {
        let row = joint(
            1,
            json!({"kind": "cylindrical"}),
            connector_at(2, &[], IDENTITY),
            connector_at(1, &[], IDENTITY),
        );
        let model = with_joints(nested(false, None), json!([row]));
        let (assembly, notes) = local_components(&model).unwrap().unwrap();
        assert!(assembly.joints.is_empty());
        assert_eq!(
            assembly.joint_notes,
            [JointNote {
                text: "joint 1: cylindrical joints are not converted yet".to_owned(),
                family: Some("cylindrical".to_owned()),
            }]
        );
        assert_eq!(notes, ["joint 1: cylindrical joints are not converted yet"]);
    }

    #[test]
    fn a_joint_is_aligned_only_when_its_source_says_so() {
        let sensed = |sense: &str| json!({"operands": {"arity": "pair", "frame_sense": sense}});
        assert!(frame_aligned(&sensed("aligned")));
        assert!(!frame_aligned(&sensed("opposed")));
        assert!(!frame_aligned(&json!({"operands": {"arity": "pair"}})));
        // Only the operands carry the sense: a stray top-level field is not read.
        assert!(!frame_aligned(&json!({"frame_sense": "aligned"})));
    }

    #[test]
    fn a_copys_joint_to_a_part_inside_it_is_the_components_own() {
        // A frame placed twice (x = 0 and x = 100), each copy's rigid group holding its
        // block: the second copy's joint is the component's own, written through the first
        // copy, so it is one joint with the first's and the second copy stays at x = 100.
        let model = nested(
            false,
            Some(json!([
                placed(0, "frame", root(), 0.0),
                placed(1, "frame", root(), 100.0),
                placed(2, "block", inside(0), 30.0),
                placed(3, "block", inside(1), 30.0),
            ])),
        );
        let fixed = json!({"kind": "fixed"});
        let model = with_joints(
            model,
            json!([
                joint(
                    1,
                    fixed.clone(),
                    connector(Some((0, &[]))),
                    connector(Some((2, &[])))
                ),
                joint(
                    2,
                    fixed,
                    connector(Some((1, &[]))),
                    connector(Some((3, &[])))
                ),
            ]),
        );
        let (assembly, _) = local_components(&model).unwrap().unwrap();
        let sides: Vec<_> = assembly
            .joints
            .iter()
            .map(|j| (j.a.clone(), j.b.clone()))
            .collect();
        assert_eq!(sides, [(vec![0], vec![0, 0])]);
        let second = world_at(&assembly, &[1]).unwrap();
        assert!(near(&second, &at([100.0, 0.0, 0.0], 0.0)), "{second:?}");
    }

    #[test]
    fn an_open_joint_keeps_the_partner_with_a_non_identity_stored_world_pose() {
        // The second holder position reverses which local pose is identity;
        // the same partner must stay fixed in world coordinates in both cases.
        for holder_x in [0.0, 20.0] {
            for reverse in [false, true] {
                let model = nested(
                    false,
                    Some(json!([
                        placed(0, "frame", root(), holder_x),
                        placed(1, "block", inside(0), 20.0 - holder_x),
                        placed(2, "block", inside(0), -holder_x),
                    ])),
                );
                let mut assembly = local_components(&model).unwrap().unwrap().0;
                let mut joint = Joint {
                    a: vec![0, 0],
                    b: vec![0, 1],
                    frame_a: at([7.0, 0.0, 0.0], 0.0),
                    frame_b: IDENTITY,
                    motion: Motion::Rigid,
                    flip: true,
                    suppressed: false,
                };
                if reverse {
                    std::mem::swap(&mut joint.a, &mut joint.b);
                    std::mem::swap(&mut joint.frame_a, &mut joint.frame_b);
                }
                assembly.joints = vec![joint];
                assembly.remember_placements();
                let notes = place_by_joints(&mut assembly, &BTreeSet::new());
                assert!(notes.is_empty(), "{notes:?}");
                for (path, x) in [(vec![0], holder_x), (vec![0, 0], 20.0), (vec![0, 1], 27.0)] {
                    assert!(
                        near(
                            &world_at(&assembly, &path).unwrap(),
                            &at([x, 0.0, 0.0], 0.0)
                        ),
                        "{path:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn a_joint_carries_a_connected_holder_below_an_identity_organiser() {
        // The inner holder has a stored local pose. Its two blocks are connected;
        // the other block under the organiser has no connection to that holder.
        let model = with_mid(nested(
            false,
            Some(json!([
                placed(0, "frame", root(), 0.0),
                placed(1, "mid", inside(0), 10.0),
                placed(2, "block", inside(1), 2.0),
                placed(3, "block", inside(1), 7.0),
                placed(4, "block", inside(0), 40.0),
                placed(5, "block", root(), 20.0),
            ])),
        ));
        let mut assembly = local_components(&model).unwrap().unwrap().0;
        let rigid = |a, b, frame_a| Joint {
            a,
            b,
            frame_a,
            frame_b: IDENTITY,
            motion: Motion::Rigid,
            flip: true,
            suppressed: false,
        };
        assembly.joints = vec![
            rigid(vec![0, 0, 0], vec![0, 0, 1], at([5.0, 0.0, 0.0], 0.0)),
            rigid(vec![0, 0, 0], vec![1], at([-50.0, 0.0, 0.0], 0.0)),
        ];
        assembly.remember_placements();
        let notes = place_by_joints(&mut assembly, &BTreeSet::new());
        assert!(notes.is_empty(), "{notes:?}");
        for (path, x) in [
            (vec![0], 0.0),
            (vec![0, 0], 68.0),
            (vec![0, 0, 0], 70.0),
            (vec![0, 0, 1], 75.0),
            (vec![0, 1], 40.0),
            (vec![1], 20.0),
        ] {
            assert!(
                near(
                    &world_at(&assembly, &path).unwrap(),
                    &at([x, 0.0, 0.0], 0.0)
                ),
                "{path:?}"
            );
        }
        assert!(assembly.joints.iter().all(|j| !j.suppressed));
    }

    #[test]
    fn a_joint_carries_a_holder_through_its_only_origin_body_connector() {
        let model = with_mid(nested(
            false,
            Some(json!([
                placed(0, "frame", root(), 0.0),
                placed(1, "mid", inside(0), 10.0),
                placed(2, "block", inside(1), 0.0),
                placed(3, "block", inside(1), 5.0),
                placed(4, "block", inside(0), 40.0),
                placed(5, "block", root(), 20.0),
            ])),
        ));
        let mut assembly = local_components(&model).unwrap().unwrap().0;
        assembly.joints = vec![Joint {
            a: vec![0, 0, 0],
            b: vec![1],
            frame_a: at([-50.0, 0.0, 0.0], 0.0),
            frame_b: IDENTITY,
            motion: Motion::Rigid,
            flip: true,
            suppressed: false,
        }];
        let notes = place_by_joints(&mut assembly, &BTreeSet::new());
        assert!(notes.is_empty(), "{notes:?}");
        for (path, x) in [
            (vec![0], 0.0),
            (vec![0, 0], 70.0),
            (vec![0, 0, 0], 70.0),
            (vec![0, 0, 1], 75.0),
            (vec![0, 1], 40.0),
            (vec![1], 20.0),
        ] {
            assert!(
                near(
                    &world_at(&assembly, &path).unwrap(),
                    &at([x, 0.0, 0.0], 0.0)
                ),
                "{path:?}"
            );
        }
    }

    #[test]
    fn an_unrelated_internal_joint_does_not_bind_an_identity_organiser() {
        let model = nested(
            false,
            Some(json!([
                placed(0, "frame", root(), 0.0),
                placed(1, "block", inside(0), 0.0),
                placed(2, "block", inside(0), 10.0),
                placed(3, "block", inside(0), 20.0),
                placed(4, "block", root(), 50.0),
            ])),
        );
        let mut assembly = local_components(&model).unwrap().unwrap().0;
        let rigid = |a, b, frame_a| Joint {
            a,
            b,
            frame_a,
            frame_b: IDENTITY,
            motion: Motion::Rigid,
            flip: true,
            suppressed: false,
        };
        assembly.joints = vec![
            rigid(vec![0, 0], vec![1], IDENTITY),
            rigid(vec![0, 1], vec![0, 2], at([10.0, 0.0, 0.0], 0.0)),
        ];
        let notes = place_by_joints(&mut assembly, &BTreeSet::new());
        assert!(notes.is_empty(), "{notes:?}");
        for (path, x) in [
            (vec![0], 0.0),
            (vec![0, 0], 50.0),
            (vec![0, 1], 10.0),
            (vec![0, 2], 20.0),
            (vec![1], 50.0),
        ] {
            assert!(
                near(
                    &world_at(&assembly, &path).unwrap(),
                    &at([x, 0.0, 0.0], 0.0)
                ),
                "{path:?}"
            );
        }
    }

    #[test]
    fn conflicting_outside_joints_do_not_promote_a_smaller_posed_holder() {
        let model = with_mid(nested(
            false,
            Some(json!([
                placed(0, "frame", root(), 0.0),
                placed(1, "mid", inside(0), 0.0),
                placed(2, "block", inside(1), 5.0),
                placed(3, "block", inside(1), 15.0),
                placed(4, "block", inside(0), 40.0),
            ])),
        ));
        let mut assembly = local_components(&model).unwrap().unwrap().0;
        let to_root = |part, x| Joint {
            a: vec![],
            b: part,
            frame_a: at([x, 0.0, 0.0], 0.0),
            frame_b: IDENTITY,
            motion: Motion::Rigid,
            flip: true,
            suppressed: false,
        };
        assembly.joints = vec![to_root(vec![0, 0, 0], 100.0), to_root(vec![0, 0, 1], 200.0)];
        let notes = place_by_joints(
            &mut assembly,
            &BTreeSet::from([vec![0, 0, 0], vec![0, 0, 1]]),
        );
        assert_eq!(notes.len(), 2);
        assert!(assembly.joints.iter().all(|joint| joint.suppressed));
        for (path, x) in [
            (vec![0], 0.0),
            (vec![0, 0], 0.0),
            (vec![0, 0, 0], 5.0),
            (vec![0, 0, 1], 15.0),
            (vec![0, 1], 40.0),
        ] {
            assert!(
                near(
                    &world_at(&assembly, &path).unwrap(),
                    &at([x, 0.0, 0.0], 0.0)
                ),
                "{path:?}"
            );
        }
    }

    #[test]
    fn a_child_pose_keeps_the_layout_of_its_identity_holder() {
        let model = nested(
            false,
            Some(json!([
                placed(0, "frame", root(), 0.0),
                placed(1, "block", inside(0), 5.0),
                placed(2, "block", inside(0), 30.0),
                placed(3, "block", root(), 100.0),
            ])),
        );
        let mut assembly = local_components(&model).unwrap().unwrap().0;
        assembly.joints = vec![Joint {
            a: vec![0, 0],
            b: vec![1],
            frame_a: IDENTITY,
            frame_b: IDENTITY,
            motion: Motion::Rigid,
            flip: true,
            suppressed: false,
        }];
        let notes = place_by_joints(&mut assembly, &BTreeSet::from([vec![0, 1]]));
        assert!(notes.is_empty(), "{notes:?}");
        for (path, x) in [
            (vec![0], 95.0),
            (vec![0, 0], 100.0),
            (vec![0, 1], 125.0),
            (vec![1], 100.0),
        ] {
            assert!(
                near(
                    &world_at(&assembly, &path).unwrap(),
                    &at([x, 0.0, 0.0], 0.0)
                ),
                "{path:?}"
            );
        }
    }

    #[test]
    fn a_closing_joint_to_the_root_does_not_override_a_child_pose() {
        for open_first in [false, true] {
            let model = nested(
                false,
                Some(json!([
                    placed(0, "frame", root(), 5.0),
                    placed(1, "block", inside(0), 15.0),
                ])),
            );
            let mut assembly = local_components(&model).unwrap().unwrap().0;
            let to_root = |part, x| Joint {
                a: vec![],
                b: part,
                frame_a: at([x, 0.0, 0.0], 0.0),
                frame_b: IDENTITY,
                motion: Motion::Rigid,
                flip: true,
                suppressed: false,
            };
            let open = to_root(vec![0, 0], 10.0);
            let closed = to_root(vec![0], 5.0);
            assembly.joints = if open_first {
                vec![open, closed]
            } else {
                vec![closed, open]
            };
            let notes = place_by_joints(&mut assembly, &BTreeSet::from([vec![0], vec![0, 0]]));
            assert_eq!(notes.len(), 1);
            assert!(assembly.joints[usize::from(!open_first)].suppressed);
            for (path, x) in [(vec![0], 5.0), (vec![0, 0], 20.0)] {
                assert!(
                    near(
                        &world_at(&assembly, &path).unwrap(),
                        &at([x, 0.0, 0.0], 0.0)
                    ),
                    "{path:?}"
                );
            }
        }
    }

    #[test]
    fn a_closing_group_places_an_isolated_child_pose_in_either_joint_order() {
        for open_first in [false, true] {
            let model = nested(
                false,
                Some(json!([
                    placed(0, "frame", root(), 100.0),
                    placed(1, "block", inside(0), 5.0),
                    placed(2, "block", inside(0), 20.0),
                    placed(3, "block", inside(0), 30.0),
                ])),
            );
            let mut assembly = local_components(&model).unwrap().unwrap().0;
            let rigid = |a, b, frame_a| Joint {
                a,
                b,
                frame_a,
                frame_b: IDENTITY,
                motion: Motion::Rigid,
                flip: true,
                suppressed: false,
            };
            let open = rigid(vec![0, 0], vec![0, 1], IDENTITY);
            let closed = rigid(vec![0, 1], vec![0, 2], at([10.0, 0.0, 0.0], 0.0));
            assembly.joints = if open_first {
                vec![open, closed]
            } else {
                vec![closed, open]
            };
            let notes = place_by_joints(&mut assembly, &BTreeSet::from([vec![0], vec![0, 0]]));
            assert!(notes.is_empty(), "{notes:?}");
            for (path, x) in [
                (vec![0], 100.0),
                (vec![0, 0], 120.0),
                (vec![0, 1], 120.0),
                (vec![0, 2], 130.0),
            ] {
                assert!(
                    near(
                        &world_at(&assembly, &path).unwrap(),
                        &at([x, 0.0, 0.0], 0.0)
                    ),
                    "{path:?}"
                );
            }
            assert!(assembly.joints.iter().all(|joint| !joint.suppressed));
        }
    }

    #[test]
    fn an_outer_joint_places_an_unposed_holder_with_its_internal_joint() {
        for (posed, q, held_at) in [
            (false, 35.0, None),
            (false, 55.0, None),
            (true, 35.0, None),
            (false, 35.0, Some(0.0)),
            (false, 35.0, Some(20.0)),
        ] {
            for outer_first in [false, true] {
                let mut holder = placed(0, "frame", root(), 0.0);
                holder["native_ref"] = json!("f3d:xref:reference#0");
                let model = nested(
                    false,
                    Some(json!([
                        holder,
                        placed(1, "block", inside(0), 5.0),
                        placed(2, "block", inside(0), q),
                        placed(3, "block", root(), 0.0),
                    ])),
                );
                let internal = joint(
                    1,
                    json!({"kind": "fixed"}),
                    connector_at(1, &[], at([30.0, 0.0, 0.0], 0.0)),
                    connector_at(2, &[], facing(IDENTITY)),
                );
                let outer = joint(
                    2,
                    json!({"kind": "fixed"}),
                    connector_at(3, &[], at([100.0, 0.0, 0.0], 0.0)),
                    connector_at(1, &[], facing(IDENTITY)),
                );
                // A is already placed before the outer joint reaches the holder.
                let anchor = joint(
                    3,
                    json!({"kind": "fixed"}),
                    connector(None),
                    connector_at(3, &[], facing(IDENTITY)),
                );
                let mut rows = if outer_first {
                    json!([anchor, outer, internal])
                } else {
                    json!([anchor, internal, outer])
                };
                if let Some(x) = held_at {
                    // Ground the holder, or place it with an earlier joint.
                    rows.as_array_mut().unwrap().insert(
                        1,
                        joint(
                            4,
                            json!({"kind": "fixed"}),
                            connector(None),
                            connector_at(0, &[], facing(at([-x, 0.0, 0.0], 0.0))),
                        ),
                    );
                }
                let mut wire = serde_json::to_value(&with_joints(model.clone(), rows).ir).unwrap();
                if posed {
                    wire["native"]["f3d"]["xref_references"] = json!([
                        {"id": "f3d:xref:reference#0", "current_pose": {"carrier_record_index": 7}}
                    ]);
                }
                let model = Model {
                    ir: cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap(),
                    ..model
                };
                let (assembly, notes) = local_components(&model).unwrap().unwrap();
                let held = posed || held_at.is_some();
                let offset = held_at.unwrap_or(if posed { 0.0 } else { 130.0 - q });
                for (path, x) in [
                    (vec![0], offset),
                    (vec![0, 0], offset + q - 30.0),
                    (vec![0, 1], offset + q),
                ] {
                    assert!(
                        near(
                            &world_at(&assembly, &path).unwrap(),
                            &at([x, 0.0, 0.0], 0.0)
                        ),
                        "posed={posed}, outer_first={outer_first}, path={path:?}"
                    );
                }
                let start = 1 + usize::from(held_at.is_some());
                let outer_index = start + usize::from(!outer_first);
                assert_eq!(assembly.joints[outer_index].suppressed, held);
                assert!(!assembly.joints[start + usize::from(outer_first)].suppressed);
                assert_eq!(notes.len(), usize::from(held), "{notes:?}");
                for joint in assembly.joints.iter().filter(|joint| !joint.suppressed) {
                    let a = world_at(&assembly, &joint.a).unwrap();
                    let b = world_at(&assembly, &joint.b).unwrap();
                    assert!(near(
                        &compose(&compose(&a, &joint.frame_a), &joint.meeting()),
                        &compose(&b, &joint.frame_b)
                    ));
                }
                if held {
                    assert!(notes[0].contains("its parts are placed by other joints"));
                }
            }
        }
    }

    #[test]
    fn joints_that_place_the_parts_of_a_holder_apart_do_not_move_the_holder() {
        // An identity holder whose two blocks are each joined to the origin at a place of its
        // own: the holder is an organiser, so each joint places its block and the holder stays.
        let mut holder = placed(0, "frame", root(), 0.0);
        holder["native_ref"] = json!("f3d:xref:reference#0");
        let model = nested(
            false,
            Some(json!([
                holder,
                placed(1, "block", inside(0), 5.0),
                placed(2, "block", inside(0), 60.0),
            ])),
        );
        let to_origin = |id: u32, key: u32, x: f64| {
            let mut origin = connector(None);
            origin["frame"] = json!(at([x, 0.0, 0.0], 0.0));
            joint(
                id,
                json!({"kind": "fixed"}),
                origin,
                connector_at(key, &[], facing(IDENTITY)),
            )
        };
        let rows = json!([to_origin(1, 1, 100.0), to_origin(2, 2, 200.0)]);
        let (assembly, notes) = local_components(&with_joints(model, rows))
            .unwrap()
            .unwrap();
        assert!(notes.is_empty(), "{notes:?}");
        for (path, x) in [(vec![0], 0.0), (vec![0, 0], 100.0), (vec![0, 1], 200.0)] {
            let world = world_at(&assembly, &path).unwrap();
            assert!(near(&world, &at([x, 0.0, 0.0], 0.0)), "{path:?}: {world:?}");
        }
        assert!(assembly.joints.iter().all(|joint| !joint.suppressed));
    }

    #[test]
    fn a_joint_to_a_part_of_a_holder_leaves_the_parts_nothing_binds_to_it() {
        // An identity holder of two blocks and no joint between them: the joint from the
        // origin to the first places that block alone, and the second stays where it is.
        let mut holder = placed(0, "frame", root(), 0.0);
        holder["native_ref"] = json!("f3d:xref:reference#0");
        let model = nested(
            false,
            Some(json!([
                holder,
                placed(1, "block", inside(0), 5.0),
                placed(2, "block", inside(0), 60.0),
            ])),
        );
        let mut origin = connector(None);
        origin["frame"] = json!(at([100.0, 0.0, 0.0], 0.0));
        let rows = json!([joint(
            1,
            json!({"kind": "fixed"}),
            origin,
            connector_at(1, &[], facing(IDENTITY)),
        )]);
        let (assembly, notes) = local_components(&with_joints(model, rows))
            .unwrap()
            .unwrap();
        assert!(notes.is_empty(), "{notes:?}");
        for (path, x) in [(vec![0], 0.0), (vec![0, 0], 100.0), (vec![0, 1], 60.0)] {
            let world = world_at(&assembly, &path).unwrap();
            assert!(near(&world, &at([x, 0.0, 0.0], 0.0)), "{path:?}: {world:?}");
        }
    }

    #[test]
    fn a_holder_whose_joint_to_another_holds_as_placed_is_not_moved_by_others() {
        // Two identity holders of posed blocks, joined to each other by a joint that holds as
        // the file places them. A joint from the first to the second's block that cannot hold
        // is kept suppressed rather than moving the first holder to meet it.
        let model = nested(
            false,
            Some(json!([
                placed(0, "frame", root(), 0.0),
                posed_child(
                    placed(1, "block", inside(0), 5.0),
                    "f3d:Design/BulkStream.dat:design-component-occurrence#11"
                ),
                placed(2, "frame", root(), 0.0),
                posed_child(
                    placed(3, "block", inside(2), 70.0),
                    "f3d:Design/BulkStream.dat:design-component-occurrence#12"
                ),
            ])),
        );
        let model = with_current_poses(
            model,
            &[
                "f3d:Design/BulkStream.dat:design-component-occurrence#11",
                "f3d:Design/BulkStream.dat:design-component-occurrence#12",
            ],
        );
        let model = with_joints(
            model,
            json!([
                joint(
                    1,
                    json!({"kind": "fixed"}),
                    connector_at(0, &[], IDENTITY),
                    connector_at(2, &[], facing(IDENTITY))
                ),
                joint(
                    2,
                    json!({"kind": "fixed"}),
                    connector_at(0, &[], at([100.0, 0.0, 0.0], 0.0)),
                    connector_at(3, &[], facing(IDENTITY))
                ),
            ]),
        );
        let (assembly, _) = local_components(&model).unwrap().unwrap();
        for (path, x) in [(vec![0], 0.0), (vec![0, 0], 5.0), (vec![1, 0], 70.0)] {
            let world = world_at(&assembly, &path).unwrap();
            assert!(near(&world, &at([x, 0.0, 0.0], 0.0)), "{path:?}: {world:?}");
        }
        let suppressed: Vec<bool> = assembly.joints.iter().map(|j| j.suppressed).collect();
        assert_eq!(suppressed, [false, true]);
    }

    /// `model` with the `design_component_occurrences` records `ids` carrying current poses,
    /// so the occurrences that name them as their `native_ref` are posed.
    fn with_current_poses(model: Model, ids: &[&str]) -> Model {
        let mut wire = serde_json::to_value(&model.ir).unwrap();
        let occurrences: Vec<_> = wire["model"]["occurrences"]
            .as_array()
            .unwrap()
            .iter()
            .collect();
        let worlds = world_placements(&occurrences).unwrap();
        let records = ids.iter().filter_map(|native| {
            let occurrence = occurrences.iter().find(|row| row["native_ref"] == *native)?;
            let mut pose = worlds[occurrence["id"].as_str().unwrap()];
            for row in &mut pose { row[3] /= 10.0; }
            Some(json!({"id": native, "current_pose": {"carrier_record_index": 3, "transform": pose}}))
        }).collect::<Vec<_>>();
        wire["native"]["f3d"]["design_component_occurrences"] = json!(records);
        Model {
            ir: cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap(),
            ..model
        }
    }

    /// Adds a component `mid` to `model`'s definitions.
    fn with_mid(model: Model) -> Model {
        let mut wire = serde_json::to_value(&model.ir).unwrap();
        wire["model"]["product_definitions"]
            .as_array_mut()
            .unwrap()
            .push(json!({"id": "f3d:model:component#mid", "kind": "part", "label": "Mid", "bodies": []}));
        Model {
            ir: cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap(),
            ..model
        }
    }

    fn posed_child(mut row: Value, native: &str) -> Value {
        row["native_ref"] = json!(native);
        row
    }

    #[test]
    fn a_holder_a_joint_moves_carries_the_parts_the_file_poses_inside_it() {
        // The holder sits at its own origin with two posed parts inside it at x = 5 and 30. A
        // joint puts the first part at x = 100, so the holder goes to x = 95 and, in the world,
        // the second part to x = 125: the parts' stored poses are relative to the holder.
        let model = nested(
            false,
            Some(json!([
                placed(0, "frame", root(), 0.0),
                posed_child(
                    placed(1, "block", inside(0), 5.0),
                    "f3d:Design/BulkStream.dat:design-component-occurrence#11"
                ),
                posed_child(
                    placed(2, "block", inside(0), 30.0),
                    "f3d:Design/BulkStream.dat:design-component-occurrence#12"
                ),
                placed(3, "block", root(), 0.0),
            ])),
        );
        let model = with_current_poses(
            model,
            &[
                "f3d:Design/BulkStream.dat:design-component-occurrence#11",
                "f3d:Design/BulkStream.dat:design-component-occurrence#12",
            ],
        );
        let model = with_joints(
            model,
            json!([
                joint(
                    1,
                    json!({"kind": "fixed"}),
                    connector(None),
                    connector_at(3, &[], facing(IDENTITY))
                ),
                joint(
                    2,
                    json!({"kind": "fixed"}),
                    connector_at(3, &[], at([100.0, 0.0, 0.0], 0.0)),
                    connector_at(1, &[], facing(IDENTITY))
                ),
            ]),
        );
        let (assembly, notes) = local_components(&model).unwrap().unwrap();
        assert!(notes.is_empty(), "{notes:?}");
        for (path, x) in [(vec![0], 95.0), (vec![0, 0], 100.0), (vec![0, 1], 125.0)] {
            let world = world_at(&assembly, &path).unwrap();
            assert!(near(&world, &at([x, 0.0, 0.0], 0.0)), "{path:?}: {world:?}");
        }
    }

    #[test]
    fn parts_the_file_poses_move_with_a_holder_through_two_levels_of_nesting() {
        // A holder holds a middle component (at x = 10) which holds two posed parts (at 5 and
        // 30). A joint puts the first part at x = 100: holder 85, middle 95, second part 125.
        let model = nested(
            false,
            Some(json!([
                placed(0, "frame", root(), 0.0),
                placed(1, "mid", inside(0), 10.0),
                posed_child(
                    placed(2, "block", inside(1), 5.0),
                    "f3d:Design/BulkStream.dat:design-component-occurrence#11"
                ),
                posed_child(
                    placed(3, "block", inside(1), 30.0),
                    "f3d:Design/BulkStream.dat:design-component-occurrence#12"
                ),
                placed(4, "block", root(), 0.0),
            ])),
        );
        let model = with_current_poses(
            with_mid(model),
            &[
                "f3d:Design/BulkStream.dat:design-component-occurrence#11",
                "f3d:Design/BulkStream.dat:design-component-occurrence#12",
            ],
        );
        let model = with_joints(
            model,
            json!([
                joint(
                    1,
                    json!({"kind": "fixed"}),
                    connector(None),
                    connector_at(4, &[], facing(IDENTITY))
                ),
                joint(
                    2,
                    json!({"kind": "fixed"}),
                    connector_at(4, &[], at([100.0, 0.0, 0.0], 0.0)),
                    connector_at(2, &[], facing(IDENTITY))
                ),
            ]),
        );
        let (assembly, notes) = local_components(&model).unwrap().unwrap();
        assert!(notes.is_empty(), "{notes:?}");
        for (path, x) in [
            (vec![0], 85.0),
            (vec![0, 0], 95.0),
            (vec![0, 0, 0], 100.0),
            (vec![0, 0, 1], 125.0),
        ] {
            let world = world_at(&assembly, &path).unwrap();
            assert!(near(&world, &at([x, 0.0, 0.0], 0.0)), "{path:?}: {world:?}");
        }
    }

    #[test]
    fn joints_that_reach_nothing_placed_are_placed_from_their_outermost_occurrence() {
        // The frame (stored at x = 100) holds block 2 rigidly, and block 2 holds block 1 at
        // its own origin; nothing joins them to the design. The block-to-block joint comes
        // first, yet the frame stays where it is stored and both blocks move to it.
        let fixed = json!({"kind": "fixed"});
        let model = with_joints(
            nested(false, None),
            json!([
                joint(
                    1,
                    fixed.clone(),
                    connector_at(1, &[], IDENTITY),
                    connector_at(2, &[], IDENTITY)
                ),
                joint(
                    2,
                    fixed,
                    connector_at(0, &[], at([5.0, 0.0, 0.0], 0.0)),
                    connector_at(2, &[], IDENTITY)
                ),
            ]),
        );
        let (assembly, notes) = local_components(&model).unwrap().unwrap();
        assert!(notes.is_empty(), "{notes:?}");
        let frame = world_at(&assembly, &[0]).unwrap();
        assert!(near(&frame, &at([100.0, 0.0, 0.0], 0.0)), "{frame:?}");
        // Block 2 faces the frame, and block 1 faces block 2.
        let block_2 = world_at(&assembly, &[0, 1]).unwrap();
        assert!(
            near(&block_2, &facing(at([105.0, 0.0, 0.0], 0.0))),
            "{block_2:?}"
        );
        let block_1 = world_at(&assembly, &[0, 0]).unwrap();
        assert!(near(&block_1, &at([105.0, 0.0, 0.0], 0.0)), "{block_1:?}");
    }

    #[test]
    fn a_part_placed_inside_an_occurrence_stays_when_a_later_joint_moves_the_occurrence() {
        // The design holds block 2 (inside the frame) at x = 50, then the frame at x = 20:
        // moving the frame leaves block 2 where its own joint put it.
        let fixed = json!({"kind": "fixed"});
        let model = with_joints(
            nested(false, None),
            json!([
                joint(
                    1,
                    fixed.clone(),
                    connector(None),
                    connector_at(2, &[], facing(at([-50.0, 0.0, 0.0], 0.0)))
                ),
                joint(
                    2,
                    fixed,
                    connector(None),
                    connector_at(0, &[], facing(at([-20.0, 0.0, 0.0], 0.0)))
                ),
            ]),
        );
        let (assembly, notes) = local_components(&model).unwrap().unwrap();
        assert!(notes.is_empty(), "{notes:?}");
        let frame = world_at(&assembly, &[0]).unwrap();
        assert!(near(&frame, &at([20.0, 0.0, 0.0], 0.0)), "{frame:?}");
        let block = world_at(&assembly, &[0, 1]).unwrap();
        assert!(near(&block, &at([50.0, 0.0, 0.0], 0.0)), "{block:?}");
    }

    #[test]
    fn an_occurrence_the_file_poses_is_checked_by_its_joint_not_moved() {
        // CADIR retained x = 100 from a component operation. The native current pose
        // selects x = 120; a joint that would put it at x = 20 is checked, not solved.
        let mut frame = placed(0, "frame", root(), 100.0);
        frame["native_ref"] = json!("f3d:Design/BulkStream.dat:design-component-occurrence#7");
        let model = nested(
            false,
            Some(json!([
                frame,
                placed(1, "block", inside(0), 0.0),
                placed(2, "block", inside(0), 30.0),
            ])),
        );
        let fixed = json!({"kind": "fixed"});
        let joints = json!([joint(
            1,
            fixed,
            connector(None),
            connector_at(0, &[], facing(at([-20.0, 0.0, 0.0], 0.0)))
        )]);
        let mut wire =
            serde_json::to_value(&with_joints(model.clone(), joints.clone()).ir).unwrap();
        wire["native"]["f3d"]["design_component_occurrences"] = json!([
            {"id": "f3d:Design/BulkStream.dat:design-component-occurrence#7",
             "current_pose": {"carrier_record_index": 9, "transform": at([12.0, 0.0, 0.0], 0.0)}},
        ]);
        let posed = Model {
            ir: cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap(),
            ..model.clone()
        };
        let (assembly, notes) = local_components(&posed).unwrap().unwrap();
        let frame = world_at(&assembly, &[0]).unwrap();
        assert!(near(&frame, &at([120.0, 0.0, 0.0], 0.0)), "{frame:?}");
        assert_eq!(notes.len(), 1, "{notes:?}");
        // A decoder-bound display pose works without any native current pose,
        // survives compact metadata extraction, and outranks this touching joint
        // (1 mm from where the joint puts the frame, so its blocks stay in the model's extent).
        wire["native"]["f3d"]["design_component_occurrences"] = json!([]);
        let occurrence = wire["model"]["occurrences"][0]["id"].clone();
        wire["native"]["f3d"]["display_poses"] = json!([{
            "id": "f3d:scene:display-pose#0", "stream": "world", "byte_offset": 0,
            "path": [1, 7], "binding": {"state": "bound", "occurrence": occurrence},
            "transform": [[1,0,0,21],[0,1,0,0],[0,0,1,0],[0,0,0,1]]
        }]);
        let displayed = Model {
            ir: cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap(),
            ..model.clone()
        };
        for mode in [crate::PartPositions::Smart, crate::PartPositions::LastShown] {
            let assembly = local_assembly_with_positions(&displayed, mode)
                .unwrap()
                .unwrap();
            assert!(near(
                &world_at(&assembly, &[0]).unwrap(),
                &at([21.0, 0.0, 0.0], 0.0)
            ));
        }
        let assembly = local_assembly_with_positions(&displayed, crate::PartPositions::FirstPlaced)
            .unwrap()
            .unwrap();
        assert!(near(
            &world_at(&assembly, &[0]).unwrap(),
            &at([20.0, 0.0, 0.0], 0.0)
        ));
        // Without the pose the same joint places the frame.
        let (assembly, _) = local_components(&with_joints(model, joints))
            .unwrap()
            .unwrap();
        let frame = world_at(&assembly, &[0]).unwrap();
        assert!(near(&frame, &at([20.0, 0.0, 0.0], 0.0)), "{frame:?}");
    }

    #[test]
    fn stored_placements_survive_joint_solving_in_nested_components() {
        let model = with_joints(
            nested(false, None),
            json!([joint(
                1,
                json!({"kind": "revolute"}),
                connector_at(2, &[], IDENTITY),
                connector_at(1, &[], facing(at([10.0, 0.0, 0.0], 0.0)))
            )]),
        );
        let mut assembly = local_assembly(&model).unwrap().unwrap();
        assert!(near(
            &world_at(&assembly, &[0, 1]).unwrap(),
            &at([110.0, 0.0, 0.0], 0.0)
        ));
        assembly.restore_stored_placements();
        assert!(near(
            &world_at(&assembly, &[0, 1]).unwrap(),
            &at([130.0, 0.0, 0.0], 0.0)
        ));
    }

    #[test]
    fn a_contained_pose_keeps_its_child_while_an_unposed_sibling_can_move() {
        let mut frame = placed(0, "frame", root(), 100.0);
        frame["native_ref"] = json!("f3d:xref:reference#1");
        let mut child = placed(1, "block", inside(0), 0.0);
        child["native_ref"] = json!("f3d:xref/member/Design:occurrence#1");
        let mut sibling = placed(2, "block", inside(0), 30.0);
        sibling["native_ref"] = json!("f3d:xref/member/Design:occurrence#2");
        let model = nested(false, Some(json!([frame, child, sibling])));
        let model = with_joints(
            model,
            json!([
                joint(
                    1,
                    json!({"kind": "fixed"}),
                    connector(Some((0, &[]))),
                    connector_at(1, &[], facing(at([-20.0, 0.0, 0.0], 0.0)))
                ),
                joint(
                    2,
                    json!({"kind": "fixed"}),
                    connector(Some((0, &[]))),
                    connector_at(2, &[], facing(at([-40.0, 0.0, 0.0], 0.0)))
                ),
            ]),
        );
        let mut wire = serde_json::to_value(&model.ir).unwrap();
        wire["native"]["f3d"]["xref_references"] = json!([
            {"id": "f3d:xref:reference#1", "contained_poses": [
                {"contained_record_index": 11}
            ]}
        ]);
        wire["native"]["f3d"]["design_component_occurrences"] = json!([
            {"id": "f3d:xref/member/Design:occurrence#1", "record_index": 11},
            {"id": "f3d:xref/member/Design:occurrence#2", "record_index": 12},
        ]);
        let model = Model {
            ir: cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap(),
            ..model
        };
        let (assembly, notes) = local_components(&model).unwrap().unwrap();
        assert!(near(
            &world_at(&assembly, &[0, 0]).unwrap(),
            &at([100.0, 0.0, 0.0], 0.0)
        ));
        assert!(assembly.joints[0].suppressed);
        assert_eq!(notes.len(), 1, "{notes:?}");
        assert!(near(
            &world_at(&assembly, &[0, 1]).unwrap(),
            &at([140.0, 0.0, 0.0], 0.0)
        ));
        assert!(!assembly.joints[1].suppressed);

        // A reference's own current pose holds the reference, while its children
        // still need their joints unless they carry current poses of their own.
        wire["native"]["f3d"]["xref_references"][0]["current_pose"] =
            json!({"carrier_record_index": 21, "transform": at([10.0, 0.0, 0.0], 0.0)});
        let own_pose = Model {
            ir: cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap(),
            ..model
        };
        let (assembly, notes) = local_components(&own_pose).unwrap().unwrap();
        assert!(near(
            &world_at(&assembly, &[0, 0]).unwrap(),
            &at([120.0, 0.0, 0.0], 0.0)
        ));
        assert!(!assembly.joints[0].suppressed);
        assert!(notes.is_empty(), "{notes:?}");
    }

    #[test]
    fn contained_pose_targets_stay_in_their_member_and_owner_scope() {
        let mut wire = json!({
            "model": {"occurrences": [
                {"id": "owner", "native_ref": "f3d:Design:owner"},
                {"id": "insert", "native_ref": "reference", "parent": {"occurrence": "owner"}},
                {"id": "child", "native_ref": "member:child", "parent": {"occurrence": "insert"}},
                {"id": "other", "native_ref": "reference"},
                {"id": "other-child", "native_ref": "member:child", "parent": {"occurrence": "other"}},
                {"id": "nested", "native_ref": "nested-reference", "parent": {"occurrence": "insert"}},
                {"id": "nested-child", "native_ref": "nested:child", "parent": {"occurrence": "nested"}}
            ]},
            "native": {"f3d": {
                "xref_references": [
                    {"id": "reference", "contained_poses": [
                        {"contained_record_index": 11, "owner_record_indices": [5]}
                    ]},
                    {"id": "nested-reference"}
                ],
                "design_component_occurrences": [
                    {"id": "f3d:Design:owner", "record_index": 5},
                    {"id": "member:child", "record_index": 11},
                    {"id": "nested:child", "record_index": 11}
                ]
            }}
        });
        assert_eq!(
            source_posed(&wire),
            BTreeSet::from(["insert".to_owned(), "other".to_owned(), "child".to_owned()])
        );
        // Two candidates with the same record in one instance are ambiguous.
        wire["model"]["occurrences"]
            .as_array_mut()
            .unwrap()
            .push(json!({
                "id": "duplicate", "native_ref": "member:child", "parent": {"occurrence": "insert"}
            }));
        assert_eq!(
            source_posed(&wire),
            BTreeSet::from(["insert".to_owned(), "other".to_owned()])
        );
    }

    #[test]
    fn a_joint_moves_its_first_part_to_where_its_frames_meet() {
        // A hinge at block 2's own origin and 10 mm past block 1's, facing it: whatever block
        // 2's stored placement, the joint puts it at x = 110, and block 1 stays.
        let hinge = || {
            (
                connector_at(2, &[], at([0.0, 0.0, 0.0], 0.0)),
                connector_at(1, &[], facing(at([10.0, 0.0, 0.0], 0.0))),
            )
        };
        let (a, b) = hinge();
        let (world, joint, notes) = joined(json!({"kind": "revolute"}), a, b);
        assert!(notes.is_empty(), "{notes:?}");
        assert!(near(&world, &at([110.0, 0.0, 0.0], 0.0)), "{world:?}");
        assert_eq!(joint.frame_a, at([0.0, 0.0, 0.0], 0.0));
        // Turned half a radian, the second frame turns half a radian from the first, so block
        // 2 turns back by as much about the hinge.
        let (a, b) = hinge();
        let (world, _, _) = joined(json!({"kind": "revolute", "angle": 0.5}), a, b);
        assert!(near(&world, &at([110.0, 0.0, 0.0], -0.5)), "{world:?}");
        // A slider 7 mm along the second connector's z, which points down.
        let (a, b) = hinge();
        let (world, _, _) = joined(json!({"kind": "slider", "distance": 7.0}), a, b);
        assert!(near(&world, &at([110.0, 0.0, -7.0], 0.0)), "{world:?}");
        // C0 = C1 * T(5, 0, 0) * Rz(pi/2) * F: the second frame's x points
        // along world x, so the first part sits at x = 115, turned -pi/2.
        let (a, b) = hinge();
        let (world, _, _) = joined(
            json!({"kind": "fixed", "angle": std::f64::consts::FRAC_PI_2,
                   "translation_offset": [5.0, 0.0, 0.0]}),
            a,
            b,
        );
        assert!(
            near(&world, &at([115.0, 0.0, 0.0], -std::f64::consts::FRAC_PI_2)),
            "{world:?}"
        );
    }

    #[test]
    fn joint_offsets_close_on_the_second_connector_for_each_frame_sense() {
        let first = at([1.0, 2.0, 3.0], 0.2);
        let second = at([10.0, 20.0, 30.0], -0.3);
        for (kind, source_offset) in [
            (
                json!({"kind": "fixed", "angle": 0.7, "translation_offset": [4.0, 6.0, 8.0]}),
                at([4.0, 6.0, 8.0], 0.7),
            ),
            (json!({"kind": "revolute", "angle": 0.7}), turn(0.7)),
            (
                json!({"kind": "slider", "distance": 7.0, "translation_offset": [4.0, 6.0, 8.0]}),
                at([4.0, 6.0, 15.0], 0.0),
            ),
        ] {
            for (sense, facing) in [("opposed", FACING), ("aligned", IDENTITY)] {
                for anchor_a in [false, true] {
                    for already_closed in [false, true] {
                        let (mut a, mut b) = (at([30.0, 40.0, 50.0], 0.4), IDENTITY);
                        // Derive the missing pose directly from C0 = C1 * offset * facing.
                        let b_to_a = compose(&compose(&second, &source_offset), &facing);
                        let (expected_a, expected_b) = if anchor_a {
                            (a, compose(&compose(&a, &first), &inverse(&b_to_a)))
                        } else {
                            (compose(&compose(&b, &b_to_a), &inverse(&first)), b)
                        };
                        if already_closed {
                            (a, b) = (expected_a, expected_b);
                        }
                        let mut a_row = placed(2, "block", inside(0), 0.0);
                        a_row["transform"] = json!(a);
                        let mut b_row = placed(1, "block", inside(0), 0.0);
                        b_row["transform"] = json!(b);
                        if anchor_a || already_closed {
                            a_row = posed_child(
                                a_row,
                                "f3d:Design/BulkStream.dat:design-component-occurrence#11",
                            );
                        }
                        if !anchor_a || already_closed {
                            b_row = posed_child(
                                b_row,
                                "f3d:Design/BulkStream.dat:design-component-occurrence#12",
                            );
                        }
                        let mut row = joint(
                            1,
                            kind.clone(),
                            connector_at(2, &[], first),
                            connector_at(1, &[], second),
                        );
                        row["operands"]["frame_sense"] = json!(sense);
                        let model = with_current_poses(
                            with_joints(
                                nested(
                                    false,
                                    Some(json!([placed(0, "frame", root(), 100.0), b_row, a_row])),
                                ),
                                json!([row]),
                            ),
                            &[
                                "f3d:Design/BulkStream.dat:design-component-occurrence#11",
                                "f3d:Design/BulkStream.dat:design-component-occurrence#12",
                            ],
                        );
                        let (assembly, notes) = local_components(&model).unwrap().unwrap();
                        assert!(notes.is_empty(), "{kind}, {sense}, anchor_a={anchor_a}, closed={already_closed}: {notes:?}");
                        let holder = at([100.0, 0.0, 0.0], 0.0);
                        let wa = world_at(&assembly, &[0, 1]).unwrap();
                        let wb = world_at(&assembly, &[0, 0]).unwrap();
                        assert!(
                            near(&wa, &compose(&holder, &expected_a)),
                            "{kind}, {sense}: {wa:?}"
                        );
                        assert!(
                            near(&wb, &compose(&holder, &expected_b)),
                            "{kind}, {sense}: {wb:?}"
                        );
                        let joint = &assembly.joints[0];
                        assert!(!joint.suppressed);
                        assert_eq!(joint.flip, sense == "aligned");
                        assert!(near(&joint.frame_a, &first));
                        // The exact same normalized frames used for placement go to the writer.
                        assert!(near(
                            &compose(&compose(&wa, &joint.frame_a), &joint.meeting()),
                            &compose(&wb, &joint.frame_b),
                        ));
                        assert!(near(
                            &compose(&wa, &first),
                            &compose(&compose(&compose(&wb, &second), &source_offset), &facing),
                        ));
                    }
                }
            }
        }
    }

    #[test]
    fn an_offset_joint_is_rechecked_after_outer_joints_place_its_parts() {
        // The internal joint closes at the stored poses: A is 5 mm along B's x.
        // Outer joints then hold A at x = 10 and its holder at x = 20. B travels
        // with the holder, while A stays held. The internal joint must be suppressed.
        let aligned = |mut row: Value| {
            row["operands"]["frame_sense"] = json!("aligned");
            row
        };
        let root_at = |x| {
            let mut connector = connector(None);
            connector["frame"] = json!(at([x, 0.0, 0.0], 0.0));
            connector
        };
        let model = with_joints(
            nested(
                false,
                Some(json!([
                    placed(0, "frame", root(), 3.0),
                    placed(1, "block", inside(0), 5.0),
                    placed(2, "block", inside(0), 0.0),
                ])),
            ),
            json!([
                aligned(joint(
                    1,
                    json!({"kind": "fixed", "translation_offset": [5.0, 0.0, 0.0]}),
                    connector_at(1, &[], IDENTITY),
                    connector_at(2, &[], IDENTITY)
                )),
                aligned(joint(
                    2,
                    json!({"kind": "fixed"}),
                    root_at(10.0),
                    connector_at(1, &[], IDENTITY)
                )),
                aligned(joint(
                    3,
                    json!({"kind": "fixed"}),
                    root_at(20.0),
                    connector_at(0, &[], IDENTITY)
                )),
            ]),
        );
        let (assembly, notes) = local_components(&model).unwrap().unwrap();
        assert_eq!(
            world_at(&assembly, &[0, 0]),
            Some(at([10.0, 0.0, 0.0], 0.0))
        );
        assert_eq!(
            world_at(&assembly, &[0, 1]),
            Some(at([20.0, 0.0, 0.0], 0.0))
        );
        assert_eq!(
            assembly
                .joints
                .iter()
                .map(|joint| joint.suppressed)
                .collect::<Vec<_>>(),
            [true, false, false]
        );
        assert_eq!(notes.len(), 1, "{notes:?}");
        assert!(notes[0].starts_with("joint 1: its parts are placed by other joints"));
    }

    #[test]
    fn a_carriage_offset_keeps_its_three_mm_stored_pose() {
        // Synthetic carriage and rail: C0 at (0, 5, 0), z down Y; C1 at
        // (-40, 8, 0), z up Y. C1 * T(40, 0, 0) * F meets C0 at y = 8
        // when the carriage is at y = 3. The same rule places an unposed carriage.
        let carriage = [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 0.0, -1.0, 5.0],
            [0.0, 1.0, 0.0, 0.0],
        ];
        let rail = [
            [1.0, 0.0, 0.0, -40.0],
            [0.0, 0.0, 1.0, 8.0],
            [0.0, -1.0, 0.0, 0.0],
        ];
        for stored_y in [0.0, 3.0] {
            let mut moving = placed(2, "block", inside(0), 0.0);
            moving["transform"] = json!(at([0.0, stored_y, 0.0], 0.0));
            let model = with_joints(
                nested(
                    false,
                    Some(json!([
                        placed(0, "frame", root(), 0.0),
                        placed(1, "block", inside(0), 0.0),
                        moving
                    ])),
                ),
                json!([joint(
                    1,
                    json!({"kind": "fixed", "angle": 0.0, "translation_offset": [40.0, 0.0, 0.0]}),
                    connector_at(2, &[], carriage),
                    connector_at(1, &[], rail),
                )]),
            );
            let (assembly, notes) = local_components(&model).unwrap().unwrap();
            assert!(notes.is_empty(), "{notes:?}");
            let world = world_at(&assembly, &[0, 1]).unwrap();
            assert!(near(&world, &at([0.0, 3.0, 0.0], 0.0)), "{world:?}");
            assert_eq!(world_at(&assembly, &[0, 0]), Some(IDENTITY));
            let joint = &assembly.joints[0];
            assert!(!joint.flip);
            assert!(!joint.suppressed);
            assert!(near(&joint.frame_a, &carriage));
            assert!(near(
                &compose(&compose(&world, &joint.frame_a), &joint.meeting()),
                &joint.frame_b,
            ));
        }
    }

    #[test]
    fn a_part_joined_by_its_face_stands_on_the_face_it_is_joined_to() {
        // As Fusion stores a pin jointed onto a block: the pin's frame on its bottom face, z
        // out of the face (down), and the block's on its top face 10 mm up, z up. The pin
        // stands on the block, upright, whatever its stored placement.
        let (world, _, notes) = joined(
            json!({"kind": "fixed"}),
            connector_at(2, &[], facing(IDENTITY)),
            connector_at(1, &[], at([0.0, 0.0, 10.0], 0.0)),
        );
        assert!(notes.is_empty(), "{notes:?}");
        assert!(near(&world, &at([100.0, 0.0, 10.0], 0.0)), "{world:?}");
    }

    #[test]
    fn a_frame_named_through_its_container_is_moved_into_the_part_it_is_on() {
        // Block 2's frame, given in the Frame occurrence's space (which places block 2 at
        // x = 30 in it): 30 mm in, it is block 2's own origin.
        let (world, joint, _) = joined(
            json!({"kind": "revolute"}),
            connector_at(0, &["2"], at([30.0, 0.0, 0.0], 0.0)),
            connector_at(1, &[], facing(at([10.0, 0.0, 0.0], 0.0))),
        );
        assert!(near(&joint.frame_a, &IDENTITY), "{:?}", joint.frame_a);
        assert!(near(&world, &at([110.0, 0.0, 0.0], 0.0)), "{world:?}");
    }

    #[test]
    fn revolute_limits_are_kept_and_a_one_sided_one_is_left_out() {
        let (_, joint, notes) = joined(
            json!({"kind": "revolute", "angular_limits": {"bounds": "range", "minimum": -1.0, "maximum": 1.0}}),
            connector_at(1, &[], IDENTITY),
            connector_at(2, &[], IDENTITY),
        );
        assert!(notes.is_empty());
        assert_eq!(
            joint.motion,
            Motion::Revolute {
                angle: 0.0,
                limits: Some((-1.0, 1.0))
            }
        );
        let (_, joint, notes) = joined(
            json!({"kind": "revolute", "angular_limits": {"bounds": "minimum", "minimum": -1.0}}),
            connector_at(1, &[], IDENTITY),
            connector_at(2, &[], IDENTITY),
        );
        assert!(matches!(
            joint.motion,
            Motion::Revolute { limits: None, .. }
        ));
        assert_eq!(notes, ["joint 1: its limit on one side only is left out"]);
    }

    #[test]
    fn joints_that_place_a_part_two_ways_are_reported_and_suppressed() {
        let rigid = json!({"kind": "fixed"});
        let model = with_joints(
            nested(false, None),
            json!([
                joint(
                    1,
                    rigid.clone(),
                    connector_at(1, &[], IDENTITY),
                    connector_at(2, &[], IDENTITY)
                ),
                joint(
                    2,
                    rigid,
                    connector_at(1, &[], IDENTITY),
                    connector_at(2, &[], at([1.0, 0.0, 0.0], 0.0))
                ),
            ]),
        );
        let (assembly, notes) = local_components(&model).unwrap().unwrap();
        assert_eq!(notes.len(), 1, "{notes:?}");
        assert!(notes[0].starts_with("joint 2: its parts are placed by other joints"));
        let suppressed: Vec<bool> = assembly.joints.iter().map(|j| j.suppressed).collect();
        assert_eq!(suppressed, [false, true]);
    }

    fn joint(key: u32, kind: Value, a: Value, b: Value) -> Value {
        json!({
            "id": format!("f3d:model:joint#{key}"),
            "operands": {"arity": "pair", "kind": kind, "connectors": [a, b]},
            "suppressed": false,
        })
    }

    /// Block 2 stored 10 mm above block 1, turned 0.3 rad about z, both inside the frame at
    /// x = 100, and joined by one joint `kind`: a frame on block 2's origin facing down, and
    /// one on block 1's facing up. Block 2's placement in the world, the joint and the notes.
    fn on_its_axis(kind: Value) -> (Placement3x4, Joint, Vec<String>) {
        let mut turned = placed(2, "block", inside(0), 0.0);
        turned["transform"] = json!(at([0.0, 0.0, 10.0], 0.3));
        let rows = json!([
            placed(0, "frame", root(), 100.0),
            placed(1, "block", inside(0), 0.0),
            turned
        ]);
        let model = with_joints(
            nested(false, Some(rows)),
            json!([joint(
                1,
                kind,
                connector_at(2, &[], facing(IDENTITY)),
                connector_at(1, &[], IDENTITY)
            )]),
        );
        let (assembly, notes) = local_components(&model).unwrap().unwrap();
        let world = world_at(&assembly, &[0, 1]).unwrap();
        let joint = assembly.joints[0].clone();
        // Wherever it leaves them, the joint holds its parts where they are.
        let a = compose(&world, &joint.frame_a);
        let b = compose(&world_at(&assembly, &[0, 0]).unwrap(), &joint.frame_b);
        assert!(near(&compose(&a, &joint.meeting()), &b), "{joint:?}");
        (world, joint, notes)
    }

    #[test]
    fn a_revolute_whose_parts_turn_on_its_axis_holds_them_as_built() {
        // The parts are placed 0.3 rad apart about the axis; the source's angle of 0.5 rad
        // aligns the frames, so the joint has turned -0.2 rad from there. They stay as placed.
        let (world, joint, notes) = on_its_axis(json!({"kind": "revolute", "angle": 0.5,
            "angular_limits": {"bounds": "range", "minimum": -1.0, "maximum": 1.0}}));
        assert!(near(&world, &at([100.0, 0.0, 10.0], 0.3)), "{world:?}");
        let Motion::Revolute { angle, limits } = joint.motion else {
            panic!("{:?}", joint.motion);
        };
        assert!((angle + 0.2).abs() < 1e-9, "{angle}");
        assert_eq!(limits, Some((-1.0, 1.0)));
        assert!(notes.is_empty(), "{notes:?}");
        // The limits hold from the aligned frames: -0.2 rad is inside these.
        let (_, joint, notes) = on_its_axis(json!({"kind": "revolute", "angle": 0.5,
            "angular_limits": {"bounds": "range", "minimum": -0.25, "maximum": 0.0}}));
        assert!(matches!(
            joint.motion,
            Motion::Revolute {
                limits: Some(_),
                ..
            }
        ));
        assert!(notes.is_empty(), "{notes:?}");
        // Limits the angle lies outside are left out, with the reason.
        let (_, joint, notes) = on_its_axis(json!({"kind": "revolute", "angle": 0.0,
            "angular_limits": {"bounds": "range", "minimum": -0.2, "maximum": 0.2}}));
        assert!(matches!(
            joint.motion,
            Motion::Revolute { limits: None, .. }
        ));
        assert_eq!(
            notes,
            ["joint 1: its limits are left out: its angle lies outside them"]
        );
    }

    #[test]
    fn a_fixed_joint_its_parts_already_meet_either_way_holds_them_as_built() {
        // Block 2 stored where block 1 is, their joint frames coinciding with z the same way:
        // Fusion mated them flipped. They stay; the joint is not turned over to mate them.
        let rows = |turned: f64| {
            let mut second = placed(2, "block", inside(0), 0.0);
            second["transform"] = json!(at([0.0, 0.0, 0.0], turned));
            json!([
                placed(0, "frame", root(), 100.0),
                placed(1, "block", inside(0), 0.0),
                second
            ])
        };
        let joined = |turned: f64| {
            let model = with_joints(
                nested(false, Some(rows(turned))),
                json!([joint(
                    1,
                    json!({"kind": "fixed"}),
                    connector_at(2, &[], IDENTITY),
                    connector_at(1, &[], IDENTITY)
                )]),
            );
            let (assembly, notes) = local_components(&model).unwrap().unwrap();
            assert!(notes.is_empty(), "{notes:?}");
            let world = world_at(&assembly, &[0, 1]).unwrap();
            let joint = assembly.joints[0].clone();
            let a = compose(&world, &joint.frame_a);
            let b = compose(&world_at(&assembly, &[0, 0]).unwrap(), &joint.frame_b);
            assert!(near(&compose(&a, &joint.meeting()), &b), "{joint:?}");
            world
        };
        assert!(near(&joined(0.0), &at([100.0, 0.0, 0.0], 0.0)));
        // Turned about the axis, the parts do not meet as the joint says: it places them.
        assert!(!near(&joined(0.3), &at([100.0, 0.0, 0.0], 0.3)));
    }

    #[test]
    fn a_slider_whose_parts_lie_on_its_axis_holds_them_as_built() {
        let (world, joint, notes) = on_its_axis(json!({"kind": "slider", "distance": 2.0}));
        assert!(near(&world, &at([100.0, 0.0, 10.0], 0.3)), "{world:?}");
        let Motion::Slider { distance } = joint.motion else {
            panic!("{:?}", joint.motion);
        };
        assert!((distance - 10.0).abs() < 1e-9, "{distance}");
        assert!(notes.is_empty(), "{notes:?}");
    }

    /// A block placed at the origin, and a design inserted into this one (as an .f3z carries
    /// it, named `Gripper`) holding a block 5 mm along x, joined by `joints`.
    fn with_inserted_design(joints: Value) -> Model {
        let model = nested(false, Some(json!([])));
        let mut wire = serde_json::to_value(&model.ir).unwrap();
        let mut finger = placed(9, "block", json!({}), 5.0);
        finger["id"] = json!("f3d:xref/role-1/model:occurrence#finger");
        finger["parent"] = inside(8);
        wire["model"]["occurrences"] = json!([
            placed(0, "block", root(), 0.0),
            {
                "id": "f3d:model:occurrence#8",
                "prototype": {"scope": "external",
                    "document": {"resolution": "path", "path": "gripper.f3d"}},
                "parent": root(),
                "ordinal": 1,
                "transform": IDENTITY,
                "scale": [1.0, 1.0, 1.0],
            },
            finger,
        ]);
        wire["model"]["assembly_joints"] = joints;
        wire["native"]["f3d"]["xref_designs"] = json!([
            {"id": "f3d:xref:design#0", "display_name": "Gripper",
             "target_file_name": "gripper.f3d"},
        ]);
        Model {
            ir: cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap(),
            ..model
        }
    }

    /// A connector on the component keyed `key` inside a design inserted into this one.
    fn inserted_connector(key: &str) -> Value {
        json!({"operand": {"container": {"container": "external",
                "external_document": {"resolution": "document_id", "document_id": "d"}},
              "object": "o", "subelements": [key]},
            "frame": IDENTITY, "detached": false})
    }

    #[test]
    fn a_design_inserted_into_this_one_is_placed_and_joined_as_a_whole() {
        // The joint's frame on the gripper is the finger's own, 5 mm along x inside it: it
        // faces the block's 50 mm up, and the whole gripper stands so the finger's frame is
        // there.
        let model = with_inserted_design(json!([joint(
            1,
            json!({"kind": "fixed"}),
            inserted_connector("finger"),
            connector_at(0, &[], at([0.0, 0.0, 50.0], 0.0)),
        )]));
        let (assembly, notes) = local_components(&model).unwrap().unwrap();
        assert!(notes.is_empty(), "{notes:?}");
        let gripper = assembly
            .placements
            .iter()
            .position(|p| assembly.parts[p.part].name == "Gripper")
            .unwrap();
        let inner = assembly.parts[assembly.placements[gripper].part]
            .assembly
            .as_ref()
            .unwrap();
        assert_eq!(inner.placements.len(), 1);
        assert_eq!(inner.placements[0].transform[0][3], 5.0);
        assert_eq!(assembly.joints[0].a, [gripper]);
        let world = world_at(&assembly, &[gripper]).unwrap();
        let finger = at([5.0, 0.0, 0.0], 0.0);
        assert!(
            near(
                &compose(&world, &finger),
                &facing(at([0.0, 0.0, 50.0], 0.0))
            ),
            "{world:?}"
        );
        // A key no component inside it has joins nothing.
        let model = with_inserted_design(json!([joint(
            1,
            json!({"kind": "fixed"}),
            inserted_connector("thumb"),
            connector_at(0, &[], IDENTITY),
        )]));
        let (assembly, notes) = local_components(&model).unwrap().unwrap();
        assert!(assembly.joints.is_empty());
        assert_eq!(notes, ["joint 1: it joins a component of another design"]);
    }

    #[test]
    fn fixed_joints_join_the_placements_they_name() {
        let fixed = json!({"kind": "fixed", "angle": 0.25, "translation_offset": [5.0, 0.0, 0.0]});
        let model = with_joints(
            nested(false, None),
            json!([
                // Between the two blocks inside the frame.
                joint(
                    1,
                    fixed.clone(),
                    connector(Some((1, &[]))),
                    connector(Some((2, &[])))
                ),
                // The frame to the design itself.
                joint(2, fixed.clone(), connector(None), connector(Some((0, &[])))),
                // The second block, named through the frame it sits in: its frame is in the
                // frame's space, where the block is stored 30 mm along X.
                joint(
                    3,
                    fixed.clone(),
                    connector(Some((1, &[]))),
                    connector_at(0, &["2"], at([30.0, 0.0, 0.0], 0.0))
                ),
            ]),
        );
        let (assembly, left_out) = local_components(&model).unwrap().unwrap();
        assert!(left_out.is_empty(), "{left_out:?}");
        let sides: Vec<(Vec<usize>, Vec<usize>)> = assembly
            .joints
            .iter()
            .map(|j| (j.a.clone(), j.b.clone()))
            .collect();
        assert_eq!(
            sides,
            [
                (vec![0, 0], vec![0, 1]),
                (vec![], vec![0]),
                (vec![0, 0], vec![0, 1])
            ]
        );
        for joint in &assembly.joints {
            assert!(near(&joint.frame_a, &IDENTITY), "{joint:?}");
            assert!(
                near(&joint.frame_b, &at([5.0, 0.0, 0.0], 0.25)),
                "{joint:?}"
            );
        }
    }

    #[test]
    fn a_grounded_nested_occurrence_keeps_its_world_placement() {
        let model = with_joints(
            nested(false, None),
            json!([{
                "id": "f3d:model:joint#1",
                "operands": {
                    "arity": "grounded",
                    "connector": connector_at(0, &["2"], at([8.0, 9.0, 10.0], 0.5)),
                    "offset_frame": at([1.0, 2.0, 3.0], 0.25),
                },
                "suppressed": false,
            }]),
        );
        let (assembly, notes) = local_components(&model).unwrap().unwrap();
        assert!(notes.is_empty(), "{notes:?}");
        assert_eq!(assembly.joints.len(), 1);
        let ground = &assembly.joints[0];
        assert!(ground.a.is_empty());
        assert_eq!(ground.b, [0, 1]);
        assert_eq!(ground.motion, Motion::Rigid);
        let world = at([130.0, 0.0, 0.0], 0.0);
        assert_eq!(world_at(&assembly, &ground.b), Some(world));
        assert_eq!(ground.frame_a, world);
        assert_eq!(ground.frame_b, FACING);
    }

    #[test]
    fn joints_that_cannot_be_written_are_left_out_with_the_reason() {
        let fixed = json!({"kind": "fixed"});
        let model = with_joints(
            nested(false, None),
            json!([
                joint(
                    1,
                    json!({"kind": "ball"}),
                    connector(Some((1, &[]))),
                    connector(Some((2, &[])))
                ),
                joint(
                    2,
                    fixed.clone(),
                    connector(Some((1, &[]))),
                    connector(Some((1, &[])))
                ),
                joint(
                    3,
                    fixed.clone(),
                    connector(Some((1, &[]))),
                    connector(Some((9, &[])))
                ),
                joint(
                    4,
                    fixed.clone(),
                    connector(Some((1, &[]))),
                    json!({"operand": {"container": {"container": "external",
                        "external_document": {"resolution": "missing"}}, "object": "x"},
                        "frame": IDENTITY, "detached": false})
                ),
            ]),
        );
        let (assembly, left_out) = local_components(&model).unwrap().unwrap();
        assert!(assembly.joints.is_empty());
        assert_eq!(
            left_out,
            [
                "joint 1: ball joints are not converted yet",
                "joint 2: it joins a component to itself",
                "joint 3: it joins a component the design does not place",
                "joint 4: it joins a component of another design",
            ]
        );
    }

    #[test]
    fn a_component_holding_others_converts_wherever_it_is_placed() {
        // Two frames, each holding the two blocks in the same places.
        let model = nested(
            false,
            Some(json!([
                placed(0, "frame", root(), 0.0),
                placed(1, "frame", root(), 100.0),
                placed(2, "block", inside(0), 0.0),
                placed(3, "block", inside(0), 30.0),
                placed(4, "block", inside(1), 0.0),
                placed(5, "block", inside(1), 30.0),
            ])),
        );
        let fixed = json!({"kind": "fixed"});
        let model = with_joints(
            model,
            json!([
                // The frame's own joint, stored once for each frame.
                joint(
                    1,
                    fixed.clone(),
                    connector(Some((4, &[]))),
                    connector(Some((5, &[])))
                ),
                joint(
                    2,
                    fixed.clone(),
                    connector(Some((2, &[]))),
                    connector(Some((3, &[])))
                ),
                // From one frame's block to the other frame's, and to the design.
                joint(
                    3,
                    fixed.clone(),
                    connector(Some((2, &[]))),
                    connector(Some((4, &[])))
                ),
                joint(4, fixed, connector(None), connector(Some((3, &[])))),
            ]),
        );
        let (assembly, left_out) = local_components(&model).unwrap().unwrap();
        assert_eq!(assembly.parts.len(), 1, "one frame, placed twice");
        assert_eq!(assembly.placements.len(), 2);
        // The frame's joint is written once, through the first frame: the design holds the
        // frame once, so both frames' blocks follow it.
        let sides: Vec<_> = assembly
            .joints
            .iter()
            .map(|j| (j.a.clone(), j.b.clone()))
            .collect();
        // A joint from one frame's block to something outside that frame cannot hold the block
        // alone, as both frames hold it: it holds that whole frame.
        assert_eq!(
            sides,
            [
                (vec![0, 0], vec![0, 1]),
                (vec![0], vec![1]),
                (vec![], vec![0]),
            ]
        );
        assert_eq!(
            left_out,
            [
                format!("joint 3: {WHOLE_COPY}"),
                format!("joint 4: {WHOLE_COPY}"),
            ]
        );
        // The second frame is placed so that its first block meets the first frame's.
        let first = world_at(&assembly, &[0, 0]).unwrap();
        let second = world_at(&assembly, &[1, 0]).unwrap();
        for axis in 0..3 {
            assert!(
                (first[axis][3] - second[axis][3]).abs() < 1e-9,
                "{first:?} {second:?}"
            );
        }
    }

    #[test]
    fn a_nested_occurrence_named_by_its_key_alone_is_found_inside_its_container() {
        // As inside a design inserted into the frame: the block's id ends in its key but is
        // not built from the frame's.
        let mut block = placed(1, "block", inside(0), 0.0);
        block["id"] = json!("f3d:inserted/model:occurrence#a1");
        let model = nested(
            false,
            Some(json!([
                placed(0, "frame", root(), 100.0),
                block,
                placed(2, "block", inside(0), 30.0),
            ])),
        );
        let model = with_joints(
            model,
            json!([joint(
                1,
                json!({"kind": "fixed"}),
                connector(Some((0, &["a1"]))),
                connector(Some((2, &[])))
            )]),
        );
        let (assembly, left_out) = local_components(&model).unwrap().unwrap();
        assert!(left_out.is_empty(), "{left_out:?}");
        let sides: Vec<_> = assembly
            .joints
            .iter()
            .map(|j| (j.a.clone(), j.b.clone()))
            .collect();
        assert_eq!(sides, [(vec![0, 0], vec![0, 1])]);
    }

    fn placed(key: u32, definition: &str, parent: Value, x: f64) -> Value {
        json!({
            "id": format!("f3d:model:occurrence#{key}"),
            "prototype": {"scope": "local", "definition": format!("f3d:model:component#{definition}")},
            "parent": parent,
            "ordinal": key,
            "transform": [[1.0, 0.0, 0.0, x], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0]],
            "scale": [1.0, 1.0, 1.0],
        })
    }

    fn inside(key: u32) -> Value {
        json!({"kind": "occurrence", "occurrence": format!("f3d:model:occurrence#{key}")})
    }

    #[test]
    fn placements_the_source_hides_are_placed_hidden() {
        // Block 2 is a hidden occurrence; the frame and block 1 are shown.
        let mut hidden_block = placed(2, "block", inside(0), 30.0);
        hidden_block["visible"] = json!(false);
        let model = nested(
            false,
            Some(json!([
                placed(0, "frame", root(), 100.0),
                placed(1, "block", inside(0), 0.0),
                hidden_block,
            ])),
        );
        let assembly = local_assembly(&model).unwrap().unwrap();
        let frame = assembly.parts[assembly.placements[0].part]
            .assembly
            .as_ref()
            .unwrap();
        let hidden: Vec<bool> = frame.placements.iter().map(|p| p.hidden).collect();
        assert_eq!(hidden, [false, true]);
        assert!(!assembly.placements[0].hidden);

        // When the block's only body is hidden, every placement of it is hidden; the part
        // is placed all the same, so its geometry stays in the design.
        let mut wire = serde_json::to_value(&nested(false, None).ir).unwrap();
        wire["model"]["bodies"][0]["visible"] = json!(false);
        let model = Model {
            ir: cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap(),
            ..nested(false, None)
        };
        let assembly = local_assembly(&model).unwrap().unwrap();
        let frame = assembly.parts[assembly.placements[0].part]
            .assembly
            .as_ref()
            .unwrap();
        assert_eq!(frame.placements.len(), 2);
        assert!(frame.placements.iter().all(|p| p.hidden));
        assert!(frame
            .parts
            .iter()
            .all(|part| !part.ir.model.bodies.is_empty()));
    }

    #[test]
    fn a_designs_root_component_is_held_where_it_is() {
        // The frame is the document's root component, and the first block the root of a
        // design inserted inside it: both are held; the second block is not.
        let mut frame = placed(0, "frame", root(), 100.0);
        frame["id"] = json!("f3d:model:occurrence#root-0");
        let inside_root =
            json!({"kind": "occurrence", "occurrence": "f3d:model:occurrence#root-0"});
        let mut first = placed(1, "block", inside_root.clone(), 0.0);
        first["id"] = json!("f3d:model:occurrence#root-1");
        let second = placed(2, "block", inside_root, 30.0);
        let assembly = local_assembly(&nested(false, Some(json!([frame, first, second]))))
            .unwrap()
            .unwrap();
        let mut held = assembly.held.clone();
        held.sort();
        assert_eq!(held, [vec![0], vec![0, 0]]);
    }

    #[test]
    fn components_inside_components_become_sub_assemblies() {
        let assembly = local_assembly(&nested(false, None)).unwrap().unwrap();
        assert_eq!(assembly.parts.len(), 1);
        assert_eq!(assembly.placements.len(), 1);
        assert_eq!(assembly.placements[0].transform[0][3], 100.0);
        let frame = &assembly.parts[0];
        assert_eq!(frame.name, "Frame");
        assert!(frame.ir.model.bodies.is_empty());
        let inner = frame.assembly.as_ref().unwrap();
        assert_eq!(inner.parts.len(), 1, "Block is read once");
        assert_eq!(inner.parts[0].name, "Block");
        assert_eq!(inner.parts[0].ir.model.bodies.len(), 1);
        let xs: Vec<f64> = inner.placements.iter().map(|p| p.transform[0][3]).collect();
        assert_eq!(xs, [0.0, 30.0]);
    }

    #[test]
    fn a_component_holding_others_keeps_its_own_bodies_beside_them() {
        let assembly = local_assembly(&nested(true, None)).unwrap().unwrap();
        let inner = assembly.parts[0].assembly.as_ref().unwrap();
        assert_eq!(inner.parts.len(), 2);
        assert_eq!(inner.own_part, Some(1));
        assert_eq!(inner.parts[1].name, "Frame bodies");
        assert_eq!(inner.placements.len(), 3);
        assert_eq!(inner.placements[2].part, 1);
        assert_eq!(inner.placements[2].transform, IDENTITY);
    }

    /// The x of each block placed in the frame a placement holds, by the part it places.
    fn block_xs(assembly: &Assembly, placement: usize) -> Vec<f64> {
        let frame = &assembly.parts[assembly.placements[placement].part];
        let inner = frame.assembly.as_ref().unwrap();
        inner.placements.iter().map(|p| p.transform[0][3]).collect()
    }

    #[test]
    fn a_component_placed_twice_with_different_children_is_a_definition_per_layout() {
        let mut report = Report::default();
        let model = nested(
            false,
            Some(json!([
                placed(0, "frame", root(), 0.0),
                placed(1, "frame", root(), 100.0),
                placed(2, "block", inside(0), 0.0),
                placed(3, "block", inside(1), 30.0),
            ])),
        );
        let (assembly, _) = local_components_with_threads(
            &model,
            &BTreeMap::new(),
            1,
            crate::PartPositions::LastShown,
            &mut report,
        )
        .unwrap()
        .unwrap();
        assert_eq!(assembly.parts.len(), 2);
        assert_eq!(assembly.parts[0].name, "Frame");
        assert_eq!(assembly.parts[1].name, "Frame (2)");
        assert_eq!(assembly.placements[0].part, 0);
        assert_eq!(assembly.placements[1].part, 1);
        assert_eq!(block_xs(&assembly, 0), [0.0]);
        assert_eq!(block_xs(&assembly, 1), [30.0]);
        assert!(report.losses().iter().any(|l| l.detail
            == "1 components written as 2 definitions because their instances hold different layouts"));
    }

    #[test]
    fn a_different_set_of_children_is_a_different_layout() {
        let model = nested(
            true,
            Some(json!([
                placed(0, "frame", root(), 0.0),
                placed(1, "frame", root(), 100.0),
                placed(2, "block", inside(0), 0.0),
                placed(3, "block", inside(0), 30.0),
                placed(4, "block", inside(1), 0.0),
            ])),
        );
        let assembly = local_assembly(&model).unwrap().unwrap();
        assert_eq!(assembly.parts.len(), 2);
        let counts: Vec<usize> = (0..2)
            .map(|i| {
                let sub = assembly.parts[assembly.placements[i].part]
                    .assembly
                    .as_ref()
                    .unwrap();
                // The children and the component's own bodies beside them.
                assert!(sub.own_part.is_some());
                sub.placements.len()
            })
            .collect();
        assert_eq!(counts, [3, 2]);
    }

    #[test]
    fn a_layout_that_differs_only_deep_inside_separates_every_level_above_it() {
        // outer > frame > block, the block in a different place in the second outer.
        let model = nested(
            false,
            Some(json!([
                placed(0, "outer", root(), 0.0),
                placed(1, "outer", root(), 500.0),
                placed(2, "frame", inside(0), 10.0),
                placed(3, "frame", inside(1), 10.0),
                placed(4, "block", inside(2), 0.0),
                placed(5, "block", inside(3), 30.0),
            ])),
        );
        let mut wire = serde_json::to_value(&model.ir).unwrap();
        wire["model"]["product_definitions"]
            .as_array_mut()
            .unwrap()
            .push(json!({"id": "f3d:model:component#outer", "kind": "part", "label": "Outer", "bodies": []}));
        let model = Model {
            ir: cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap(),
            ..model
        };
        let assembly = local_assembly(&model).unwrap().unwrap();
        assert_eq!(assembly.parts.len(), 2);
        let names: Vec<&str> = assembly.parts.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["Outer", "Outer (2)"]);
        let frames: Vec<&Assembly> = assembly
            .parts
            .iter()
            .map(|outer| {
                let sub = outer.assembly.as_ref().unwrap();
                assert_eq!(sub.parts.len(), 1);
                sub.parts[0].assembly.as_ref().unwrap()
            })
            .collect();
        assert_eq!(frames[0].placements[0].transform[0][3], 0.0);
        assert_eq!(frames[1].placements[0].transform[0][3], 30.0);
        let named: Vec<&str> = assembly
            .parts
            .iter()
            .map(|p| p.assembly.as_ref().unwrap().parts[0].name.as_str())
            .collect();
        assert_eq!(named, ["Frame", "Frame (2)"]);
    }

    #[test]
    fn numeric_noise_below_the_tolerance_does_not_split_a_definition() {
        let mut noisy = placed(3, "block", inside(1), 30.0 + 1e-10);
        noisy["transform"][1][1] = json!(1.0 - 1e-12);
        let mut report = Report::default();
        let model = nested(
            false,
            Some(json!([
                placed(0, "frame", root(), 0.0),
                placed(1, "frame", root(), 100.0),
                placed(2, "block", inside(0), 30.0),
                noisy,
            ])),
        );
        let (assembly, _) = local_components_with_threads(
            &model,
            &BTreeMap::new(),
            1,
            crate::PartPositions::LastShown,
            &mut report,
        )
        .unwrap()
        .unwrap();
        assert_eq!(assembly.parts.len(), 1);
        assert_eq!(assembly.placements[0].part, assembly.placements[1].part);
        assert!(!report
            .losses()
            .iter()
            .any(|l| l.detail.contains("different layouts")));
        // A difference past the tolerance does split.
        let model = nested(
            false,
            Some(json!([
                placed(0, "frame", root(), 0.0),
                placed(1, "frame", root(), 100.0),
                placed(2, "block", inside(0), 30.0),
                placed(3, "block", inside(1), 30.001),
            ])),
        );
        assert_eq!(local_assembly(&model).unwrap().unwrap().parts.len(), 2);
    }

    #[test]
    fn nested_components_inside_a_missing_parent_convert_as_stored() {
        // A block placed inside an occurrence the design does not have.
        let err = local_assembly(&nested(
            false,
            Some(json!([
                placed(0, "block", root(), 0.0),
                placed(1, "block", inside(7), 0.0),
            ])),
        ))
        .unwrap_err();
        assert_eq!(
            err,
            "some components are placed inside components the design does not place"
        );
    }

    #[test]
    fn a_design_without_components_is_not_an_assembly() {
        let part = crate::read(
            Format::InventorPart,
            &fixture("synthetic_cube.ipt"),
            &mut Report::default(),
        )
        .unwrap();
        assert!(local_assembly(&part).unwrap().is_none());
    }

    #[test]
    fn fusion_components_without_bodies_are_reported() {
        let mut report = Report::default();
        let model = crate::read(
            Format::FusionArchive,
            &fixture("design_copy_paste.f3d"),
            &mut report,
        )
        .unwrap();
        assert!(model.assembly.is_none());
        let loss = report
            .losses()
            .iter()
            .find(|loss| loss.subject == "components")
            .unwrap();
        assert_eq!(loss.severity, Severity::Degraded);
        assert!(
            loss.detail
                .contains("cadmpeg does not say which bodies belong to component"),
            "{}",
            loss.detail
        );
    }

    #[test]
    fn file_names_come_from_either_separator() {
        assert_eq!(file_name(r"C:\Work\Parts\Bracket.ipt"), "Bracket.ipt");
        assert_eq!(file_name("/home/a/Bracket.ipt"), "Bracket.ipt");
        assert_eq!(file_name("Bracket.ipt"), "Bracket.ipt");
    }

    #[test]
    fn placements_must_be_rigid_and_whole() {
        let identity = serde_json::json!([[1, 0, 0, 5], [0, 1, 0, 0], [0, 0, 1, 0]]);
        let row =
            |scale: serde_json::Value| serde_json::json!({"transform": identity, "scale": scale});
        assert_eq!(
            transform(&row(serde_json::json!([1, 1, 1]))).unwrap()[0],
            [1.0, 0.0, 0.0, 5.0]
        );
        assert!(transform(&row(serde_json::json!([2, 1, 1]))).is_err());
        assert!(transform(&serde_json::json!({"transform": [[1, 0, 0]]})).is_err());
    }

    #[test]
    fn parallel_slices_keep_parts_names_histories_and_tree_order_identical() {
        let mut unnamed = nested(true, None);
        let mut wire = serde_json::to_value(&unnamed.ir).unwrap();
        for definition in wire["model"]["product_definitions"].as_array_mut().unwrap() {
            definition.as_object_mut().unwrap().remove("name");
        }
        unnamed.ir = cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap();
        for model in [
            nested(true, None),
            unnamed,
            with_history(true),
            with_history(false),
            with_components(true),
        ] {
            let serial = local_components_with_threads(
                &model,
                &BTreeMap::new(),
                1,
                crate::PartPositions::LastShown,
                &mut Report::default(),
            );
            let parallel = local_components_with_threads(
                &model,
                &BTreeMap::new(),
                4,
                crate::PartPositions::LastShown,
                &mut Report::default(),
            );
            assert_eq!(format!("{serial:?}"), format!("{parallel:?}"));
        }
    }

    #[test]
    fn compact_component_rows_keep_headers_history_placements_and_arena_order() {
        for mut model in [
            nested(true, None),
            with_history(true),
            with_history(false),
            with_components(true),
            with_inserted_design(json!([])),
        ] {
            // Selection must preserve source order, even when ids sort differently.
            model.ir.model.points.reverse();
            model.ir.fragment = true;
            let expanded = serde_json::to_value(&model.ir).unwrap();
            let text = serde_json::to_string(&model.ir).unwrap();
            let packed = slicing::Packed::read(&text).unwrap();
            let document = packed.document().unwrap();
            let index = slicing::Index::packed(&packed, &document).unwrap();
            assert_eq!(source_posed(&expanded), source_posed(&document));
            for occurrence in listed(expanded.pointer("/model/occurrences")) {
                assert_eq!(
                    inserted_design_name(&expanded, occurrence),
                    inserted_design_name(&document, occurrence)
                );
            }
            let mut selections = vec![(HistoryOwner::Root, Vec::new())];
            for definition in &model.ir.model.product_definitions {
                selections.push((
                    HistoryOwner::Definition(definition.id.as_str()),
                    definition.bodies.iter().map(|id| id.as_str()).collect(),
                ));
            }
            selections.push((
                HistoryOwner::Unknown,
                model
                    .ir
                    .model
                    .bodies
                    .iter()
                    .map(|body| body.id.as_str())
                    .collect(),
            ));
            for (owner, bodies) in selections {
                let expected = only(&model, &expanded, &bodies, owner, "part".to_owned()).unwrap();
                let actual =
                    only_indexed(&model, &index, &bodies, owner, "part".to_owned()).unwrap();
                assert_eq!(
                    serde_json::to_value(&actual.ir).unwrap(),
                    serde_json::to_value(&expected.ir).unwrap()
                );
            }
        }
    }
    #[test]
    fn smart_positions_admit_free_parts_and_reject_bounds_or_nonrigid_copies() {
        use crate::PartPositions;
        for (x, copies, expected, reason) in [
            (10.0, false, 10.0, "current"),
            (1000.0, false, 0.0, "bounds"),
            (10.0, true, 0.0, "instances"),
        ] {
            let model = with_components(true);
            let mut wire = serde_json::to_value(&model.ir).unwrap();
            wire["model"]["occurrences"][0]["native_ref"] =
                json!("f3d:Design/BulkStream.dat:design-component-occurrence#11");
            wire["native"]["f3d"]["design_component_occurrences"] = json!([
                {"id":"f3d:Design/BulkStream.dat:design-component-occurrence#11", "transform": at([0.0,0.0,0.0],0.0), "current_pose":{"transform":at([x/10.0,0.0,0.0],0.0)}}
            ]);
            if copies {
                wire["model"]["occurrences"][0]["native_ref"] = json!("f3d:xref:reference#1");
                wire["model"]["occurrences"][1]["native_ref"] = json!("f3d:xref:reference#2");
                wire["native"]["f3d"]["design_component_occurrences"] = json!([]);
                wire["native"]["f3d"]["xref_references"] = json!([
                    {"id":"f3d:xref:reference#1", "relative_path":"same.f3d", "transform": at([0.0,0.0,0.0],0.0), "current_pose":{"transform":at([x/10.0,0.0,0.0],0.0)}},
                    {"id":"f3d:xref:reference#2", "relative_path":"same.f3d", "transform":at([3.0,0.0,0.0],0.0)}
                ]);
            }
            let model = Model {
                ir: cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap(),
                ..model
            };
            let mut report = Report::default();
            let (assembly, _) = local_components_with_threads(
                &model,
                &BTreeMap::new(),
                1,
                PartPositions::Smart,
                &mut report,
            )
            .unwrap()
            .unwrap();
            assert_eq!(
                assembly.placements[0].transform[0][3], expected,
                "{reason}: {report:?}"
            );
            let counts = report.part_positions().unwrap().1;
            assert_eq!(counts.total(), 2);
            match reason {
                "current" => assert_eq!(counts.current, 1),
                "bounds" => assert_eq!(counts.bounds, 1),
                _ => assert_eq!(counts.instances, 1),
            }
            let shown = local_assembly_with_positions(&model, PartPositions::LastShown)
                .unwrap()
                .unwrap();
            assert_eq!(shown.placements[0].transform[0][3], x);
            let first = local_assembly_with_positions(&model, PartPositions::FirstPlaced)
                .unwrap()
                .unwrap();
            assert_eq!(first.placements[0].transform[0][3], 0.0);
        }
    }

    #[test]
    fn smart_positions_admit_rigid_inserted_copies_independently_of_record_order() {
        for reversed in [false, true] {
            let model = with_components(true);
            let mut wire = serde_json::to_value(&model.ir).unwrap();
            let mut third = wire["model"]["occurrences"][1].clone();
            third["id"] = json!("f3d:model:occurrence#third");
            third["ordinal"] = json!(2);
            third["transform"] = json!(at([100.0, 0.0, 0.0], 0.0));
            wire["model"]["occurrences"]
                .as_array_mut()
                .unwrap()
                .push(third);
            wire["model"]["occurrences"][0]["native_ref"] = json!("f3d:xref:reference#1");
            wire["model"]["occurrences"][1]["native_ref"] = json!("f3d:xref:reference#2");
            let mut records = vec![
                json!({"id":"f3d:xref:reference#1","relative_path":"same.f3d","current_pose":{"transform":at([1.0,0.0,0.0],0.0)}}),
                json!({"id":"f3d:xref:reference#2","relative_path":"same.f3d","current_pose":{"transform":at([4.0,0.0,0.0],0.0)}}),
            ];
            if reversed {
                records.reverse();
                wire["model"]["occurrences"]
                    .as_array_mut()
                    .unwrap()
                    .reverse();
            }
            wire["native"]["f3d"]["xref_references"] = json!(records);
            let model = Model {
                ir: cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap(),
                ..model
            };
            let mut report = Report::default();
            let (assembly, _) = local_components_with_threads(
                &model,
                &BTreeMap::new(),
                1,
                crate::PartPositions::Smart,
                &mut report,
            )
            .unwrap()
            .unwrap();
            assert_eq!(
                assembly
                    .placements
                    .iter()
                    .map(|p| p.transform[0][3])
                    .collect::<Vec<_>>(),
                vec![10.0, 40.0, 100.0]
            );
            let counts = report.part_positions().unwrap().1;
            assert_eq!(counts.current, 2);
            assert_eq!(counts.first_placed, 1);
            assert_eq!(counts.instances, 0);
        }
    }

    #[test]
    fn smart_positions_reject_a_pose_that_breaks_a_touching_fixed_joint() {
        let model = with_components(true);
        let mut wire = serde_json::to_value(&model.ir).unwrap();
        let mut records = Vec::new();
        for (index, x) in [(0, 10.0), (1, 30.0)] {
            wire["model"]["occurrences"][index]["native_ref"] = json!(format!(
                "f3d:Design/BulkStream.dat:design-component-occurrence#{index}"
            ));
            records.push(json!({"id":format!("f3d:Design/BulkStream.dat:design-component-occurrence#{index}"),"transform":at([if index==0 {0.0}else{3.0},0.0,0.0],0.0),"current_pose":{"transform":at([x/10.0,0.0,0.0],0.0)}}));
        }
        wire["native"]["f3d"]["design_component_occurrences"] = json!(records);
        let model = Model {
            ir: cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap(),
            ..model
        };
        let model = with_joints(
            model,
            json!([joint(
                1,
                json!({"kind":"fixed"}),
                connector_at(0, &[], at([30.0, 0.0, 0.0], 0.0)),
                connector_at(1, &[], facing(IDENTITY))
            )]),
        );
        let mut report = Report::default();
        let (assembly, _) = local_components_with_threads(
            &model,
            &BTreeMap::new(),
            1,
            crate::PartPositions::Smart,
            &mut report,
        )
        .unwrap()
        .unwrap();
        assert_eq!(assembly.placements[0].transform[0][3], 0.0);
        assert!(report.part_positions().unwrap().1.joints > 0);
    }

    /// `model` with these saved-scene displayed poses (occurrence key, x in mm).
    fn with_displayed(model: &Model, poses: &[(u32, f64)]) -> Model {
        let mut wire = serde_json::to_value(&model.ir).unwrap();
        wire["native"]["f3d"]["display_poses"] = json!(poses
            .iter()
            .map(|(key, x)| json!({
                "id": format!("f3d:scene:display-pose#{key}"), "stream": "world", "byte_offset": 0,
                "path": [1, key], "binding": {"state": "bound", "occurrence": format!("f3d:model:occurrence#{key}")},
                "transform": [[1,0,0,x],[0,1,0,0],[0,0,1,0],[0,0,0,1]]
            }))
            .collect::<Vec<_>>());
        Model {
            ir: cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap(),
            ..model.clone()
        }
    }

    /// Two frames, each holding two blocks, so the frame component can be shared.
    fn two_frames() -> Model {
        nested(
            false,
            Some(json!([
                placed(0, "frame", root(), 100.0),
                placed(1, "block", inside(0), 0.0),
                placed(2, "block", inside(0), 30.0),
                placed(3, "frame", root(), 300.0),
                placed(4, "block", inside(3), 0.0),
                placed(5, "block", inside(3), 30.0),
            ])),
        )
    }

    #[test]
    fn displayed_poses_that_keep_the_relative_placement_keep_a_shared_component_shared() {
        use crate::PartPositions::{FirstPlaced, LastShown, Smart};
        let model = two_frames();
        let first = local_assembly_with_positions(&model, FirstPlaced)
            .unwrap()
            .unwrap();
        // Scene matrices are float32: each instance carries its own rounding.
        let same = with_displayed(
            &model,
            &[
                (0, 100.0002),
                (1, 100.0001),
                (2, 129.9998),
                (3, 300.0003),
                (4, 299.9999),
                (5, 330.0002),
            ],
        );
        for mode in [Smart, LastShown] {
            let shown = local_assembly_with_positions(&same, mode).unwrap().unwrap();
            assert_eq!(shown.parts.len(), first.parts.len(), "{mode:?}");
            assert_eq!(shown.placements.len(), first.placements.len(), "{mode:?}");
        }
        // A block that really sits elsewhere in one instance needs its own copy.
        let differs = with_displayed(
            &model,
            &[
                (0, 100.0),
                (1, 100.0),
                (2, 130.0),
                (3, 300.0),
                (4, 300.0),
                (5, 336.0),
            ],
        );
        let shown = local_assembly_with_positions(&differs, LastShown)
            .unwrap()
            .unwrap();
        assert!(shown.parts.len() > first.parts.len());
    }

    #[test]
    fn a_displayed_holder_that_carries_a_body_outside_the_extent_keeps_its_first_placement() {
        use crate::PartPositions::{FirstPlaced, LastShown, Smart};
        // Only the holder is displayed; its blocks follow it, 4900 mm outside the model.
        let far = with_displayed(&two_frames(), &[(0, 5000.0)]);
        let moved = local_assembly_with_positions(&far, LastShown)
            .unwrap()
            .unwrap();
        assert!(near(
            &world_at(&moved, &[0, 0]).unwrap(),
            &at([5000.0, 0.0, 0.0], 0.0)
        ));
        let mut report = Report::default();
        let (kept, _) =
            local_components_with_threads(&far, &BTreeMap::new(), 1, Smart, &mut report)
                .unwrap()
                .unwrap();
        let first = local_assembly_with_positions(&far, FirstPlaced)
            .unwrap()
            .unwrap();
        for path in [vec![0], vec![0, 0], vec![0, 1], vec![1, 1]] {
            assert!(
                near(
                    &world_at(&kept, &path).unwrap(),
                    &world_at(&first, &path).unwrap()
                ),
                "{path:?}"
            );
        }
        let counts = report.part_positions().unwrap().1;
        assert_eq!((counts.displayed, counts.bounds), (0, 1), "{counts:?}");
        assert_eq!(counts.total(), 6);
        // A part with a pose of its own far outside the model takes its displayed holder
        // back to the first placement too, along with the parts below it.
        let apart = with_displayed(&two_frames(), &[(0, 100.0), (1, 5000.0), (2, 130.0)]);
        let (kept, _) = local_components_with_threads(
            &apart,
            &BTreeMap::new(),
            1,
            Smart,
            &mut Report::default(),
        )
        .unwrap()
        .unwrap();
        for path in [vec![0], vec![0, 0], vec![0, 1]] {
            assert!(
                near(
                    &world_at(&kept, &path).unwrap(),
                    &world_at(&first, &path).unwrap()
                ),
                "{path:?}"
            );
        }
    }

    #[test]
    fn a_conflicting_holder_keeps_its_first_placement_inside_a_holder_that_moves() {
        use crate::PartPositions::{FirstPlaced, LastShown, Smart};
        let model = nested(
            false,
            Some(json!([
                placed(0, "frame", root(), 100.0),
                placed(1, "frame", inside(0), 0.0),
                placed(2, "block", inside(1), 0.0),
                placed(3, "block", inside(1), 30.0),
                placed(4, "block", inside(0), 60.0),
            ])),
        );
        // Blocks 2 and 3 ask for frame worlds 100 and 130, which disagree, and the outer
        // frame is shown 1 mm along.
        let shown = with_displayed(&model, &[(0, 101.0), (2, 100.0), (3, 160.0)]);
        let first = local_assembly_with_positions(&shown, FirstPlaced)
            .unwrap()
            .unwrap();
        for mode in [Smart, LastShown] {
            let mut report = Report::default();
            let (placed, _) =
                local_components_with_threads(&shown, &BTreeMap::new(), 1, mode, &mut report)
                    .unwrap()
                    .unwrap();
            assert!(near(
                &world_at(&placed, &[0]).unwrap(),
                &at([101.0, 0.0, 0.0], 0.0)
            ));
            // The conflicted holder stays where it was first placed, not 1 mm along.
            assert!(
                near(
                    &world_at(&placed, &[0, 0]).unwrap(),
                    &world_at(&first, &[0, 0]).unwrap()
                ),
                "{mode:?}"
            );
            assert_eq!(report.part_positions().unwrap().1.holder_conflicts, 1);
        }
    }
}
