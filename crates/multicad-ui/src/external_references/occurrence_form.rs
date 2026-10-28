//! Authored occurrence choices and result selectors. Geometry and poses become
//! authoritative only through current native catalogue/preparation receipts.
use super::ExternalTool;
use multicad_core::{
    document::{Document, EntityData, EntityId},
    model::OccurrencePath,
    references::GeometryIntent,
    sketch_external::{
        self, Behavior, ExternalCatalog, ExternalGeometry, OccurrenceCaptureRequest,
        OccurrenceContext, OccurrenceSource, QueryMethod, ResultSelector,
    },
};
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Consumer {
    Unset,
    World,
    Occurrence(OccurrenceContext),
}
impl Consumer {
    pub fn context(&self) -> Result<Option<OccurrenceContext>, String> {
        match self {
            Self::Unset => {
                Err("Choose the receiving sketch occurrence before finding results".into())
            }
            Self::World => Ok(None),
            Self::Occurrence(value) => Ok(Some(value.clone())),
        }
    }
}
pub(super) struct Form {
    pub sketch: EntityId,
    pub reference: EntityId,
    pub sources: Vec<OccurrenceSource>,
    pub contexts: Vec<OccurrenceContext>,
    pub source: Option<OccurrenceSource>,
    pub consumer: Consumer,
    pub method: QueryMethod,
    pub intent: Option<GeometryIntent>,
    pub catalog: Option<ExternalCatalog>,
    pub catalog_resources: Option<multicad_core::project_file::ProjectResources>,
    pub selected: Option<ResultSelector>,
    pub prepared: Option<OccurrenceCaptureRequest>,
    pub previous: Option<OccurrenceCaptureRequest>,
}
impl Form {
    pub fn new(
        document: &Document,
        sketch: EntityId,
        tool: ExternalTool,
        previous: Option<OccurrenceCaptureRequest>,
    ) -> Result<Self, String> {
        let sources =
            sketch_external::external_occurrence_catalog(document).map_err(|e| e.to_string())?;
        let contexts = sketch_external::sketch_occurrence_contexts(document, sketch)
            .map_err(|e| e.to_string())?;
        let consumer = if let Some(previous) = &previous {
            previous
                .query
                .sketch_context
                .clone()
                .map_or(Consumer::World, Consumer::Occurrence)
        } else {
            match contexts.as_slice() {
                [] => Consumer::World,
                [only] => Consumer::Occurrence(only.clone()),
                _ => Consumer::Unset,
            }
        };
        Ok(Self {
            sketch,
            reference: previous
                .as_ref()
                .map_or_else(EntityId::new, |p| p.query.reference),
            source: previous.as_ref().map(|p| p.query.source.clone()),
            method: previous.as_ref().map_or(
                if tool == ExternalTool::Intersect {
                    QueryMethod::Intersect
                } else {
                    QueryMethod::Coplanar
                },
                |p| p.query.method,
            ),
            intent: previous.as_ref().map(|p| p.query.intent.clone()),
            sources,
            contexts,
            consumer,
            previous,
            catalog: None,
            catalog_resources: None,
            selected: None,
            prepared: None,
        })
    }
    pub fn clear_results(&mut self) {
        self.catalog = None;
        self.catalog_resources = None;
        self.selected = None;
        self.prepared = None;
    }
    pub fn clear_geometry(&mut self) {
        self.intent = None;
        self.clear_results();
    }
    pub fn query(&self, document: &Document) -> Result<sketch_external::ExternalQuery, String> {
        let query = sketch_external::ExternalQuery {
            reference: self.reference,
            sketch: self.sketch,
            source: self.source.clone().ok_or("Choose a source occurrence")?,
            sketch_context: self.consumer.context()?,
            intent: self
                .intent
                .clone()
                .ok_or("Pick exact source geometry in the viewport")?,
            method: self.method,
        };
        sketch_external::validate_occurrence_query(document, &query).map_err(|e| e.to_string())?;
        Ok(query)
    }
    pub fn accept_catalog(
        &mut self,
        document: &Document,
        resources: &multicad_core::project_file::ProjectResources,
        catalog: ExternalCatalog,
    ) -> Result<(), String> {
        catalog.validate().map_err(|e| e.to_string())?;
        if catalog.query != self.query(document)? {
            return Err("Reference controls changed while native results were being found".into());
        }
        self.catalog = Some(catalog);
        self.catalog_resources = Some(resources.clone());
        self.selected = None;
        self.prepared = None;
        Ok(())
    }
    pub fn select_result(
        &mut self,
        selector: &ResultSelector,
        behavior: Behavior,
    ) -> Result<(), String> {
        let catalog = self
            .catalog
            .as_ref()
            .ok_or("Find current native results first")?;
        let request = if let Some(previous) = &self.previous {
            OccurrenceCaptureRequest::repair(catalog, selector, behavior, previous)
        } else {
            OccurrenceCaptureRequest::new(catalog, selector, behavior)
        }
        .map_err(|e| e.to_string())?;
        self.selected = Some(selector.clone());
        self.prepared = Some(request);
        Ok(())
    }
    pub fn request(
        &self,
        document: &Document,
        behavior: Behavior,
    ) -> Result<OccurrenceCaptureRequest, String> {
        let mut request = self
            .prepared
            .clone()
            .ok_or("Choose one current exact result before Preview")?;
        if request.query != self.query(document)? {
            return Err("Reference controls changed. Find exact results again".into());
        }
        request.behavior = behavior;
        Ok(request)
    }
}
pub(super) fn geometry_label(geometry: &ExternalGeometry) -> String {
    match geometry {
        ExternalGeometry::Point {
            position_mm: [x, y],
        } => format!("Point · X {x:.4} / Y {y:.4} mm"),
        ExternalGeometry::Line { a_mm, b_mm } => format!(
            "Line · {:.4} mm",
            (a_mm[0] - b_mm[0]).hypot(a_mm[1] - b_mm[1])
        ),
        ExternalGeometry::Circle { radius_mm, .. } => {
            format!("Circle · diameter {:.4} mm", radius_mm * 2.)
        }
        ExternalGeometry::Arc { .. } => "Circular arc".into(),
        ExternalGeometry::BSpline { poles_mm, basis } => format!(
            "{}spline · degree {} · {} poles",
            if basis.weights.is_some() {
                "Rational "
            } else {
                ""
            },
            basis.degree,
            poles_mm.len()
        ),
    }
}
pub(super) fn occurrence_label(
    document: &Document,
    component: EntityId,
    path: &OccurrencePath,
    paths: &[OccurrencePath],
) -> (String, String) {
    let component_name = document
        .get(component)
        .map_or("Missing Part", |e| e.name.as_str());
    let ids = |path: &OccurrencePath| {
        std::iter::once(path.root)
            .chain(path.members.iter().copied())
            .map(|id| id.to_string())
            .collect::<Vec<_>>()
            .join(" / ")
    };
    let full = ids(path);
    let digest = multicad_core::project_file::ContentHash::of(full.as_bytes()).to_string();
    let hashes: Vec<_> = paths
        .iter()
        .filter(|other| *other != path)
        .map(|other| {
            multicad_core::project_file::ContentHash::of(ids(other).as_bytes()).to_string()
        })
        .collect();
    let width = (6..=64)
        .step_by(2)
        .find(|width| {
            !hashes
                .iter()
                .any(|other| other[..*width] == digest[..*width])
        })
        .unwrap_or(64);
    let key = &digest[..width];
    let names = std::iter::once(path.root)
        .chain(path.members.iter().copied())
        .map(|id| {
            document
                .get(id)
                .map_or("Missing occurrence", |e| e.name.as_str())
        })
        .collect::<Vec<_>>()
        .join(" / ");
    (
        format!(
            "{component_name} · {} [{key}]",
            names.rsplit(" / ").next().unwrap_or(&names)
        ),
        format!(
            "Part: {component_name}\nOccurrence: {names}\nFull path IDs: {full}\nComponent ID: {component}"
        ),
    )
}
pub(super) fn source_history(document: &Document, source: &OccurrenceSource) -> Vec<EntityId> {
    let mut ids = vec![source.component];
    if let Some(EntityData::Part { features, .. }) = document.get(source.component).map(|e| &e.data)
    {
        ids.extend(features.iter().copied());
    }
    ids
}
