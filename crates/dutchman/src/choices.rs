//! What a conversion keeps, as a preset and per-choice switches, and what keeping it is
//! expected to cost before the design is read.
//!
//! Presets: `Fast` keeps the shape only; `Standard` (the default) keeps feature history,
//! sketches and joints, but leaves feature history out when the estimate says
//! it would not fit in the memory this machine has free, and says so; `Full` keeps everything.
//! Per-choice switches (`--no-history`, `--no-sketches`, `--no-joints`) narrow any preset.

use dutchman_core::Format;
use std::path::Path;

/// A named set of choices.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, clap::ValueEnum)]
pub enum Preset {
    /// The shape only: one import, no feature history, sketches or joints. Least memory.
    Fast,
    /// Feature history, sketches and joints, with history left out (and the
    /// report saying so) when it would not fit in free memory.
    #[default]
    Standard,
    /// Everything, whatever it costs.
    Full,
}

impl Preset {
    /// The next lighter preset, when there is one.
    pub fn lighter(self) -> Option<Preset> {
        match self {
            Preset::Full => Some(Preset::Standard),
            Preset::Standard => Some(Preset::Fast),
            Preset::Fast => None,
        }
    }

    /// The name the CLI, MCP and the app use for it.
    pub fn name(self) -> &'static str {
        match self {
            Preset::Fast => "fast",
            Preset::Standard => "standard",
            Preset::Full => "full",
        }
    }
}

/// What a conversion keeps, after its preset and switches are applied.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Choices {
    /// Feature history, replayed and checked in SolidCraft when it maps.
    pub history: bool,
    /// Sketches, also when history is left out (kept for reference).
    pub sketches: bool,
    /// Assembly joints.
    pub joints: bool,
    /// Standard only: leave history out when it would not fit in free memory.
    pub history_within_budget: bool,
}

impl Choices {
    /// The choices `preset` makes, before any switch narrows them.
    pub fn of(preset: Preset) -> Choices {
        match preset {
            Preset::Fast => Choices {
                history: false,
                sketches: false,
                joints: false,
                history_within_budget: false,
            },
            Preset::Standard => Choices {
                history: true,
                sketches: true,
                joints: true,
                history_within_budget: true,
            },
            Preset::Full => Choices {
                history: true,
                sketches: true,
                joints: true,
                history_within_budget: false,
            },
        }
    }
}

/// What converting a file with some choices is expected to cost.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Estimate {
    /// Expected peak memory, in bytes.
    pub peak_bytes: u64,
    /// Expected time, rounded up to whole seconds for compatibility.
    pub seconds: u64,
    /// Expected elapsed time before rounding; time bands apply to this value.
    pub seconds_exact: f64,
    /// Shared compatibility band. Phase models use their total-memory envelope;
    /// older combined models retain their heuristic. Prefer the separate bands.
    pub band: f64,
    /// Relative peak-memory band (0.3 is ±30 %); unmeasured formats retain a heuristic.
    pub memory_band: f64,
    /// Relative time band, or None when measured timings do not support the current model.
    pub time_band: Option<f64>,
}

impl Estimate {
    /// Independent history fits cannot make retaining history cheaper than omitting it.
    fn at_least(self, without_history: Self) -> Self {
        Self {
            peak_bytes: self.peak_bytes.max(without_history.peak_bytes),
            seconds: self.seconds.max(without_history.seconds),
            seconds_exact: self.seconds_exact.max(without_history.seconds_exact),
            ..self
        }
    }
}

/// A human-readable estimate shared by the CLI and desktop app.
pub fn estimate_text(estimate: Estimate) -> String {
    format!(
        "{}, {}",
        memory_text(estimate, false),
        time_text(estimate, false)
    )
}

/// The same estimate with its measured error bands, as the command line prints it.
pub fn estimate_text_with_bands(estimate: Estimate) -> String {
    format!(
        "{}, {}",
        memory_text(estimate, true),
        time_text(estimate, true)
    )
}

fn memory_text(estimate: Estimate, band: bool) -> String {
    if estimate.peak_bytes < 100_000_000 {
        "under 0.1 GB".to_owned()
    } else if band {
        format!(
            "about {:.1} GB (±{:.0} %)",
            gb(estimate.peak_bytes),
            estimate.memory_band * 100.0
        )
    } else {
        format!("about {:.1} GB", gb(estimate.peak_bytes))
    }
}

/// A time without a measured band for its format reads as rough.
fn time_text(estimate: Estimate, band: bool) -> String {
    if estimate.seconds < 60 {
        return "under a minute".to_owned();
    }
    let minutes = estimate.seconds.div_ceil(60);
    match estimate.time_band {
        Some(time_band) if band => format!("about {minutes} min (±{:.0} %)", time_band * 100.0),
        Some(_) => format!("about {minutes} min"),
        None => format!("roughly {minutes} min"),
    }
}

/// The result of checking a conversion against currently available memory.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MemoryCheck {
    Ok(Option<Estimate>),
    TooBig {
        /// The whole estimate, including the history check, as reports give it.
        estimate: Estimate,
        /// The conversion alone: what was compared with free memory, and what to tell people.
        conversion: Estimate,
        available: u64,
        lighter: Option<Preset>,
    },
}

impl MemoryCheck {
    /// The same note for every front end. An estimate from a file's size does not stop a
    /// conversion: memory is measured while it runs.
    pub fn warning(self) -> Option<String> {
        match self {
            Self::Ok(_) => None,
            Self::TooBig { conversion, available, .. } => Some(format!(
                "the estimate of about {:.1} GB is above the {:.1} GB free. Estimates from a file's size can be far off, so the conversion ran, watching memory",
                gb(conversion.peak_bytes), gb(available)
            )),
        }
    }
}

/// Bytes as decimal gigabytes, as displayed by every front end.
pub fn gb(bytes: u64) -> f64 {
    bytes as f64 / 1_000_000_000.0
}

/// Checks without decoding; unknown estimates or available memory never block a run.
pub fn memory_check(input: &Path, options: &crate::ConvertOptions) -> MemoryCheck {
    let available = if options.no_memory_check || options.preset == Preset::Fast {
        None
    } else {
        available_memory()
    };
    memory_check_with(input, options, available)
}

pub(crate) fn memory_check_with(
    input: &Path,
    options: &crate::ConvertOptions,
    available: Option<u64>,
) -> MemoryCheck {
    let choices = options.choices();
    let estimate = estimate_with_options(input, &choices, options);
    if options.no_memory_check || options.preset == Preset::Fast {
        return MemoryCheck::Ok(estimate);
    }
    let (Some(full), Some(conversion), Some(available)) = (
        estimate,
        conversion_estimate(input, &choices, options),
        available,
    ) else {
        return MemoryCheck::Ok(estimate);
    };
    // Reports include checking, but a check that cannot fit is skipped after reading.
    // It must not prevent conversion or consume Standard's history-retention budget.
    if conversion.peak_bytes <= available {
        return MemoryCheck::Ok(estimate);
    }
    MemoryCheck::TooBig {
        estimate: full,
        conversion,
        available,
        lighter: options.preset.lighter(),
    }
}

/// Conversion-only preflight. A history check that cannot fit is skipped after reading;
/// its extra memory must not make Standard discard otherwise usable editable history.
pub(crate) fn conversion_estimate(
    input: &Path,
    choices: &Choices,
    options: &crate::ConvertOptions,
) -> Option<Estimate> {
    estimate_with_options(
        input,
        choices,
        &crate::ConvertOptions {
            no_verify: true,
            ..options.clone()
        },
    )
}

struct Coefficients {
    format: Format,
    model: CostModel,
    band: f64,
    memory_band: f64,
    time_band: Option<f64>,
}

impl Coefficients {
    fn template(&self) -> Estimate {
        Estimate {
            peak_bytes: 0,
            seconds: 0,
            seconds_exact: 0.0,
            band: self.band,
            memory_band: self.memory_band,
            time_band: self.time_band,
        }
    }
}

enum CostModel {
    Stages(Stages),
    Legacy(LegacyCost),
}

struct Stages {
    read: StageCost,
    convert: StageCost,
    history: StageCost,
    check: StageCost,
    geometry_check: StageCost,
}

impl Stages {
    fn split(&self, sizes: Sizes, choices: &Choices, template: Estimate) -> ReadConvertSplit {
        let geometry = self.convert.estimate(sizes, template);
        let convert = if choices.history {
            self.history.estimate(sizes, template).at_least(geometry)
        } else {
            geometry
        };
        ReadConvertSplit {
            read: self.read.estimate(sizes, template),
            convert,
        }
    }
}

