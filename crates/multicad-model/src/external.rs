//! Exact external reference evaluation and source-only discovery. Native handles
//! remain on this worker; only stamped owned geometry and preparations escape.
use crate::{Compiler, Model, ModelError, ModelSnapshot, error, external_geometry};
use multicad_core::{
    commands::{Actor, PreparationSnapshot, Request},
    constraint_sketch::source::Source,
    document::{Document, EntityData, EntityId},
    model,
    parameters::EvaluatedParameters,
    project_file::ContentHash,
    references::{self, ReferenceReport, TopologySnapshot},
    sketch::Frame,
    sketch_external::{
        self as external, ExternalCatalog, ExternalQuery, ExternalResultReceipt,
        OccurrenceCaptureRequest,
    },
};
use multicad_kernel::{GeometryPolicy, Solid};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};
pub struct ExternalCapturePrepared {
    pub edit: multicad_solver::SketchEdit,
    pub model: Arc<ModelSnapshot>,
    pub report: ReferenceReport,
}
pub(crate) struct Sources<'a> {
    pub document: &'a Document,
    pub revision: u64,
    pub parameters: &'a EvaluatedParameters,
    pub body_heads: &'a BTreeMap<EntityId, EntityId>,
    pub solids: &'a BTreeMap<EntityId, Solid>,
    pub topologies: &'a BTreeMap<EntityId, TopologySnapshot>,
    pub policy: GeometryPolicy,
}
fn checkpoint(interrupt: Option<&dyn Fn() -> bool>) -> Result<(), ModelError> {
    if interrupt.is_some_and(|f| f()) {
        Err(error(None, "External reference evaluation superseded"))
    } else {
        Ok(())
    }
}
impl Sources<'_> {
    fn input<'a>(
        &'a self,
        query: &'a ExternalQuery,
        frame: Frame,
    ) -> Result<external_geometry::Inputs<'a>, ModelError> {
        external::validate_occurrence_query(self.document, query)
            .map_err(|e| error(Some(query.reference), e))?;
        let source=self.solids.get(&query.source.source).or_else(||self.body_heads.get(&query.source.source).and_then(|id|self.solids.get(id))).ok_or_else(||error(Some(query.reference),"Exact external source solid is unavailable; rebuild or explicitly repair its source"))?;
        let topology = self.topologies.get(&query.source.source).ok_or_else(|| {
            error(
                Some(query.reference),
                "External source topology is unavailable",
            )
        })?;
        let source_to_context = model::resolve_occurrence_relative_pose(
            self.document,
            (&query.source.path, query.source.component),
            query
                .sketch_context
                .as_ref()
                .map(|c| (&c.path, c.component)),
            self.parameters,
            self.revision,
            self.topologies,
        )
        .map_err(|e| error(Some(query.reference), e))?;
        Ok(external_geometry::Inputs {
            document: self.document,
            revision: self.revision,
            query,
            source,
            topology,
            source_to_context,
            frame,
            policy: self.policy,
        })
    }
    pub(crate) fn frame(
        &self,
        sketch: EntityId,
        report: &ReferenceReport,
    ) -> Result<Frame, ModelError> {
        Source::read_for_evaluation(self.document, sketch, self.revision, Some(report))
            .map(|(_, frame, _)| frame)
            .map_err(|e| error(Some(sketch), e))
    }
    fn receipt(
        &self,
        request: &OccurrenceCaptureRequest,
        frame: Frame,
        geometry: external::ExternalGeometry,
    ) -> Result<ExternalResultReceipt, ModelError> {
        let id = request.query.reference;
        request
            .output
            .validate_for(&geometry)
            .map_err(|e| error(Some(id), e))?;
        let projection = external::external_pose_projection(self.document, id)
            .map_err(|e| error(Some(id), e))?
            .ok_or_else(|| error(Some(id), "External pose dependency receipt unavailable"))?;
        Ok(ExternalResultReceipt {
            document: self.document.id(),
            revision: self.revision,
            source_hash: ContentHash::document(self.document).map_err(|e| error(Some(id), e))?,
            query_hash: request.query.hash().map_err(|e| error(Some(id), e))?,
            dependency_hash: references::dependency_hash(self.document, id)
                .map_err(|e| error(Some(id), e))?,
            plane_hash: external::frame_hash(frame).map_err(|e| error(Some(id), e))?,
            pose_hash: projection.fingerprint,
            selector: request.selector.clone(),
            geometry,
        })
    }
    pub(crate) fn populate(
        &self,
        report: &mut ReferenceReport,
        sketch: Option<EntityId>,
        interrupt: Option<&dyn Fn() -> bool>,
    ) -> Result<(), ModelError> {
        if report.external_errors.is_empty() && report.external_results.is_empty() {
            return Ok(());
        }
        for entity in self.document.entities() {
            if !external::is_occurrence_reference(self.document, entity.id) {
                continue;
            }
            let request = external::read_occurrence(self.document, entity.id)
                .map_err(|e| error(Some(entity.id), e))?;
            if sketch.is_some_and(|id| request.query.sketch != id) {
                continue;
            }
            checkpoint(interrupt)?;
            let result = (|| {
                let frame = self.frame(request.query.sketch, report)?;
                let set = external_geometry::geometry(self.input(&request.query, frame)?)?;
                let choice = external::select_external_result(
                    &set.results,
                    set.coincident,
                    request.query.method,
                    &request.selector,
                )
                .map_err(|e| error(Some(entity.id), e))?;
                self.receipt(&request, frame, choice.geometry.clone())
            })();
            match result {
                Ok(receipt) => {
                    report.external_results.insert(entity.id, receipt);
                    report.external_errors.remove(&entity.id);
                }
                Err(e) => {
                    report.external_results.remove(&entity.id);
                    report.external_errors.insert(entity.id, e.to_string());
                }
            }
        }
        Ok(())
    }
}
impl Compiler {
    pub fn query_external_catalog(
        &self,
        document: &Document,
        revision: u64,
        query: &ExternalQuery,
        interrupt: Option<&dyn Fn() -> bool>,
    ) -> Result<ExternalCatalog, ModelError> {
        self.external_catalog_context(document, revision, query, interrupt)
            .map(|(catalog, _)| catalog)
    }
    pub(crate) fn external_catalog_context(
        &self,
        document: &Document,
        revision: u64,
        query: &ExternalQuery,
        interrupt: Option<&dyn Fn() -> bool>,
    ) -> Result<(ExternalCatalog, Frame), ModelError> {
        checkpoint(interrupt)?;
        external::validate_occurrence_query(document, query)
            .map_err(|e| error(Some(query.reference), e))?;
        let mut roots = BTreeSet::from([query.source.source]);
        let Some(EntityData::Sketch { plane, .. }) = document.get(query.sketch).map(|e| &e.data)
        else {
            return Err(error(Some(query.reference), "Receiving sketch is missing"));
        };
        roots.insert(*plane);
        let relative = model::occurrence_pose_projection(
            document,
            &query.source.path,
            query.sketch_context.as_ref().map(|c| &c.path),
        )
        .map_err(|e| error(Some(query.reference), e))?;
        roots.extend(relative.geometry_dependencies);
        if let Some(context) = &query.sketch_context {
            roots.extend(
                model::occurrence_pose_projection(document, &context.path, None)
                    .map_err(|e| error(Some(query.reference), e))?
                    .geometry_dependencies,
            );
        }
        roots.retain(|id| {
            !matches!(
                document.get(*id).map(|e| &e.data),
                Some(EntityData::Parameter { .. })
            )
        });
        let native = self.compile_sources_interruptible(
            document,
            revision,
            &roots.into_iter().collect::<Vec<_>>(),
            interrupt,
        )?;
        checkpoint(interrupt)?;
        let parameters = multicad_core::parameters::evaluate(document)
            .map_err(|e| error(Some(query.reference), e))?;
        let topologies = topologies(&native);
        let sources = Sources {
            document,
            revision,
            parameters: &parameters,
            body_heads: &native.snapshot().body_sources,
            solids: &native.solids,
            topologies: &topologies,
            policy: self.policy,
        };
        let frame = sources.frame(query.sketch, &native.snapshot().references)?;
        let world = if let Some(context) = &query.sketch_context {
            model::resolve_occurrence_pose(
                document,
                &context.path,
                context.component,
                &parameters,
                revision,
                &topologies,
            )
            .map_err(|e| {
                error(
                    Some(query.reference),
                    format!("Consumer display placement is unavailable: {e}"),
                )
            })?
        } else {
            Default::default()
        };
        let catalog = external_geometry::query(sources.input(query, frame)?, world)?;
        checkpoint(interrupt)?;
        Ok((catalog, frame))
    }
    pub fn prepare_external_capture(
        &self,
        snapshot: &PreparationSnapshot,
        actor: Actor,
        request: &OccurrenceCaptureRequest,
        interrupt: Option<&dyn Fn() -> bool>,
    ) -> Result<ExternalCapturePrepared, ModelError> {
        self.prepare_external(snapshot, actor, request, false, interrupt)
    }
    pub fn prepare_external_repair(
        &self,
        snapshot: &PreparationSnapshot,
        actor: Actor,
        request: &OccurrenceCaptureRequest,
        interrupt: Option<&dyn Fn() -> bool>,
    ) -> Result<ExternalCapturePrepared, ModelError> {
        self.prepare_external(snapshot, actor, request, true, interrupt)
    }
    fn prepare_external(
        &self,
        snapshot: &PreparationSnapshot,
        actor: Actor,
        request: &OccurrenceCaptureRequest,
        repair: bool,
        interrupt: Option<&dyn Fn() -> bool>,
    ) -> Result<ExternalCapturePrepared, ModelError> {
        let session = snapshot.session();
        let revision = session.revision();
        let catalog =
            self.query_external_catalog(session.document(), revision, &request.query, interrupt)?;
        let edits = external::occurrence_capture_edits(
            session.document(),
            revision,
            request,
            &catalog,
            repair,
        )
        .map_err(|e| error(Some(request.query.reference), e))?;
        let prepared = session
            .prepare(Request {
                expected_revision: revision,
                actor,
                label: if repair {
                    "Edit occurrence sketch reference"
                } else {
                    "Capture occurrence sketch reference"
                }
                .into(),
                edits,
            })
            .map_err(|e| error(Some(request.query.reference), e))?;
        self.finish_external(prepared, revision, request.query.sketch, interrupt)
    }
    pub fn prepare_external_remove(
        &self,
        snapshot: &PreparationSnapshot,
        actor: Actor,
        sketch: EntityId,
        reference: EntityId,
        interrupt: Option<&dyn Fn() -> bool>,
    ) -> Result<ExternalCapturePrepared, ModelError> {
        let session = snapshot.session();
        let edits = external::remove_occurrence_edits(session.document(), sketch, reference)
            .map_err(|e| error(Some(reference), e))?;
        let prepared = session
            .prepare(Request {
                expected_revision: session.revision(),
                actor,
                label: "Remove occurrence sketch reference".into(),
                edits,
            })
            .map_err(|e| error(Some(reference), e))?;
        self.finish_external(prepared, session.revision(), sketch, interrupt)
    }
    fn finish_external(
        &self,
        prepared: multicad_core::commands::Prepared,
        revision: u64,
        sketch: EntityId,
        interrupt: Option<&dyn Fn() -> bool>,
    ) -> Result<ExternalCapturePrepared, ModelError> {
        checkpoint(interrupt)?;
        let model = self.compile_interruptible(prepared.document(), revision, interrupt)?;
        let report = model.snapshot().references.clone();
        if !report
            .for_model(prepared.document())
            .map_err(|e| error(Some(sketch), e))?
            .all_resolved()
        {
            return Err(error(
                Some(sketch),
                format!(
                    "Native external Apply requires a complete current reference report: {}",
                    report
                        .external_errors
                        .values()
                        .next()
                        .cloned()
                        .unwrap_or_else(|| "repair unresolved native geometry references".into())
                ),
            ));
        }
        let semantic = multicad_solver::evaluate_document_sketch_with_parameters(
            prepared.document(),
            revision,
            sketch,
            Some(&report),
            prepared.parameters(),
        )
        .map_err(|e| error(Some(sketch), e))?;
        if semantic.solved().is_none() {
            return Err(error(
                Some(sketch),
                "Occurrence reference candidate has no valid constrained sketch solve",
            ));
        }
        checkpoint(interrupt)?;
        let evaluation = semantic.evaluation.clone();
        Ok(ExternalCapturePrepared {
            edit: multicad_solver::SketchEdit {
                preview: Some(prepared),
                evaluation,
                semantic: Some(semantic),
            },
            model: Arc::new(model.snapshot().clone()),
            report,
        })
    }
}
pub(crate) fn topologies(model: &Model) -> BTreeMap<EntityId, TopologySnapshot> {
    model
        .snapshot()
        .shapes
        .iter()
        .map(|(id, s)| (*id, s.topology.clone()))
        .chain(
            model
                .snapshot()
                .body_sources
                .iter()
                .filter_map(|(body, producer)| {
                    model.snapshot().shapes.get(producer).map(|s| {
                        let mut t = s.topology.clone();
                        t.owner = *body;
                        (*body, t)
                    })
                }),
        )
        .chain(
            model
                .snapshot()
                .occurrences
                .iter()
                .map(|(id, s)| (*id, s.topology.clone())),
        )
        .collect()
}
/// Populate unconsumed references too. A failed result stays an inspectable error;
/// source-only fallback never recurses into this full-model enrichment path.
pub(crate) fn complete_report(
    compiler: &Compiler,
    sources: &Sources<'_>,
    report: &mut ReferenceReport,
    full: bool,
    interrupt: Option<&dyn Fn() -> bool>,
) -> Result<(), ModelError> {
    if !full {
        return Ok(());
    }
    sources.populate(report, None, interrupt)?;
    let ids = report.external_errors.keys().copied().collect::<Vec<_>>();
    for id in ids {
        checkpoint(interrupt)?;
        let request =
            external::read_occurrence(sources.document, id).map_err(|e| error(Some(id), e))?;
        match compiler.external_catalog_context(
            sources.document,
            sources.revision,
            &request.query,
            interrupt,
        ) {
            Ok((catalog, frame)) => {
                let receipt = external::select_external_choice(&catalog, &request.selector)
                    .map_err(|e| error(Some(id), e))
                    .and_then(|choice| sources.receipt(&request, frame, choice.geometry.clone()));
                match receipt {
                    Ok(receipt) => {
                        report.external_results.insert(id, receipt);
                        report.external_errors.remove(&id);
                    }
                    Err(e) => {
                        report.external_errors.insert(id, e.to_string());
                    }
                }
            }
            Err(e) => {
                report.external_errors.insert(id, e.to_string());
            }
        }
    }
    Ok(())
}
