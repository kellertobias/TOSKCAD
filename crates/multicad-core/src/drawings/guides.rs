//! Drawing guides: floor plan, section and elevation intent authored in the
//! model. A guide resolves to drawing-view frames and section ranges from its
//! own values and the current levels, and generated views keep a link to it,
//! so a level or guide edit changes the linked views. Generation updates the
//! views it made earlier instead of duplicating them.
use super::{
    DrawingBook, DrawingView, Orientation, Page, PaperSize, Scale, SectionRange, SectionWindow,
    ViewKind, ViewSource, read, replace_edits, source::invalid,
};
use crate::{
    commands::{Actor, CommandError, Prepared, Request, Session},
    constraint_sketch::Dimension,
    document::{Definition, Document, DocumentError, Entity, EntityData, EntityId, Link, Mutation},
    model::{read_level, resolve_level},
    parameters::{EvaluatedParameters, Unit},
    references::ReferenceFrame,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub const GUIDE_KIND: &str = "drawings.guide";
pub const GUIDE_CAPABILITY: &str = "drawings.guide.1";
/// Views that carry a link to the guide that generated them.
pub const GUIDED_VIEW_CAPABILITY: &str = "drawings.guided_views.1";

/// The view a guide produces. Few guides exist per document, so the size
/// difference between variants is not worth boxing authored values.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum GuideKind {
    /// A horizontal cut on every listed level, looking down.
    Plan {
        /// Cut plane height above each level's floor.
        cut_height: Dimension,
        /// How far below the cut plane uncut geometry is shown.
        view_depth: Dimension,
    },
    /// A vertical cut along a plan line from `start` to `end`. The view looks
    /// across the line toward its left side; material on its right side is
    /// removed. The cut is limited to the line's length.
    Section {
        start: [Dimension; 2],
        end: [Dimension; 2],
        /// Depth shown behind the cut plane; None shows everything.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        depth: Option<Dimension>,
    },
    /// An uncut exterior view looking in a plan direction, in degrees from
    /// +X (0 looks toward +X, 90 toward +Y).
    Elevation { direction_deg: f64 },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GuideDefinition {
    pub kind: GuideKind,
    /// Plans repeat on each level. Sections are limited vertically to the
    /// levels' extent when any are listed.
    #[serde(default)]
    pub levels: Vec<EntityId>,
    /// Parts, Instances or assemblies shown in the generated views.
    pub sources: Vec<EntityId>,
    pub scale: Scale,
}

/// The link from a generated view back to its guide.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GuideLink {
    pub guide: EntityId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub level: Option<EntityId>,
}

fn dimensions(kind: &GuideKind) -> Vec<&Dimension> {
    match kind {
        GuideKind::Plan {
            cut_height,
            view_depth,
        } => vec![cut_height, view_depth],
        GuideKind::Section { start, end, depth } => {
            let mut d: Vec<_> = start.iter().chain(end).collect();
            d.extend(depth);
            d
        }
        GuideKind::Elevation { .. } => vec![],
    }
}
impl GuideDefinition {
    pub fn validate(&self) -> Result<(), DocumentError> {
        if self.sources.is_empty() || self.sources.len() > 4096 {
            return Err(invalid("A drawing guide shows 1–4096 sources"));
        }
        let unique = |ids: &[EntityId]| {
            ids.iter().all(|id| !id.is_nil())
                && ids.iter().collect::<BTreeSet<_>>().len() == ids.len()
        };
        if !unique(&self.sources) || !unique(&self.levels) || self.levels.len() > 256 {
            return Err(invalid("Guide sources and levels must be distinct"));
        }
        if self.scale.numerator == 0 || self.scale.denominator == 0 {
            return Err(invalid("Guide scale must be positive"));
        }
        match &self.kind {
            GuideKind::Plan { .. } if self.levels.is_empty() => {
                return Err(invalid("A floor plan guide needs at least one level"));
            }
            GuideKind::Elevation { direction_deg } if !direction_deg.is_finite() => {
                return Err(invalid("Elevation direction must be finite"));
            }
            _ => {}
        }
        for d in dimensions(&self.kind) {
            crate::model::check_dimension(d, Unit::Millimeter)?;
        }
        Ok(())
    }
    pub fn inputs(&self) -> Vec<Link> {
        let parameters: BTreeSet<_> = dimensions(&self.kind)
            .into_iter()
            .filter_map(Dimension::expression)
            .flat_map(|e| e.bindings.iter().map(|b| b.target))
            .collect();
        parameters
            .into_iter()
            .map(|id| Link::new(id, format!("parameter/{id}")))
            .chain(self.levels.iter().map(|l| Link::new(*l, "level")))
            .chain(self.sources.iter().map(|s| Link::new(*s, "source")))
            .collect()
    }
    /// The level of each view this guide generates: every level for a plan,
    /// one view without a level otherwise.
    pub fn targets(&self) -> Vec<Option<EntityId>> {
        match self.kind {
            GuideKind::Plan { .. } => self.levels.iter().copied().map(Some).collect(),
            _ => vec![None],
        }
    }
}