// Existing combined models for formats without phase measurements.
struct LegacyCost {
    base_bytes: u64,
    bytes_per_byte: f64,
    base_seconds: f64,
    seconds_per_mib: f64,
    history_factor: f64,
    history_seconds_factor: f64,
    startup: Option<StartupTime>,
    check_share: f64,
}

/// Linear costs from archive metadata. Memory after decode and checking is additional
/// to the preceding peak, so adding the three stages gives one consistent total.
struct StageCost {
    base_bytes: u64,
    // Some checking fits plateau at the observed assembly scale; this is an
    // empirical estimate, not a ceiling on what an unseen design can allocate.
    work_bytes_limit: Option<u64>,
    bytes_per_byte: f64,
    bytes_per_member: f64,
    base_seconds: f64,
    seconds_per_mib: f64,
    seconds_per_member: f64,
}

impl StageCost {
    fn estimate(&self, sizes: Sizes, template: Estimate) -> Estimate {
        let seconds_exact = self.base_seconds
            + self.seconds_per_mib * sizes.bytes as f64 / (1 << 20) as f64
            + self.seconds_per_member * sizes.members as f64;
        let work = (self.bytes_per_byte * sizes.bytes as f64
            + self.bytes_per_member * sizes.members as f64)
            .ceil() as u64;
        Estimate {
            peak_bytes: self
                .base_bytes
                .saturating_add(work.min(self.work_bytes_limit.unwrap_or(u64::MAX))),
            seconds: seconds_exact.ceil() as u64,
            seconds_exact,
            ..template
        }
    }
}

// Fixed elapsed startup is independent of content/history scaling. Windows history
// includes the memory guard and CLI discovery. A possible CLI-probe timeout is not
// normal startup work; only its ordinary startup cost belongs in the estimate.
struct StartupTime {
    windows_shape: f64,
    windows_history: f64,
    mac_shape: f64,
    mac_history: f64,
    windows_verify_shape: f64,
    windows_verify_history: f64,
    // FreeCAD recipe data support a much smaller content term than the old guess.
    content_rates: Option<(f64, f64)>,
}

