//! Bounded DXF and SVG artwork import into canonical constrained-sketch curves.
//! Parsers read only the supplied bytes: no scripts, external references,
//! fonts or raster content are evaluated. Unsupported content is counted and
//! reported instead of being approximated.
pub mod dxf;
pub mod geometry;
pub mod plan;
pub mod provenance;
pub mod svg;

use geometry::Segment;
use serde::{Deserialize, Serialize};

pub use plan::{ImportOptions, ImportPlan, ImportPreview};
pub use provenance::{CAPABILITY, PROVENANCE_TYPE, Provenance};

/// The serialized source limit; parsing never reads beyond the supplied bytes.
pub const MAX_FILE_BYTES: usize = 16 * 1024 * 1024;
/// Parsed curve budget before layer selection. Sketch limits apply afterwards.
pub const MAX_SEGMENTS: usize = 20_000;
pub const MAX_DIAGNOSTIC_KINDS: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Format {
    Dxf,
    Svg,
}
impl Format {
    pub fn from_file_name(name: &str) -> Option<Self> {
        let ext = name.rsplit_once('.')?.1.to_ascii_lowercase();
        match ext.as_str() {
            "dxf" => Some(Self::Dxf),
            "svg" => Some(Self::Svg),
            _ => None,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Dxf => "DXF",
            Self::Svg => "SVG",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceUnit {
    Millimeter,
    Centimeter,
    Meter,
    Inch,
    Foot,
    /// 1/72 inch.
    Point,
    /// CSS reference pixel, 1/96 inch.
    Pixel,
}
impl SourceUnit {
    pub const ALL: [Self; 7] = [
        Self::Millimeter,
        Self::Centimeter,
        Self::Meter,
        Self::Inch,
        Self::Foot,
        Self::Point,
        Self::Pixel,
    ];
    pub fn millimeters(self) -> f64 {
        match self {
            Self::Millimeter => 1.,
            Self::Centimeter => 10.,
            Self::Meter => 1000.,
            Self::Inch => 25.4,
            Self::Foot => 304.8,
            Self::Point => 25.4 / 72.,
            Self::Pixel => 25.4 / 96.,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Millimeter => "mm",
            Self::Centimeter => "cm",
            Self::Meter => "m",
            Self::Inch => "in",
            Self::Foot => "ft",
            Self::Point => "pt",
            Self::Pixel => "px (1/96 in)",
        }
    }
}

/// How file coordinates become millimeters before the user scale factor.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "unit", rename_all = "snake_case")]
pub enum UnitChoice {
    /// DXF `$INSUNITS`, or SVG root width/height/viewBox (px = 1/96 in when
    /// no absolute size is declared). Unitless DXF requires an explicit unit.
    FromFile,
    /// One DXF drawing unit or one SVG user unit equals this unit.
    Explicit(SourceUnit),
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Diagnostic {
    /// Stable kind such as `dxf.insert` or `svg.text`.
    pub kind: String,
    pub count: usize,
    pub message: String,
}

/// One connected source path. Its segments are in traversal order with shared
/// endpoints; `closed` means the final end returns to the first start.
#[derive(Clone, Debug, PartialEq)]
pub struct Path {
    pub layer: String,
    pub segments: Vec<Segment>,
    pub closed: bool,
}

/// Parsed artwork in file millimeters with Y up, before scale and placement.
#[derive(Clone, Debug, PartialEq)]
pub struct Artwork {
    pub format: Format,
    pub paths: Vec<Path>,
    /// Layers or groups in first-appearance order, including empty ones.
    pub layers: Vec<String>,
    pub unsupported: Vec<Diagnostic>,
    /// Human-readable interpretation notes, such as the unit policy used.
    pub notes: Vec<String>,
    pub millimeters_per_unit: f64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportError(pub String);
impl std::fmt::Display for ImportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for ImportError {}
pub(crate) fn fail<T>(message: impl Into<String>) -> Result<T, ImportError> {
    Err(ImportError(message.into()))
}

pub fn parse(format: Format, bytes: &[u8], units: UnitChoice) -> Result<Artwork, ImportError> {
    if bytes.len() > MAX_FILE_BYTES {
        return fail(format!(
            "{} file exceeds the {} MiB import limit",
            format.label(),
            MAX_FILE_BYTES / (1024 * 1024)
        ));
    }
    let artwork = match format {
        Format::Dxf => dxf::parse(bytes, units)?,
        Format::Svg => svg::parse(bytes, units)?,
    };
    if artwork
        .paths
        .iter()
        .any(|p| p.segments.iter().any(|s| !s.finite()))
    {
        return fail("Imported geometry contains nonfinite or out-of-range values");
    }
    Ok(artwork)
}

/// Accumulates unsupported content by stable kind with a bounded kind list.
#[derive(Default)]
pub(crate) struct Diagnostics(std::collections::BTreeMap<String, (usize, String)>);
impl Diagnostics {
    pub fn add(&mut self, kind: &str, message: impl Into<String>) {
        if let Some(entry) = self.0.get_mut(kind) {
            entry.0 += 1;
        } else if self.0.len() < MAX_DIAGNOSTIC_KINDS {
            self.0.insert(kind.into(), (1, message.into()));
        } else if let Some(entry) = self.0.get_mut("other") {
            entry.0 += 1;
        } else {
            self.0
                .insert("other".into(), (1, "Further unsupported content".into()));
        }
    }
    pub fn finish(self) -> Vec<Diagnostic> {
        self.0
            .into_iter()
            .map(|(kind, (count, message))| Diagnostic {
                kind,
                count,
                message,
            })
            .collect()
    }
}

/// Builds connected paths and enforces the parsed-segment budget.
#[derive(Default)]
pub(crate) struct Collector {
    pub paths: Vec<Path>,
    pub layers: Vec<String>,
    segments: usize,
}
impl Collector {
    pub fn layer(&mut self, name: &str) {
        if !self.layers.iter().any(|l| l == name) {
            self.layers.push(name.into());
        }
    }
    pub fn push(
        &mut self,
        layer: &str,
        segments: Vec<Segment>,
        closed: bool,
    ) -> Result<(), ImportError> {
        if segments.is_empty() {
            return Ok(());
        }
        self.layer(layer);
        self.segments += segments.len();
        if self.segments > MAX_SEGMENTS {
            return fail(format!(
                "Artwork exceeds the {MAX_SEGMENTS}-curve parsing budget"
            ));
        }
        self.paths.push(Path {
            layer: layer.into(),
            segments,
            closed,
        });
        Ok(())
    }
}
