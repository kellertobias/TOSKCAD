//! Authored view intent. Exact evaluated vectors belong to a revision-stamped
//! result, never to the canonical page or a viewport screenshot.
use super::{Scale, source::invalid};
use crate::{
    document::{DocumentError, EntityId, Link},
    references::ReferenceFrame,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub const VIEW_CAPABILITY: &str = "drawings.views.1";
/// Finite-depth or section-only ranges and lateral section windows.
pub const SECTION_CAPABILITY: &str = "drawings.sections.1";
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ViewSource {
    Model {
        entities: Vec<EntityId>,
    },
    FlatPattern {
        feature: EntityId,
    },
    Exploded {
        assembly: EntityId,
        feature: EntityId,
    },
}
impl ViewSource {
    pub fn entities(&self) -> Vec<EntityId> {
        match self {
            Self::Model { entities } => entities.clone(),
            Self::FlatPattern { feature } => vec![*feature],
            Self::Exploded { assembly, feature } => vec![*assembly, *feature],
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ViewKind {
    Projection,
    /// Remove material on the positive-normal side of this plane, offset from
    /// the view frame along its normal. Only actual cut faces receive hatching.
    Section {
        offset_mm: f64,
        /// How far behind the cut plane geometry is shown.
        #[serde(default, skip_serializing_if = "SectionRange::is_unlimited")]
        range: SectionRange,
        /// Lateral limits on the cut plane; geometry outside is not shown.
        #[serde(default, skip_serializing_if = "SectionWindow::is_unbounded")]
        window: SectionWindow,
    },
    Detail {
        center_mm: [f64; 2],
        radius_mm: f64,
    },
}
/// The viewing interval behind a section plane, measured against the view
/// direction from the cut plane.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum SectionRange {
    /// Every retained solid behind the plane, without a far limit.
    #[default]
    Unlimited,
    /// Uncut geometry only within this depth behind the plane. Solids crossing
    /// the far limit are clipped there; farther solids are excluded.
    Depth { depth_mm: f64 },
    /// Only the exact zero-thickness plane intersection and its hatching.
    SectionOnly,
}
impl SectionRange {
    pub fn is_unlimited(&self) -> bool {
        *self == Self::Unlimited
    }
}
/// A finite section line or region: limits in the view frame's u (right) and
/// v (up) coordinates, in model millimeters from the frame origin.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SectionWindow {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub u_mm: Option<[f64; 2]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub v_mm: Option<[f64; 2]>,
}
impl SectionWindow {
    pub fn is_unbounded(&self) -> bool {
        self.u_mm.is_none() && self.v_mm.is_none()
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DrawingView {
    pub id: EntityId,
    pub name: String,
    pub source: ViewSource,
    /// u points right and v points up on paper; view is from positive normal.
    pub frame: ReferenceFrame,
    pub kind: ViewKind,
    pub position_mm: [f64; 2],
    pub scale: Scale,
    pub hidden_lines: bool,
    pub tangent_edges: bool,
    /// Vector approximation bound for nonanalytic rendering/export consumers.
    /// Exact supported curve records remain available separately.
    pub paper_chord_tolerance_mm: f64,
    /// The drawing guide that generated this view. Its frame, kind and
    /// sources then follow the guide and its levels.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guide: Option<super::GuideLink>,
    /// None includes all equipment; an explicit empty set hides equipment only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub equipment_categories: Option<BTreeSet<crate::equipment::Category>>,
}
impl DrawingView {
    pub fn validate(&self) -> Result<(), DocumentError> {
        self.frame.validate().map_err(invalid)?;
        let sources = self.source.entities();
        if sources.is_empty() {
            return Err(invalid(
                "Choose at least one Part or component for this drawing view",
            ));
        }
        if self.id.is_nil()
            || self.name.trim().is_empty()
            || self.name.len() > 256
            || sources.len() > 4096
            || sources.iter().any(|id| id.is_nil())
            || sources.iter().copied().collect::<BTreeSet<_>>().len() != sources.len()
            || self.position_mm.iter().any(|v| !v.is_finite())
            || self.scale.numerator == 0
            || self.scale.denominator == 0
            || !self.paper_chord_tolerance_mm.is_finite()
            || !(0.001..=1.).contains(&self.paper_chord_tolerance_mm)
        {
            return Err(invalid(
                "Drawing view requires a name, unique sources, finite placement and positive scale/tolerance",
            ));
        }
        match self.kind {
            ViewKind::Section { offset_mm, .. }
                if !offset_mm.is_finite() || offset_mm.abs() > 1e7 =>
            {
                Err(invalid("Section offset is outside model bounds"))
            }
            ViewKind::Section {
                range: SectionRange::Depth { depth_mm },
                ..
            } if !depth_mm.is_finite() || !(1e-5..=1e7).contains(&depth_mm) => Err(invalid(
                "Section depth must be positive and within model bounds",
            )),
            ViewKind::Section { window, .. }
                if [window.u_mm, window.v_mm]
                    .into_iter()
                    .flatten()
                    .any(|[a, b]| {
                        !a.is_finite()
                            || !b.is_finite()
                            || a.abs() > 1e7
                            || b.abs() > 1e7
                            || b - a < 1e-5
                    }) =>
            {
                Err(invalid(
                    "Section window limits must be finite with the first below the second",
                ))
            }
            ViewKind::Detail {
                center_mm,
                radius_mm,
            } if center_mm.iter().any(|v| !v.is_finite() || v.abs() > 1e7)
                || !radius_mm.is_finite()
                || !(1e-5..=1e7).contains(&radius_mm) =>
            {
                Err(invalid(
                    "Detail region needs a finite center and positive radius",
                ))
            }
            _ => Ok(()),
        }
    }
    /// The inverse of `paper_point`: view model coordinates under a paper point.
    pub fn model_point(&self, paper_mm: [f64; 2]) -> [f64; 2] {
        let center = match self.kind {
            ViewKind::Detail { center_mm, .. } => center_mm,
            _ => [0.; 2],
        };
        let scale = self.scale.numerator as f64 / self.scale.denominator as f64;
        [
            center[0] + (paper_mm[0] - self.position_mm[0]) / scale,
            center[1] - (paper_mm[1] - self.position_mm[1]) / scale,
        ]
    }
    pub fn paper_point(&self, point_mm: [f64; 2]) -> [f64; 2] {
        let center = match self.kind {
            ViewKind::Detail { center_mm, .. } => center_mm,
            _ => [0.; 2],
        };
        let scale = self.scale.numerator as f64 / self.scale.denominator as f64;
        [
            self.position_mm[0] + (point_mm[0] - center[0]) * scale,
            self.position_mm[1] - (point_mm[1] - center[1]) * scale,
        ]
    }
}
pub fn view_inputs(book: &super::DrawingBook) -> Vec<Link> {
    book.pages
        .iter()
        .flat_map(|p| &p.views)
        .flat_map(|v| v.source.entities())
        .chain(
            book.pages
                .iter()
                .flat_map(|p| &p.views)
                .filter_map(|v| v.guide)
                .flat_map(|l| std::iter::once(l.guide).chain(l.level)),
        )
        .chain(
            book.pages
                .iter()
                .flat_map(|p| &p.annotations)
                .flat_map(|a| match &a.kind {
                    super::AnnotationKind::Dimension { measure, .. } => {
                        measure.references().iter().map(|r| r.source).collect()
                    }
                    _ => vec![],
                }),
        )
        .collect::<BTreeSet<_>>()
        .into_iter()
        .map(|id| Link::new(id, "drawing.view.source"))
        .collect()
}
pub fn has_annotations(book: &super::DrawingBook) -> bool {
    book.pages.iter().any(|p| !p.annotations.is_empty())
}
pub fn has_views(book: &super::DrawingBook) -> bool {
    book.pages.iter().any(|p| !p.views.is_empty())
}
/// Whether any view uses a section range or window that older readers lack.
pub fn has_section_limits(book: &super::DrawingBook) -> bool {
    book.pages.iter().flat_map(|p| &p.views).any(|v| {
        matches!(v.kind, ViewKind::Section { range, window, .. } if !range.is_unlimited() || !window.is_unbounded())
    })
}

/// Exact supported 2D output primitives. B-spline knots include multiplicities
/// and the finite trim domain; downstream writers must not silently drop them.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum VectorCurve {
    /// Filled section area triangulated to the projection tolerance.
    HatchTriangle {
        points: [[f64; 2]; 3],
    },
    Line {
        a: [f64; 2],
        b: [f64; 2],
    },
    Ellipse {
        center: [f64; 2],
        u: [f64; 2],
        v: [f64; 2],
        major_mm: f64,
        minor_mm: f64,
        start_rad: f64,
        end_rad: f64,
    },
    BSpline {
        degree: u32,
        periodic: bool,
        poles: Vec<[f64; 2]>,
        weights: Vec<f64>,
        knots: Vec<f64>,
        multiplicities: Vec<u32>,
        first: f64,
        last: f64,
    },
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VectorRole {
    Visible,
    Hidden,
    Section,
    Hatch,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DrawingVector {
    pub source: EntityId,
    pub material_part: EntityId,
    pub role: VectorRole,
    pub curve: VectorCurve,
    /// Bounded native curve sampling for preview only. Exact curve above is
    /// retained for dimensional/analytic export consumers.
    pub preview_mm: Vec<[f64; 2]>,
    pub line_width_mm: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DrawingStamp {
    pub document: EntityId,
    pub revision: u64,
    pub source_hash: crate::project_file::ContentHash,
}
impl DrawingStamp {
    pub fn check(&self, document: &crate::document::Document, revision: u64) -> Result<(), String> {
        if self.document != document.id()
            || self.revision != revision
            || self.source_hash != crate::project_file::ContentHash::document(document)?
        {
            Err("Drawing result is stale; regenerate from current source before export".into())
        } else {
            Ok(())
        }
    }
}
/// Font height for model-derived labels, in paper millimeters.
pub const SOURCE_LABEL_MM: f64 = 3.5;
/// Model-derived label at a model-space view coordinate.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceLabel {
    pub source: EntityId,
    pub at_mm: [f64; 2],
    pub text: String,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum ViewEvaluation {
    Ready {
        vectors: Vec<DrawingVector>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        labels: Vec<SourceLabel>,
        warnings: Vec<String>,
    },
    Unresolved {
        message: String,
    },
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DrawingEvaluation {
    pub stamp: DrawingStamp,
    pub views: std::collections::BTreeMap<EntityId, ViewEvaluation>,
    /// Evaluated model dimensions, by annotation identity.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub annotations: std::collections::BTreeMap<EntityId, super::AnnotationEvaluation>,
}
impl DrawingEvaluation {
    pub fn checked_page<'a>(
        &'a self,
        document: &crate::document::Document,
        revision: u64,
        book: EntityId,
        page: EntityId,
    ) -> Result<Vec<(&'a ViewEvaluation, super::DrawingView)>, String> {
        self.stamp.check(document, revision)?;
        let book = super::read(document, book).map_err(|e| e.to_string())?;
        let page = book
            .pages
            .iter()
            .find(|p| p.id == page)
            .ok_or("Drawing page is missing")?;
        for annotation in &page.annotations {
            if let super::AnnotationKind::Dimension { .. } = annotation.kind {
                match self.annotations.get(&annotation.id) {
                    Some(super::AnnotationEvaluation::Ready { .. }) => {}
                    Some(super::AnnotationEvaluation::Detached { message }) => {
                        return Err(format!("Detached dimension: {message}"));
                    }
                    None => return Err("Drawing dimension has not been evaluated".into()),
                }
            }
        }
        page.views
            .iter()
            .map(|view| {
                let result = self
                    .views
                    .get(&view.id)
                    .ok_or("Drawing view has not been evaluated")?;
                match result {
                    ViewEvaluation::Unresolved { message } => Err(message.clone()),
                    _ => Ok((result, view.clone())),
                }
            })
            .collect()
    }
}

/// Where a section view's cut plane, and its far limit, appear in another view
/// of the same page. Segments are in the parent view's model coordinates.
#[derive(Clone, Debug, PartialEq)]
pub struct SectionGuide {
    pub section: EntityId,
    pub cut: [[f64; 2]; 2],
    pub far: Option<[[f64; 2]; 2]>,
}
fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn minus(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
/// The guide of `section` in `parent`, when the parent sees the cut plane
/// edge-on. `extent` is the parent's drawn bounds (min, max) in its model
/// coordinates; unlimited guide ends extend to it. The guide is trimmed to
/// the section's lateral window along its u axis.
pub fn section_guide(
    parent: &DrawingView,
    section: &DrawingView,
    extent: ([f64; 2], [f64; 2]),
) -> Option<SectionGuide> {
    let ViewKind::Section {
        offset_mm,
        range,
        window,
    } = section.kind
    else {
        return None;
    };
    let (pf, sf) = (&parent.frame, &section.frame);
    let (np, ns) = (cross(pf.u, pf.v), cross(sf.u, sf.v));
    if dot(np, ns).abs() > 1e-6 {
        return None;
    }
    let (ka, kb) = (dot(ns, pf.u), dot(ns, pf.v));
    let direction = [-kb, ka];
    // Parameter t runs along the guide; trim it to the parent's bounds...
    let corners = [
        extent.0,
        [extent.1[0], extent.0[1]],
        extent.1,
        [extent.0[0], extent.1[1]],
    ];
    let line = |depth: f64| -> Option<[[f64; 2]; 2]> {
        let rhs = depth + dot(ns, minus(sf.origin_mm, pf.origin_mm));
        let base = [rhs * ka, rhs * kb];
        let along = |p: [f64; 2]| (p[0] - base[0]) * direction[0] + (p[1] - base[1]) * direction[1];
        let mut lo = corners
            .iter()
            .map(|p| along(*p))
            .fold(f64::INFINITY, f64::min)
            - 5.;
        let mut hi = corners
            .iter()
            .map(|p| along(*p))
            .fold(f64::NEG_INFINITY, f64::max)
            + 5.;
        // ...and to the section's lateral window along its own u axis.
        if let Some([u0, u1]) = window.u_mm {
            let start = dot(sf.u, minus(pf.origin_mm, sf.origin_mm))
                + base[0] * dot(sf.u, pf.u)
                + base[1] * dot(sf.u, pf.v);
            let rate = direction[0] * dot(sf.u, pf.u) + direction[1] * dot(sf.u, pf.v);
            if rate.abs() > 1e-9 {
                let (a, b) = ((u0 - start) / rate, (u1 - start) / rate);
                lo = lo.max(a.min(b));
                hi = hi.min(a.max(b));
            }
        }
        (hi > lo)
            .then(|| [lo, hi].map(|t| [base[0] + t * direction[0], base[1] + t * direction[1]]))
    };
    Some(SectionGuide {
        section: section.id,
        cut: line(offset_mm)?,
        far: match range {
            SectionRange::Depth { depth_mm } => line(offset_mm - depth_mm),
            _ => None,
        },
    })
}

/// A section through a finite line picked in `parent` from `a` to `b` (parent
/// model coordinates). The cut plane contains the line and the parent's view
/// direction; the section looks toward the left of a→b as drawn, keeping the
/// material on that side, with the parent's normal up. Its window spans the
/// picked segment. Returns the frame, with the cut at offset 0, and window.
pub fn section_from_line(
    parent: &DrawingView,
    a: [f64; 2],
    b: [f64; 2],
) -> Option<(ReferenceFrame, SectionWindow)> {
    let f = &parent.frame;
    let d = [0, 1, 2].map(|i| (b[0] - a[0]) * f.u[i] + (b[1] - a[1]) * f.v[i]);
    let length = dot(d, d).sqrt();
    if !length.is_finite() || length < 1e-6 {
        return None;
    }
    let frame = ReferenceFrame {
        origin_mm: [0, 1, 2].map(|i| f.origin_mm[i] + a[0] * f.u[i] + a[1] * f.v[i]),
        u: d.map(|x| x / length),
        v: cross(f.u, f.v),
    };
    frame.validate().ok()?;
    Some((
        frame,
        SectionWindow {
            u_mm: Some([0., length]),
            v_mm: None,
        },
    ))
}