// Fresh Linux measurements use the shared-index main build, four requested workers,
// six public Fusion assemblies and five smaller public/synthetic Fusion, Inventor and
// SolidWorks parts. Read, conversion (history on/off), and checking (history on/off)
// have independent memory/time fits. Small foreign parts calibrate floors only;
// their previous content slopes remain heuristics for larger files. Formats without
// phase measurements keep their existing combined model. See docs/presets.md for
// the coefficients' scope and measured misses. All format-dependent numbers live here.
const COEFFICIENTS: &[Coefficients] = &[
    Coefficients {
        format: Format::FusionArchive,
        model: CostModel::Stages(Stages {
            read: StageCost {
                base_bytes: 15_000_000,
                work_bytes_limit: None,
                bytes_per_byte: 5.7,
                bytes_per_member: 1_700_000.0,
                base_seconds: 0.0,
                seconds_per_mib: 0.1,
                seconds_per_member: 0.061,
            },
            convert: StageCost {
                base_bytes: 12_000_000,
                work_bytes_limit: None,
                bytes_per_byte: 15.0,
                bytes_per_member: 0.0,
                base_seconds: 0.0,
                seconds_per_mib: 0.11,
                seconds_per_member: 0.062,
            },
            history: StageCost {
                base_bytes: 9_000_000,
                work_bytes_limit: None,
                bytes_per_byte: 26.0,
                bytes_per_member: 0.0,
                base_seconds: 0.0,
                seconds_per_mib: 0.35,
                seconds_per_member: 0.0,
            },
            check: StageCost {
                base_bytes: 23_000_000,
                work_bytes_limit: Some(1_191_000_000),
                bytes_per_byte: 260.0,
                bytes_per_member: 0.0,
                base_seconds: 0.032,
                seconds_per_mib: 0.38,
                seconds_per_member: 0.0,
            },
            geometry_check: StageCost {
                base_bytes: 25_000_000,
                work_bytes_limit: Some(740_000_000),
                bytes_per_byte: 230.0,
                bytes_per_member: 0.0,
                base_seconds: 0.03,
                seconds_per_mib: 0.46,
                seconds_per_member: 0.0,
            },
        }),
        band: 0.45,
        memory_band: 0.45,
        time_band: None,
    },
    Coefficients {
        format: Format::FusionMultiDocument,
        model: CostModel::Stages(Stages {
            read: StageCost {
                base_bytes: 29_000_000,
                work_bytes_limit: None,
                bytes_per_byte: 20.0,
                bytes_per_member: 0.0,
                base_seconds: 0.011,
                seconds_per_mib: 1.2,
                seconds_per_member: 0.0,
            },
            convert: StageCost {
                base_bytes: 24_000_000,
                work_bytes_limit: None,
                bytes_per_byte: 16.0,
                bytes_per_member: 0.0,
                base_seconds: 0.02,
                seconds_per_mib: 0.32,
                seconds_per_member: 0.0,
            },
            history: StageCost {
                base_bytes: 12_000_000,
                work_bytes_limit: None,
                bytes_per_byte: 19.0,
                bytes_per_member: 0.0,
                base_seconds: 0.019,
                seconds_per_mib: 0.38,
                seconds_per_member: 0.0,
            },
            check: StageCost {
                base_bytes: 23_000_000,
                work_bytes_limit: None,
                bytes_per_byte: 25.0,
                bytes_per_member: 0.0,
                base_seconds: 0.032,
                seconds_per_mib: 1.8,
                seconds_per_member: 0.0,
            },
            geometry_check: StageCost {
                base_bytes: 25_000_000,
                work_bytes_limit: Some(2_055_000_000),
                bytes_per_byte: 230.0,
                bytes_per_member: 0.0,
                base_seconds: 0.03,
                seconds_per_mib: 0.69,
                seconds_per_member: 0.0,
            },
        }),
        band: 0.30,
        memory_band: 0.30,
        time_band: None,
    },
    Coefficients {
        format: Format::InventorPart,
        model: CostModel::Stages(Stages {
            read: StageCost {
                base_bytes: 15_000_000,
                work_bytes_limit: None,
                bytes_per_byte: 20.0,
                bytes_per_member: 0.0,
                base_seconds: 0.003,
                seconds_per_mib: 1.0,
                seconds_per_member: 0.0,
            },
            convert: StageCost {
                base_bytes: 18_000_000,
                work_bytes_limit: None,
                bytes_per_byte: 5.0,
                bytes_per_member: 0.0,
                base_seconds: 0.014,
                seconds_per_mib: 1.0,
                seconds_per_member: 0.0,
            },
            history: StageCost {
                base_bytes: 17_000_000,
                work_bytes_limit: None,
                bytes_per_byte: 30.0,
                bytes_per_member: 0.0,
                base_seconds: 0.042,
                seconds_per_mib: 3.0,
                seconds_per_member: 0.0,
            },
            check: StageCost {
                base_bytes: 20_000_000,
                work_bytes_limit: None,
                bytes_per_byte: 55.0,
                bytes_per_member: 0.0,
                base_seconds: 0.002,
                seconds_per_mib: 0.0,
                seconds_per_member: 0.0,
            },
            geometry_check: StageCost {
                base_bytes: 30_000_000,
                work_bytes_limit: None,
                bytes_per_byte: 27.5,
                bytes_per_member: 0.0,
                base_seconds: 0.046,
                seconds_per_mib: 0.0,
                seconds_per_member: 0.0,
            },
        }),
        band: 1.00,
        memory_band: 1.00,
        time_band: None,
    },
    Coefficients {
        format: Format::InventorAssembly,
        model: CostModel::Legacy(LegacyCost {
            base_bytes: 100_000_000,
            bytes_per_byte: 30.0,
            base_seconds: 1.0,
            seconds_per_mib: 2.0,
            history_factor: 3.0,
            history_seconds_factor: 3.0,
            startup: None,
            check_share: 1.1,
        }),
        band: 1.0,
        memory_band: 1.0,
        time_band: None,
    },
    Coefficients {
        format: Format::SolidWorksPart,
        model: CostModel::Stages(Stages {
            read: StageCost {
                base_bytes: 22_000_000,
                work_bytes_limit: None,
                bytes_per_byte: 20.0,
                bytes_per_member: 0.0,
                base_seconds: 0.006,
                seconds_per_mib: 2.5,
                seconds_per_member: 0.0,
            },
            convert: StageCost {
                base_bytes: 11_000_000,
                work_bytes_limit: None,
                bytes_per_byte: 5.0,
                bytes_per_member: 0.0,
                base_seconds: 0.016,
                seconds_per_mib: 2.5,
                seconds_per_member: 0.0,
            },
            history: StageCost {
                base_bytes: 20_000_000,
                work_bytes_limit: None,
                bytes_per_byte: 22.5,
                bytes_per_member: 0.0,
                base_seconds: 0.017,
                seconds_per_mib: 6.0,
                seconds_per_member: 0.0,
            },
            check: StageCost {
                base_bytes: 26_000_000,
                work_bytes_limit: None,
                bytes_per_byte: 46.75,
                bytes_per_member: 0.0,
                base_seconds: 0.023,
                seconds_per_mib: 0.0,
                seconds_per_member: 0.0,
            },
            geometry_check: StageCost {
                base_bytes: 20_000_000,
                work_bytes_limit: None,
                bytes_per_byte: 27.5,
                bytes_per_member: 0.0,
                base_seconds: 0.025,
                seconds_per_mib: 0.0,
                seconds_per_member: 0.0,
            },
        }),
        band: 1.00,
        memory_band: 1.00,
        time_band: None,
    },
    Coefficients {
        format: Format::SolidWorksAssembly,
        model: CostModel::Legacy(LegacyCost {
            base_bytes: 100_000_000,
            bytes_per_byte: 30.0,
            base_seconds: 1.0,
            seconds_per_mib: 5.0,
            history_factor: 3.0,
            history_seconds_factor: 3.0,
            startup: None,
            check_share: 1.1,
        }),
        band: 1.0,
        memory_band: 1.0,
        time_band: None,
    },
    Coefficients {
        format: Format::FreeCadDocument,
        model: CostModel::Legacy(LegacyCost {
            base_bytes: 29_000_000,
            bytes_per_byte: 150.0,
            base_seconds: 1.0,
            seconds_per_mib: 2.0,
            history_factor: 1.04,
            history_seconds_factor: 3.0,
            startup: Some(StartupTime {
                windows_shape: 0.183,
                windows_history: 1.600,
                mac_shape: 0.025,
                mac_history: 0.050,
                windows_verify_shape: 0.350,
                windows_verify_history: 1.900,
                content_rates: Some((0.2, 0.01)),
            }),
            check_share: 1.1,
        }),
        band: 1.0,
        memory_band: 0.5,
        time_band: None,
    },
    Coefficients {
        format: Format::CatiaPart,
        model: CostModel::Legacy(LegacyCost {
            base_bytes: 100_000_000,
            bytes_per_byte: 30.0,
            base_seconds: 1.0,
            seconds_per_mib: 2.0,
            history_factor: 3.0,
            history_seconds_factor: 3.0,
            startup: None,
            check_share: 1.1,
        }),
        band: 1.0,
        memory_band: 1.0,
        time_band: None,
    },
    Coefficients {
        format: Format::CreoPart,
        model: CostModel::Legacy(LegacyCost {
            base_bytes: 100_000_000,
            bytes_per_byte: 30.0,
            base_seconds: 1.0,
            seconds_per_mib: 2.0,
            history_factor: 3.0,
            history_seconds_factor: 3.0,
            startup: None,
            check_share: 1.1,
        }),
        band: 1.0,
        memory_band: 1.0,
        time_band: None,
    },
    Coefficients {
        format: Format::NxPart,
        model: CostModel::Legacy(LegacyCost {
            base_bytes: 100_000_000,
            bytes_per_byte: 30.0,
            base_seconds: 1.0,
            seconds_per_mib: 2.0,
            history_factor: 3.0,
            history_seconds_factor: 3.0,
            startup: None,
            check_share: 1.1,
        }),
        band: 1.0,
        memory_band: 1.0,
        time_band: None,
    },
    Coefficients {
        format: Format::CreoView,
        model: CostModel::Legacy(LegacyCost {
            base_bytes: 100_000_000,
            bytes_per_byte: 30.0,
            base_seconds: 1.0,
            seconds_per_mib: 2.0,
            history_factor: 3.0,
            history_seconds_factor: 3.0,
            startup: None,
            check_share: 1.1,
        }),
        band: 1.0,
        memory_band: 1.0,
        time_band: None,
    },
    Coefficients {
        format: Format::Rhino,
        model: CostModel::Legacy(LegacyCost {
            base_bytes: 100_000_000,
            bytes_per_byte: 30.0,
            base_seconds: 1.0,
            seconds_per_mib: 2.0,
            history_factor: 3.0,
            history_seconds_factor: 3.0,
            startup: None,
            check_share: 1.1,
        }),
        band: 1.0,
        memory_band: 1.0,
        time_band: None,
    },
    Coefficients {
        format: Format::Acis,
        model: CostModel::Legacy(LegacyCost {
            base_bytes: 100_000_000,
            bytes_per_byte: 30.0,
            base_seconds: 1.0,
            seconds_per_mib: 2.0,
            history_factor: 3.0,
            history_seconds_factor: 3.0,
            startup: None,
            check_share: 1.1,
        }),
        band: 1.0,
        memory_band: 1.0,
        time_band: None,
    },
    Coefficients {
        format: Format::Step,
        model: CostModel::Legacy(LegacyCost {
            base_bytes: 100_000_000,
            bytes_per_byte: 20.0,
            base_seconds: 1.0,
            seconds_per_mib: 1.0,
            history_factor: 2.0,
            history_seconds_factor: 2.0,
            startup: None,
            check_share: 1.1,
        }),
        band: 1.0,
        memory_band: 1.0,
        time_band: None,
    },
    Coefficients {
        format: Format::Iges,
        model: CostModel::Legacy(LegacyCost {
            base_bytes: 100_000_000,
            bytes_per_byte: 20.0,
            base_seconds: 1.0,
            seconds_per_mib: 1.0,
            history_factor: 2.0,
            history_seconds_factor: 2.0,
            startup: None,
            check_share: 1.1,
        }),
        band: 1.0,
        memory_band: 1.0,
        time_band: None,
    },
    Coefficients {
        format: Format::UnclassifiedPart,
        model: CostModel::Legacy(LegacyCost {
            base_bytes: 100_000_000,
            bytes_per_byte: 30.0,
            base_seconds: 1.0,
            seconds_per_mib: 2.0,
            history_factor: 3.0,
            history_seconds_factor: 3.0,
            startup: None,
            check_share: 1.1,
        }),
        band: 1.0,
        memory_band: 1.0,
        time_band: None,
    },
];

/// Estimate without native decoding, using automatic part workers. Nested Fusion ZIP
/// containers are inflated only to read their bounded directories.
pub fn estimate(input: &Path, choices: &Choices) -> Option<Estimate> {
    read_sizes(input)?.calculate(choices)
}

/// Estimate with the existing worker-selection API. Phase timings are calibrated at
/// four workers; other counts have no measured speedup model and remain rough.
pub fn estimate_with_threads(input: &Path, choices: &Choices, _threads: usize) -> Option<Estimate> {
    estimate(input, choices)
}

/// Estimate with verification and preflight context, without running a CLI probe.
/// Replay adds the separately calibrated checking cost.
pub fn estimate_with_options(
    input: &Path,
    choices: &Choices,
    options: &crate::ConvertOptions,
) -> Option<Estimate> {
    estimate_from_sizes(read_sizes(input)?, choices, options)
}

/// Pure estimate from sizes already read, including the requested history check.
pub fn estimate_from_sizes(
    sizes: Sizes,
    choices: &Choices,
    options: &crate::ConvertOptions,
) -> Option<Estimate> {
    let mut result = sizes.calculate(choices)?;
    if !options.no_verify && options.to == crate::OutputFormat::Solidcraft {
        result = combine_held_stages(result, check_cost(sizes, choices, result));
    }
    // Explicit target versions bypass history's version discovery. We cannot subtract
    // its cost from a combined startup measurement, so retain a rough estimate.
    if options.solidcraft_version.is_some() || options.no_memory_check {
        result.time_band = None;
    }
    Some(result)
}

