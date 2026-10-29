//! DXF/SVG import as one guarded sketch transaction. Imported curves and their
//! provenance record commit together; a failed solve or stale revision leaves
//! the document unchanged.
use crate::{EditError, InputError, SketchEdit, transaction::prepare_edit_with};
use multicad_core::{
    commands::{Actor, CommandError, ErrorCode, Request, Session},
    constraint_sketch::source::Source,
    document::{DocumentError, EntityData, EntityId, Mutation},
    references::ReferenceReport,
    sketch::Datum,
    sketch_import::{ImportPlan, Provenance, provenance::sha256_hex},
};

#[derive(Clone, Debug, PartialEq)]
pub enum ImportTarget {
    Existing(EntityId),
    /// A new sketch on an origin datum plane, created in the same transaction.
    New {
        datum: Datum,
        name: String,
    },
}

/// The exact bytes are hashed here so provenance always matches the parsed file.
pub struct ImportSource<'a> {
    pub file_name: &'a str,
    pub bytes: &'a [u8],
}

pub struct ImportEdit {
    pub edit: SketchEdit,
    pub sketch: EntityId,
    pub curves: Vec<EntityId>,
    pub provenance: EntityId,
}

fn input(sketch: Option<EntityId>, message: impl ToString) -> EditError {
    EditError::Input(InputError {
        source: sketch,
        message: message.to_string(),
    })
}

fn provenance(
    plan: &ImportPlan,
    file: &ImportSource,
    curves: &[EntityId],
) -> Result<Provenance, DocumentError> {
    let name = std::path::Path::new(file.file_name)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(file.file_name);
    Ok(Provenance {
        format: plan.format,
        file_name: name.into(),
        sha256: sha256_hex(file.bytes),
        byte_length: file.bytes.len() as u64,
        units: plan.options.units,
        millimeters_per_unit: plan.millimeters_per_unit,
        scale: plan.options.scale,
        insertion_mm: plan.options.insertion_mm,
        rotation_deg: plan.options.rotation_deg,
        weld_tolerance_mm: plan.options.weld_tolerance_mm,
        layers: plan
            .layers
            .iter()
            .filter(|l| l.selected)
            .map(|l| l.name.clone())
            .collect(),
        curves: curves.to_vec(),
        unsupported: plan.unsupported.clone(),
    })
}

pub fn prepare_import(
    session: &Session,
    expected_revision: u64,
    actor: Actor,
    target: &ImportTarget,
    report: Option<&ReferenceReport>,
    plan: &ImportPlan,
    file: &ImportSource,
) -> Result<ImportEdit, EditError> {
    if session.revision() != expected_revision {
        return Err(EditError::Command(CommandError {
            code: ErrorCode::StaleRevision,
            message: "Import is based on a stale document revision".into(),
            involved: match target {
                ImportTarget::Existing(id) => vec![*id],
                ImportTarget::New { .. } => vec![],
            },
            parameter: None,
        }));
    }
    let label = format!("Import {}", plan.format.label());
    match target {
        ImportTarget::Existing(sketch) => {
            let sketch = *sketch;
            let (source, _, _) =
                Source::read_for_evaluation(session.document(), sketch, expected_revision, report)
                    .map_err(|e| input(Some(sketch), e))?;
            let appended = plan
                .append_to(&source)
                .map_err(|e| input(Some(sketch), e))?;
            if appended.curves.is_empty() {
                return Err(input(
                    Some(sketch),
                    "No importable curves on the selected layers",
                ));
            }
            let record = provenance(plan, file, &appended.curves).map_err(|e| input(None, e))?;
            let entity = record.entity(sketch).map_err(|e| input(None, e))?;
            let provenance = entity.id;
            // Schema-1 source stores solved coordinates. A semantic sketch is
            // seeded beside its current solution so it keeps its solved branch.
            let existing_semantic = matches!(
                session.document().get(sketch).map(|e| &e.data),
                Some(EntityData::Sketch { definition, .. }) if definition.schema_version >= 2
            );
            let seed = if existing_semantic {
                let evaluated = crate::evaluate_document_sketch_with_parameters(
                    session.document(),
                    expected_revision,
                    sketch,
                    report,
                    session.parameters(),
                )
                .map_err(EditError::Input)?;
                let mut solved = evaluated
                    .solved()
                    .ok_or_else(|| {
                        input(Some(sketch), "Resolve the current sketch before importing")
                    })?
                    .clone();
                solved.points.extend(
                    appended.source.sketch.points[source.sketch.points.len()..]
                        .iter()
                        .cloned(),
                );
                solved.curves.extend(
                    appended.source.sketch.curves[source.sketch.curves.len()..]
                        .iter()
                        .cloned(),
                );
                Some(solved)
            } else {
                None
            };
            let edit = prepare_edit_with(
                session,
                expected_revision,
                actor,
                sketch,
                appended.source,
                report,
                seed.as_ref(),
                Some(&label),
                vec![Mutation::Insert(entity)],
            )?;
            Ok(ImportEdit {
                edit,
                sketch,
                curves: appended.curves,
                provenance,
            })
        }
        ImportTarget::New { datum, name } => {
            let (sketch, mut edits) = Source::create(*datum, name).map_err(|e| input(None, e))?;
            let empty = Source {
                u: datum.frame().u,
                sketch: Default::default(),
            };
            let appended = plan.append_to(&empty).map_err(|e| input(None, e))?;
            if appended.curves.is_empty() {
                return Err(input(None, "No importable curves on the selected layers"));
            }
            let semantic = appended.source.minimum_schema_version() >= 2;
            let mut evaluation = None;
            let source = if semantic {
                appended.source
            } else {
                let solved = crate::evaluate(&appended.source.sketch).map_err(EditError::Input)?;
                let candidate = solved.candidate.clone();
                evaluation = Some(solved);
                match candidate {
                    Some(sketch) => Source {
                        u: appended.source.u,
                        sketch,
                    },
                    None => appended.source,
                }
            };
            for edit in &mut edits {
                if let Mutation::Insert(entity) = edit
                    && entity.id == sketch
                    && let EntityData::Sketch { definition, .. } = &mut entity.data
                {
                    *definition = source.definition().map_err(|e| input(None, e))?;
                }
            }
            let record = provenance(plan, file, &appended.curves).map_err(|e| input(None, e))?;
            let entity = record.entity(sketch).map_err(|e| input(None, e))?;
            let provenance = entity.id;
            edits.push(Mutation::Insert(entity));
            let prepared = session
                .prepare(Request {
                    expected_revision,
                    actor,
                    label,
                    edits,
                })
                .map_err(EditError::Command)?;
            let (evaluation, semantic) = match evaluation {
                Some(evaluation) => (evaluation, None),
                None => {
                    let evaluated = crate::evaluate_document_sketch_with_parameters(
                        prepared.document(),
                        expected_revision,
                        sketch,
                        report,
                        prepared.parameters(),
                    )
                    .map_err(EditError::Input)?;
                    (evaluated.evaluation.clone(), Some(evaluated))
                }
            };
            let preview = evaluation.candidate.is_some().then_some(prepared);
            Ok(ImportEdit {
                edit: SketchEdit {
                    evaluation,
                    preview,
                    semantic,
                },
                sketch,
                curves: appended.curves,
                provenance,
            })
        }
    }
}
