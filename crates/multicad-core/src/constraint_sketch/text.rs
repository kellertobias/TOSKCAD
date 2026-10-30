//! Parametric sketch text. Canonical source keeps only the string, bundled
//! font identity, cap height and placement; closed glyph outlines are derived
//! deterministically from the font bundled in the binary. TrueType glyph
//! contours are piecewise quadratic, so every contour maps exactly onto one
//! closed, clamped degree-2 B-spline with C0 Bezier joints. No polyline
//! approximation or system font participates.
use crate::document::{DocumentError, EntityId};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Project capability recorded whenever a constrained sketch carries text.
pub const CAPABILITY: &str = "core.sketch_text.1";
pub const MAX_TEXT_CHARS: usize = 64;
pub const MAX_TEXTS_PER_SKETCH: usize = 16;
/// The exact native spline region budget is 64 curves per sketch; text
/// outlines therefore stay inside it even before other curves are counted.
pub const MAX_TEXT_CONTOURS: usize = 64;
pub const MIN_HEIGHT_MM: f64 = 0.1;
pub const MAX_HEIGHT_MM: f64 = 10_000.;
const MAX_CONTOUR_POLES: usize = crate::constraint_sketch::MAX_SPLINE_POLES;

/// Fonts bundled in the binary. New fonts are new variants; an existing
/// variant never changes its outline source.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextFont {
    /// Hack Regular 3.003 (MIT, Bitstream Vera licence) from the pinned
    /// `epaint_default_fonts` crate that the application already ships.
    #[default]
    HackRegular,
}
impl TextFont {
    pub const ALL: [Self; 1] = [Self::HackRegular];
    pub fn label(self) -> &'static str {
        match self {
            Self::HackRegular => "Hack Regular",
        }
    }
    fn data(self) -> &'static [u8] {
        match self {
            Self::HackRegular => epaint_default_fonts::HACK_REGULAR,
        }
    }
}

/// Horizontal anchor of the baseline origin.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextAlignment {
    #[default]
    Left,
    Center,
    Right,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SketchText {
    pub id: EntityId,
    pub style: TextStyle,
}

