//! Native check of a staged multi-reference repair set. Rows are staged on the
//! cumulative candidate with fresh topology and catalogues; the whole candidate
//! must compile, resolve every driving reference and solve every affected
//! sketch before a single attributable transaction becomes commit-capable.
use crate::{Compiler, ModelError, ModelSnapshot, error, external::topologies};
use multicad_core::{
    commands::{Actor, PreparationSnapshot, Prepared, Request},
    document::{Document, DocumentError, EntityId},
    references::{ReferenceReport, Resolution, TopologySnapshot},
    sketch_external::{
        ExternalCatalog, ExternalQuery, RepairNative, RepairRow, RepairSet, stage_repair_set,
    },
};
use std::{collections::BTreeMap, sync::Arc};

/// The outcome of staging and checking a repair set. `prepared` exists only
/// when every row staged, the full model compiled with a complete reference
/// report and every affected sketch solved.
pub struct RepairSetPrepared {
    pub rows: Vec<RepairRow>,
    pub sketches: Vec<EntityId>,
    /// Reasons Apply stays disabled, in display order.
    pub blockers: Vec<String>,
    pub prepared: Option<Prepared>,
    pub report: Option<ReferenceReport>,
    pub model: Option<Arc<ModelSnapshot>>,
}
impl RepairSetPrepared {
    pub fn can_apply(&self) -> bool {
        self.prepared.is_some() && self.blockers.is_empty()
    }
}

struct Native<'a> {
    compiler: &'a Compiler,
    revision: u64,
    interrupt: Option<&'a dyn Fn() -> bool>,
}
fn document_error(e: impl std::fmt::Display) -> DocumentError {
    DocumentError::Invalid(e.to_string())
}
impl RepairNative for Native<'_> {
    fn topology(
        &self,
        candidate: &Document,
        source: EntityId,
    ) -> Result<TopologySnapshot, DocumentError> {
        let model = self
            .compiler
            .compile_sources_interruptible(candidate, self.revision, &[source], self.interrupt)
            .map_err(document_error)?;
        topologies(&model)
            .remove(&source)
            .ok_or_else(|| document_error("Replacement source has no current native topology"))
    }
    fn catalog(
        &self,
        candidate: &Document,
        query: &ExternalQuery,
    ) -> Result<ExternalCatalog, DocumentError> {
        self.compiler
            .query_external_catalog(candidate, self.revision, query, self.interrupt)
            .map_err(document_error)
    }
}

fn unresolved(document: &Document, report: &ReferenceReport) -> Vec<String> {
    let name = |id: EntityId| {
        document
            .get(id)
            .map(|e| e.name.clone())
            .unwrap_or_else(|| id.to_string())
    };
    let mut out: Vec<String> = report
        .entries
        .iter()
        .filter(|d| !matches!(d.resolution, Resolution::Resolved { .. }))
        .map(|d| {
            let status = match &d.resolution {
                Resolution::Missing => "referenced geometry was removed".into(),
                Resolution::MissingSource => "source geometry is unavailable".into(),
                Resolution::Ambiguous { candidates } => {
                    format!("ambiguous between {} candidates", candidates.len())
                }
                Resolution::Stale { .. } => "stale geometry".into(),
                Resolution::UnavailableFrame => "planar frame is unavailable".into(),
                Resolution::Invalid { message } => message.clone(),
                Resolution::Resolved { .. } => unreachable!(),
            };
            format!("{} still unresolved: {status}", name(d.address.owner))
        })
        .collect();
    out.extend(
        report
            .external_errors
            .iter()
            .map(|(id, e)| format!("{} still unresolved: {e}", name(*id))),
    );
    out
}

impl Compiler {
    /// Stage `set` on the Session's current source and check the complete
    /// candidate. The live document never changes here; commit the returned
    /// preview to apply every row at once.
    pub fn prepare_repair_set(
        &self,
        snapshot: &PreparationSnapshot,
        actor: Actor,
        set: &RepairSet,
        resources: Option<&str>,
        interrupt: Option<&dyn Fn() -> bool>,
    ) -> Result<RepairSetPrepared, ModelError> {
        let session = snapshot.session();
        let revision = session.revision();
        let native = Native {
            compiler: self,
            revision,
            interrupt,
        };
        let staged = stage_repair_set(session.document(), revision, resources, set, &native)
            .map_err(|e| error(None, e))?;
        if interrupt.is_some_and(|f| f()) {
            return Err(error(None, "Repair set check superseded"));
        }
        let mut out = RepairSetPrepared {
            sketches: staged.sketches.iter().copied().collect(),
            blockers: staged
                .rows
                .iter()
                .filter_map(|r| {
                    r.error
                        .as_ref()
                        .map(|e| format!("Row {}: {e}", r.index + 1))
                })
                .collect(),
            rows: staged.rows.clone(),
            prepared: None,
            report: None,
            model: None,
        };
        if !staged.complete() {
            return Ok(out);
        }
        let rows = set.steps.len();
        let prepared = session
            .prepare(Request {
                expected_revision: revision,
                actor,
                label: format!(
                    "Repair {rows} sketch reference{}",
                    if rows == 1 { "" } else { "s" }
                ),
                edits: staged.edits.clone(),
            })
            .map_err(|e| error(None, e))?;
        if prepared.document() != &staged.candidate {
            return Err(error(
                None,
                "Repair set transaction disagrees with its staged candidate",
            ));
        }
        let model = match self.compile_interruptible(prepared.document(), revision, interrupt) {
            Ok(model) => model,
            Err(e) => {
                out.blockers
                    .push(format!("Repaired model cannot rebuild: {e}"));
                return Ok(out);
            }
        };
        let report = model.snapshot().references.clone();
        let driving = report
            .for_model(prepared.document())
            .map_err(|e| error(None, e))?;
        out.blockers
            .extend(unresolved(prepared.document(), &driving));
        let mut solved = BTreeMap::new();
        for sketch in &staged.sketches {
            let state = multicad_solver::evaluate_document_sketch_with_parameters(
                prepared.document(),
                revision,
                *sketch,
                Some(&report),
                prepared.parameters(),
            );
            let name = prepared
                .document()
                .get(*sketch)
                .map(|e| e.name.clone())
                .unwrap_or_default();
            match state {
                Ok(semantic) if semantic.solved().is_some() => {
                    solved.insert(*sketch, semantic.evaluation.diagnostics.state);
                }
                Ok(semantic) => out.blockers.push(format!(
                    "{name} has no valid constrained solve: {:?}",
                    semantic.evaluation.diagnostics.state
                )),
                Err(e) => out.blockers.push(format!("{name} cannot evaluate: {e}")),
            }
        }
        out.report = Some(report);
        out.model = Some(Arc::new(model.snapshot().clone()));
        if out.blockers.is_empty() {
            out.prepared = Some(prepared);
        }
        Ok(out)
    }
}