// Replay runs in another process while conversion still holds its copies. Keep
// the overlap policy here so every total, including the desktop row, agrees.
fn combine_held_stages(conversion: Estimate, check: Estimate) -> Estimate {
    let seconds_exact = conversion.seconds_exact + check.seconds_exact;
    Estimate {
        peak_bytes: conversion.peak_bytes.saturating_add(check.peak_bytes),
        seconds: seconds_exact.ceil() as u64,
        seconds_exact,
        time_band: None,
        ..conversion
    }
}

fn check_cost(sizes: Sizes, choices: &Choices, conversion: Estimate) -> Estimate {
    let Some(coefficients) = COEFFICIENTS.iter().find(|c| c.format == sizes.format) else {
        return Estimate {
            peak_bytes: 0,
            seconds: 0,
            seconds_exact: 0.0,
            ..conversion
        };
    };
    match &coefficients.model {
        CostModel::Stages(stages) => {
            let geometry = stages
                .geometry_check
                .estimate(sizes, coefficients.template());
            if choices.history {
                stages
                    .check
                    .estimate(sizes, coefficients.template())
                    .at_least(geometry)
            } else {
                geometry
            }
        }
        CostModel::Legacy(legacy) => {
            let seconds_exact = if cfg!(target_os = "windows") {
                legacy.startup.as_ref().map_or(0.0, |startup| {
                    let shape = startup.windows_verify_shape - startup.windows_shape;
                    if choices.history {
                        (startup.windows_verify_history - startup.windows_history).max(shape)
                    } else {
                        shape
                    }
                })
            } else {
                0.0
            };
            Estimate {
                peak_bytes: (conversion.peak_bytes as f64 * legacy.check_share).ceil() as u64,
                seconds: seconds_exact.ceil() as u64,
                seconds_exact,
                time_band: None,
                ..conversion
            }
        }
    }
}

/// The same extra checking cost used by the desktop, CLI estimates and runtime guard.
pub(crate) fn history_check_estimate(
    input: &Path,
    options: &crate::ConvertOptions,
) -> Option<Estimate> {
    let sizes = read_sizes(input)?;
    let choices = options.choices();
    let conversion = estimate_from_sizes(
        sizes,
        &choices,
        &crate::ConvertOptions {
            no_verify: true,
            ..options.clone()
        },
    )?;
    Some(check_cost(sizes, &choices, conversion))
}

/// File metadata and bounded archive sizes, read once before interactive estimates.
#[derive(Clone, Copy, Debug)]
pub struct Sizes {
    format: Format,
    bytes: u64,
    members: u64,
}

/// Reads sizes without decoding a design. Call off the UI thread.
pub fn read_sizes(input: &Path) -> Option<Sizes> {
    let format = Format::from_path(input)?;
    if !format.can_convert() {
        return None;
    }
    let metadata = std::fs::metadata(input).ok()?;
    if !metadata.is_file() {
        return None;
    }
    let (bytes, members, _designs) = if matches!(
        format,
        Format::FusionArchive | Format::FusionMultiDocument | Format::FreeCadDocument
    ) {
        archive_size(
            input,
            matches!(format, Format::FusionArchive | Format::FusionMultiDocument),
        )?
    } else {
        (metadata.len(), 0, 0)
    };
    Some(Sizes {
        format,
        bytes,
        members,
    })
}

impl Sizes {
    fn calculate(self, choices: &Choices) -> Option<Estimate> {
        let coefficients = COEFFICIENTS.iter().find(|c| c.format == self.format)?;
        Some(match &coefficients.model {
            CostModel::Stages(stages) => {
                let split = stages.split(self, choices, coefficients.template());
                combine_held_stages(split.read, split.convert)
            }
            CostModel::Legacy(legacy) => legacy.estimate(self, choices, coefficients.template()),
        })
    }
}

impl LegacyCost {
    fn estimate(&self, sizes: Sizes, choices: &Choices, template: Estimate) -> Estimate {
        let factor = if choices.history {
            self.history_factor
        } else {
            1.0
        };
        let seconds_factor = if choices.history {
            self.history_seconds_factor
        } else {
            1.0
        };
        let time_rate = self
            .startup
            .as_ref()
            .filter(|_| cfg!(target_os = "windows"))
            .and_then(|s| s.content_rates)
            .map_or(self.seconds_per_mib * seconds_factor, |(shape, history)| {
                if choices.history {
                    history
                } else {
                    shape
                }
            });
        let fixed = self
            .startup
            .as_ref()
            .map_or(self.base_seconds * seconds_factor, |startup| {
                if cfg!(target_os = "windows") {
                    if choices.history {
                        startup.windows_history
                    } else {
                        startup.windows_shape
                    }
                } else if cfg!(target_os = "macos") {
                    if choices.history {
                        startup.mac_history
                    } else {
                        startup.mac_shape
                    }
                } else {
                    self.base_seconds * seconds_factor
                }
            });
        let seconds_exact = fixed + time_rate * sizes.bytes as f64 / (1 << 20) as f64;
        let estimate = Estimate {
            peak_bytes: (self.base_bytes as f64 + self.bytes_per_byte * sizes.bytes as f64 * factor)
                .ceil() as u64,
            seconds: seconds_exact.ceil() as u64,
            seconds_exact,
            ..template
        };
        if choices.history {
            // Also cover combined models, including platform-specific startup/content
            // rates whose history and geometry fits may cross at larger sizes.
            estimate.at_least(self.estimate(
                sizes,
                &Choices {
                    history: false,
                    ..*choices
                },
                template,
            ))
        } else {
            estimate
        }
    }
}

/// Open a zip only after bounding its directory and entry count.
pub(crate) fn bounded_archive<R: std::io::Read + std::io::Seek>(
    file: R,
) -> Option<zip::ZipArchive<R>> {
    dutchman_serve::upload::bounded_archive(file, 10_000)
        .ok()
        .map(|(archive, _)| archive)
}

fn archive_size(input: &Path, fusion: bool) -> Option<(u64, u64, usize)> {
    let mut archive = bounded_archive(std::fs::File::open(input).ok()?)?;
    let bytes = match archive.decompressed_size() {
        Some(bytes) => u64::try_from(bytes).unwrap_or(u64::MAX),
        None => {
            let mut total = 0_u64;
            for index in 0..archive.len() {
                total = total.saturating_add(archive.by_index_raw(index).ok()?.size());
            }
            total
        }
    };
    let members = archive.len() as u64;
    let (nested_bytes, designs) = if fusion {
        nested_size(&mut archive)
    } else {
        (0, 0)
    };
    Some((bytes.max(nested_bytes), members, designs))
}

/// The uncompressed size of the designs a Fusion archive holds as zips of their own (an `.f3z`
/// holds each design as an `.f3d`), read from each inner directory: one member at a time is
/// read, never the designs inside it; 0 when none can be read.
fn nested_size<R: std::io::Read + std::io::Seek>(archive: &mut zip::ZipArchive<R>) -> (u64, usize) {
    /// The most bytes of inner archives read in all.
    const LIMIT: u64 = 2 << 30;
    let mut total = 0_u64;
    let mut read = 0_u64;
    let mut designs = 0_usize;
    for index in 0..archive.len() {
        let Ok(mut member) = archive.by_index(index) else {
            continue;
        };
        if !member
            .name()
            .is_ok_and(|name| name.to_ascii_lowercase().ends_with(".f3d"))
        {
            continue;
        }
        read = read.saturating_add(member.size());
        if read > LIMIT {
            break;
        }
        if let Some(size) = nested_content_size(&mut member, LIMIT) {
            total = total.saturating_add(size);
            designs += 1;
        }
    }
    (total, designs)
}

/// Read only a bounded nested archive's directory, never its design contents.
pub(crate) fn nested_content_size(reader: &mut impl std::io::Read, limit: u64) -> Option<u64> {
    use std::io::Read;
    let mut bytes = Vec::new();
    reader
        .take(limit.saturating_add(1))
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() as u64 > limit {
        return None;
    }
    let archive = bounded_archive(std::io::Cursor::new(bytes))?;
    archive
        .decompressed_size()
        .and_then(|n| u64::try_from(n).ok())
}

/// Available memory, including process limits where available.
pub use dutchman_core::available_memory;

/// Conversion and checking costs for the desktop, sharing the file row's total.
#[derive(Clone, Copy, Debug)]
pub struct DesktopEstimate {
    /// Reading and converting together, before checking.
    pub conversion: Estimate,
    /// Separate measured costs, when calibrated for this format.
    pub split: Option<ReadConvertSplit>,
    /// Extra memory and time caused by checking, when enabled and available.
    pub check: Option<Estimate>,
    pub total: Estimate,
}

