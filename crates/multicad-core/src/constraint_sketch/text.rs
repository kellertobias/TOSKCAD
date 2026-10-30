//! Parametric sketch text. Canonical source keeps only the string, bundled
//! font identity, size and placement. Closed glyph outlines are derived
//! deterministically from fonts bundled in the binary. TrueType glyph
//! contours are piecewise quadratic, so every contour maps exactly onto one
//! closed, clamped degree-2 B-spline with C0 Bezier joints. No polyline
//! approximation or system font participates.
//!
//! Placement (CAD-149): the text is a block of one or more lines. The block
//! box spans the widest line's advance width horizontally, and from the
//! first line's cap height down to the last line's baseline vertically. One
//! of nine anchors on that box sits at `position_mm`. Lines align left,
//! center or right inside the box.
use crate::document::{DocumentError, EntityId};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Project capability recorded whenever a constrained sketch carries text.
/// Version 2 stores the anchor model (CAD-149); version 1 files (baseline
/// origin and alignment, CAD-142) still open and migrate on read.
pub const CAPABILITY: &str = "core.sketch_text.2";
/// The CAD-142 storage form, still readable.
pub const LEGACY_CAPABILITY: &str = "core.sketch_text.1";
/// Characters per element, newlines included.
pub const MAX_TEXT_CHARS: usize = 64;
pub const MAX_TEXT_LINES: usize = 16;
pub const MAX_TEXTS_PER_SKETCH: usize = 16;
/// The exact native spline region budget is 64 curves per sketch; text
/// outlines therefore stay inside it even before other curves are counted.
pub const MAX_TEXT_CONTOURS: usize = 64;
pub const MIN_HEIGHT_MM: f64 = 0.1;
pub const MAX_HEIGHT_MM: f64 = 10_000.;
const MAX_CONTOUR_POLES: usize = crate::constraint_sketch::MAX_SPLINE_POLES;

/// Cantarell Regular 0.05, SIL Open Font License 1.1. Vendored unmodified;
/// see `fonts/Cantarell-Regular.txt`.
const CANTARELL_REGULAR: &[u8] = include_bytes!("../../fonts/Cantarell-Regular.ttf");

/// Fonts bundled in the binary. New fonts are new variants; an existing
/// variant never changes its outline source.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextFont {
    /// Cantarell Regular 0.05, a proportional sans-serif outline font
    /// (OFL-1.1). The default for new text.
    #[default]
    CantarellRegular,
    /// Hack Regular 3.003 (MIT, Bitstream Vera licence), monospaced, from the
    /// pinned `epaint_default_fonts` crate that the application already
    /// ships. Text created before CAD-149 uses it.
    HackRegular,
}
impl TextFont {
    pub const ALL: [Self; 2] = [Self::CantarellRegular, Self::HackRegular];
    pub fn label(self) -> &'static str {
        match self {
            Self::CantarellRegular => "Cantarell Regular",
            Self::HackRegular => "Hack Regular",
        }
    }
    /// Characters of the common set whose outlines in this font are not
    /// clean region boundaries: Cantarell draws `|` as two touching bars and
    /// its `ç` cedilla touches the c. They are refused for that font.
    pub fn excluded(self) -> &'static str {
        match self {
            Self::CantarellRegular => "|ç",
            Self::HackRegular => "",
        }
    }
    /// Whether this font's outline for `c` is verified as a clean region.
    pub fn supports(self, c: char) -> bool {
        supported_char(c) && !self.excluded().contains(c)
    }
    fn data(self) -> &'static [u8] {
        match self {
            Self::CantarellRegular => CANTARELL_REGULAR,
            Self::HackRegular => epaint_default_fonts::HACK_REGULAR,
        }
    }
}

/// Horizontal alignment of each line inside the text block.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextAlignment {
    #[default]
    Left,
    Center,
    Right,
}

