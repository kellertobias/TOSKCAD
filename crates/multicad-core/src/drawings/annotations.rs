//! Drawing annotations: associative dimensions on persistent model references,
//! notes, leaders, weld symbols, tables and sheet-space drafting geometry.
//! Annotations are page content only; none of them drives the model.
use super::{DrawingView, Page, source::invalid};
use crate::{
    document::{DocumentError, EntityId},
    inspection::GeometryReference,
};
use serde::{Deserialize, Serialize};

pub const ANNOTATION_CAPABILITY: &str = "drawings.annotations.1";
pub const MAX_ANNOTATIONS: usize = 2048;
/// Paper text height of annotation text, in mm.
pub const ANNOTATION_TEXT_MM: f64 = 2.5;
/// Paper arrowhead length, in mm.
pub const ARROW_MM: f64 = 2.5;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Annotation {
    pub id: EntityId,
    pub kind: AnnotationKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LinearDirection {
    /// Along the view's u (paper horizontal).
    Horizontal,
    /// Along the view's v (paper vertical).
    Vertical,
    /// Along the line between the projected points.
    Aligned,
}

/// What a dimension measures. References are persistent model topology; the
/// value is recomputed from current geometry on every evaluation.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum DimensionMeasure {
    /// Distance between the closest points of two references, projected into
    /// the view plane along the chosen direction.
    Linear {
        a: GeometryReference,
        b: GeometryReference,
        direction: LinearDirection,
    },
    Radius {
        edge: GeometryReference,
    },
    Diameter {
        edge: GeometryReference,
    },
    /// The model angle between two straight edges.
    Angle {
        a: GeometryReference,
        b: GeometryReference,
    },
}
impl DimensionMeasure {
    pub fn references(&self) -> Vec<&GeometryReference> {
        match self {
            Self::Linear { a, b, .. } | Self::Angle { a, b } => vec![a, b],
            Self::Radius { edge } | Self::Diameter { edge } => vec![edge],
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WeldSymbol {
    Fillet,
    SquareGroove,
    VGroove,
    BevelGroove,
    Plug,
    Spot,
    Seam,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WeldSide {
    Arrow,
    Other,
    Both,
}

/// Sheet-space drafting geometry in paper millimeters, independent of model
/// sketches and features.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum DraftGeometry {
    Line {
        a: [f64; 2],
        b: [f64; 2],
    },
    Circle {
        center: [f64; 2],
        radius_mm: f64,
    },
    /// Counterclockwise on paper (y down) from `start_deg` by `sweep_deg`.
    Arc {
        center: [f64; 2],
        radius_mm: f64,
        start_deg: f64,
        sweep_deg: f64,
    },
    /// Parallel hatch lines clipped to a closed polygon.
    Hatch {
        boundary: Vec<[f64; 2]>,
        angle_deg: f64,
        spacing_mm: f64,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum AnnotationKind {
    /// Associative model dimension in a view on the same page. `offset_mm` is
    /// the paper distance of the dimension line from the measured geometry.
    Dimension {
        view: EntityId,
        measure: Box<DimensionMeasure>,
        offset_mm: f64,
        precision: u8,
        /// Non-driving reference dimension, shown in parentheses.
        reference: bool,
    },
    Note {
        position_mm: [f64; 2],
        text: String,
    },
    /// Arrow at the first point; text at the last.
    Leader {
        points_mm: Vec<[f64; 2]>,
        text: String,
    },
    /// A weld symbol on a leader: arrow at the first point, reference line from
    /// the last point.
    Weld {
        points_mm: Vec<[f64; 2]>,
        symbol: WeldSymbol,
        side: WeldSide,
        size: String,
        length: String,
        all_around: bool,
        field: bool,
        tail: String,
    },
    /// A placed schedule. Content is authored or supplied by a producer.
    Table {
        position_mm: [f64; 2],
        title: String,
        columns: Vec<String>,
        rows: Vec<Vec<String>>,
        widths_mm: Vec<f64>,
        row_height_mm: f64,
    },
    Draft {
        geometry: DraftGeometry,
        line_width_mm: f64,
    },
    /// A dimension between two sheet points, reading in page-scale units.
    DraftDimension {
        a: [f64; 2],
        b: [f64; 2],
        offset_mm: f64,
        precision: u8,
    },
}

fn finite(points: &[[f64; 2]]) -> bool {
    points.iter().flatten().all(|v| v.is_finite())
}
fn text(value: &str, max: usize) -> Result<(), DocumentError> {
    if value.len() > max
        || value
            .chars()
            .any(|c| c.is_control() && c != '\n' && c != '\t')
    {
        Err(invalid(
            "Annotation text is too long or contains control characters",
        ))
    } else {
        Ok(())
    }
}

impl Annotation {
    /// Sheet points that must lie on the paper.
    pub fn anchor_points(&self) -> Vec<[f64; 2]> {
        match &self.kind {
            AnnotationKind::Dimension { .. } => vec![],
            AnnotationKind::Note { position_mm, .. }
            | AnnotationKind::Table { position_mm, .. } => vec![*position_mm],
            AnnotationKind::Leader { points_mm, .. } | AnnotationKind::Weld { points_mm, .. } => {
                points_mm.clone()
            }
            AnnotationKind::Draft { geometry, .. } => match geometry {
                DraftGeometry::Line { a, b } => vec![*a, *b],
                DraftGeometry::Circle { center, radius_mm }
                | DraftGeometry::Arc {
                    center, radius_mm, ..
                } => vec![
                    [center[0] - radius_mm, center[1] - radius_mm],
                    [center[0] + radius_mm, center[1] + radius_mm],
                ],
                DraftGeometry::Hatch { boundary, .. } => boundary.clone(),
            },
            AnnotationKind::DraftDimension { a, b, .. } => vec![*a, *b],
        }
    }
    pub fn validate(&self, page: &Page) -> Result<(), DocumentError> {
        let [w, h] = page.settings.dimensions();
        if self.id.is_nil() {
            return Err(invalid("Annotation identity is required"));
        }
        let points = self.anchor_points();
        if !finite(&points)
            || points
                .iter()
                .any(|p| p[0] < 0. || p[1] < 0. || p[0] > w || p[1] > h)
        {
            return Err(invalid("Annotation geometry must lie on the paper"));
        }
        match &self.kind {
            AnnotationKind::Dimension {
                view,
                measure,
                offset_mm,
                precision,
                ..
            } => {
                if !page.views.iter().any(|v| v.id == *view) {
                    return Err(invalid("A dimension needs a view on the same page"));
                }
                if !offset_mm.is_finite() || offset_mm.abs() > 500. || *precision > 6 {
                    return Err(invalid(
                        "Dimension offset must be within 500 mm and precision at most 6",
                    ));
                }
                for reference in measure.references() {
                    reference.validate().map_err(invalid)?;
                }
                if let DimensionMeasure::Linear { a, b, .. } | DimensionMeasure::Angle { a, b } =
                    measure.as_ref()
                    && a.intent.id == b.intent.id
                {
                    return Err(invalid(
                        "A dimension needs independent reference identities",
                    ));
                }
            }
            AnnotationKind::Note { text: t, .. } => text(t, 4096)?,
            AnnotationKind::Leader { points_mm, text: t } => {
                text(t, 1024)?;
                if !(2..=16).contains(&points_mm.len()) {
                    return Err(invalid("A leader has 2 to 16 points"));
                }
            }
            AnnotationKind::Weld {
                points_mm,
                size,
                length,
                tail,
                ..
            } => {
                for t in [size, length, tail] {
                    text(t, 64)?;
                }
                if !(2..=16).contains(&points_mm.len()) {
                    return Err(invalid("A weld symbol leader has 2 to 16 points"));
                }
            }
            AnnotationKind::Table {
                title,
                columns,
                rows,
                widths_mm,
                row_height_mm,
                ..
            } => {
                text(title, 256)?;
                if columns.is_empty()
                    || columns.len() > 32
                    || rows.len() > 256
                    || widths_mm.len() != columns.len()
                    || rows.iter().any(|r| r.len() != columns.len())
                    || widths_mm
                        .iter()
                        .chain(std::iter::once(row_height_mm))
                        .any(|v| !v.is_finite() || !(2.0..=500.).contains(v))
                {
                    return Err(invalid(
                        "A table needs 1 to 32 columns with widths, matching rows and sizes of 2 to 500 mm",
                    ));
                }
                for cell in columns.iter().chain(rows.iter().flatten()) {
                    text(cell, 256)?;
                }
                let [x0, y0, x1, y1] = table_bounds(self).expect("table");
                if x0 < 0. || y0 < 0. || x1 > w || y1 > h {
                    return Err(invalid("The table must fit on the paper"));
                }
            }
            AnnotationKind::Draft {
                geometry,
                line_width_mm,
            } => {
                if !line_width_mm.is_finite() || !(0.05..=2.).contains(line_width_mm) {
                    return Err(invalid("Drafting line width must be 0.05 to 2 mm"));
                }
                match geometry {
                    DraftGeometry::Line { a, b } if a == b => {
                        return Err(invalid("A drafting line needs two distinct points"));
                    }
                    DraftGeometry::Circle { radius_mm, .. }
                    | DraftGeometry::Arc { radius_mm, .. }
                        if !radius_mm.is_finite() || *radius_mm < 0.01 =>
                    {
                        return Err(invalid("Drafting radius must be positive"));
                    }
                    DraftGeometry::Arc {
                        start_deg,
                        sweep_deg,
                        ..
                    } if !start_deg.is_finite()
                        || !sweep_deg.is_finite()
                        || sweep_deg.abs() < 1e-6
                        || sweep_deg.abs() > 360. =>
                    {
                        return Err(invalid("Drafting arc needs a finite sweep up to 360°"));
                    }
                    DraftGeometry::Hatch {
                        boundary,
                        angle_deg,
                        spacing_mm,
                    } if !(3..=256).contains(&boundary.len())
                        || !angle_deg.is_finite()
                        || !spacing_mm.is_finite()
                        || *spacing_mm < 0.2 =>
                    {
                        return Err(invalid(
                            "A hatch needs 3 to 256 boundary points and spacing of at least 0.2 mm",
                        ));
                    }
                    _ => {}
                }
            }
            AnnotationKind::DraftDimension {
                a,
                b,
                offset_mm,
                precision,
            } => {
                if a == b || !offset_mm.is_finite() || offset_mm.abs() > 500. || *precision > 6 {
                    return Err(invalid(
                        "A drafting dimension needs distinct points, offset within 500 mm and precision at most 6",
                    ));
                }
            }
        }
        Ok(())
    }
}

/// Paper bounds [x0, y0, x1, y1] of a table.
pub fn table_bounds(annotation: &Annotation) -> Option<[f64; 4]> {
    let AnnotationKind::Table {
        position_mm,
        title,
        rows,
        widths_mm,
        row_height_mm,
        ..
    } = &annotation.kind
    else {
        return None;
    };
    let width: f64 = widths_mm.iter().sum();
    let lines = rows.len() + 1 + usize::from(!title.is_empty());
    Some([
        position_mm[0],
        position_mm[1],
        position_mm[0] + width,
        position_mm[1] + lines as f64 * row_height_mm,
    ])
}

/// A rigid sheet transform: rotate about `about`, then translate. Lengths and
/// angles are preserved, so drafting dimensions keep their values.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SheetTransform {
    pub translate_mm: [f64; 2],
    pub rotate_deg: f64,
    pub about_mm: [f64; 2],
}
impl SheetTransform {
    pub fn translation(translate_mm: [f64; 2]) -> Self {
        Self {
            translate_mm,
            rotate_deg: 0.,
            about_mm: [0.; 2],
        }
    }
    pub fn apply(&self, p: [f64; 2]) -> [f64; 2] {
        let (s, c) = self.rotate_deg.to_radians().sin_cos();
        let d = [p[0] - self.about_mm[0], p[1] - self.about_mm[1]];
        [
            self.about_mm[0] + c * d[0] - s * d[1] + self.translate_mm[0],
            self.about_mm[1] + s * d[0] + c * d[1] + self.translate_mm[1],
        ]
    }
}
/// Move or rotate a sheet annotation. Dimensions move only their offset and
/// are not transformed; tables, notes and text rotate their anchor only.
pub fn transform(kind: &mut AnnotationKind, t: SheetTransform) -> Result<(), String> {
    let map = |p: &mut [f64; 2]| *p = t.apply(*p);
    match kind {
        AnnotationKind::Dimension { .. } => {
            return Err("Model dimensions follow their view; change the offset instead".into());
        }
        AnnotationKind::Note { position_mm, .. } | AnnotationKind::Table { position_mm, .. } => {
            map(position_mm)
        }
        AnnotationKind::Leader { points_mm, .. } | AnnotationKind::Weld { points_mm, .. } => {
            points_mm.iter_mut().for_each(map)
        }
        AnnotationKind::Draft { geometry, .. } => match geometry {
            DraftGeometry::Line { a, b } => {
                map(a);
                map(b);
            }
            DraftGeometry::Circle { center, .. } => map(center),
            DraftGeometry::Arc {
                center, start_deg, ..
            } => {
                map(center);
                *start_deg += t.rotate_deg;
            }
            DraftGeometry::Hatch {
                boundary,
                angle_deg,
                ..
            } => {
                boundary.iter_mut().for_each(map);
                *angle_deg += t.rotate_deg;
            }
        },
        AnnotationKind::DraftDimension { a, b, .. } => {
            map(a);
            map(b);
        }
    }
    Ok(())
}

/// Paper graphics of an evaluated annotation.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AnnotationGraphic {
    pub lines: Vec<[[f64; 2]; 2]>,
    /// Arrowheads as (tip, unit direction the arrow points).
    pub arrows: Vec<([f64; 2], [f64; 2])>,
    /// Arcs as (center, radius, start rad, end rad) on paper.
    pub arcs: Vec<([f64; 2], f64, f64, f64)>,
    pub text: String,
    pub text_mm: [f64; 2],
    pub text_angle_deg: f64,
    /// Further horizontal single-line labels, each at the left end of its
    /// baseline, such as table cells.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub labels: Vec<([f64; 2], String)>,
    /// The measured model value: millimeters, or degrees for angles.
    pub value: f64,
    pub unit: String,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum AnnotationEvaluation {
    Ready {
        graphic: AnnotationGraphic,
    },
    /// A reference no longer resolves; the old value is not shown.
    Detached {
        message: String,
    },
}

fn sub(a: [f64; 2], b: [f64; 2]) -> [f64; 2] {
    [a[0] - b[0], a[1] - b[1]]
}
fn add(a: [f64; 2], b: [f64; 2]) -> [f64; 2] {
    [a[0] + b[0], a[1] + b[1]]
}
fn scale(a: [f64; 2], s: f64) -> [f64; 2] {
    [a[0] * s, a[1] * s]
}
fn length(a: [f64; 2]) -> f64 {
    a[0].hypot(a[1])
}
fn unit(a: [f64; 2]) -> [f64; 2] {
    let l = length(a);
    if l < 1e-12 {
        [1., 0.]
    } else {
        scale(a, 1. / l)
    }
}
fn number(value: f64, precision: u8) -> String {
    format!("{value:.*}", usize::from(precision))
}
fn wrap(text: String, reference: bool) -> String {
    if reference { format!("({text})") } else { text }
}

/// Model point in the view's model coordinates (u, v).
pub fn view_coordinates(view: &DrawingView, world_mm: [f64; 3]) -> [f64; 2] {
    let d: [f64; 3] = std::array::from_fn(|i| world_mm[i] - view.frame.origin_mm[i]);
    let dot = |a: [f64; 3]| a[0] * d[0] + a[1] * d[1] + a[2] * d[2];
    [dot(view.frame.u), dot(view.frame.v)]
}

/// A linear dimension between two model points. The value is measured in
/// the view plane, in model units, independent of view and page scale.
pub fn linear_dimension(
    view: &DrawingView,
    a_world: [f64; 3],
    b_world: [f64; 3],
    direction: LinearDirection,
    offset_mm: f64,
    precision: u8,
    reference: bool,
) -> AnnotationGraphic {
    let (a, b) = (
        view_coordinates(view, a_world),
        view_coordinates(view, b_world),
    );
    let value = match direction {
        LinearDirection::Horizontal => (b[0] - a[0]).abs(),
        LinearDirection::Vertical => (b[1] - a[1]).abs(),
        LinearDirection::Aligned => length(sub(b, a)),
    };
    let (pa, pb) = (view.paper_point(a), view.paper_point(b));
    // Dimension line endpoints on paper, offset from the geometry.
    let (da, db) = match direction {
        LinearDirection::Horizontal => {
            let y = pa[1].min(pb[1]) - offset_mm;
            ([pa[0], y], [pb[0], y])
        }
        LinearDirection::Vertical => {
            let x = pa[0].max(pb[0]) + offset_mm;
            ([x, pa[1]], [x, pb[1]])
        }
        LinearDirection::Aligned => {
            let along = unit(sub(pb, pa));
            let normal = [along[1], -along[0]];
            (
                add(pa, scale(normal, offset_mm)),
                add(pb, scale(normal, offset_mm)),
            )
        }
    };
    let along = unit(sub(db, da));
    let mid = scale(add(da, db), 0.5);
    let angle = along[1].atan2(along[0]).to_degrees();
    let angle = if angle > 90. || angle <= -90. {
        angle - 180. * angle.signum()
    } else {
        angle
    };
    AnnotationGraphic {
        lines: vec![[pa, da], [pb, db], [da, db]],
        arrows: vec![(da, scale(along, -1.)), (db, along)],
        arcs: vec![],
        text: wrap(number(value, precision), reference),
        text_mm: add(mid, [0., -1.]),
        text_angle_deg: angle,
        labels: vec![],
        value,
        unit: "mm".into(),
    }
}

/// A radius or diameter dimension of a circle given in model space.
#[allow(clippy::too_many_arguments)]
pub fn radial_dimension(
    view: &DrawingView,
    center_world: [f64; 3],
    rim_world: [f64; 3],
    radius_mm: f64,
    diameter: bool,
    offset_mm: f64,
    precision: u8,
    reference: bool,
) -> AnnotationGraphic {
    let c = view.paper_point(view_coordinates(view, center_world));
    let rim = view.paper_point(view_coordinates(view, rim_world));
    let out = unit(sub(rim, c));
    let knee = add(rim, scale(out, offset_mm.max(2.)));
    let side = if out[0] >= 0. { 1. } else { -1. };
    let shelf = add(knee, [6. * side, 0.]);
    let value = if diameter { 2. * radius_mm } else { radius_mm };
    let text = wrap(
        format!(
            "{}{}",
            if diameter { "Ø" } else { "R" },
            number(value, precision)
        ),
        reference,
    );
    let opposite = sub(c, sub(rim, c));
    let mut arrows = vec![(rim, out)];
    if diameter {
        arrows.push((opposite, scale(out, -1.)));
    }
    let width = text.chars().count() as f64 * super::CHAR_MM;
    AnnotationGraphic {
        lines: vec![[if diameter { opposite } else { c }, knee], [knee, shelf]],
        arrows,
        arcs: vec![],
        text_mm: add(shelf, [if side > 0. { 1. } else { -1. - width }, -1.]),
        text,
        text_angle_deg: 0.,
        labels: vec![],
        value,
        unit: "mm".into(),
    }
}

/// An angle dimension between two model lines. The value is the model angle
/// in degrees; the arc is drawn about the projected intersection.
#[allow(clippy::too_many_arguments)]
pub fn angle_dimension(
    view: &DrawingView,
    a: ([f64; 3], [f64; 3]),
    b: ([f64; 3], [f64; 3]),
    angle_deg: f64,
    offset_mm: f64,
    precision: u8,
    reference: bool,
) -> AnnotationGraphic {
    let p = |w| view.paper_point(view_coordinates(view, w));
    let (a0, a1, b0, b1) = (p(a.0), p(a.1), p(b.0), p(b.1));
    let (da, db) = (sub(a1, a0), sub(b1, b0));
    let cross = da[0] * db[1] - da[1] * db[0];
    // Intersection of the projected lines, or the midpoint between them.
    let center = if cross.abs() > 1e-9 {
        let t = ((b0[0] - a0[0]) * db[1] - (b0[1] - a0[1]) * db[0]) / cross;
        add(a0, scale(da, t))
    } else {
        scale(add(a0, b0), 0.5)
    };
    let radius = offset_mm.abs().max(4.);
    let far = |q0: [f64; 2], q1: [f64; 2]| {
        if length(sub(q1, center)) >= length(sub(q0, center)) {
            q1
        } else {
            q0
        }
    };
    let (ua, ub) = (
        unit(sub(far(a0, a1), center)),
        unit(sub(far(b0, b1), center)),
    );
    let (sa, sb) = (ua[1].atan2(ua[0]), ub[1].atan2(ub[0]));
    let mut end = sb;
    while end < sa {
        end += std::f64::consts::TAU;
    }
    let (start, end) = if end - sa > std::f64::consts::PI {
        (sb, sa + std::f64::consts::TAU)
    } else {
        (sa, end)
    };
    let mid = (start + end) / 2.;
    let point = |angle: f64| add(center, [radius * angle.cos(), radius * angle.sin()]);
    AnnotationGraphic {
        lines: vec![[center, point(start)], [center, point(end)]],
        arrows: vec![
            (point(start), [start.sin(), -start.cos()]),
            (point(end), [-end.sin(), end.cos()]),
        ],
        arcs: vec![(center, radius, start, end)],
        text: wrap(format!("{}°", number(angle_deg, precision)), reference),
        text_mm: add(
            center,
            [(radius + 2.) * mid.cos(), (radius + 2.) * mid.sin()],
        ),
        text_angle_deg: 0.,
        labels: vec![],
        value: angle_deg,
        unit: "deg".into(),
    }
}

/// A sheet dimension between two paper points, reading in page-scale units
/// (paper length divided by the page scale).
pub fn draft_dimension(
    page: &Page,
    a: [f64; 2],
    b: [f64; 2],
    offset_mm: f64,
    precision: u8,
) -> AnnotationGraphic {
    let s = page.settings.scale;
    let ratio = s.numerator as f64 / s.denominator as f64;
    let along = unit(sub(b, a));
    let normal = [along[1], -along[0]];
    let (da, db) = (
        add(a, scale(normal, offset_mm)),
        add(b, scale(normal, offset_mm)),
    );
    let value = length(sub(b, a)) / ratio;
    let angle = along[1].atan2(along[0]).to_degrees();
    let angle = if angle > 90. || angle <= -90. {
        angle - 180. * angle.signum()
    } else {
        angle
    };
    AnnotationGraphic {
        lines: vec![[a, da], [b, db], [da, db]],
        arrows: vec![(da, scale(along, -1.)), (db, along)],
        arcs: vec![],
        text: number(value, precision),
        text_mm: add(scale(add(da, db), 0.5), [0., -1.]),
        text_angle_deg: angle,
        labels: vec![],
        value,
        unit: "mm".into(),
    }
}

/// Paper graphics of a non-dimension annotation, from source alone.
pub fn sheet_graphic(annotation: &Annotation) -> AnnotationGraphic {
    let mut g = AnnotationGraphic::default();
    match &annotation.kind {
        AnnotationKind::Dimension { .. } | AnnotationKind::DraftDimension { .. } => {}
        AnnotationKind::Note { position_mm, text } => {
            g.text = text.clone();
            g.text_mm = *position_mm;
        }
        AnnotationKind::Leader { points_mm, text } => {
            g.lines = points_mm.windows(2).map(|w| [w[0], w[1]]).collect();
            g.arrows = vec![(points_mm[0], unit(sub(points_mm[0], points_mm[1])))];
            let last = *points_mm.last().expect("validated leader");
            g.text = text.clone();
            g.text_mm = add(last, [1., -1.]);
        }
        AnnotationKind::Weld {
            points_mm,
            symbol,
            side,
            size,
            length: weld_length,
            all_around,
            field,
            tail,
        } => {
            g.lines = points_mm.windows(2).map(|w| [w[0], w[1]]).collect();
            g.arrows = vec![(points_mm[0], unit(sub(points_mm[0], points_mm[1])))];
            let start = *points_mm.last().expect("validated weld leader");
            let end = add(start, [24., 0.]);
            g.lines.push([start, end]);
            // Symbol glyphs below (arrow side) or above (other side) the line.
            let mut glyph = |above: bool| {
                let s = if above { -1. } else { 1. };
                let o = add(start, [8., 0.]);
                match symbol {
                    WeldSymbol::Fillet => {
                        g.lines.push([o, add(o, [0., 4. * s])]);
                        g.lines.push([add(o, [0., 4. * s]), add(o, [4., 0.])]);
                    }
                    WeldSymbol::SquareGroove => {
                        g.lines.push([o, add(o, [0., 4. * s])]);
                        g.lines.push([add(o, [2., 0.]), add(o, [2., 4. * s])]);
                    }
                    WeldSymbol::VGroove => {
                        g.lines.push([add(o, [2., 0.]), add(o, [0., 4. * s])]);
                        g.lines.push([add(o, [2., 0.]), add(o, [4., 4. * s])]);
                    }
                    WeldSymbol::BevelGroove => {
                        g.lines.push([o, add(o, [0., 4. * s])]);
                        g.lines.push([o, add(o, [3., 4. * s])]);
                    }
                    WeldSymbol::Plug => {
                        g.lines.push([o, add(o, [0., 2.5 * s])]);
                        g.lines.push([add(o, [0., 2.5 * s]), add(o, [4., 2.5 * s])]);
                        g.lines.push([add(o, [4., 2.5 * s]), add(o, [4., 0.])]);
                    }
                    WeldSymbol::Spot => {
                        g.arcs
                            .push((add(o, [2., 2. * s]), 2., 0., std::f64::consts::TAU))
                    }
                    WeldSymbol::Seam => {
                        g.arcs
                            .push((add(o, [2., 0.]), 2., 0., std::f64::consts::TAU));
                        g.lines.push([add(o, [-1., -1.]), add(o, [5., -1.])]);
                        g.lines.push([add(o, [-1., 1.]), add(o, [5., 1.])]);
                    }
                }
            };
            match side {
                WeldSide::Arrow => glyph(false),
                WeldSide::Other => glyph(true),
                WeldSide::Both => {
                    glyph(false);
                    glyph(true);
                }
            }
            if *all_around {
                g.arcs.push((start, 1.5, 0., std::f64::consts::TAU));
            }
            if *field {
                g.lines.push([start, add(start, [0., -6.])]);
                g.lines.push([add(start, [0., -6.]), add(start, [3., -5.])]);
                g.lines.push([add(start, [3., -5.]), add(start, [0., -4.])]);
            }
            if !tail.is_empty() {
                g.lines.push([end, add(end, [3., -3.])]);
                g.lines.push([end, add(end, [3., 3.])]);
            }
            let mut parts = vec![];
            if !size.is_empty() {
                parts.push(size.clone());
            }
            if !weld_length.is_empty() {
                parts.push(weld_length.clone());
            }
            if !tail.is_empty() {
                parts.push(format!("· {tail}"));
            }
            g.text = parts.join(" ");
            g.text_mm = add(
                start,
                [
                    1.,
                    if matches!(side, WeldSide::Other) {
                        4.
                    } else {
                        -1.
                    },
                ],
            );
        }
        AnnotationKind::Table {
            position_mm,
            title,
            columns,
            rows,
            widths_mm,
            row_height_mm,
        } => {
            let [x0, y0, x1, y1] = table_bounds(annotation).expect("table");
            let lines = rows.len() + 1 + usize::from(!title.is_empty());
            for i in 0..=lines {
                let y = y0 + i as f64 * row_height_mm;
                g.lines.push([[x0, y], [x1, y]]);
            }
            let header = y0 + if title.is_empty() { 0. } else { *row_height_mm };
            let mut x = x0;
            g.lines.push([[x0, y0], [x0, y1]]);
            for w in widths_mm {
                x += w;
                g.lines.push([[x, header], [x, y1]]);
            }
            g.lines.push([[x1, y0], [x1, header]]);
            // Each cell's baseline centers its text vertically in its row.
            let baseline = |row: usize| {
                y0 + row as f64 * row_height_mm + (row_height_mm + ANNOTATION_TEXT_MM * 0.7) / 2.
            };
            let mut row = 0;
            if !title.is_empty() {
                g.labels.push(([x0 + 1., baseline(row)], title.clone()));
                row += 1;
            }
            for cells in std::iter::once(columns).chain(rows.iter()) {
                let mut x = position_mm[0];
                for (cell, w) in cells.iter().zip(widths_mm) {
                    if !cell.is_empty() {
                        g.labels.push(([x + 1., baseline(row)], cell.clone()));
                    }
                    x += w;
                }
                row += 1;
            }
        }
        AnnotationKind::Draft { geometry, .. } => match geometry {
            DraftGeometry::Line { a, b } => g.lines.push([*a, *b]),
            DraftGeometry::Circle { center, radius_mm } => {
                g.arcs
                    .push((*center, *radius_mm, 0., std::f64::consts::TAU))
            }
            DraftGeometry::Arc {
                center,
                radius_mm,
                start_deg,
                sweep_deg,
            } => {
                let start = start_deg.to_radians();
                g.arcs
                    .push((*center, *radius_mm, start, start + sweep_deg.to_radians()));
            }
            DraftGeometry::Hatch {
                boundary,
                angle_deg,
                spacing_mm,
            } => {
                for i in 0..boundary.len() {
                    g.lines
                        .push([boundary[i], boundary[(i + 1) % boundary.len()]]);
                }
                g.lines
                    .extend(hatch_lines(boundary, *angle_deg, *spacing_mm));
            }
        },
    }
    g
}

/// Parallel lines at `angle_deg` and `spacing_mm`, clipped to a polygon by
/// even-odd crossings.
pub fn hatch_lines(boundary: &[[f64; 2]], angle_deg: f64, spacing_mm: f64) -> Vec<[[f64; 2]; 2]> {
    let (s, c) = angle_deg.to_radians().sin_cos();
    let (along, across) = ([c, s], [-s, c]);
    let dot = |p: [f64; 2], d: [f64; 2]| p[0] * d[0] + p[1] * d[1];
    let offsets: Vec<f64> = boundary.iter().map(|p| dot(*p, across)).collect();
    let (lo, hi) = offsets
        .iter()
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), v| {
            (a.min(*v), b.max(*v))
        });
    let mut out = vec![];
    let count = ((hi - lo) / spacing_mm).floor().min(10_000.) as usize;
    for k in 1..=count {
        let d = lo + k as f64 * spacing_mm;
        let mut hits = vec![];
        for i in 0..boundary.len() {
            let (p, q) = (boundary[i], boundary[(i + 1) % boundary.len()]);
            let (dp, dq) = (dot(p, across), dot(q, across));
            if (dp <= d) != (dq <= d) {
                let t = (d - dp) / (dq - dp);
                let x = add(p, scale(sub(q, p), t));
                hits.push(dot(x, along));
            }
        }
        hits.sort_by(f64::total_cmp);
        for pair in hits.as_chunks::<2>().0 {
            let point = |t: f64| add(scale(along, t), scale(across, d));
            out.push([point(pair[0]), point(pair[1])]);
        }
    }
    out
}

/// Approximate paper bounds of a graphic, including its text.
pub fn graphic_bounds(g: &AnnotationGraphic) -> Option<[f64; 4]> {
    let mut b = [
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    ];
    let mut grow = |p: [f64; 2]| {
        b[0] = b[0].min(p[0]);
        b[1] = b[1].min(p[1]);
        b[2] = b[2].max(p[0]);
        b[3] = b[3].max(p[1]);
    };
    for line in &g.lines {
        line.iter().for_each(|p| grow(*p));
    }
    for (center, r, _, _) in &g.arcs {
        grow([center[0] - r, center[1] - r]);
        grow([center[0] + r, center[1] + r]);
    }
    if !g.text.is_empty() {
        let lines = g.text.lines().count().max(1) as f64;
        let chars = g.text.lines().map(|l| l.chars().count()).max().unwrap_or(0) as f64;
        grow(g.text_mm);
        grow(add(
            g.text_mm,
            [chars * super::CHAR_MM, -lines * super::LINE_MM],
        ));
        grow(add(
            g.text_mm,
            [chars * super::CHAR_MM, lines * super::LINE_MM],
        ));
    }
    b[0].is_finite().then_some(b)
}

fn overlap(a: [f64; 4], b: [f64; 4]) -> bool {
    a[0] < b[2] && b[0] < a[2] && a[1] < b[3] && b[1] < a[3]
}
/// Layout problems of a page: anything outside the printable area, anything
/// over the title block, and sheet annotations over views or each other.
/// Model dimensions may sit next to their view and are checked against the
/// title block and printable area only.
pub fn page_conflicts(
    page: &Page,
    index: usize,
    total: usize,
    evaluation: &super::DrawingEvaluation,
) -> Result<Vec<String>, DocumentError> {
    let layout = super::layout_page(page, index, total)?;
    let printable = layout.printable_mm;
    let title = layout
        .title_cells
        .iter()
        .fold(None::<[f64; 4]>, |acc, cell| {
            let r = cell.rect_mm;
            Some(acc.map_or(r, |a| {
                [
                    a[0].min(r[0]),
                    a[1].min(r[1]),
                    a[2].max(r[2]),
                    a[3].max(r[3]),
                ]
            }))
        });
    let mut problems = vec![];
    let mut views = vec![];
    for view in &page.views {
        if let Some(super::ViewEvaluation::Ready { vectors, .. }) = evaluation.views.get(&view.id) {
            let mut b = [
                f64::INFINITY,
                f64::INFINITY,
                f64::NEG_INFINITY,
                f64::NEG_INFINITY,
            ];
            for p in vectors.iter().flat_map(|v| &v.preview_mm) {
                let q = view.paper_point(*p);
                b = [
                    b[0].min(q[0]),
                    b[1].min(q[1]),
                    b[2].max(q[0]),
                    b[3].max(q[1]),
                ];
            }
            if b[0].is_finite() {
                views.push((view.name.clone(), b));
            }
        }
    }
    let mut placed: Vec<(String, [f64; 4], bool)> = vec![];
    for annotation in &page.annotations {
        let (graphic, dimension) = match &annotation.kind {
            AnnotationKind::Dimension { .. } => match evaluation.annotations.get(&annotation.id) {
                Some(AnnotationEvaluation::Ready { graphic }) => (graphic.clone(), true),
                _ => continue,
            },
            AnnotationKind::DraftDimension {
                a,
                b,
                offset_mm,
                precision,
            } => (draft_dimension(page, *a, *b, *offset_mm, *precision), false),
            _ => (sheet_graphic(annotation), false),
        };
        let bounds = match table_bounds(annotation) {
            Some(b) => Some(b),
            None => graphic_bounds(&graphic),
        };
        if let Some(b) = bounds {
            placed.push((format!("annotation {}", annotation.id), b, dimension));
        }
    }
    let inside = |b: [f64; 4]| {
        b[0] >= printable[0] && b[1] >= printable[1] && b[2] <= printable[2] && b[3] <= printable[3]
    };
    for (name, b) in &views {
        if !inside(*b) {
            problems.push(format!("View {name} extends outside the printable area"));
        }
        if title.is_some_and(|t| overlap(*b, t)) {
            problems.push(format!("View {name} overlaps the title block"));
        }
    }
    for (i, (name, b, dimension)) in placed.iter().enumerate() {
        if !inside(*b) {
            problems.push(format!("The {name} extends outside the printable area"));
        }
        if title.is_some_and(|t| overlap(*b, t)) {
            problems.push(format!("The {name} overlaps the title block"));
        }
        if !dimension {
            for (view, v) in &views {
                if overlap(*b, *v) {
                    problems.push(format!("The {name} overlaps view {view}"));
                }
            }
            for (other, o, other_dimension) in &placed[i + 1..] {
                if !other_dimension && overlap(*b, *o) {
                    problems.push(format!("The {name} overlaps the {other}"));
                }
            }
        }
    }
    Ok(problems)
}
