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
    },
    Detail {
        center_mm: [f64; 2],
        radius_mm: f64,
    },
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
            ViewKind::Section { offset_mm } if !offset_mm.is_finite() || offset_mm.abs() > 1e7 => {
                Err(invalid("Section offset is outside model bounds"))
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
        .collect::<BTreeSet<_>>()
        .into_iter()
        .map(|id| Link::new(id, "drawing.view.source"))
        .collect()
}
pub fn has_views(book: &super::DrawingBook) -> bool {
    book.pages.iter().any(|p| !p.views.is_empty())
}

/// Exact supported 2D output primitives. B-spline knots include multiplicities
/// and the finite trim domain; downstream writers must not silently drop them.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum VectorCurve {
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
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum ViewEvaluation {
    Ready {
        vectors: Vec<DrawingVector>,
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