pub fn read_guide(document: &Document, id: EntityId) -> Result<GuideDefinition, DocumentError> {
    match document.get(id).map(|e| &e.data) {
        Some(EntityData::Extension { definition, .. }) if definition.type_id == GUIDE_KIND => {
            if definition.schema_version != 1 {
                return Err(invalid("Drawing guide schema is unavailable"));
            }
            let guide: GuideDefinition =
                serde_json::from_value(definition.data.clone()).map_err(invalid)?;
            guide.validate()?;
            Ok(guide)
        }
        Some(_) => Err(invalid(format!("{id} is not a drawing guide"))),
        None => Err(DocumentError::Missing(id)),
    }
}
pub fn guides(document: &Document) -> impl Iterator<Item = &Entity> {
    document.entities().filter(|e| {
        matches!(&e.data, EntityData::Extension { definition, .. } if definition.type_id == GUIDE_KIND)
    })
}

fn mm(d: &Dimension, parameters: &EvaluatedParameters) -> Result<f64, DocumentError> {
    d.value_in(Unit::Millimeter, Some(parameters))
        .map_err(|e| invalid(e.to_string()))
}

/// The resolved frame and kind of one generated view.
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedGuideView {
    pub frame: ReferenceFrame,
    pub kind: ViewKind,
}
pub fn resolve_guide_view(
    document: &Document,
    parameters: &EvaluatedParameters,
    guide: &GuideDefinition,
    level: Option<EntityId>,
) -> Result<ResolvedGuideView, DocumentError> {
    match &guide.kind {
        GuideKind::Plan {
            cut_height,
            view_depth,
        } => {
            let level = level
                .filter(|l| guide.levels.contains(l))
                .ok_or_else(|| invalid("The level is no longer part of this floor plan guide"))?;
            let resolved = resolve_level(document, parameters, level)?;
            let depth = mm(view_depth, parameters)?;
            if depth <= 1e-3 {
                return Err(invalid("Plan view depth must be positive"));
            }
            Ok(ResolvedGuideView {
                frame: ReferenceFrame {
                    origin_mm: [0., 0., resolved.elevation_mm + mm(cut_height, parameters)?],
                    u: [1., 0., 0.],
                    v: [0., 1., 0.],
                },
                kind: ViewKind::Section {
                    offset_mm: 0.,
                    range: SectionRange::Depth { depth_mm: depth },
                    window: SectionWindow::default(),
                },
            })
        }
        GuideKind::Section { start, end, depth } => {
            let a = [mm(&start[0], parameters)?, mm(&start[1], parameters)?];
            let b = [mm(&end[0], parameters)?, mm(&end[1], parameters)?];
            let length = (b[0] - a[0]).hypot(b[1] - a[1]);
            if length <= 1e-3 {
                return Err(invalid("A section line needs two distinct points"));
            }
            let u = [(b[0] - a[0]) / length, (b[1] - a[1]) / length, 0.];
            let depth = depth.as_ref().map(|d| mm(d, parameters)).transpose()?;
            if depth.is_some_and(|d| d <= 1e-3) {
                return Err(invalid("Section depth must be positive"));
            }
            let mut v_mm = None;
            for level in &guide.levels {
                let r = resolve_level(document, parameters, *level)?;
                let [low, high] = v_mm.unwrap_or([f64::INFINITY, f64::NEG_INFINITY]);
                v_mm = Some([
                    low.min(r.elevation_mm - r.slab_mm),
                    high.max(r.elevation_mm + r.height_mm),
                ]);
            }
            Ok(ResolvedGuideView {
                frame: ReferenceFrame {
                    origin_mm: [a[0], a[1], 0.],
                    u,
                    v: [0., 0., 1.],
                },
                kind: ViewKind::Section {
                    offset_mm: 0.,
                    range: depth.map_or(SectionRange::Unlimited, |depth_mm| SectionRange::Depth {
                        depth_mm,
                    }),
                    window: SectionWindow {
                        u_mm: Some([0., length]),
                        v_mm,
                    },
                },
            })
        }
        GuideKind::Elevation { direction_deg } => {
            let (s, c) = direction_deg.to_radians().sin_cos();
            // The view is seen from the positive normal, which points back
            // toward the viewer; u = Z × normal keeps v pointing up.
            Ok(ResolvedGuideView {
                frame: ReferenceFrame {
                    origin_mm: [0.; 3],
                    u: [s, -c, 0.],
                    v: [0., 0., 1.],
                },
                kind: ViewKind::Projection,
            })
        }
    }
}