/// Which point of the text block box sits at `position_mm`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextAnchor {
    TopLeft,
    TopCenter,
    TopRight,
    LeftCenter,
    Center,
    RightCenter,
    #[default]
    BottomLeft,
    BottomCenter,
    BottomRight,
}
impl TextAnchor {
    /// Row-major from the top-left, as shown in a 3 × 3 picker.
    pub const ALL: [Self; 9] = [
        Self::TopLeft,
        Self::TopCenter,
        Self::TopRight,
        Self::LeftCenter,
        Self::Center,
        Self::RightCenter,
        Self::BottomLeft,
        Self::BottomCenter,
        Self::BottomRight,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::TopLeft => "Top left",
            Self::TopCenter => "Top center",
            Self::TopRight => "Top right",
            Self::LeftCenter => "Left center",
            Self::Center => "Center",
            Self::RightCenter => "Right center",
            Self::BottomLeft => "Bottom left",
            Self::BottomCenter => "Bottom center",
            Self::BottomRight => "Bottom right",
        }
    }
    /// Horizontal fraction of the box: 0 left, 1 center, 2 right.
    fn column(self) -> u8 {
        match self {
            Self::TopLeft | Self::LeftCenter | Self::BottomLeft => 0,
            Self::TopCenter | Self::Center | Self::BottomCenter => 1,
            Self::TopRight | Self::RightCenter | Self::BottomRight => 2,
        }
    }
    /// Vertical position in the box: 0 top, 1 middle, 2 bottom.
    fn row(self) -> u8 {
        match self {
            Self::TopLeft | Self::TopCenter | Self::TopRight => 0,
            Self::LeftCenter | Self::Center | Self::RightCenter => 1,
            Self::BottomLeft | Self::BottomCenter | Self::BottomRight => 2,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SketchText {
    pub id: EntityId,
    pub style: TextStyle,
}

/// Everything an author edits; shared by the native tool and MCP.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "TextStyleRecord")]
pub struct TextStyle {
    /// Supported characters (printable ASCII and listed Latin-1); `\n`
    /// separates lines.
    pub text: String,
    pub font: TextFont,
    /// Size: the cap height (height of capital letters such as H) in mm.
    pub height_mm: f64,
    /// Where the anchor point of the text block sits, in sketch mm.
    pub position_mm: [f64; 2],
    pub anchor: TextAnchor,
    /// Counterclockwise rotation of the block about the anchor, from sketch U,
    /// in degrees.
    pub rotation_deg: f64,
    /// Alignment of each line inside the block.
    pub alignment: TextAlignment,
}

/// Stored form. CAD-142 wrote `origin_mm` (a baseline point) and no anchor;
/// that reads as the matching bottom anchor, which is the same placement for
/// single-line text.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TextStyleRecord {
    text: String,
    font: TextFont,
    height_mm: f64,
    #[serde(default)]
    position_mm: Option<[f64; 2]>,
    #[serde(default)]
    origin_mm: Option<[f64; 2]>,
    #[serde(default)]
    anchor: Option<TextAnchor>,
    rotation_deg: f64,
    alignment: TextAlignment,
}
impl TryFrom<TextStyleRecord> for TextStyle {
    type Error = String;
    fn try_from(r: TextStyleRecord) -> Result<Self, String> {
        let (position_mm, anchor) = match (r.position_mm, r.origin_mm, r.anchor) {
            (Some(position), None, Some(anchor)) => (position, anchor),
            (None, Some(origin), None) => (origin, TextAnchor::legacy(r.alignment)),
            _ => {
                return Err(
                    "sketch text needs position_mm with anchor, or the legacy origin_mm without anchor"
                        .into(),
                );
            }
        };
        Ok(Self {
            text: r.text,
            font: r.font,
            height_mm: r.height_mm,
            position_mm,
            anchor,
            rotation_deg: r.rotation_deg,
            alignment: r.alignment,
        })
    }
}
impl TextAnchor {
    /// CAD-142 placed the baseline's left end, middle or right end at the
    /// origin; for one line that is exactly the bottom anchor.
    pub fn legacy(alignment: TextAlignment) -> Self {
        match alignment {
            TextAlignment::Left => Self::BottomLeft,
            TextAlignment::Center => Self::BottomCenter,
            TextAlignment::Right => Self::BottomRight,
        }
    }
}

impl TextStyle {
    /// Cantarell text whose bottom-left anchor (the first character's
    /// baseline start, for one line) sits at `position_mm`.
    pub fn new(text: impl Into<String>, height_mm: f64, position_mm: [f64; 2]) -> Self {
        Self {
            text: text.into(),
            font: TextFont::CantarellRegular,
            height_mm,
            position_mm,
            anchor: TextAnchor::BottomLeft,
            rotation_deg: 0.,
            alignment: TextAlignment::Left,
        }
    }
    pub fn lines(&self) -> std::str::Split<'_, char> {
        self.text.split('\n')
    }
}

