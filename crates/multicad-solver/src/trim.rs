//! Checked, attributable trim/extend preparation. Canonical source and derived
//! solved geometry remain separate, including for literal-only old sketches.
use crate::{Diagnostics, EditError, InputError, SketchEdit};
use multicad_core::{
    commands::{Actor, CommandError, ErrorCode, Session},
    document::{EntityData, EntityId},
    project_file::ContentHash,
    references::ReferenceReport,
    sketch_trim::{
        self, DerivedGeometry, GeometryStamp, TrimError, TrimPolicy, TrimReport, TrimRequest,
    },
};

#[derive(Debug)]
pub enum TrimEditError {
    Edit(EditError),
    Geometry(TrimError),
    Unsolved {
        sketch: EntityId,
        diagnostics: Box<Diagnostics>,
    },
}
impl std::fmt::Display for TrimEditError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Edit(e) => e.fmt(f),
            Self::Geometry(e) => e.fmt(f),
            Self::Unsolved { diagnostics, .. } => write!(
                f,
                "Resolve the current sketch before trim/extend ({:?})",
                diagnostics.state
            ),
        }
    }
}
impl std::error::Error for TrimEditError {}
impl From<EditError> for TrimEditError {
    fn from(e: EditError) -> Self {
        Self::Edit(e)
    }
}
impl From<TrimError> for TrimEditError {
    fn from(e: TrimError) -> Self {
        Self::Geometry(e)
    }
}
pub struct TrimEdit {
    pub edit: SketchEdit,
    pub report: TrimReport,
    /// Upgrade is part of the same preview/undo step; source guesses and literal
    /// dimensions are otherwise preserved rather than baking solved coordinates.
    pub upgraded_source: bool,
}
pub fn prepare_trim(
    session: &Session,
    expected_revision: u64,
    actor: Actor,
    sketch: EntityId,
    report: Option<&ReferenceReport>,
    request: &TrimRequest,
    policy: TrimPolicy,
) -> Result<TrimEdit, TrimEditError> {
    if session.revision() != expected_revision {
        return Err(EditError::Command(CommandError {
            code: ErrorCode::StaleRevision,
            message: "Trim/extend is based on a stale document revision".into(),
            involved: vec![sketch],
            parameter: None,
        })
        .into());
    }
    let input = |message: String| {
        EditError::Input(InputError {
            source: Some(sketch),
            message,
        })
    };
    if report.is_some_and(|r| r.revision != expected_revision) {
        return Err(input("Trim/extend reference report is stale".into()).into());
    }
    let evaluated = crate::evaluate_document_sketch_with_parameters(
        session.document(),
        expected_revision,
        sketch,
        report,
        session.parameters(),
    )
    .map_err(EditError::Input)?;
    let Some(solved) = evaluated.solved() else {
        return Err(TrimEditError::Unsolved {
            sketch,
            diagnostics: Box::new(evaluated.evaluation.diagnostics.clone()),
        });
    };
    let expected = GeometryStamp {
        document: session.document().id(),
        sketch,
        revision: expected_revision,
        document_hash: ContentHash::document(session.document()).map_err(input)?,
    };
    let stamp = GeometryStamp {
        document: evaluated.stamp.document,
        sketch: evaluated.sketch,
        revision: evaluated.stamp.revision,
        document_hash: evaluated.stamp.source_hash,
    };
    let canonical = evaluated.canonical();
    let draft = sketch_trim::draft(
        canonical,
        expected,
        DerivedGeometry {
            stamp,
            canonical_hash: sketch_trim::canonical_hash(canonical)?,
            solved: Some(solved),
        },
        request,
        policy,
    )?;
    let upgraded_source = matches!(&session.document().get(sketch).expect("evaluated sketch").data,EntityData::Sketch {definition,..} if definition.schema_version==1);
    let edit = crate::canonical_edit::prepare_with(
        session,
        expected_revision,
        actor,
        crate::canonical_edit::CanonicalInput {
            sketch,
            source: draft.source,
            report,
            seed: Some(&draft.seed),
            label: match request {
                TrimRequest::Trim(_) => "Trim line",
                TrimRequest::Extend(_) => "Extend line",
                TrimRequest::TrimCurve(_) => "Trim curve",
                TrimRequest::ExtendArc(_) => "Extend arc",
            },
        },
    )?;
    Ok(TrimEdit {
        edit,
        report: draft.report,
        upgraded_source,
    })
}
