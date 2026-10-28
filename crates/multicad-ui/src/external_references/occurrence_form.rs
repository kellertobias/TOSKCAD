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
    pub labels: super::labels::Labels,
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
        let entries = sources
            .iter()
            .map(|s| (s.component, s.path.clone()))
            .chain(contexts.iter().map(|c| (c.component, c.path.clone())))
            .chain(
                previous
                    .iter()
                    .map(|p| (p.query.source.component, p.query.source.path.clone())),
            )
            .chain(previous.iter().filter_map(|p| {
                p.query
                    .sketch_context
                    .as_ref()
                    .map(|c| (c.component, c.path.clone()))
            }));
        let labels = super::labels::Labels::new(document, entries);
        Ok(Self {
            labels,
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
    /// Changing only the occurrence keeps an explicit history choice on that
    /// same Part. Reopening the selected menu row is source-neutral.
    pub fn choose_source(&mut self, selected: &OccurrenceSource) -> bool {
        if self.source.as_ref().is_some_and(|current| {
            current.component == selected.component && current.path == selected.path
        }) {
            return false;
        }
        let mut next = selected.clone();
        if let Some(current) = &self.source
            && current.component == next.component
        {
            next.source = current.source;
        }
        self.source = Some(next);
        self.clear_geometry();
        true
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
    let labels = super::labels::Labels::new(
        document,
        paths
            .iter()
            .cloned()
            .chain(std::iter::once(path.clone()))
            .map(|path| (component, path)),
    );
    let value = labels.get(component, path).expect("explicit path label");
    (value.full.clone(), value.help.clone())
}
pub(super) fn source_history(document: &Document, source: &OccurrenceSource) -> Vec<EntityId> {
    let mut ids = vec![source.component];
    if let Some(EntityData::Part { features, .. }) = document.get(source.component).map(|e| &e.data)
    {
        ids.extend(features.iter().copied());
    }
    ids
}

#[cfg(test)]
mod source_choice_tests {
    use super::*;
    use crate::external_references::occurrence_context_tests::context_fixture;
    #[test]
    fn retained_feature_survives_reselecting_the_occurrence_and_switching_its_copies() {
        let (session, sketch, body, contexts) = context_fixture();
        let mut form = Form::new(session.document(), sketch, ExternalTool::Project, None).unwrap();
        let first = form
            .sources
            .iter()
            .find(|s| s.path == contexts[0].path)
            .unwrap()
            .clone();
        let copy = form
            .sources
            .iter()
            .find(|s| s.path == contexts[1].path)
            .unwrap()
            .clone();
        let retained = source_history(session.document(), &first)[1];
        assert_ne!(retained, body);
        assert!(form.choose_source(&first));
        form.source.as_mut().unwrap().source = retained;
        assert!(!form.choose_source(&first));
        assert_eq!(form.source.as_ref().unwrap().source, retained);
        assert!(form.choose_source(&copy));
        assert_eq!(form.source.as_ref().unwrap().source, retained);
        assert_eq!(form.source.as_ref().unwrap().path, copy.path);
        // A new component must use its own history, never retain another Part's feature.
        let different = OccurrenceSource {
            component: EntityId::new(),
            source: EntityId::new(),
            path: OccurrencePath {
                root: EntityId::new(),
                members: vec![],
            },
        };
        assert!(form.choose_source(&different));
        assert_eq!(form.source.as_ref(), Some(&different));
        assert!(!form.choose_source(&different));
    }
}