fn invalid(message: impl Into<String>) -> DocumentError {
    DocumentError::Invalid(message.into())
}

/// Characters whose bundled outlines are verified to form closed,
/// non-intersecting, correctly nested contours (see the native region test).
pub fn supported_char(c: char) -> bool {
    matches!(c, ' '..='~') || SUPPORTED_LATIN1.contains(c)
}
pub const SUPPORTED_LATIN1: &str = "ÄÖÜäöüßéèàç°±µ";

/// One exact closed glyph contour, already placed in sketch coordinates.
#[derive(Clone, Debug, PartialEq)]
pub struct TextContour {
    /// Stable identity derived from text ID, character index and contour index.
    pub id: EntityId,
    pub poles: Vec<[f64; 2]>,
    pub basis: crate::constraint_sketch::SplineBasis,
}
impl TextContour {
    /// Display and hit-test samples of the exact quadratic spans, closed
    /// (first point repeated last). Never used for manufacturing geometry.
    pub fn samples(&self, per_span: usize) -> Vec<[f64; 2]> {
        let per_span = per_span.max(1);
        let mut points = vec![self.poles[0]];
        for span in self.poles.windows(3).step_by(2) {
            for step in 1..=per_span {
                let t = step as f64 / per_span as f64;
                points.push(std::array::from_fn(|i| {
                    (1. - t) * (1. - t) * span[0][i]
                        + 2. * t * (1. - t) * span[1][i]
                        + t * t * span[2][i]
                }));
            }
        }
        points
    }
}

impl TextStyle {
    /// Structural checks shared by every entry point. Nothing is changed when
    /// this fails, and messages name the correction.
    pub fn validate(&self) -> Result<(), DocumentError> {
        let chars = self.text.chars().count();
        if self.text.trim().is_empty() {
            return Err(invalid(
                "Text is empty; type at least one visible character",
            ));
        }
        if chars > MAX_TEXT_CHARS {
            return Err(invalid(format!(
                "Text has {chars} characters; the limit is {MAX_TEXT_CHARS} per text element, line breaks included"
            )));
        }
        if let Some(c) = self
            .text
            .chars()
            .find(|c| *c != '\n' && supported_char(*c) && !self.font.supports(*c))
        {
            let other = TextFont::ALL
                .into_iter()
                .find(|f| f.supports(c))
                .map_or("another font", TextFont::label);
            return Err(invalid(format!(
                "Character {c:?} is not supported by {}: its outline pieces touch and cannot bound a clean profile; use {other} for it",
                self.font.label()
            )));
        }
        if let Some(c) = self
            .text
            .chars()
            .find(|c| *c != '\n' && !supported_char(*c))
        {
            return Err(invalid(format!(
                "Character {c:?} (U+{:04X}) is not supported by the bundled {} outlines; use printable ASCII, line breaks or one of {SUPPORTED_LATIN1}",
                c as u32,
                self.font.label()
            )));
        }
        let lines: Vec<&str> = self.lines().collect();
        if lines.len() > MAX_TEXT_LINES {
            return Err(invalid(format!(
                "Text has {} lines; the limit is {MAX_TEXT_LINES}",
                lines.len()
            )));
        }
        if lines.first().is_some_and(|l| l.trim().is_empty())
            || lines.last().is_some_and(|l| l.trim().is_empty())
        {
            return Err(invalid(
                "Text starts or ends with an empty line; remove it so the anchor box fits the lettering",
            ));
        }
        if !self.height_mm.is_finite() || !(MIN_HEIGHT_MM..=MAX_HEIGHT_MM).contains(&self.height_mm)
        {
            return Err(invalid(format!(
                "Text size (cap height) must be between {MIN_HEIGHT_MM} and {MAX_HEIGHT_MM} mm"
            )));
        }
        if self
            .position_mm
            .iter()
            .any(|v| !v.is_finite() || v.abs() > 1e9)
        {
            return Err(invalid(
                "Text anchor position must be finite sketch coordinates",
            ));
        }
        if !self.rotation_deg.is_finite() || self.rotation_deg.abs() > 360. {
            return Err(invalid(
                "Text rotation must be between -360 and 360 degrees",
            ));
        }
        Ok(())
    }
    /// Number of closed contours without placing them; used for budgets.
    pub fn contour_count(&self) -> Result<usize, DocumentError> {
        let font = Font::load(self.font)?;
        let mut count = 0;
        for c in self.text.chars().filter(|c| *c != '\n') {
            count += font.glyph(c)?.len();
        }
        Ok(count)
    }
    /// Block layout in font units: per-line widths, the block width, the
    /// line pitch and the anchor offset.
    fn layout(&self, font: &Font) -> Result<Layout, DocumentError> {
        let mut widths = vec![];
        for line in self.lines() {
            let width: f64 = line
                .chars()
                .map(|c| font.advance(c))
                .collect::<Result<Vec<_>, _>>()?
                .into_iter()
                .sum();
            widths.push(width);
        }
        let width = widths.iter().copied().fold(0., f64::max);
        let pitch = font.line_pitch;
        let bottom = -((widths.len() - 1) as f64 * pitch);
        let top = font.cap_height;
        let anchor = [
            match self.anchor.column() {
                0 => 0.,
                1 => width / 2.,
                _ => width,
            },
            match self.anchor.row() {
                0 => top,
                1 => (top + bottom) / 2.,
                _ => bottom,
            },
        ];
        Ok(Layout {
            widths,
            width,
            pitch,
            top,
            bottom,
            anchor,
            scale: self.height_mm / font.cap_height,
        })
    }
    /// The four corners of the block box in sketch coordinates:
    /// bottom-left, bottom-right, top-right, top-left (before rotation).
    pub fn block_corners(&self) -> Result<[[f64; 2]; 4], DocumentError> {
        self.validate()?;
        let font = Font::load(self.font)?;
        let layout = self.layout(&font)?;
        let place = self.placer(&layout);
        Ok([
            place(0., layout.bottom),
            place(layout.width, layout.bottom),
            place(layout.width, layout.top),
            place(0., layout.top),
        ])
    }
    /// Font units (block coordinates) to sketch coordinates.
    fn placer(&self, layout: &Layout) -> impl Fn(f64, f64) -> [f64; 2] {
        let (sin, cos) = exact_sin_cos(self.rotation_deg);
        let [ax, ay] = layout.anchor;
        let scale = layout.scale;
        let origin = self.position_mm;
        move |x: f64, y: f64| {
            let (x, y) = ((x - ax) * scale, (y - ay) * scale);
            [origin[0] + x * cos - y * sin, origin[1] + x * sin + y * cos]
        }
    }
}

