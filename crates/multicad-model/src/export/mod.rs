//! Native output jobs rebuild an immutable source snapshot. No Scene, display
//! mesh, retained viewport state or foreign native handle is an export input.
mod publish;
mod worker;
use crate::{Compiler, Model};
use multicad_core::{
    commands::Actor,
    document::{Document, EntityId},
    exchange::*,
    project_file::ContentHash,
};
use std::{collections::BTreeMap, path::Path};
pub use worker::ExportWorker;
pub const MAX_NATIVE_EXPORT_FILES: usize = 128;
pub const MAX_STEP_BYTES: u64 = 64 * 1024 * 1024;
#[derive(Clone, Copy, Default)]
pub struct Exporter {
    pub compiler: Compiler,
}
struct Built {
    model: Option<Model>,
    preview: ExportPreview,
}
fn failure(kind: ErrorKind, id: Option<EntityId>, message: impl ToString) -> ExportError {
    ExportError::new(kind, id, message.to_string())
}
fn checkpoint(interrupt: Option<&dyn Fn() -> bool>) -> Result<(), ExportError> {
    if interrupt.is_some_and(|f| f()) {
        Err(failure(
            ErrorKind::Cancelled,
            None,
            "Export cancelled before publication",
        ))
    } else {
        Ok(())
    }
}
fn unsupported(output: &RecipeOutput) -> Option<String> {
    match output {
        RecipeOutput::Step {
            units,
            hierarchy,
            names,
            aids,
        } => {
            if *units != LengthUnit::Millimeter {
                Some("STEP output currently supports millimeters only".into())
            } else if *hierarchy != HierarchyMode::SeparateParts {
                Some("Assembly-preserving STEP containers need an assembly writer; use separate part files".into())
            } else if *names != NameHandling::FileNamesOnly {
                Some(
                    "STEP product-name attributes are unavailable; file naming remains supported"
                        .into(),
                )
            } else if *aids != AidPolicy::IncludeAll {
                Some("Removable-aid exclusion requires a typed aid provider".into())
            } else {
                None
            }
        }
        _ => Some(format!(
            "{} output provider is not installed",
            output.extension().to_ascii_uppercase()
        )),
    }
}
impl Exporter {
    pub fn preview(
        &self,
        document: &Document,
        revision: u64,
        set: EntityId,
        actor: &Actor,
        interrupt: Option<&dyn Fn() -> bool>,
    ) -> Result<ExportPreview, ExportError> {
        Ok(self
            .build(document, revision, set, actor, interrupt)?
            .preview)
    }
    fn provider(&self) -> String {
        let build = multicad_core::BuildInfo::current()
            .commit
            .map(str::to_owned)
            .unwrap_or_else(|| {
                static PROCESS: std::sync::OnceLock<EntityId> = std::sync::OnceLock::new();
                format!("unversioned-process:{}", PROCESS.get_or_init(EntityId::new))
            });
        let p = self.compiler.policy;
        format!(
            "step-files.1;build:{build};occt:{};policy:{:?}",
            multicad_kernel::source_identity(),
            [
                p.linear_tolerance_mm,
                p.boolean_fuzzy_mm,
                p.minimum_feature_mm,
                p.maximum_coordinate_mm
            ]
        )
    }
    fn build(
        &self,
        document: &Document,
        revision: u64,
        set: EntityId,
        actor: &Actor,
        interrupt: Option<&dyn Fn() -> bool>,
    ) -> Result<Built, ExportError> {
        if actor.id.trim().is_empty() || actor.id.len() > 256 {
            return Err(failure(
                ErrorKind::Invalid,
                None,
                "Invalid export actor identity",
            ));
        }
        checkpoint(interrupt)?;
        let selection = resolve(document, revision, set)?;
        let supported = selection
            .recipes
            .iter()
            .any(|r| unsupported(&r.output).is_none());
        let model = if supported && !selection.parts.is_empty() {
            Some(
                self.compiler
                    .compile_interruptible(document, revision, interrupt)
                    .map_err(|e| failure(ErrorKind::Invalid, e.entity, e.message))?,
            )
        } else {
            None
        };
        checkpoint(interrupt)?;
        let mut excluded = vec![];
        let mut selected = BTreeMap::new();
        if let Some(model) = &model {
            if model.snapshot().stamp.source_hash != selection.stamp.source_hash {
                return Err(failure(
                    ErrorKind::Stale,
                    None,
                    "Native model source differs from export selection",
                ));
            }
            for part in &selection.parts {
                checkpoint(interrupt)?;
                let reference = &part.reference;
                if reference.occurrence_path.len() > 1 {
                    return Err(failure(
                        ErrorKind::Unsupported,
                        Some(reference.part),
                        "Nested assembly output needs the assembly evaluator; no local/unplaced fallback is exported",
                    ));
                }
                let key = if let Some(instance) = reference.occurrence_path.first() {
                    let occurrence =
                        model.snapshot().occurrences.get(instance).ok_or_else(|| {
                            failure(
                                ErrorKind::Unsupported,
                                Some(*instance),
                                "Current occurrence geometry is unavailable",
                            )
                        })?;
                    if occurrence.component != reference.part {
                        return Err(failure(
                            ErrorKind::Invalid,
                            Some(*instance),
                            "Occurrence component does not match resolved export scope",
                        ));
                    }
                    Some((*instance, occurrence.producer))
                } else {
                    model
                        .snapshot()
                        .bodies
                        .get(&reference.part)
                        .map(|producer| (*producer, *producer))
                };
                let Some((solid, producer)) = key else {
                    excluded.push(ExcludedPart {
                        reference: reference.clone(),
                        reason: if model.snapshot().consumed_bodies.contains(&reference.part) {
                            ExclusionReason::ConsumedProducer
                        } else {
                            ExclusionReason::NoCurrentSolid
                        },
                    });
                    continue;
                };
                let shape = model.solid(solid).ok_or_else(|| {
                    failure(
                        ErrorKind::Invalid,
                        Some(solid),
                        "Native output binding has no current solid",
                    )
                })?;
                let bounds = shape
                    .bounds()
                    .map_err(|e| failure(ErrorKind::Invalid, Some(solid), e))?;
                let volume = shape
                    .volume_mm3()
                    .map_err(|e| failure(ErrorKind::Invalid, Some(solid), e))?;
                let count = shape
                    .solid_count()
                    .map_err(|e| failure(ErrorKind::Invalid, Some(solid), e))?;
                selected.insert(
                    reference.clone(),
                    (solid, producer, [bounds.min, bounds.max], volume, count),
                );
            }
        }
        let mut effective = selection.clone();
        if model.is_some() {
            effective
                .parts
                .retain(|p| selected.contains_key(&p.reference));
        }
        let names = filenames(&effective, document.name())?;
        let mut recipes = Vec::new();
        let mut artifacts = Vec::new();
        for recipe in &selection.recipes {
            let requested: Vec<_> = names.iter().filter(|n| n.recipe == recipe.id).collect();
            let status = if let Some(reason) = unsupported(&recipe.output) {
                RecipeStatus::Unsupported(reason)
            } else if requested.is_empty() {
                RecipeStatus::Empty
            } else {
                RecipeStatus::Ready
            };
            if status == RecipeStatus::Ready {
                for output in requested.iter() {
                    let OutputSubject::Part(part) = &output.subject else {
                        return Err(failure(
                            ErrorKind::Invalid,
                            Some(recipe.id),
                            "STEP output needs a Part subject",
                        ));
                    };
                    let &(solid, producer, bounds_mm, volume_mm3, solid_count) =
                        selected.get(part).ok_or_else(|| {
                            failure(
                                ErrorKind::Invalid,
                                Some(part.part),
                                "Current output availability was not evaluated",
                            )
                        })?;
                    artifacts.push(ExportArtifact {
                        output: (*output).clone(),
                        producer,
                        solid,
                        bounds_mm,
                        volume_mm3,
                        solid_count,
                    });
                }
            }
            recipes.push(RecipeReport {
                recipe: recipe.id,
                status,
                files: requested.len(),
            });
        }
        if artifacts.len() > MAX_NATIVE_EXPORT_FILES {
            return Err(failure(
                ErrorKind::Budget,
                Some(set),
                "Native export is bounded to 128 files per run; narrow the saved scope",
            ));
        }
        let provider = self.provider();
        let identity: Vec<_> = artifacts
            .iter()
            .map(|a| (&a.output, a.producer, a.solid, a.solid_count))
            .collect();
        let bytes = serde_json::to_vec(&(
            &provider,
            selection.stamp.source_hash,
            set,
            &identity,
            &excluded,
            &recipes,
        ))
        .map_err(|e| failure(ErrorKind::Invalid, None, e))?;
        let key = PreviewKey {
            source: selection.stamp.clone(),
            set,
            plan_hash: ContentHash::of(&bytes),
        };
        Ok(Built {
            model,
            preview: ExportPreview {
                requested_by: actor.clone(),
                key,
                provider,
                selection,
                recipes,
                artifacts,
                excluded,
            },
        })
    }
    pub fn run(
        &self,
        document: &Document,
        revision: u64,
        expected: &PreviewKey,
        root: &Path,
        actor: &Actor,
        interrupt: Option<&dyn Fn() -> bool>,
    ) -> Result<RunReceipt, ExportError> {
        expected.source.check(document, revision)?;
        let built = self.build(document, revision, expected.set, actor, interrupt)?;
        if built.preview.key != *expected {
            return Err(failure(
                ErrorKind::Stale,
                Some(expected.set),
                "Output plan changed; preview again before running",
            ));
        }
        if !built.preview.can_run() {
            return Err(failure(
                if built
                    .preview
                    .recipes
                    .iter()
                    .any(|r| matches!(r.status, RecipeStatus::Unsupported(_)))
                {
                    ErrorKind::Unsupported
                } else {
                    ErrorKind::Empty
                },
                Some(expected.set),
                "Export run requires nonempty supported recipes; inspect the preview",
            ));
        }
        publish::run(self, &built, root, interrupt)
    }
}