/// A guided view with its frame, kind and sources taken from the current
/// guide and levels. Views without a guide are returned unchanged.
pub fn resolve_guided(
    document: &Document,
    parameters: &EvaluatedParameters,
    view: &DrawingView,
) -> Result<DrawingView, DocumentError> {
    let Some(link) = view.guide else {
        return Ok(view.clone());
    };
    let guide = read_guide(document, link.guide).map_err(|e| match e {
        DocumentError::Missing(_) => invalid("The drawing guide for this view was deleted"),
        e => e,
    })?;
    if let Some(level) = link.level {
        read_level(document, level).map_err(|e| match e {
            DocumentError::Missing(_) => invalid("The level for this plan view was deleted"),
            e => e,
        })?;
    }
    let resolved = resolve_guide_view(document, parameters, &guide, link.level)?;
    let mut out = view.clone();
    out.frame = resolved.frame;
    out.kind = resolved.kind;
    out.source = ViewSource::Model {
        entities: guide.sources.clone(),
    };
    Ok(out)
}

/// What a generation run changed. Stale views link to this guide but no
/// longer match one of its targets; they are reported, never deleted.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GenerationReport {
    pub book: EntityId,
    pub created: Vec<EntityId>,
    pub updated: Vec<EntityId>,
    pub stale: Vec<EntityId>,
}

fn view_name(document: &Document, guide_name: &str, level: Option<EntityId>) -> String {
    let name = match level.and_then(|l| document.get(l)) {
        Some(level) => format!("{guide_name} – {}", level.name),
        None => guide_name.to_owned(),
    };
    name.chars().take(256).collect()
}