struct Layout {
    widths: Vec<f64>,
    width: f64,
    pitch: f64,
    top: f64,
    bottom: f64,
    anchor: [f64; 2],
    scale: f64,
}

impl SketchText {
    pub fn validate(&self) -> Result<(), DocumentError> {
        if self.id.is_nil() {
            return Err(invalid("Text identity is nil"));
        }
        self.style.validate()
    }
    /// Deterministic exact outlines in sketch coordinates.
    pub fn contours(&self) -> Result<Vec<TextContour>, DocumentError> {
        self.validate()?;
        let style = &self.style;
        let font = Font::load(style.font)?;
        let layout = style.layout(&font)?;
        let place = style.placer(&layout);
        let mut contours = vec![];
        let mut index = 0usize;
        for (number, (line, width)) in style.lines().zip(&layout.widths).enumerate() {
            let mut pen_x = match style.alignment {
                TextAlignment::Left => 0.,
                TextAlignment::Center => (layout.width - width) / 2.,
                TextAlignment::Right => layout.width - width,
            };
            let baseline = -(number as f64 * layout.pitch);
            for c in line.chars() {
                for (contour, segments) in font.glyph(c)?.into_iter().enumerate() {
                    let mut poles = Vec::with_capacity(segments.len() * 2 + 1);
                    poles.push(place(
                        pen_x + segments[0].start[0],
                        baseline + segments[0].start[1],
                    ));
                    for segment in &segments {
                        poles.push(place(
                            pen_x + segment.control[0],
                            baseline + segment.control[1],
                        ));
                        poles.push(place(pen_x + segment.end[0], baseline + segment.end[1]));
                    }
                    if poles.len() > MAX_CONTOUR_POLES {
                        return Err(invalid(format!(
                            "Glyph {c:?} has a contour with more than {MAX_CONTOUR_POLES} poles"
                        )));
                    }
                    let spans = segments.len();
                    let mut multiplicities = vec![2; spans + 1];
                    multiplicities[0] = 3;
                    multiplicities[spans] = 3;
                    let basis = crate::constraint_sketch::SplineBasis {
                        degree: 2,
                        knots: (0..=spans).map(|k| k as f64).collect(),
                        multiplicities,
                        weights: None,
                    };
                    basis.validate(poles.len())?;
                    contours.push(TextContour {
                        id: derived_id(self.id, index as u64, contour as u64),
                        poles,
                        basis,
                    });
                }
                pen_x += font.advance(c)?;
                index += 1;
            }
            // The line break keeps its character index.
            index += 1;
        }
        if contours.len() > MAX_TEXT_CONTOURS {
            return Err(invalid(format!(
                "Text produces {} glyph contours; the exact region budget allows {MAX_TEXT_CONTOURS} per sketch. Shorten it or split it into another sketch",
                contours.len()
            )));
        }
        Ok(contours)
    }
    /// The text's sketch point at its anchor.
    pub fn anchor_mm(&self) -> [f64; 2] {
        self.style.position_mm
    }
    /// Whether `point` (sketch mm) lies inside one of this text's glyph
    /// outlines (counters included). Used for click selection only; the
    /// test uses display samples, never manufacturing geometry.
    pub fn contains(&self, point: [f64; 2]) -> bool {
        let Ok(contours) = self.contours() else {
            return false;
        };
        contours
            .iter()
            .any(|contour| inside_polygon(&contour.samples(8), point))
    }
    /// Whether `curve` is one of this text's derived contour identities.
    pub fn owns_contour(&self, curve: EntityId) -> bool {
        self.contours()
            .is_ok_and(|contours| contours.iter().any(|c| c.id == curve))
    }
}