/// Everything an author edits; shared by the native tool and MCP.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TextStyle {
    /// One line of supported characters (printable ASCII and listed Latin-1).
    pub text: String,
    pub font: TextFont,
    /// Cap height (height of capital letters such as H) in mm.
    pub height_mm: f64,
    /// Baseline anchor in sketch coordinates, mm.
    pub origin_mm: [f64; 2],
    /// Counterclockwise rotation of the baseline from sketch U, degrees.
    pub rotation_deg: f64,
    pub alignment: TextAlignment,
}
impl TextStyle {
    pub fn new(text: impl Into<String>, height_mm: f64, origin_mm: [f64; 2]) -> Self {
        Self {
            text: text.into(),
            font: TextFont::HackRegular,
            height_mm,
            origin_mm,
            rotation_deg: 0.,
            alignment: TextAlignment::Left,
        }
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
                "Text has {chars} characters; the limit is {MAX_TEXT_CHARS} per text element"
            )));
        }
        if let Some(c) = self.text.chars().find(|c| !supported_char(*c)) {
            return Err(invalid(format!(
                "Character {c:?} (U+{:04X}) is not supported by the bundled {} outlines; use printable ASCII or one of {SUPPORTED_LATIN1}",
                c as u32,
                self.font.label()
            )));
        }
        if !self.height_mm.is_finite() || !(MIN_HEIGHT_MM..=MAX_HEIGHT_MM).contains(&self.height_mm)
        {
            return Err(invalid(format!(
                "Text height must be between {MIN_HEIGHT_MM} and {MAX_HEIGHT_MM} mm"
            )));
        }
        if self
            .origin_mm
            .iter()
            .any(|v| !v.is_finite() || v.abs() > 1e9)
        {
            return Err(invalid("Text position must be finite sketch coordinates"));
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
        for c in self.text.chars() {
            count += font.glyph(c)?.len();
        }
        Ok(count)
    }
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
        let scale = style.height_mm / font.cap_height;
        let total_advance: f64 = style
            .text
            .chars()
            .map(|c| font.advance(c))
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .sum();
        let start = match style.alignment {
            TextAlignment::Left => 0.,
            TextAlignment::Center => -total_advance / 2.,
            TextAlignment::Right => -total_advance,
        };
        let (sin, cos) = exact_sin_cos(style.rotation_deg);
        let place = |x: f64, y: f64| {
            let (x, y) = (x * scale, y * scale);
            [
                style.origin_mm[0] + x * cos - y * sin,
                style.origin_mm[1] + x * sin + y * cos,
            ]
        };
        let mut pen_x = start;
        let mut contours = vec![];
        for (index, c) in style.text.chars().enumerate() {
            for (contour, segments) in font.glyph(c)?.into_iter().enumerate() {
                let mut poles = Vec::with_capacity(segments.len() * 2 + 1);
                poles.push(place(pen_x + segments[0].start[0], segments[0].start[1]));
                for segment in &segments {
                    poles.push(place(pen_x + segment.control[0], segment.control[1]));
                    poles.push(place(pen_x + segment.end[0], segment.end[1]));
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
        }
        if contours.len() > MAX_TEXT_CONTOURS {
            return Err(invalid(format!(
                "Text produces {} glyph contours; the exact region budget allows {MAX_TEXT_CONTOURS} per sketch. Shorten it or split it into another sketch",
                contours.len()
            )));
        }
        Ok(contours)
    }
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
        let cap_height = metrics
            .cap_height
            .map(f64::from)
            .filter(|h| *h > 0.)
            .ok_or_else(|| invalid(format!("Bundled font {} has no cap height", kind.label())))?;
        Ok(Self {
            font,
            cap_height,
            kind,
        })
    }
    fn glyph_id(&self, c: char) -> Result<skrifa::GlyphId, DocumentError> {
        use skrifa::MetadataProvider;
        if !supported_char(c) {
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
mod tests {
    use super::*;
    #[test]
    fn operations_add_edit_remove_and_refuse_without_change() {
        let source = super::super::source::Source {
            u: [1., 0., 0.],
            sketch: Default::default(),
        };
        let add = TextOperation::Add {
            style: TextStyle::new("ABO", 5., [1., 2.]),
        };
        let (added, id) = add.apply(&source).unwrap();
        assert_eq!(added.sketch.texts.len(), 1);
        assert_eq!(added.minimum_schema_version(), 2);
        let json = serde_json::to_value(&added).unwrap();
        let back: super::super::source::Source = serde_json::from_value(json).unwrap();
        assert_eq!(back, added, "text round-trips exactly through source JSON");
        let mut style = TextStyle::new("ABD", 6., [1., 2.]);
        let (edited, same) = TextOperation::Edit {
            text: id,
            style: style.clone(),
        }
        .apply(&added)
        .unwrap();
        assert_eq!(same, id);
        assert_eq!(edited.sketch.texts[0].style.text, "ABD");
        style.text.clear();
        assert!(
            TextOperation::Edit { text: id, style }
                .apply(&edited)
                .unwrap_err()
                .to_string()
                .contains("empty")
        );
        let (removed, _) = TextOperation::Remove { text: id }.apply(&edited).unwrap();
        assert!(removed.sketch.texts.is_empty());
        assert!(TextOperation::Remove { text: id }.apply(&removed).is_err());
        // Serialized sketches without text keep their previous JSON shape.
        assert!(
            !serde_json::to_string(&removed.sketch)
                .unwrap()
                .contains("texts")
        );
    }
    fn text(s: &str) -> SketchText {
        SketchText {
            id: EntityId::new(),
            style: TextStyle::new(s, 10., [0., 0.]),
        }
    }
    #[test]
    fn abo_maps_to_exact_closed_degree_two_contours() {
        let t = text("ABO");
        let contours = t.contours().unwrap();
        // A: outline + counter, B: outline + two counters, O: outline + counter.
        assert_eq!(contours.len(), 7);
        for c in &contours {
            assert_eq!(c.basis.degree, 2);
            assert_eq!(c.poles.first(), c.poles.last(), "closed contour");
            c.basis.validate(c.poles.len()).unwrap();
        }
        assert_eq!(contours, t.contours().unwrap(), "deterministic");
        let ids: std::collections::BTreeSet<_> = contours.iter().map(|c| c.id).collect();
        assert_eq!(ids.len(), 7);
        // Cap height of H is exactly the requested height.
        let h = text("H").contours().unwrap();
        let top = h[0].poles.iter().map(|p| p[1]).fold(f64::MIN, f64::max);
        let bottom = h[0].poles.iter().map(|p| p[1]).fold(f64::MAX, f64::min);
        assert!(
            (top - 10.).abs() < 1e-9 && bottom.abs() < 1e-9,
            "{top} {bottom}"
        );
    }
    #[test]
    fn placement_changes_coordinates_but_not_identities() {
        let mut t = text("AB");
        let before = t.contours().unwrap();
        t.style.origin_mm = [5., -3.];
        t.style.height_mm = 4.;
        t.style.rotation_deg = 90.;
        let after = t.contours().unwrap();
        assert_eq!(
            before.iter().map(|c| c.id).collect::<Vec<_>>(),
            after.iter().map(|c| c.id).collect::<Vec<_>>()
        );
        let p = before[0].poles[0];
        let q = after[0].poles[0];
        let s = 4. / 10.;
        assert_eq!(q, [5. - p[1] * s, -3. + p[0] * s]);
        t.style.alignment = TextAlignment::Right;
        t.style.rotation_deg = 0.;
        let right = t.contours().unwrap();
        let max_x = right
            .iter()
            .flat_map(|c| c.poles.iter())
            .map(|p| p[0])
            .fold(f64::MIN, f64::max);
        assert!(max_x <= 5. + 1e-9);
    }
    #[test]
    fn invalid_text_is_refused_with_actionable_messages() {
        for (s, h, needle) in [
            ("", 5., "empty"),
            ("   ", 5., "empty"),
            ("A\nB", 5., "not supported"),
            ("Ж", 5., "not supported"),
            ("AB", 0., "height"),
            ("AB", -1., "height"),
            ("AB", f64::NAN, "height"),
        ] {
            let mut t = text(s);
            t.style.height_mm = h;
            let e = t.validate().unwrap_err().to_string();
            assert!(e.contains(needle), "{s:?}: {e}");
        }
        let long = text(&"O".repeat(40));
        assert!(long.validate().is_ok());
        let e = long.contours().unwrap_err().to_string();
        assert!(e.contains("budget"), "{e}");
    }
    #[test]
    fn every_supported_character_has_exact_closed_contours() {
        let all: String = (' '..='~').chain(SUPPORTED_LATIN1.chars()).collect();
        for c in all.chars() {
            let t = text(&c.to_string());
            if c == ' ' {
                assert!(t.validate().is_err());
                continue;
            }
            let contours = t.contours().unwrap_or_else(|e| panic!("{c:?}: {e}"));
            assert!(!contours.is_empty(), "{c:?}");
        }
    }
}