/// Book edits that create or update every view of one guide. `book` names
/// the drawing book; None reuses a book that already holds this guide's
/// views, or creates a new one.
pub fn generation_edits(
    document: &Document,
    parameters: &EvaluatedParameters,
    guide_id: EntityId,
    guide_name: &str,
    guide: &GuideDefinition,
    book: Option<EntityId>,
) -> Result<(Vec<Mutation>, GenerationReport), DocumentError> {
    let linked = |b: &DrawingBook| {
        b.pages
            .iter()
            .flat_map(|p| &p.views)
            .any(|v| v.guide.is_some_and(|l| l.guide == guide_id))
    };
    let existing = match book {
        Some(id) => Some((id, read(document, id)?)),
        None => super::books(document)
            .filter_map(|id| read(document, id).ok().map(|b| (id, b)))
            .find(|(_, b)| linked(b)),
    };
    let new_book = existing.is_none();
    let (book_id, mut book, mut entity) = match existing {
        Some((id, b)) => (id, b, None),
        None => {
            let b = DrawingBook { pages: vec![] };
            let entity = b.entity(format!("{guide_name} drawings"))?;
            (entity.id, b, Some(entity))
        }
    };
    let mut report = GenerationReport {
        book: book_id,
        ..Default::default()
    };
    let targets = guide.targets();
    for level in &targets {
        let resolved = resolve_guide_view(document, parameters, guide, *level)?;
        let name = view_name(document, guide_name, *level);
        let link = GuideLink {
            guide: guide_id,
            level: *level,
        };
        if let Some(view) = book
            .pages
            .iter_mut()
            .flat_map(|p| &mut p.views)
            .find(|v| v.guide == Some(link))
        {
            view.name = name;
            view.frame = resolved.frame;
            view.kind = resolved.kind;
            view.source = ViewSource::Model {
                entities: guide.sources.clone(),
            };
            report.updated.push(view.id);
            continue;
        }
        let mut page = Page::new(super::next_number(&book));
        page.settings.paper = PaperSize::A3;
        page.settings.orientation = Orientation::Landscape;
        page.settings.scale = guide.scale;
        page.info.title = name.clone();
        let [w, h] = page.settings.dimensions();
        let view = DrawingView {
            id: EntityId::new(),
            name,
            source: ViewSource::Model {
                entities: guide.sources.clone(),
            },
            frame: resolved.frame,
            kind: resolved.kind,
            position_mm: [w / 2., h / 2.],
            scale: guide.scale,
            hidden_lines: false,
            tangent_edges: false,
            paper_chord_tolerance_mm: 0.05,
            guide: Some(link),
        };
        report.created.push(view.id);
        page.views.push(view);
        book.pages.push(page);
    }
    report.stale = book
        .pages
        .iter()
        .flat_map(|p| &p.views)
        .filter(|v| {
            v.guide
                .is_some_and(|l| l.guide == guide_id && !targets.contains(&l.level))
        })
        .map(|v| v.id)
        .collect();
    let edits = if new_book {
        let mut entity = entity.take().expect("new book entity");
        let EntityData::Drawing { definition, inputs } = &mut entity.data else {
            unreachable!()
        };
        definition.schema_version = 2;
        *inputs = super::view_inputs(&book);
        definition.data = serde_json::to_value(&book).map_err(invalid)?;
        vec![Mutation::Insert(entity)]
    } else {
        replace_edits(document, book_id, &book)?
    };
    Ok((edits, report))
}

fn command_error(e: DocumentError) -> CommandError {
    CommandError::from(e)
}
/// The result of a guide transaction.
pub struct GuidePrepared {
    pub guide: EntityId,
    pub prepared: Prepared,
    /// Present when pages were generated in the same transaction.
    pub generation: Option<GenerationReport>,
}