/// The last listed (topmost drawn) text whose glyph outlines contain `point`.
pub fn text_at(texts: &[SketchText], point: [f64; 2]) -> Option<EntityId> {
    texts.iter().rev().find(|t| t.contains(point)).map(|t| t.id)
}

/// Even-odd crossing test against a closed sample polygon.
fn inside_polygon(polygon: &[[f64; 2]], p: [f64; 2]) -> bool {
    let mut inside = false;
    for edge in polygon.windows(2) {
        let (a, b) = (edge[0], edge[1]);
        if (a[1] > p[1]) != (b[1] > p[1]) {
            let x = a[0] + (p[1] - a[1]) / (b[1] - a[1]) * (b[0] - a[0]);
            if p[0] < x {
                inside = !inside;
            }
        }
    }
    inside
}

/// Multiples of 90 degrees keep exact axis-aligned outlines.
fn exact_sin_cos(degrees: f64) -> (f64, f64) {
    let quarter = degrees / 90.;
    if quarter == quarter.round() {
        match (quarter as i64).rem_euclid(4) {
            0 => (0., 1.),
            1 => (1., 0.),
            2 => (0., -1.),
            _ => (-1., 0.),
        }
    } else {
        degrees.to_radians().sin_cos()
    }
}

/// Name-based UUID (version 8 layout) from SHA-256, stable across sessions.
pub fn derived_id(text: EntityId, glyph: u64, contour: u64) -> EntityId {
    let mut hash = Sha256::new();
    hash.update(b"multicad.sketch_text.contour.1");
    hash.update(text.to_string().as_bytes());
    hash.update(glyph.to_le_bytes());
    hash.update(contour.to_le_bytes());
    let digest = hash.finalize();
    let mut bytes = [0u8; 16];
    bytes.copy_from_slice(&digest[..16]);
    bytes[6] = (bytes[6] & 0x0f) | 0x80;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    EntityId::parse(&uuid::Uuid::from_bytes(bytes).to_string()).expect("formatted UUID parses")
}

/// One quadratic Bezier span in font units; straight spans use their midpoint
/// as control point, which is the exact degree-2 representation of a line.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Span {
    start: [f64; 2],
    control: [f64; 2],
    end: [f64; 2],
}