/// Desktop totals add the durations as displayed for each stage. A sub-minute
/// stage contributes its displayed upper bound and keeps the total an upper bound.
/// Raw estimates and JSON retain fractional seconds and their usual rounding.
pub fn desktop_estimate_text(parts: DesktopEstimate) -> String {
    let stages = match parts.split {
        Some(split) => [Some(split.read), Some(split.convert), parts.check],
        None => [Some(parts.conversion), None, parts.check],
    };
    let mut minutes = 0_u64;
    let mut under = false;
    for stage in stages.into_iter().flatten() {
        minutes = minutes.saturating_add(stage.seconds.div_ceil(60).max(1));
        under |= stage.seconds < 60;
    }
    let time = if under {
        if minutes == 1 {
            "under a minute".to_owned()
        } else {
            format!("under {minutes} min")
        }
    } else {
        time_text(
            Estimate {
                seconds: minutes.saturating_mul(60),
                ..parts.total
            },
            false,
        )
    };
    format!("{}, {time}", memory_text(parts.total, false))
}

/// Read peak and additional conversion cost beyond that peak.
#[derive(Clone, Copy, Debug)]
pub struct ReadConvertSplit {
    pub read: Estimate,
    pub convert: Estimate,
}

fn read_convert_split(sizes: Sizes, choices: &Choices) -> Option<ReadConvertSplit> {
    let coefficients = COEFFICIENTS.iter().find(|c| c.format == sizes.format)?;
    match &coefficients.model {
        CostModel::Stages(stages) => Some(stages.split(sizes, choices, coefficients.template())),
        CostModel::Legacy(_) => None,
    }
}

/// Desktop estimate, including checking when a CLI was found.
pub fn desktop_estimate(
    sizes: Sizes,
    options: &crate::ConvertOptions,
    checker_found: bool,
) -> Option<Estimate> {
    desktop_estimate_parts(sizes, options, checker_found).map(|parts| parts.total)
}