/// Create (`existing` None) or edit a drawing guide, optionally generating
/// or updating its drawing views in the same transaction.
#[allow(clippy::too_many_arguments)]
pub fn prepare_guide(
    session: &Session,
    expected_revision: u64,
    actor: Actor,
    existing: Option<EntityId>,
    name: &str,
    guide: &GuideDefinition,
    generate: bool,
    book: Option<EntityId>,
) -> Result<GuidePrepared, CommandError> {
    guide.validate().map_err(command_error)?;
    let document = session.document();
    for source in &guide.sources {
        if !matches!(
            document.get(*source).map(|e| &e.data),
            Some(
                EntityData::Part { .. } | EntityData::Instance { .. } | EntityData::Assembly { .. }
            )
        ) {
            return Err(command_error(invalid(
                "Guide sources must be Parts, Instances or assemblies",
            )));
        }
    }
    let definition = Definition {
        type_id: GUIDE_KIND.into(),
        schema_version: 1,
        data: serde_json::to_value(guide).map_err(|e| command_error(invalid(e)))?,
    };
    let data = EntityData::Extension {
        definition,
        inputs: guide.inputs(),
    };
    let (id, guide_name, edit) = match existing {
        Some(id) => {
            read_guide(document, id).map_err(command_error)?;
            let mut entity = document.get(id).expect("read guide").clone();
            entity.data = data;
            (id, entity.name.clone(), Mutation::Replace(entity))
        }
        None => {
            if name.trim().is_empty() || name.len() > 256 {
                return Err(command_error(invalid("Name must contain 1–256 bytes")));
            }
            let entity = Entity::new(name, data);
            (entity.id, name.to_owned(), Mutation::Insert(entity))
        }
    };
    let mut edits = vec![edit];
    let mut generation = None;
    if generate {
        let (book_edits, report) =
            generation_edits(document, session.parameters(), id, &guide_name, guide, book)
                .map_err(command_error)?;
        edits.extend(book_edits);
        generation = Some(report);
    }
    let label = match (existing.is_some(), generate) {
        (false, false) => "Create drawing guide",
        (false, true) => "Create drawing guide and pages",
        (true, false) => "Edit drawing guide",
        (true, true) => "Edit drawing guide and update pages",
    };
    let prepared = session.prepare(Request {
        expected_revision,
        actor,
        label: label.into(),
        edits,
    })?;
    Ok(GuidePrepared {
        guide: id,
        prepared,
        generation,
    })
}

/// Whether any view carries a guide link.
pub fn has_guided_views(book: &DrawingBook) -> bool {
    book.pages
        .iter()
        .flat_map(|p| &p.views)
        .any(|v| v.guide.is_some())
}

/// Outlines of a guide for the 3D viewport, as closed four-corner loops in
/// world millimeters, sized to the given model bounds.
pub fn guide_outlines(
    document: &Document,
    parameters: &EvaluatedParameters,
    guide: &GuideDefinition,
    bounds: [[f64; 3]; 2],
) -> Vec<[[f64; 3]; 4]> {
    let margin = 500.;
    let [min, max] = bounds;
    let (x0, y0, x1, y1) = (
        min[0] - margin,
        min[1] - margin,
        max[0] + margin,
        max[1] + margin,
    );
    let mut loops = vec![];
    for level in guide.targets() {
        let Ok(view) = resolve_guide_view(document, parameters, guide, level) else {
            continue;
        };
        let f = view.frame;
        match (&guide.kind, view.kind) {
            (GuideKind::Plan { .. }, _) => {
                let z = f.origin_mm[2];
                loops.push([[x0, y0, z], [x1, y0, z], [x1, y1, z], [x0, y1, z]]);
            }
            (GuideKind::Section { .. }, ViewKind::Section { window, .. }) => {
                let [a, b] = window.u_mm.unwrap_or([0., 0.]);
                let [z0, z1] = window.v_mm.unwrap_or([min[2], max[2]]);
                let p =
                    |t: f64, z: f64| [f.origin_mm[0] + f.u[0] * t, f.origin_mm[1] + f.u[1] * t, z];
                loops.push([p(a, z0), p(b, z0), p(b, z1), p(a, z1)]);
            }
            (GuideKind::Elevation { .. }, _) => {
                // A frame on the side of the bounds the viewer stands on.
                let n = f.normal();
                let corners = [[x0, y0], [x1, y0], [x1, y1], [x0, y1]];
                let reach = corners
                    .iter()
                    .map(|c| c[0] * n[0] + c[1] * n[1])
                    .fold(f64::NEG_INFINITY, f64::max);
                let along: Vec<f64> = corners
                    .iter()
                    .map(|c| c[0] * f.u[0] + c[1] * f.u[1])
                    .collect();
                let (a, b) = along
                    .iter()
                    .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), v| {
                        (lo.min(*v), hi.max(*v))
                    });
                let p = |t: f64, z: f64| [n[0] * reach + f.u[0] * t, n[1] * reach + f.u[1] * t, z];
                loops.push([p(a, min[2]), p(b, min[2]), p(b, max[2]), p(a, max[2])]);
            }
            _ => {}
        }
    }
    loops
}