struct Font {
    font: skrifa::FontRef<'static>,
    cap_height: f64,
    /// Baseline-to-baseline distance in font units: ascent − descent + gap.
    line_pitch: f64,
    kind: TextFont,
}
impl Font {
    fn load(kind: TextFont) -> Result<Self, DocumentError> {
        use skrifa::MetadataProvider;
        let font = skrifa::FontRef::new(kind.data())
            .map_err(|e| invalid(format!("Bundled font {} is unreadable: {e}", kind.label())))?;
        let metrics = font.metrics(
            skrifa::instance::Size::unscaled(),
            skrifa::instance::LocationRef::default(),
        );
        let line_pitch =
            f64::from(metrics.ascent) - f64::from(metrics.descent) + f64::from(metrics.leading);
        if line_pitch.is_nan() || line_pitch <= 0. {
            return Err(invalid(format!(
                "Bundled font {} has no line spacing",
                kind.label()
            )));
        }
        let mut loaded = Self {
            font,
            cap_height: 0.,
            line_pitch,
            kind,
        };
        // The cap height is the top of the H outline, so H spans exactly
        // 0..size. (Hack's OS/2 cap height equals it; Cantarell's OS/2 value
        // is a few units lower than its H.)
        loaded.cap_height = loaded
            .glyph('H')?
            .iter()
            .flatten()
            .flat_map(|s| [s.start[1], s.control[1], s.end[1]])
            .fold(0., f64::max);
        if loaded.cap_height.is_nan() || loaded.cap_height <= 0. {
            return Err(invalid(format!(
                "Bundled font {} has no cap height",
                kind.label()
            )));
        }
        Ok(loaded)
    }
    fn glyph_id(&self, c: char) -> Result<skrifa::GlyphId, DocumentError> {
        use skrifa::MetadataProvider;
        if !self.kind.supports(c) {
            return Err(invalid(format!(
                "Character {c:?} is not supported by the bundled {} outlines",
                self.kind.label()
            )));
        }
        self.font
            .charmap()
            .map(c)
            .filter(|g| g.to_u32() != 0)
            .ok_or_else(|| {
                invalid(format!(
                    "Bundled font {} has no glyph for {c:?}",
                    self.kind.label()
                ))
            })
    }
    fn advance(&self, c: char) -> Result<f64, DocumentError> {
        use skrifa::MetadataProvider;
        let glyph = self.glyph_id(c)?;
        self.font
            .glyph_metrics(
                skrifa::instance::Size::unscaled(),
                skrifa::instance::LocationRef::default(),
            )
            .advance_width(glyph)
            .map(f64::from)
            .ok_or_else(|| invalid(format!("Glyph {c:?} has no advance width")))
    }
    fn glyph(&self, c: char) -> Result<Vec<Vec<Span>>, DocumentError> {
        use skrifa::MetadataProvider;
        let glyph = self.glyph_id(c)?;
        let outline = self
            .font
            .outline_glyphs()
            .get(glyph)
            .ok_or_else(|| invalid(format!("Glyph {c:?} has no outline")))?;
        let mut pen = SpanPen::default();
        outline
            .draw(
                skrifa::outline::DrawSettings::unhinted(
                    skrifa::instance::Size::unscaled(),
                    skrifa::instance::LocationRef::default(),
                ),
                &mut pen,
            )
            .map_err(|e| invalid(format!("Glyph {c:?} outline failed: {e}")))?;
        pen.finish();
        if let Some(message) = pen.error {
            return Err(invalid(format!("Glyph {c:?}: {message}")));
        }
        Ok(pen.contours)
    }
}

