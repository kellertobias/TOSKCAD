use super::*;
use crate::project_file::ContentHash;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum OutputSubject {
    Part(PartRef),
    /// Every selected Part in one file, for assembly-preserving mesh output.
    Parts(Vec<PartRef>),
    Page(PageRef),
    Pages(Vec<PageRef>),
    /// One schedule of the whole project.
    Schedule(crate::schedules::ScheduleSpec),
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NamedOutput {
    pub recipe: EntityId,
    pub subject: OutputSubject,
    pub filename: String,
}
/// Predictable ASCII filenames avoid platform-dependent Unicode normalization,
/// path separators and reserved device names. The preview shows the exact result.
pub fn filename_token(value: &str) -> String {
    let mut s = String::new();
    for c in value.chars() {
        if c.is_ascii_alphanumeric() || c == '-' {
            s.push(c);
        } else if !s.ends_with('_') {
            s.push('_');
        }
    }
    let s = s.trim_matches('_');
    let mut s = if s.is_empty() {
        "unnamed".into()
    } else {
        s.to_owned()
    };
    let upper = s.to_ascii_uppercase();
    if ["CON", "PRN", "AUX", "NUL"].contains(&upper.as_str())
        || (upper.len() == 4
            && (upper.starts_with("COM") || upper.starts_with("LPT"))
            && upper.as_bytes()[3].is_ascii_digit()
            && upper.as_bytes()[3] != b'0')
    {
        s.insert(0, '_');
    }
    s
}
fn digest(subject: &OutputSubject) -> Result<String, ExportError> {
    let bytes = serde_json::to_vec(subject)
        .map_err(|e| ExportError::new(ErrorKind::Invalid, None, e.to_string()))?;
    Ok(ContentHash::of(&bytes).to_string()[..12].into())
}
pub fn filenames(plan: &ResolvedSet, project: &str) -> Result<Vec<NamedOutput>, ExportError> {
    let revision = match &plan.naming.revision {
        RevisionRule::ContentHash => plan.stamp.source_hash.to_string()[..12].to_owned(),
        RevisionRule::Label(v) => filename_token(v),
    };
    let mut result = vec![];
    for recipe in &plan.recipes {
        let subjects: Vec<_> = match recipe.output {
            RecipeOutput::ScheduleCsv { schedule } => {
                vec![OutputSubject::Schedule(schedule.normalized())]
            }
            RecipeOutput::Pdf => {
                if plan.pages.is_empty() {
                    vec![]
                } else {
                    vec![OutputSubject::Pages(
                        plan.pages.iter().map(|p| p.reference.clone()).collect(),
                    )]
                }
            }
            RecipeOutput::DrawingDxf { .. } => plan
                .pages
                .iter()
                .map(|p| OutputSubject::Page(p.reference.clone()))
                .collect(),
            RecipeOutput::Mesh {
                hierarchy: HierarchyMode::PreserveAssembly,
                ..
            } => {
                if plan.parts.is_empty() {
                    vec![]
                } else {
                    vec![OutputSubject::Parts(
                        plan.parts.iter().map(|p| p.reference.clone()).collect(),
                    )]
                }
            }
            _ => plan
                .parts
                .iter()
                .map(|p| OutputSubject::Part(p.reference.clone()))
                .collect(),
        };
        for subject in subjects {
            if result.len() >= 4096 {
                return Err(ExportError::new(
                    ErrorKind::Budget,
                    Some(plan.set),
                    "Export plan exceeds 4096 output files",
                ));
            }
            let (label, id, page) = match &subject {
                OutputSubject::Part(p) => {
                    let source = plan
                        .parts
                        .iter()
                        .find(|s| s.reference == *p)
                        .expect("subjects from plan");
                    (source.name.clone(), p.part, String::new())
                }
                OutputSubject::Page(p) => {
                    let source = plan
                        .pages
                        .iter()
                        .find(|s| s.reference == *p)
                        .expect("subjects from plan");
                    (
                        if source.title.is_empty() {
                            "page".into()
                        } else {
                            source.title.clone()
                        },
                        p.page,
                        source.number.clone(),
                    )
                }
                OutputSubject::Pages(_) => ("drawings".into(), plan.set, "pages".into()),
                OutputSubject::Schedule(spec) => {
                    (spec.kind.token().into(), plan.set, String::new())
                }
                OutputSubject::Parts(_) => (plan.name.clone(), plan.set, String::new()),
            };
            let mut stem = plan.naming.template.clone();
            for (token, value) in [
                ("project", filename_token(project)),
                ("set", filename_token(&plan.name)),
                ("part", filename_token(&label)),
                ("id", id.to_string()),
                ("occurrence", digest(&subject)?),
                ("recipe", filename_token(&recipe.name)),
                ("revision", revision.clone()),
                ("page", filename_token(&page)),
            ] {
                stem = stem.replace(&format!("{{{token}}}"), &value);
            }
            result.push(NamedOutput {
                recipe: recipe.id,
                filename: format!("{}.{}", filename_token(&stem), recipe.output.extension()),
                subject,
            });
        }
    }
    disambiguate(&mut result)?;
    for output in &result {
        if output.filename.len() > 200 {
            return Err(ExportError::new(
                ErrorKind::Invalid,
                Some(output.recipe),
                "Output filename exceeds 200 bytes; use a shorter naming rule",
            ));
        }
    }
    Ok(result)
}
/// Outputs whose names coincide (ignoring ASCII case, as macOS and Windows
/// do) each get a short suffix from their recipe and subject identity, so
/// duplicate Part names, several occurrences or repeated recipe names never
/// overwrite one another. The suffix depends only on identity, so it stays
/// the same however the plan is ordered. Unique names stay as they are.
fn disambiguate(outputs: &mut [NamedOutput]) -> Result<(), ExportError> {
    let key = |name: &str| name.to_ascii_lowercase();
    let mut counts = BTreeMap::<String, usize>::new();
    for output in outputs.iter() {
        *counts.entry(key(&output.filename)).or_default() += 1;
    }
    let colliding: BTreeSet<String> = counts
        .into_iter()
        .filter(|(_, n)| *n > 1)
        .map(|(name, _)| name)
        .collect();
    if colliding.is_empty() {
        return Ok(());
    }
    let taken: BTreeSet<String> = outputs
        .iter()
        .map(|o| key(&o.filename))
        .filter(|name| !colliding.contains(name))
        .collect();
    for length in [6, 12, 24] {
        let mut renamed = vec![];
        for output in outputs.iter() {
            if !colliding.contains(&key(&output.filename)) {
                renamed.push(None);
                continue;
            }
            let bytes = serde_json::to_vec(&(output.recipe, &output.subject))
                .map_err(|e| ExportError::new(ErrorKind::Invalid, None, e.to_string()))?;
            let suffix = &ContentHash::of(&bytes).to_string()[..length];
            let (stem, extension) = output
                .filename
                .rsplit_once('.')
                .expect("generated names have an extension");
            renamed.push(Some(format!("{stem}_{suffix}.{extension}")));
        }
        let mut seen = taken.clone();
        if renamed.iter().flatten().all(|name| seen.insert(key(name))) {
            for (output, name) in outputs.iter_mut().zip(renamed) {
                if let Some(name) = name {
                    output.filename = name;
                }
            }
            return Ok(());
        }
    }
    let first = outputs
        .iter()
        .find(|o| colliding.contains(&key(&o.filename)))
        .expect("a collision");
    Err(ExportError::new(
        ErrorKind::Invalid,
        Some(first.recipe),
        format!("Output filename collision: {}", first.filename),
    ))
}
