//! Exact drawing generation consumes owned native model solids. External source
//! producers register against the same view contract rather than a mesh path.
use crate::{Model, ModelError, compiler::error};
use multicad_core::{
    document::{Document, EntityData, EntityId},
    drawings::*,
    materials::{self, HatchPattern},
};
use multicad_kernel::{DrawingHatch, DrawingProjection, DrawingSolid, PlaneFrame, Solid};
use std::collections::{BTreeMap, BTreeSet};

pub enum DrawingShape<'a> {
    Borrowed(&'a Solid),
    Owned(Solid),
}
impl DrawingShape<'_> {
    fn solid(&self) -> &Solid {
        match self {
            Self::Borrowed(s) => s,
            Self::Owned(s) => s,
        }
    }
}
pub struct DrawingBody<'a> {
    pub identity: EntityId,
    pub material_part: EntityId,
    pub shape: DrawingShape<'a>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum DrawingSourceKind {
    Model,
    FlatPattern,
    Exploded,
}
#[derive(Clone, Copy, Debug)]
pub struct DrawingBudget {
    pub views: usize,
    pub vectors: usize,
    pub preview_points: usize,
}
impl Default for DrawingBudget {
    fn default() -> Self {
        Self {
            views: 256,
            vectors: 200_000,
            preview_points: 2_000_000,
        }
    }
}
pub trait DrawingSourceProvider {
    fn resolve<'a>(
        &self,
        document: &Document,
        model: &'a Model,
        source: &ViewSource,
    ) -> Result<Vec<DrawingBody<'a>>, ModelError>;
}
pub struct DrawingProviders {
    providers: BTreeMap<DrawingSourceKind, Box<dyn DrawingSourceProvider>>,
}
impl Default for DrawingProviders {
    fn default() -> Self {
        Self {
            providers: BTreeMap::from([(
                DrawingSourceKind::Model,
                Box::new(ModelDrawingSource) as Box<dyn DrawingSourceProvider>,
            )]),
        }
    }
}
impl DrawingProviders {
    pub fn register(
        &mut self,
        kind: DrawingSourceKind,
        provider: Box<dyn DrawingSourceProvider>,
    ) -> Result<(), String> {
        if self.providers.contains_key(&kind) {
            return Err("A drawing source provider is already registered".into());
        }
        self.providers.insert(kind, provider);
        Ok(())
    }
    fn resolve<'a>(
        &self,
        document: &Document,
        model: &'a Model,
        source: &ViewSource,
    ) -> Result<Vec<DrawingBody<'a>>, ModelError> {
        let kind = match source {
            ViewSource::Model { .. } => DrawingSourceKind::Model,
            ViewSource::FlatPattern { .. } => DrawingSourceKind::FlatPattern,
            ViewSource::Exploded { .. } => DrawingSourceKind::Exploded,
        };
        self.providers
            .get(&kind)
            .ok_or_else(|| {
                error(
                    None,
                    format!("{kind:?} drawing source provider is unavailable"),
                )
            })?
            .resolve(document, model, source)
    }
}
struct ModelDrawingSource;
impl DrawingSourceProvider for ModelDrawingSource {
    fn resolve<'a>(
        &self,
        document: &Document,
        model: &'a Model,
        source: &ViewSource,
    ) -> Result<Vec<DrawingBody<'a>>, ModelError> {
        let ViewSource::Model { entities } = source else {
            return Err(error(None, "Expected model drawing source"));
        };
        let hierarchy =
            multicad_core::hierarchy::Index::new(document).map_err(|e| error(None, e))?;
        let mut selected = BTreeSet::new();
        let mut pending: Vec<_> = entities.iter().map(|id| (*id, true)).collect();
        let mut containers = BTreeSet::new();
        while let Some((id, explicit)) = pending.pop() {
            let entity = document.get(id).ok_or_else(|| {
                error(
                    Some(id),
                    "Drawing source is missing; repair the source selection",
                )
            })?;
            match entity.data {
                EntityData::Part { .. } => {
                    if model.snapshot().consumed_bodies.contains(&id) {
                        if explicit {
                            return Err(error(
                                Some(id),
                                "Selected Part is consumed by a later operation",
                            ));
                        }
                    } else {
                        selected.insert(id);
                    }
                }
                EntityData::Instance { .. } => {
                    selected.insert(id);
                }
                EntityData::Assembly { .. } | EntityData::Group { .. } => {
                    if containers.insert(id) {
                        pending.extend(
                            hierarchy
                                .children(Some(id))
                                .map_err(|e| error(Some(id), e))?
                                .into_iter()
                                .map(|id| (id, false)),
                        );
                    }
                }
                _ if explicit => {
                    return Err(error(
                        Some(id),
                        "Choose Parts, placed Parts, components or groups as drawing sources",
                    ));
                }
                _ => {}
            }
        }
        if selected.is_empty() {
            return Err(error(None, "Drawing source contains no final solid Parts"));
        }
        let mut bodies = vec![];
        for id in selected {
            // A placed Assembly, such as a symbol, projects its world-placed
            // leaf Parts so each keeps its own material and section hatch.
            if let Some(occurrence) = model.snapshot().occurrences.get(&id)
                && !matches!(
                    document.get(occurrence.component).map(|e| &e.data),
                    Some(EntityData::Part { .. })
                )
            {
                let leaves: Vec<_> = model
                    .snapshot()
                    .leaf_occurrences
                    .values()
                    .filter(|leaf| leaf.path.root == id)
                    .collect();
                if leaves.is_empty() {
                    return Err(error(Some(id), "Placed component has no leaf Parts"));
                }
                for leaf in leaves {
                    bodies.push(DrawingBody {
                        identity: id,
                        material_part: leaf.component,
                        shape: DrawingShape::Borrowed(
                            model.leaf_solid(&leaf.path).ok_or_else(|| {
                                error(Some(id), "Placed leaf solid is unavailable")
                            })?,
                        ),
                    });
                }
                continue;
            }
            if !model.snapshot().occurrences.contains_key(&id)
                && !model.snapshot().bodies.contains_key(&id)
            {
                bodies.extend(Self::assembly_source(model, id)?);
                continue;
            }
            bodies.push(Self::body(model, id)?);
        }
        Ok(bodies)
    }
}
impl ModelDrawingSource {
    /// An Instance inside an Assembly projects its world-placed leaves when
    /// exactly one placed path contains it; a definition-only Part of an
    /// Assembly projects its own definition in component coordinates (a
    /// detail view of the component).
    fn assembly_source(model: &Model, id: EntityId) -> Result<Vec<DrawingBody<'_>>, ModelError> {
        let snapshot = model.snapshot();
        let mut prefixes = BTreeSet::new();
        let mut leaves = vec![];
        for leaf in snapshot.leaf_occurrences.values() {
            if let Some(at) = leaf.path.members.iter().position(|m| *m == id) {
                prefixes.insert((leaf.path.root, leaf.path.members[..=at].to_vec()));
                leaves.push(leaf);
            }
        }
        match prefixes.len() {
            0 => {}
            1 => {
                return leaves
                    .into_iter()
                    .map(|leaf| {
                        Ok(DrawingBody {
                            identity: id,
                            material_part: leaf.component,
                            shape: DrawingShape::Borrowed(
                                model.leaf_solid(&leaf.path).ok_or_else(|| {
                                    error(Some(id), "Placed leaf solid is unavailable")
                                })?,
                            ),
                        })
                    })
                    .collect();
            }
            n => {
                return Err(error(
                    Some(id),
                    format!(
                        "The member appears in {n} placed copies; choose the placed copy instead"
                    ),
                ));
            }
        }
        let producer = snapshot
            .body_sources
            .get(&id)
            .ok_or_else(|| error(Some(id), "Drawing Part has no current native solid"))?;
        Ok(vec![DrawingBody {
            identity: id,
            material_part: id,
            shape: DrawingShape::Borrowed(
                model
                    .solid(*producer)
                    .ok_or_else(|| error(Some(id), "Drawing source solid is unavailable"))?,
            ),
        }])
    }
    fn body(model: &Model, id: EntityId) -> Result<DrawingBody<'_>, ModelError> {
        let (producer, part) =
            if let Some(occurrence) = model.snapshot().occurrences.get(&id) {
                (id, occurrence.component)
            } else {
                (
                    *model.snapshot().bodies.get(&id).ok_or_else(|| {
                        error(Some(id), "Drawing Part has no current native solid")
                    })?,
                    id,
                )
            };
        Ok(DrawingBody {
            identity: id,
            material_part: part,
            shape: DrawingShape::Borrowed(
                model
                    .solid(producer)
                    .ok_or_else(|| error(Some(producer), "Drawing source solid is unavailable"))?,
            ),
        })
    }
}
impl Model {
    pub fn drawing_view(
        &self,
        document: &Document,
        revision: u64,
        view: &DrawingView,
        providers: &DrawingProviders,
    ) -> Result<ViewEvaluation, ModelError> {
        if self.snapshot().scope != crate::EvaluationScope::Full {
            return Err(error(
                Some(view.id),
                "A source-only inspection model cannot generate drawings",
            ));
        }
        // A guided view takes its frame, cut and sources from the current
        // guide and levels; a deleted guide or level is reported here.
        let guided;
        let view = if view.guide.is_some() {
            let parameters = multicad_core::parameters::evaluate(document)
                .map_err(|e| error(Some(view.id), e))?;
            guided = multicad_core::drawings::resolve_guided(document, &parameters, view)
                .map_err(|e| error(Some(view.id), e))?;
            &guided
        } else {
            view
        };
        view.validate().map_err(|e| error(Some(view.id), e))?;
        let stamp = self.snapshot().stamp;
        if stamp.document != document.id()
            || stamp.revision != revision
            || stamp.source_hash
                != multicad_core::project_file::ContentHash::document(document)
                    .map_err(|e| error(Some(view.id), e))?
        {
            return Err(error(
                Some(view.id),
                "Native model is stale for drawing generation",
            ));
        }
        let bodies = providers.resolve(document, self, &view.source)?;
        let resolver = materials::Resolver::new(document).map_err(|e| error(Some(view.id), e))?;
        let properties: Vec<_> = bodies
            .iter()
            .map(|body| {
                resolver
                    .resolve(body.material_part)
                    .map_err(|e| error(Some(body.material_part), e))
            })
            .collect::<Result<_, _>>()?;
        let scale = view.scale.numerator as f64 / view.scale.denominator as f64;
        // Walls in a downward view are drawn as an architectural plan cut.
        let wall_cuts = crate::walls::wall_cuts(document, view, &bodies)?;
        let plan_wall = |i: usize| wall_cuts.iter().any(|c| c.index == i);
        let inputs: Vec<_> = bodies
            .iter()
            .zip(&properties)
            .enumerate()
            .map(|(i, (body, props))| DrawingSolid {
                solid: body.shape.solid(),
                hatch: match props.hatch.as_ref() {
                    Some(h) => (h.pattern != HatchPattern::None).then(|| DrawingHatch {
                        angle_rad: h.angle_deg.to_radians(),
                        spacing_mm: h.spacing_mm / scale,
                        cross: h.pattern == HatchPattern::Cross,
                    }),
                    None => plan_wall(i).then(|| DrawingHatch {
                        angle_rad: crate::walls::WALL_HATCH_ANGLE_DEG.to_radians(),
                        spacing_mm: crate::walls::WALL_HATCH_SPACING_MM / scale,
                        cross: false,
                    }),
                },
            })
            .collect();
        let options = DrawingProjection {
            frame: PlaneFrame {
                origin: view.frame.origin_mm,
                u: view.frame.u,
                v: view.frame.v,
            },
            section_offset_mm: match view.kind {
                ViewKind::Section { offset_mm, .. } => Some(offset_mm),
                _ => None,
            },
            section_only: matches!(
                view.kind,
                ViewKind::Section {
                    range: SectionRange::SectionOnly,
                    ..
                }
            ),
            section_depth_mm: match view.kind {
                ViewKind::Section {
                    range: SectionRange::Depth { depth_mm },
                    ..
                } => Some(depth_mm),
                _ => None,
            },
            section_u_mm: match view.kind {
                ViewKind::Section { window, .. } => window.u_mm,
                _ => None,
            },
            section_v_mm: match view.kind {
                ViewKind::Section { window, .. } => window.v_mm,
                _ => None,
            },
            detail: match view.kind {
                ViewKind::Detail {
                    center_mm,
                    radius_mm,
                } => Some((center_mm, radius_mm)),
                _ => None,
            },
            hidden_lines: view.hidden_lines,
            tangent_edges: view.tangent_edges,
            chord_tolerance_mm: view.paper_chord_tolerance_mm / scale,
            ..Default::default()
        };
        // The kernel validates its own approximation interval. A scale demanding
        // more precision than the native contract supports fails explicitly.
        let output = if view.kind == ViewKind::Projection && !wall_cuts.is_empty() {
            crate::walls::project_plan(&inputs, options, view.frame.origin_mm[2], &wall_cuts)
        } else {
            multicad_kernel::project_drawing(&inputs, options)
        }
        .map_err(|e| error(Some(view.id), e))?;
        if output.is_empty() {
            return Err(error(
                Some(view.id),
                "View has no projected edges in its section/detail region",
            ));
        }
        let mut vectors: Vec<_> = output
            .into_iter()
            .map(|v| {
                let body = &bodies[v.source_index];
                DrawingVector {
                    source: body.identity,
                    material_part: body.material_part,
                    role: v.role,
                    line_width_mm: if v.role == VectorRole::Hatch {
                        properties[v.source_index]
                            .hatch
                            .as_ref()
                            .map(|h| h.line_width_mm)
                            .unwrap_or(if plan_wall(v.source_index) {
                                crate::walls::WALL_HATCH_LINE_MM
                            } else {
                                0.18
                            })
                    } else if v.role == VectorRole::Section {
                        0.5
                    } else if plan_wall(v.source_index) {
                        crate::walls::PLAN_PROJECTED_MM
                    } else {
                        0.25
                    },
                    curve: v.curve,
                    preview_mm: v.preview_mm,
                }
            })
            .collect();
        crate::walls::plan_vectors(document, view, &bodies, &mut vectors)?;
        crate::levels::plan_vectors(document, self, view, &bodies, &mut vectors)?;
        let warnings = properties
            .iter()
            .flat_map(|p| p.diagnostics.iter().map(|d| d.message.clone()))
            .collect();
        Ok(ViewEvaluation::Ready { vectors, warnings })
    }
    pub fn drawings(
        &self,
        document: &Document,
        revision: u64,
        providers: &DrawingProviders,
    ) -> Result<DrawingEvaluation, ModelError> {
        self.drawings_with_budget(document, revision, providers, DrawingBudget::default())
    }
    pub fn drawings_with_budget(
        &self,
        document: &Document,
        revision: u64,
        providers: &DrawingProviders,
        budget: DrawingBudget,
    ) -> Result<DrawingEvaluation, ModelError> {
        self.drawings_selected(document, revision, providers, budget, None)
    }
    /// Publish only the page currently being inspected. Other pages do not
    /// delay its dimensions or consume its preview budget.
    pub fn drawing_page(
        &self,
        document: &Document,
        revision: u64,
        providers: &DrawingProviders,
        page: EntityId,
    ) -> Result<DrawingEvaluation, ModelError> {
        self.drawings_selected(
            document,
            revision,
            providers,
            DrawingBudget::default(),
            Some(page),
        )
    }
    fn drawings_selected(
        &self,
        document: &Document,
        revision: u64,
        providers: &DrawingProviders,
        budget: DrawingBudget,
        selected_page: Option<EntityId>,
    ) -> Result<DrawingEvaluation, ModelError> {
        if self.snapshot().scope != crate::EvaluationScope::Full {
            return Err(error(
                None,
                "A source-only inspection model cannot generate drawings",
            ));
        }
        let maximum = DrawingBudget::default();
        if budget.views == 0
            || budget.views > maximum.views
            || budget.vectors == 0
            || budget.vectors > maximum.vectors
            || budget.preview_points == 0
            || budget.preview_points > maximum.preview_points
        {
            return Err(error(None, "Invalid drawing publication budget"));
        }
        if let Some(selected) = selected_page {
            let mut found = false;
            for id in books(document) {
                found |= read(document, id)
                    .map_err(|e| error(Some(id), e))?
                    .pages
                    .iter()
                    .any(|page| page.id == selected);
            }
            if !found {
                return Err(error(
                    Some(selected),
                    "The selected drawing page is missing",
                ));
            }
        }
        let mut views = BTreeMap::new();
        let (mut vectors_used, mut points_used) = (0usize, 0usize);
        let mut exhausted = false;
        for id in books(document) {
            let book = read(document, id).map_err(|e| error(Some(id), e))?;
            for view in book
                .pages
                .iter()
                .filter(|p| selected_page.is_none_or(|id| p.id == id))
                .flat_map(|p| &p.views)
            {
                let mut evaluation = if exhausted || views.len() >= budget.views {
                    ViewEvaluation::Unresolved{message:"Drawing publication budget exceeded; reduce the number or complexity of views".into()}
                } else {
                    self.drawing_view(document, revision, view, providers)
                        .unwrap_or_else(|e| ViewEvaluation::Unresolved {
                            message: e.to_string(),
                        })
                };
                if let ViewEvaluation::Ready { vectors, .. } = &evaluation {
                    let points: usize = vectors.iter().map(|v| v.preview_mm.len()).sum();
                    if vectors.len() > budget.vectors - vectors_used
                        || points > budget.preview_points - points_used
                    {
                        exhausted = true;
                        evaluation=ViewEvaluation::Unresolved{message:"Drawing vector/point publication budget exceeded; simplify the page".into()};
                    } else {
                        vectors_used += vectors.len();
                        points_used += points;
                    }
                }
                views.insert(view.id, evaluation);
            }
        }
        let mut annotations = BTreeMap::new();
        for id in books(document) {
            let book = read(document, id).map_err(|e| error(Some(id), e))?;
            for page in book
                .pages
                .iter()
                .filter(|p| selected_page.is_none_or(|id| p.id == id))
            {
                for annotation in &page.annotations {
                    if matches!(annotation.kind, AnnotationKind::Dimension { .. }) {
                        annotations.insert(annotation.id, self.dimension(page, annotation));
                    }
                }
            }
        }
        Ok(DrawingEvaluation {
            stamp: DrawingStamp {
                document: document.id(),
                revision,
                source_hash: multicad_core::project_file::ContentHash::document(document)
                    .map_err(|e| error(None, e))?,
            },
            views,
            annotations,
        })
    }
    /// Evaluate a model dimension from its current persistent references.
    /// Unresolvable references detach it; the previous value is never kept.
    pub fn dimension(&self, page: &Page, annotation: &Annotation) -> AnnotationEvaluation {
        let AnnotationKind::Dimension {
            view,
            measure,
            offset_mm,
            precision,
            reference,
        } = &annotation.kind
        else {
            return AnnotationEvaluation::Detached {
                message: "Not a model dimension".into(),
            };
        };
        let detached = |message: String| AnnotationEvaluation::Detached { message };
        let Some(view) = page.views.iter().find(|v| v.id == *view) else {
            return detached("The dimension's view is no longer on this page".into());
        };
        let resolve = |r: &multicad_core::inspection::GeometryReference| {
            crate::inspection::resolved(self, r)
                .map(|(_, _, geometry)| geometry)
                .map_err(|f| f.message)
        };
        let graphic = match measure.as_ref() {
            DimensionMeasure::Linear { a, b, direction } => {
                let query = multicad_core::inspection::MeasurementQuery {
                    a: a.clone(),
                    b: b.clone(),
                };
                match crate::inspection::measure_model(self, &query).result {
                    Ok(values) => {
                        let w = values.witnesses[0];
                        linear_dimension(
                            view, w.a_mm, w.b_mm, *direction, *offset_mm, *precision, *reference,
                        )
                    }
                    Err(failure) => return detached(failure.message),
                }
            }
            DimensionMeasure::Radius { edge } | DimensionMeasure::Diameter { edge } => {
                match resolve(edge) {
                    Ok(Some(multicad_core::references::AnalyticGeometry::CircularArc {
                        frame,
                        radius_mm,
                        start_rad,
                        sweep_rad,
                    })) => {
                        let full = sweep_rad.abs() >= std::f64::consts::TAU - 1e-9;
                        let angle = if full {
                            std::f64::consts::FRAC_PI_4
                        } else {
                            start_rad + sweep_rad / 2.
                        };
                        let (s, c) = angle.sin_cos();
                        let rim = std::array::from_fn(|i| {
                            frame.origin_mm[i] + radius_mm * (c * frame.u[i] + s * frame.v[i])
                        });
                        radial_dimension(
                            view,
                            frame.origin_mm,
                            rim,
                            radius_mm,
                            matches!(measure.as_ref(), DimensionMeasure::Diameter { .. }),
                            *offset_mm,
                            *precision,
                            *reference,
                        )
                    }
                    Ok(_) => return detached("The reference is not a circular edge".into()),
                    Err(message) => return detached(message),
                }
            }
            DimensionMeasure::Angle { a, b } => match (resolve(a), resolve(b)) {
                (
                    Ok(Some(multicad_core::references::AnalyticGeometry::Line {
                        a_mm: a0,
                        b_mm: a1,
                    })),
                    Ok(Some(multicad_core::references::AnalyticGeometry::Line {
                        a_mm: b0,
                        b_mm: b1,
                    })),
                ) => {
                    let da: [f64; 3] = std::array::from_fn(|i| a1[i] - a0[i]);
                    let db: [f64; 3] = std::array::from_fn(|i| b1[i] - b0[i]);
                    let dot = |x: [f64; 3], y: [f64; 3]| x[0] * y[0] + x[1] * y[1] + x[2] * y[2];
                    let cos = (dot(da, db).abs() / (dot(da, da) * dot(db, db)).sqrt()).min(1.);
                    angle_dimension(
                        view,
                        (a0, a1),
                        (b0, b1),
                        cos.acos().to_degrees(),
                        *offset_mm,
                        *precision,
                        *reference,
                    )
                }
                (Err(message), _) | (_, Err(message)) => return detached(message),
                _ => return detached("Angle dimensions need two straight edges".into()),
            },
        };
        AnnotationEvaluation::Ready { graphic }
    }
}