#[derive(Default)]
struct SpanPen {
    contours: Vec<Vec<Span>>,
    current: Vec<Span>,
    start: Option<[f64; 2]>,
    at: [f64; 2],
    error: Option<String>,
}
fn same(a: [f64; 2], b: [f64; 2]) -> bool {
    a[0] == b[0] && a[1] == b[1]
}
fn midpoint(a: [f64; 2], b: [f64; 2]) -> [f64; 2] {
    [(a[0] + b[0]) / 2., (a[1] + b[1]) / 2.]
}
fn straight(s: &Span) -> bool {
    same(s.control, midpoint(s.start, s.end))
}
/// Exact in font units: glyf coordinates are small integers or halves.
fn collinear(a: &Span, b: &Span) -> bool {
    let d = [a.end[0] - a.start[0], a.end[1] - a.start[1]];
    let e = [b.end[0] - b.start[0], b.end[1] - b.start[1]];
    straight(a)
        && straight(b)
        && same(a.end, b.start)
        && d[0] * e[1] - d[1] * e[0] == 0.
        && d[0] * e[0] + d[1] * e[1] > 0.
}
impl SpanPen {
    fn push(&mut self, control: [f64; 2], end: [f64; 2]) {
        if same(self.at, end) && same(self.at, control) {
            return;
        }
        if same(self.at, end) {
            // A quadratic span that returns to its start encloses no area and
            // would make the contour self-touching; fonts do not need it.
            self.error = Some("zero-length curved glyph segment".into());
            return;
        }
        let span = Span {
            start: self.at,
            control,
            end,
        };
        // Consecutive collinear straight spans become one span; overlapping
        // parameter ranges along one line would otherwise read as overlap.
        match self.current.last_mut() {
            Some(last) if collinear(last, &span) => {
                last.end = end;
                last.control = midpoint(last.start, end);
            }
            _ => self.current.push(span),
        }
        self.at = end;
    }
    fn finish(&mut self) {
        if let Some(start) = self.start.take() {
            if !same(self.at, start) {
                self.push(midpoint(self.at, start), start);
            }
            let mut spans = std::mem::take(&mut self.current);
            // Move a seam that splits one straight edge to that edge's start.
            if spans.len() >= 3 && collinear(spans.last().expect("spans"), &spans[0]) {
                let last = spans.pop().expect("spans");
                spans[0].start = last.start;
                spans[0].control = midpoint(last.start, spans[0].end);
            }
            if spans.len() >= 2 {
                self.contours.push(spans);
            } else if !spans.is_empty() {
                self.error = Some("degenerate glyph contour".into());
            }
        }
    }
}
impl skrifa::outline::OutlinePen for SpanPen {
    fn move_to(&mut self, x: f32, y: f32) {
        self.finish();
        let p = [f64::from(x), f64::from(y)];
        self.start = Some(p);
        self.at = p;
    }
    fn line_to(&mut self, x: f32, y: f32) {
        let end = [f64::from(x), f64::from(y)];
        let mid = [(self.at[0] + end[0]) / 2., (self.at[1] + end[1]) / 2.];
        self.push(mid, end);
    }
    fn quad_to(&mut self, cx0: f32, cy0: f32, x: f32, y: f32) {
        self.push(
            [f64::from(cx0), f64::from(cy0)],
            [f64::from(x), f64::from(y)],
        );
    }
    fn curve_to(&mut self, _: f32, _: f32, _: f32, _: f32, _: f32, _: f32) {
        self.error = Some("cubic outlines are not supported by the degree-2 text mapping".into());
    }
    fn close(&mut self) {
        self.finish();
    }
}

/// Typed text edit shared by the native tool and MCP. Every variant is one
/// source transaction; a refused edit leaves the source unchanged.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum TextOperation {
    Add { style: TextStyle },
    Edit { text: EntityId, style: TextStyle },
    Remove { text: EntityId },
}
impl TextOperation {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Add { .. } => "Add sketch text",
            Self::Edit { .. } => "Edit sketch text",
            Self::Remove { .. } => "Remove sketch text",
        }
    }
    /// Apply to a copy of `source`; returns the changed source and the text
    /// identity it concerns.
    pub fn apply(
        &self,
        source: &super::source::Source,
    ) -> Result<(super::source::Source, EntityId), DocumentError> {
        let mut next = source.clone();
        let id = match self {
            Self::Add { style } => {
                style.validate()?;
                let id = EntityId::new();
                next.sketch.texts.push(SketchText {
                    id,
                    style: style.clone(),
                });
                id
            }
            Self::Edit { text, style } => {
                style.validate()?;
                let existing = next
                    .sketch
                    .texts
                    .iter_mut()
                    .find(|t| t.id == *text)
                    .ok_or_else(|| invalid(format!("Sketch text {text} does not exist")))?;
                if existing.style == *style {
                    return Err(invalid("The text is unchanged; nothing to apply"));
                }
                existing.style = style.clone();
                *text
            }
            Self::Remove { text } => {
                let before = next.sketch.texts.len();
                next.sketch.texts.retain(|t| t.id != *text);
                if next.sketch.texts.len() == before {
                    return Err(invalid(format!("Sketch text {text} does not exist")));
                }
                *text
            }
        };
        next.validate()?;
        if !matches!(self, Self::Remove { .. }) {
            let text = next
                .sketch
                .texts
                .iter()
                .find(|t| t.id == id)
                .expect("applied text");
            text.contours()?;
        }
        Ok((next, id))
    }
}

#[cfg(test)]
#[path = "text_tests.rs"]
mod tests;
