//! One device-independent description of a checked drawing page, in paper
//! millimeters with y down. The SVG, PDF and drawing-DXF writers all consume
//! it, so every format shows the same geometry, text and line weights.
use super::{
    AnnotationEvaluation, AnnotationGraphic, AnnotationKind, DrawingEvaluation, FONT_MM, LINE_MM,
    VectorRole, ViewEvaluation, draft_dimension, layout_page, sheet_graphic,
};
use crate::document::{Document, EntityId};

/// What an item is, for layers, classes and styling.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Layer {
    Border,
    Title,
    Visible,
    Hidden,
    Section,
    Hatch,
    Dimension,
    Note,
    Leader,
    Weld,
    Table,
    Draft,
    /// View names and their actual paper:model scales.
    ViewLabel,
}
impl Layer {
    pub fn name(self) -> &'static str {
        match self {
            Self::Border => "border",
            Self::Title => "title",
            Self::Visible => "visible",
            Self::Hidden => "hidden",
            Self::Section => "section",
            Self::Hatch => "hatch",
            Self::Dimension => "dimension",
            Self::Note => "note",
            Self::Leader => "leader",
            Self::Weld => "weld",
            Self::Table => "table",
            Self::Draft => "draft",
            Self::ViewLabel => "view_label",
        }
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct Polyline {
    pub layer: Layer,
    pub points: Vec<[f64; 2]>,
    pub closed: bool,
    pub width_mm: f64,
    pub dashed: bool,
}
/// Counterclockwise on paper from `start_rad` to `end_rad` (y down).
#[derive(Clone, Debug, PartialEq)]
pub struct Arc {
    pub layer: Layer,
    pub center: [f64; 2],
    pub radius_mm: f64,
    pub start_rad: f64,
    pub end_rad: f64,
    pub width_mm: f64,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Fill {
    pub layer: Layer,
    pub points: Vec<[f64; 2]>,
}
/// One line of text; `at` is the left end of its baseline. `angle_deg`
/// turns clockwise on paper.
#[derive(Clone, Debug, PartialEq)]
pub struct Text {
    pub layer: Layer,
    pub at: [f64; 2],
    pub height_mm: f64,
    pub angle_deg: f64,
    pub text: String,
}
#[derive(Clone, Debug, PartialEq, Default)]
pub struct PageRender {
    pub id: EntityId,
    pub number: String,
    pub title: String,
    pub size_mm: [f64; 2],
    pub polylines: Vec<Polyline>,
    pub arcs: Vec<Arc>,
    pub fills: Vec<Fill>,
    pub texts: Vec<Text>,
    /// Paper rectangles showing the bundled application logo
    /// (`drawings::LOGO_PNG`), on the title layer.
    pub logos: Vec<[f64; 4]>,
}
impl PageRender {
    fn text(&mut self, layer: Layer, at: [f64; 2], height: f64, angle: f64, value: &str) {
        for (i, line) in value.lines().enumerate() {
            let (s, c) = angle.to_radians().sin_cos();
            let down = i as f64 * LINE_MM;
            self.texts.push(Text {
                layer,
                at: [at[0] - s * down, at[1] + c * down],
                height_mm: height,
                angle_deg: angle,
                text: line.to_owned(),
            });
        }
    }
    fn graphic(&mut self, layer: Layer, g: &AnnotationGraphic, width: f64) {
        for [a, b] in &g.lines {
            self.polylines.push(Polyline {
                layer,
                points: vec![*a, *b],
                closed: false,
                width_mm: width,
                dashed: false,
            });
        }
        for (center, r, start, end) in &g.arcs {
            self.arcs.push(Arc {
                layer,
                center: *center,
                radius_mm: *r,
                start_rad: *start,
                end_rad: *end,
                width_mm: width,
            });
        }
        for (tip, dir) in &g.arrows {
            let back = [
                tip[0] - dir[0] * super::ARROW_MM,
                tip[1] - dir[1] * super::ARROW_MM,
            ];
            let n = [-dir[1] * 0.45, dir[0] * 0.45];
            self.fills.push(Fill {
                layer,
                points: vec![
                    *tip,
                    [back[0] + n[0], back[1] + n[1]],
                    [back[0] - n[0], back[1] - n[1]],
                ],
            });
        }
        if !g.text.is_empty() {
            self.text(
                layer,
                g.text_mm,
                super::ANNOTATION_TEXT_MM,
                g.text_angle_deg,
                &g.text,
            );
        }
        for (at, label) in &g.labels {
            self.text(layer, *at, super::ANNOTATION_TEXT_MM, 0., label);
        }
    }
    /// Bounds of everything drawn, for clipping checks.
    pub fn content_bounds(&self) -> Option<[f64; 4]> {
        let mut b = [
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        ];
        let mut grow = |p: [f64; 2]| {
            b = [
                b[0].min(p[0]),
                b[1].min(p[1]),
                b[2].max(p[0]),
                b[3].max(p[1]),
            ];
        };
        self.polylines
            .iter()
            .flat_map(|p| &p.points)
            .for_each(|p| grow(*p));
        self.fills
            .iter()
            .flat_map(|f| &f.points)
            .for_each(|p| grow(*p));
        for a in &self.arcs {
            grow([a.center[0] - a.radius_mm, a.center[1] - a.radius_mm]);
            grow([a.center[0] + a.radius_mm, a.center[1] + a.radius_mm]);
        }
        for t in &self.texts {
            grow(t.at);
        }
        b[0].is_finite().then_some(b)
    }
}

/// Render one page from a current, checked evaluation. Stale results,
/// unresolved views and detached dimensions are refused.
pub fn render_page(
    document: &Document,
    revision: u64,
    book: EntityId,
    page: EntityId,
    evaluation: &DrawingEvaluation,
) -> Result<PageRender, String> {
    let checked = evaluation.checked_page(document, revision, book, page)?;
    let drawing = super::read(document, book).map_err(|e| e.to_string())?;
    let (index, page) = drawing
        .pages
        .iter()
        .enumerate()
        .find(|(_, p)| p.id == page)
        .ok_or("Drawing page is missing")?;
    let layout = layout_page(page, index + 1, drawing.pages.len()).map_err(|e| e.to_string())?;
    let mut out = PageRender {
        id: page.id,
        number: page.number.clone(),
        title: page.info.title.clone(),
        size_mm: layout.paper_mm,
        ..Default::default()
    };
    let [x0, y0, x1, y1] = layout.printable_mm;
    out.polylines.push(Polyline {
        layer: Layer::Border,
        points: vec![[x0, y0], [x1, y0], [x1, y1], [x0, y1]],
        closed: true,
        width_mm: 0.5,
        dashed: false,
    });
    title_block(&mut out, &layout);
    for cell in &layout.notes {
        let [cx0, cy0, _, _] = cell.rect_mm;
        out.text(
            Layer::Note,
            [cx0, cy0 + FONT_MM],
            FONT_MM,
            0.,
            &cell.lines.join("\n"),
        );
    }
    for (result, view) in &checked {
        let ViewEvaluation::Ready {
            vectors, labels, ..
        } = result
        else {
            continue;
        };
        for label in labels {
            out.text(
                Layer::Note,
                view.paper_point(label.at_mm),
                super::SOURCE_LABEL_MM,
                0.,
                &label.text,
            );
        }
        for vector in vectors {
            if let super::VectorCurve::HatchTriangle { points } = vector.curve {
                out.fills.push(Fill {
                    layer: Layer::Hatch,
                    points: points.map(|p| view.paper_point(p)).to_vec(),
                });
                continue;
            }
            out.polylines.push(Polyline {
                layer: match vector.role {
                    VectorRole::Visible => Layer::Visible,
                    VectorRole::Hidden => Layer::Hidden,
                    VectorRole::Section => Layer::Section,
                    VectorRole::Hatch => Layer::Hatch,
                },
                points: vector
                    .preview_mm
                    .iter()
                    .map(|p| view.paper_point(*p))
                    .collect(),
                closed: false,
                width_mm: vector.line_width_mm,
                dashed: vector.role == VectorRole::Hidden,
            });
        }
    }
    for caption in super::view_captions(page, &layout, Some(evaluation)) {
        out.text(
            Layer::ViewLabel,
            caption.at,
            caption.height_mm,
            0.,
            &caption.text,
        );
    }
    for annotation in &page.annotations {
        let layer = match &annotation.kind {
            AnnotationKind::Dimension { .. } | AnnotationKind::DraftDimension { .. } => {
                Layer::Dimension
            }
            AnnotationKind::Note { .. } => Layer::Note,
            AnnotationKind::Leader { .. } => Layer::Leader,
            AnnotationKind::Weld { .. } => Layer::Weld,
            AnnotationKind::Table { .. } => Layer::Table,
            AnnotationKind::Draft { .. } => Layer::Draft,
        };
        let (g, width) = match &annotation.kind {
            AnnotationKind::Dimension { .. } => match evaluation.annotations.get(&annotation.id) {
                Some(AnnotationEvaluation::Ready { graphic }) => (graphic.clone(), 0.18),
                _ => return Err("A dimension is not evaluated".into()),
            },
            AnnotationKind::DraftDimension {
                a,
                b,
                offset_mm,
                precision,
            } => (draft_dimension(page, *a, *b, *offset_mm, *precision), 0.18),
            AnnotationKind::Draft { line_width_mm, .. } => {
                (sheet_graphic(annotation), *line_width_mm)
            }
            _ => (sheet_graphic(annotation), 0.18),
        };
        out.graphic(layer, &g, width);
    }
    Ok(out)
}

/// The title block: a heavier outer frame, thin cell rules, small labels and
/// values at their cell's text height, and the optional logo.
fn title_block(out: &mut PageRender, layout: &super::PageLayout) {
    use super::{CELL_MM, FRAME_MM, LABEL_MM, PITCH};
    let rect = |r: [f64; 4], width: f64| Polyline {
        layer: Layer::Title,
        points: vec![[r[0], r[1]], [r[2], r[1]], [r[2], r[3]], [r[0], r[3]]],
        closed: true,
        width_mm: width,
        dashed: false,
    };
    for cell in &layout.title_cells {
        out.polylines.push(rect(cell.rect_mm, CELL_MM));
        let [x, y, _, _] = cell.rect_mm;
        let left = x + 1.2;
        out.texts.push(Text {
            layer: Layer::Title,
            at: [left, y + 1.2 + LABEL_MM],
            height_mm: LABEL_MM,
            angle_deg: 0.,
            text: cell.label.clone(),
        });
        let h = cell.height_mm;
        for (i, line) in cell.lines.iter().enumerate() {
            out.texts.push(Text {
                layer: Layer::Title,
                at: [left, y + 1.2 + LABEL_MM + 1.0 + h + i as f64 * h * PITCH],
                height_mm: h,
                angle_deg: 0.,
                text: line.clone(),
            });
        }
    }
    if let Some(frame) = super::title_block_rect(layout) {
        out.polylines.push(rect(frame, FRAME_MM));
    }
    out.logos.extend(layout.logo_mm);
}