/// Split the same total into conversion and extra checking costs. Checking is
/// counted only with a CLI; its budget is checked after reading. Geometry-only
/// conversion still checks the written shape unless checking is turned off.
pub fn desktop_estimate_parts(
    sizes: Sizes,
    options: &crate::ConvertOptions,
    checker_found: bool,
) -> Option<DesktopEstimate> {
    let mut options = options.clone();
    options.no_verify |= !checker_found;
    let choices = options.choices();
    let mut conversion_options = options.clone();
    conversion_options.no_verify = true;
    let conversion = estimate_from_sizes(sizes, &choices, &conversion_options);
    let conversion = conversion?;
    let split = read_convert_split(sizes, &choices);
    let check = (!options.no_verify && options.to == crate::OutputFormat::Solidcraft)
        .then(|| check_cost(sizes, &choices, conversion));
    let total = check.map_or(conversion, |check| combine_held_stages(conversion, check));
    Some(DesktopEstimate {
        split,
        conversion,
        check,
        total,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ConvertOptions;
    use std::io::Write;

    #[test]
    fn history_never_reduces_estimated_memory_or_time_for_any_format() {
        fn assert_at_least(history: Estimate, geometry: Estimate, context: &str) {
            assert!(
                history.peak_bytes >= geometry.peak_bytes,
                "{context}: memory"
            );
            assert!(
                history.seconds >= geometry.seconds,
                "{context}: rounded time"
            );
            assert!(
                history.seconds_exact >= geometry.seconds_exact,
                "{context}: time"
            );
        }

        for format in Format::ALL {
            for bytes in [
                0,
                1,
                5120,
                39_999,
                40_000,
                40_001,
                1 << 16,
                1 << 20,
                10 << 20,
                100 << 20,
                1 << 30,
                u64::MAX,
            ] {
                for members in [0, 1, 20, 100, 10_000, u64::MAX] {
                    let sizes = Sizes {
                        format,
                        bytes,
                        members,
                    };
                    let context = format!("{format:?}, {bytes} bytes, {members} members");
                    let history = Choices::of(Preset::Full);
                    let geometry = Choices {
                        history: false,
                        ..history
                    };
                    assert_at_least(
                        sizes.calculate(&history).unwrap(),
                        sizes.calculate(&geometry).unwrap(),
                        &context,
                    );
                    if let (Some(history), Some(geometry)) = (
                        read_convert_split(sizes, &history),
                        read_convert_split(sizes, &geometry),
                    ) {
                        assert_eq!(history.read, geometry.read, "{context}");
                        assert_at_least(history.convert, geometry.convert, &context);
                    }
                    for no_verify in [false, true] {
                        let options = ConvertOptions {
                            preset: Preset::Full,
                            no_verify,
                            ..Default::default()
                        };
                        let history_parts = desktop_estimate_parts(sizes, &options, true).unwrap();
                        let geometry_options = ConvertOptions {
                            import_only: true,
                            ..options.clone()
                        };
                        let geometry_parts =
                            desktop_estimate_parts(sizes, &geometry_options, true).unwrap();
                        assert_at_least(history_parts.total, geometry_parts.total, &context);
                        assert_eq!(
                            Some(history_parts.total),
                            estimate_from_sizes(sizes, &history, &options)
                        );
                        assert_eq!(
                            Some(geometry_parts.total),
                            estimate_from_sizes(sizes, &geometry, &geometry_options)
                        );
                        if let (Some(history), Some(geometry)) =
                            (history_parts.check, geometry_parts.check)
                        {
                            assert_at_least(history, geometry, &context);
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn fitted_fusion_stages_pin_content_member_and_retention_costs() {
        for (format, read_bytes, read_seconds, geometry, history, geometry_check, history_check) in [
            (
                Format::FusionArchive,
                782_688_320,
                16.1,
                (1_584_864_000, 17.2),
                (2_735_297_600, 35.0),
                (765_000_000, 46.03),
                // The independent history-time fit is lower (38.032 s), so use
                // the geometry check as the minimum when retaining history.
                (1_214_000_000, 46.03),
            ),
            (
                Format::FusionMultiDocument,
                2_126_152_000,
                120.011,
                (1_701_721_600, 32.02),
                (2_004_294_400, 38.019),
                (2_080_000_000, 69.03),
                (2_644_440_000, 180.032),
            ),
        ] {
            let sizes = Sizes {
                format,
                bytes: 100 << 20,
                members: 100,
            };
            for (import_only, converting, checking) in [
                (false, history, history_check),
                (true, geometry, geometry_check),
            ] {
                let options = ConvertOptions {
                    preset: Preset::Full,
                    import_only,
                    threads: 4,
                    ..Default::default()
                };
                let parts = desktop_estimate_parts(sizes, &options, true).unwrap();
                let split = parts.split.unwrap();
                let check = parts.check.unwrap();
                assert_eq!(split.read.peak_bytes, read_bytes);
                assert!((split.read.seconds_exact - read_seconds).abs() < 1e-8);
                assert_eq!(split.convert.peak_bytes, converting.0);
                assert!((split.convert.seconds_exact - converting.1).abs() < 1e-8);
                assert_eq!(check.peak_bytes, checking.0);
                assert!((check.seconds_exact - checking.1).abs() < 1e-8);
                assert_eq!(
                    parts.total.peak_bytes,
                    read_bytes + converting.0 + checking.0
                );
                assert!(
                    (parts.total.seconds_exact - read_seconds - converting.1 - checking.1).abs()
                        < 1e-8
                );
                assert_eq!(parts.total.seconds, parts.total.seconds_exact.ceil() as u64);
            }
        }
    }

    #[test]
    fn checking_ramps_from_the_small_floor_and_then_plateaus() {
        let options = ConvertOptions {
            preset: Preset::Full,
            threads: 4,
            ..Default::default()
        };
        for (import_only, small, plateau) in [
            (false, 295_629_760, 1_214_000_000),
            (true, 266_172_480, 765_000_000),
        ] {
            let options = ConvertOptions {
                import_only,
                ..options.clone()
            };
            for (bytes, expected) in [(1 << 20, small), (10 << 20, plateau), (100 << 20, plateau)] {
                let sizes = Sizes {
                    format: Format::FusionArchive,
                    bytes,
                    members: 20,
                };
                let parts = desktop_estimate_parts(sizes, &options, true).unwrap();
                assert_eq!(parts.check.unwrap().peak_bytes, expected);
            }
        }
        // A hostile size saturates the total, never wrapping after adding stages.
        let sizes = Sizes {
            format: Format::FusionArchive,
            bytes: u64::MAX,
            members: u64::MAX,
        };
        assert_eq!(
            desktop_estimate_parts(sizes, &options, true)
                .unwrap()
                .total
                .peak_bytes,
            u64::MAX
        );
    }

    #[test]
    fn the_desktop_total_includes_geometry_checking_without_history() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("synthetic.ipt");
        std::fs::write(&input, [0u8; 1024]).unwrap();
        let sizes = read_sizes(&input).unwrap();
        let options = ConvertOptions {
            import_only: true,
            ..Default::default()
        };
        let unchecked = ConvertOptions {
            no_verify: true,
            ..options.clone()
        };
        let checked = desktop_estimate_parts(sizes, &options, true).unwrap();
        let unchecked = desktop_estimate_parts(sizes, &unchecked, true).unwrap();
        assert!(checked.check.is_some());
        assert!(unchecked.check.is_none());
        assert!(checked.total.peak_bytes > unchecked.total.peak_bytes);
    }

    #[test]
    fn presets_only_narrow_and_have_a_lighter_choice() {
        for preset in [Preset::Fast, Preset::Standard, Preset::Full] {
            let options = ConvertOptions {
                preset,
                import_only: true,
                no_sketches: true,
                no_joints: true,
                ..Default::default()
            };
            let narrowed = options.choices();
            assert!(!narrowed.history && !narrowed.sketches && !narrowed.joints);
            assert_eq!(narrowed.history_within_budget, preset == Preset::Standard);
        }
        assert_eq!(Preset::Full.lighter(), Some(Preset::Standard));
        assert_eq!(Preset::Standard.lighter(), Some(Preset::Fast));
        assert_eq!(Preset::Fast.lighter(), None);
        let fast = Choices::of(Preset::Fast);
        assert!(!fast.history && !fast.sketches && !fast.joints);
        assert!(Choices::of(Preset::Full).history);
    }

    #[test]
    fn coefficient_table_covers_each_format_once() {
        assert_eq!(COEFFICIENTS.len(), Format::ALL.len());
        for format in Format::ALL {
            let entries: Vec<_> = COEFFICIENTS.iter().filter(|c| c.format == format).collect();
            assert_eq!(entries.len(), 1, "{format:?}");
            let c = entries[0];
            match &c.model {
                CostModel::Stages(stages) => {
                    for cost in [
                        &stages.read,
                        &stages.convert,
                        &stages.history,
                        &stages.check,
                        &stages.geometry_check,
                    ] {
                        assert!(cost.bytes_per_byte.is_finite() && cost.bytes_per_byte >= 0.0);
                        assert!(cost.bytes_per_member.is_finite() && cost.bytes_per_member >= 0.0);
                        assert!(cost.base_seconds.is_finite() && cost.base_seconds >= 0.0);
                        assert!(cost.seconds_per_mib.is_finite() && cost.seconds_per_mib >= 0.0);
                        assert!(
                            cost.seconds_per_member.is_finite() && cost.seconds_per_member >= 0.0
                        );
                    }
                }
                CostModel::Legacy(legacy) => {
                    assert!(legacy.base_bytes > 0);
                    assert!(legacy.bytes_per_byte.is_finite() && legacy.bytes_per_byte > 0.0);
                    assert!(legacy.base_seconds.is_finite() && legacy.base_seconds > 0.0);
                    assert!(legacy.seconds_per_mib.is_finite() && legacy.seconds_per_mib > 0.0);
                    assert!(legacy.check_share.is_finite() && legacy.check_share >= 0.0);
                    assert!(legacy.history_factor.is_finite() && legacy.history_factor > 0.0);
                    assert!(
                        legacy.history_seconds_factor.is_finite()
                            && legacy.history_seconds_factor > 0.0
                    );
                }
            }
            assert!(c.band.is_finite() && c.band > 0.0 && c.band <= 1.0);
            assert!(c.memory_band.is_finite() && c.memory_band > 0.0 && c.memory_band <= c.band);
            if let Some(time_band) = c.time_band {
                assert!(time_band.is_finite() && time_band > 0.0 && time_band <= c.band);
            }
        }
    }

    #[test]
    fn fusion_archives_count_the_designs_they_store_as_zips() {
        // An .f3z stores each design as an .f3d zip of its own: what counts is the inner
        // content, read from the inner directory, not the stored bytes.
        let mut inner = Vec::new();
        {
            let mut zip = zip::ZipWriter::new(std::io::Cursor::new(&mut inner));
            let options = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated);
            zip.start_file("Design1/BulkStream.dat", options).unwrap();
            zip.write_all(&vec![0; 4 << 20]).unwrap();
            zip.finish().unwrap();
        }
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("synthetic.f3z");
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
        let stored = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        zip.start_file("Part.f3d", stored).unwrap();
        zip.write_all(&inner).unwrap();
        zip.finish().unwrap();
        assert_eq!(archive_size(&path, true), Some((4 << 20, 1, 1)));
        assert_eq!(archive_size(&path, false), Some((inner.len() as u64, 1, 0)));
    }

    #[test]
    fn four_worker_fit_does_not_invent_a_speedup_for_other_worker_counts() {
        let mut inner = Vec::new();
        {
            let mut zip = zip::ZipWriter::new(std::io::Cursor::new(&mut inner));
            let options = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated);
            zip.start_file("Design/BulkStream.dat", options).unwrap();
            zip.write_all(&vec![0; 4 << 20]).unwrap();
            zip.finish().unwrap();
        }
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("synthetic.f3z");
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
        for name in ["a.f3d", "b.f3d", "c.f3d", "d.f3d"] {
            zip.start_file(name, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(&inner).unwrap();
        }
        zip.finish().unwrap();
        let choices = Choices::of(Preset::Fast);
        // Only four-worker runs calibrate the new phase model. The old 13/9/10
        // second expectations encoded an unmeasured serial-fraction assumption.
        let calibrated = estimate_with_threads(&path, &choices, 4).unwrap();
        assert!(calibrated.time_band.is_none());
        for threads in [0, 1, 2, 16] {
            assert_eq!(
                estimate_with_threads(&path, &choices, threads),
                Some(calibrated)
            );
        }
    }

    #[test]
    fn zip_members_are_retained_as_estimator_inputs() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("synthetic.f3d");
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
        for name in ["Design/a", "Design/b"] {
            zip.start_file(name, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(&vec![0; 1 << 20]).unwrap();
        }
        zip.finish().unwrap();
        let sizes = read_sizes(&path).unwrap();
        assert_eq!(sizes.bytes, 2 << 20);
        assert_eq!(sizes.members, 2);
        assert!(estimate_with_threads(&path, &Choices::of(Preset::Fast), 4).is_some());
    }

    #[test]
    fn fractional_time_and_unmeasured_verification_do_not_claim_coverage() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tiny.ipt");
        std::fs::write(&path, [0; 16]).unwrap();
        let choices = Choices::of(Preset::Fast);
        let options = crate::ConvertOptions {
            preset: Preset::Fast,
            no_verify: true,
            ..crate::ConvertOptions::default()
        };
        let fast = estimate_with_options(&path, &choices, &options).unwrap();
        assert!(fast.seconds_exact > 0.0 && fast.seconds_exact < 1.0);
        assert_eq!(fast.seconds, 1);
        assert!(fast.time_band.is_none());
        let verified = estimate_with_options(
            &path,
            &choices,
            &crate::ConvertOptions {
                no_verify: false,
                ..options
            },
        )
        .unwrap();
        // Geometry-only conversion still checks the written shape by default.
        assert!(verified.peak_bytes > fast.peak_bytes);
        assert!(verified.seconds_exact >= fast.seconds_exact);
        assert!(verified.time_band.is_none());
    }

    #[test]
    fn replay_memory_is_printed_but_does_not_refuse_a_conversion_that_fits() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("synthetic.ipt");
        std::fs::write(&input, [0u8; 1024]).unwrap();
        let options = crate::ConvertOptions::default();
        let choices = options.choices();
        let conversion = conversion_estimate(&input, &choices, &options).unwrap();
        let checked = estimate_with_options(&input, &choices, &options).unwrap();
        assert!(checked.peak_bytes > conversion.peak_bytes);
        assert_eq!(
            memory_check_with(&input, &options, Some(conversion.peak_bytes)),
            MemoryCheck::Ok(Some(checked))
        );
        let prepared = options.preflight(&input, Some(conversion.peak_bytes));
        assert!(prepared.memory_note.is_none());
        assert!(prepared.check_history_memory);
    }

    #[test]
    fn a_memory_warning_names_the_figure_it_compared_and_the_row_shows() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("synthetic.ipt");
        std::fs::write(&input, [0u8; 1024]).unwrap();
        let options = crate::ConvertOptions::default();
        let choices = options.choices();
        let row = estimate(&input, &choices).unwrap();
        let conversion = conversion_estimate(&input, &choices, &options).unwrap();
        assert_eq!(row.peak_bytes, conversion.peak_bytes);
        let MemoryCheck::TooBig {
            conversion: warned, ..
        } = memory_check_with(&input, &options, Some(0))
        else {
            panic!("the conversion does not fit in no memory");
        };
        assert_eq!(warned.peak_bytes, row.peak_bytes);
    }

    #[test]
    fn measured_native_floors_are_separate_from_legacy_content_growth() {
        let dir = tempfile::tempdir().unwrap();
        // Fresh phase measurements have separate preparation floors for the two
        // modes, but history cannot fall below the geometry floor (formerly 32 MB
        // versus 33 MB for Inventor). Tiny Inventor parts therefore cost the same;
        // the steeper history slope takes over above 40,000 content bytes.
        for (extension, shape_floor, history_floor, history_growth) in [
            ("ipt", 33_000_000, 33_000_000, 128_000),
            ("sldprt", 33_000_000, 42_000_000, 217_600),
        ] {
            let path = dir.path().join(format!("synthetic.{extension}"));
            std::fs::write(&path, []).unwrap();
            let fast = estimate(&path, &Choices::of(Preset::Fast)).unwrap();
            let full = estimate(&path, &Choices::of(Preset::Full)).unwrap();
            assert_eq!(fast.peak_bytes, shape_floor);
            assert_eq!(full.peak_bytes, history_floor);
            // Content growth follows the larger of the two fitted costs.
            std::fs::write(&path, [0; 5120]).unwrap();
            let fast_content = estimate(&path, &Choices::of(Preset::Fast)).unwrap();
            let full_content = estimate(&path, &Choices::of(Preset::Full)).unwrap();
            assert!(fast_content.peak_bytes > fast.peak_bytes);
            assert_eq!(fast_content.peak_bytes - fast.peak_bytes, 128_000);
            assert_eq!(full_content.peak_bytes - full.peak_bytes, history_growth);
        }
    }

    #[cfg(feature = "freecad")]
    #[test]
    fn freecad_time_separates_startup_from_the_recipe_content_term() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("synthetic.FCStd");
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
        zip.start_file("synthetic", zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(&vec![0; 1 << 20]).unwrap();
        zip.finish().unwrap();
        let fast = estimate(&path, &Choices::of(Preset::Fast)).unwrap();
        let full = estimate(&path, &Choices::of(Preset::Full)).unwrap();
        // RAM's 1.04 multiplier must not replace the existing 3x time multiplier.
        assert!(fast.seconds_exact > 0.2);
        assert!(full.seconds_exact > fast.seconds_exact);
        assert!(full.time_band.is_none());
    }

    #[test]
    fn archives_use_uncompressed_sizes_without_decoding_members() {
        let dir = tempfile::tempdir().unwrap();
        for extension in ["f3d", "f3z", "FCStd"] {
            let path = dir.path().join(format!("synthetic.{extension}"));
            let mut zip = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
            let options = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated);
            zip.start_file("not-a-design", options).unwrap();
            zip.write_all(&vec![0; 1 << 20]).unwrap();
            zip.start_file("second-member", options).unwrap();
            zip.write_all(b"synthetic").unwrap();
            zip.finish().unwrap();
            assert_eq!(archive_size(&path, false), Some(((1 << 20) + 9, 2, 0)));
            assert!(std::fs::metadata(&path).unwrap().len() < 1 << 20);
            if !Format::from_path(&path).unwrap().can_convert() {
                assert!(estimate(&path, &Choices::of(Preset::Fast)).is_none());
                continue;
            }
            let sizes = read_sizes(&path).unwrap();
            assert_eq!(sizes.bytes, (1 << 20) + 9);
            assert_eq!(sizes.members, 2);
            assert!(estimate(&path, &Choices::of(Preset::Fast)).is_some());
        }
    }

    #[test]
    fn zip64_and_descriptor_archives_are_estimated_without_inflation() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("synthetic.f3z");
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        zip.start_file("synthetic", zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(b"synthetic").unwrap();
        let mut bytes = zip.finish().unwrap().into_inner();
        let mut end = bytes.split_off(bytes.len() - 22);
        let members = u64::from(u16::from_le_bytes(end[10..12].try_into().unwrap()));
        let directory_bytes = u64::from(u32::from_le_bytes(end[12..16].try_into().unwrap()));
        let directory_offset = u64::from(u32::from_le_bytes(end[16..20].try_into().unwrap()));
        let zip64_offset = bytes.len() as u64;
        bytes.extend_from_slice(b"PK\x06\x06");
        bytes.extend_from_slice(&44_u64.to_le_bytes());
        bytes.extend_from_slice(&[45, 0, 45, 0]);
        bytes.extend_from_slice(&[0; 8]);
        for value in [members, members, directory_bytes, directory_offset] {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        bytes.extend_from_slice(b"PK\x06\x07");
        bytes.extend_from_slice(&[0; 4]);
        bytes.extend_from_slice(&zip64_offset.to_le_bytes());
        bytes.extend_from_slice(&1_u32.to_le_bytes());
        end[8..12].fill(255);
        end[12..20].fill(255);
        bytes.extend_from_slice(&end);
        std::fs::write(&path, &bytes).unwrap();
        assert_eq!(archive_size(&path, false), Some((9, 1, 0)));
        // A hostile ZIP64 count must be refused before zip can allocate its directory.
        bytes[zip64_offset as usize + 24..zip64_offset as usize + 32]
            .copy_from_slice(&u64::MAX.to_le_bytes());
        bytes[zip64_offset as usize + 32..zip64_offset as usize + 40]
            .copy_from_slice(&u64::MAX.to_le_bytes());
        std::fs::write(&path, &bytes).unwrap();
        assert!(archive_size(&path, false).is_none());

        let mut zip = zip::ZipWriter::new_stream(std::fs::File::create(&path).unwrap());
        zip.start_file("synthetic", zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(b"synthetic").unwrap();
        zip.finish().unwrap();
        assert_eq!(archive_size(&path, false), Some((9, 1, 0)));
    }

    #[test]
    fn unreadable_and_overlarge_directories_have_unknown_estimates() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("synthetic.f3d");
        assert!(estimate(&path, &Choices::of(Preset::Full)).is_none());
        std::fs::write(&path, b"not a zip").unwrap();
        assert!(estimate(&path, &Choices::of(Preset::Full)).is_none());
        let mut end = vec![0; 22];
        end[..4].copy_from_slice(b"PK\x05\x06");
        end[8..10].copy_from_slice(&10_001_u16.to_le_bytes());
        end[10..12].copy_from_slice(&10_001_u16.to_le_bytes());
        std::fs::write(&path, end).unwrap();
        assert!(archive_size(&path, false).is_none());
    }

    #[test]
    fn an_estimate_above_free_memory_warns_except_where_memory_is_not_checked() {
        let dir = tempfile::tempdir().unwrap();
        let assembly = dir.path().join("synthetic.iam");
        std::fs::write(&assembly, b"synthetic").unwrap();
        let mut options = ConvertOptions {
            no_verify: true,
            ..Default::default()
        };
        let full = estimate(&assembly, &options.choices()).unwrap();
        assert_eq!(
            memory_check_with(&assembly, &options, Some(full.peak_bytes)),
            MemoryCheck::Ok(Some(full))
        );
        let over = memory_check_with(&assembly, &options, Some(full.peak_bytes - 1));
        assert!(matches!(over, MemoryCheck::TooBig { .. }));
        assert!(over.warning().unwrap().contains("the conversion ran"));
        assert!(options
            .preflight(&assembly, Some(full.peak_bytes - 1))
            .memory_note
            .is_some());
        options.no_memory_check = true;
        assert!(matches!(
            memory_check_with(&assembly, &options, Some(0)),
            MemoryCheck::Ok(_)
        ));
        options.no_memory_check = false;
        options.preset = Preset::Fast;
        assert!(matches!(
            memory_check_with(&assembly, &options, Some(0)),
            MemoryCheck::Ok(_)
        ));
    }

    #[test]
    fn the_warning_is_shared_and_uses_decimal_gigabytes() {
        let estimate = Estimate {
            peak_bytes: 2_500_000_000,
            seconds: 60,
            seconds_exact: 60.0,
            band: 0.5,
            memory_band: 0.5,
            time_band: Some(0.5),
        };
        let check = MemoryCheck::TooBig {
            estimate: Estimate {
                peak_bytes: 5_000_000_000,
                ..estimate
            },
            conversion: estimate,
            available: 1_000_000_000,
            lighter: Some(Preset::Fast),
        };
        assert_eq!(check.warning().unwrap(), "the estimate of about 2.5 GB is above the 1.0 GB free. Estimates from a file's size can be far off, so the conversion ran, watching memory");
    }

    #[test]
    fn estimates_show_lower_bounds_and_round_minutes_up() {
        for (peak_bytes, seconds, expected) in [
            (0, 0, "under 0.1 GB, under a minute"),
            (99_999_999, 59, "under 0.1 GB, under a minute"),
            (100_000_000, 60, "about 0.1 GB, about 1 min"),
            (2_500_000_000, 121, "about 2.5 GB, about 3 min"),
            (100_000_000, 59, "about 0.1 GB, under a minute"),
            (99_999_999, 60, "under 0.1 GB, about 1 min"),
        ] {
            assert_eq!(
                estimate_text(Estimate {
                    peak_bytes,
                    seconds,
                    seconds_exact: seconds as f64,
                    band: 0.3,
                    memory_band: 0.3,
                    time_band: Some(0.3),
                }),
                expected
            );
        }
        let measured = Estimate {
            peak_bytes: 2_500_000_000,
            seconds: 121,
            seconds_exact: 121.0,
            band: 0.25,
            memory_band: 0.25,
            time_band: Some(0.2),
        };
        assert_eq!(
            estimate_text_with_bands(measured),
            "about 2.5 GB (±25 %), about 3 min (±20 %)"
        );
        let rough = Estimate {
            time_band: None,
            ..measured
        };
        assert_eq!(estimate_text(rough), "about 2.5 GB, roughly 3 min");
        assert_eq!(
            estimate_text_with_bands(rough),
            "about 2.5 GB (±25 %), roughly 3 min"
        );
        let small = Estimate {
            peak_bytes: 1,
            seconds: 1,
            seconds_exact: 1.0,
            ..rough
        };
        assert_eq!(
            estimate_text_with_bands(small),
            "under 0.1 GB, under a minute"
        );
    }
}

#[cfg(test)]
mod startup_regression_tests {
    use super::*;

    #[test]
    fn tiny_mac_conversions_do_not_charge_a_probe_timeout() {
        use std::io::Write;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tiny.f3d");
        let mut archive = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
        archive
            .start_file(
                "Design/BulkStream.dat",
                zip::write::SimpleFileOptions::default(),
            )
            .unwrap();
        archive.write_all(&[0; 16]).unwrap();
        archive.finish().unwrap();
        for preset in [Preset::Fast, Preset::Full] {
            for no_verify in [false, true] {
                let options = crate::ConvertOptions {
                    preset,
                    no_verify,
                    ..Default::default()
                };
                let estimated = estimate_with_options(&path, &options.choices(), &options).unwrap();
                if cfg!(target_os = "macos") {
                    assert!(estimated.seconds_exact > 0.0 && estimated.seconds_exact < 1.0);
                }
                assert!(estimate_text(estimated).ends_with("under a minute"));
            }
        }
    }

    #[test]
    fn multipart_exact_time_retains_subsecond_content_work() {
        use std::io::Write;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tiny.f3z");
        let mut archive = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
        archive
            .start_file("synthetic", zip::write::SimpleFileOptions::default())
            .unwrap();
        archive.write_all(&[0; 16]).unwrap();
        archive.finish().unwrap();
        let estimated = estimate(&path, &Choices::of(Preset::Full)).unwrap();
        assert!(estimated.seconds_exact > 0.0 && estimated.seconds_exact < 1.0);
        assert_eq!(estimated.seconds, 1);
    }
}

#[cfg(test)]
mod cached_estimate_tests {
    use super::*;

    #[test]
    fn desktop_total_time_adds_displayed_stages_and_subminute_bounds() {
        let stage = |seconds: f64| Estimate {
            peak_bytes: 1_000_000_000,
            seconds: seconds.ceil() as u64,
            seconds_exact: seconds,
            band: 0.5,
            memory_band: 0.5,
            time_band: None,
        };
        for (read, convert, check, expected) in [
            (40.0, 61.0, Some(61.0), "about 3.0 GB, under 5 min"),
            (60.0, 61.0, Some(120.0), "about 3.0 GB, roughly 5 min"),
            (40.0, 61.0, None, "about 2.0 GB, under 3 min"),
            (60.0, 60.0, None, "about 2.0 GB, roughly 2 min"),
            (59.0, 60.0, None, "about 2.0 GB, under 2 min"),
            (59.1, 60.0, None, "about 2.0 GB, roughly 2 min"),
            (0.01, 0.02, Some(0.03), "about 3.0 GB, under 3 min"),
        ] {
            let read = stage(read);
            let convert = stage(convert);
            let check = check.map(stage);
            let conversion = combine_held_stages(read, convert);
            let total = check.map_or(conversion, |check| combine_held_stages(conversion, check));
            let mut parts = DesktopEstimate {
                conversion,
                split: Some(ReadConvertSplit { read, convert }),
                check,
                total,
            };
            assert_eq!(desktop_estimate_text(parts), expected);
            // Presentation must leave the raw memory and fractional timing intact.
            assert_eq!(parts.total, total);
            if check.is_none() {
                parts.split = None;
                assert_eq!(desktop_estimate_text(parts), estimate_text(total));
            }
        }
    }

    #[test]
    fn desktop_parts_share_the_row_total_and_add_overlapping_memory() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("synthetic.ipt");
        std::fs::File::create(&input)
            .unwrap()
            .set_len(180_000_000)
            .unwrap();
        let sizes = read_sizes(&input).unwrap();
        for preset in [Preset::Fast, Preset::Standard, Preset::Full] {
            for no_verify in [false, true] {
                for checker in [false, true] {
                    let options = crate::ConvertOptions {
                        preset,
                        no_verify,
                        ..Default::default()
                    };
                    let parts = desktop_estimate_parts(sizes, &options, checker).unwrap();
                    let effective = Choices {
                        history: options.choices().history,
                        ..options.choices()
                    };
                    assert_eq!(
                        desktop_estimate(sizes, &options, checker),
                        Some(parts.total)
                    );
                    if let Some(check) = parts.check {
                        assert_eq!(
                            parts.total.peak_bytes,
                            parts.conversion.peak_bytes + check.peak_bytes
                        );
                        assert_eq!(
                            parts.total.seconds_exact,
                            parts.conversion.seconds_exact + check.seconds_exact
                        );
                        assert_eq!(parts.total.seconds, parts.total.seconds_exact.ceil() as u64);
                        assert_eq!(
                            Some(parts.total),
                            estimate_from_sizes(sizes, &effective, &options)
                        );
                    } else {
                        assert_eq!(parts.total, parts.conversion);
                    }
                }
            }
        }
    }

    #[test]
    fn cached_sizes_keep_existing_estimates_without_reading_again() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("synthetic.ipt");
        std::fs::File::create(&input)
            .unwrap()
            .set_len(180_000_000)
            .unwrap();
        let sizes = read_sizes(&input).unwrap();
        for preset in [Preset::Fast, Preset::Standard, Preset::Full] {
            for no_verify in [false, true] {
                let options = crate::ConvertOptions {
                    preset,
                    no_verify,
                    ..Default::default()
                };
                assert_eq!(
                    estimate_with_options(&input, &options.choices(), &options),
                    estimate_from_sizes(sizes, &options.choices(), &options)
                );
            }
        }
        std::fs::remove_file(&input).unwrap();
        let options = crate::ConvertOptions::default();
        let checked = desktop_estimate(sizes, &options, true).unwrap();
        let unchecked = desktop_estimate(sizes, &options, false).unwrap();
        assert!(checked.peak_bytes > unchecked.peak_bytes);
        let off = crate::ConvertOptions {
            no_verify: true,
            ..options.clone()
        };
        assert_eq!(desktop_estimate(sizes, &off, true), Some(unchecked));
    }
}
