//! Shared explicit native body choices for modeling forms. Building this list
//! reads source/history and hierarchy only; it never evaluates native geometry.
use crate::viewport::Scene;
use multicad_core::{
    document::{Document, EntityData},
    hierarchy::Index,
    model::BodyInput,
};

pub(super) struct BodyChoice {
    pub input: BodyInput,
    pub label: String,
    pub path: String,
    pub producer: String,
}
/// Cheap toolbar readiness; paths and full history checks are built once when
/// opening a form, rather than reconstructing them during stable UI painting.
pub(super) fn available(
    document: &Document,
    revision: u64,
    scene: &Scene,
) -> Result<usize, String> {
    current(document, revision, scene)?;
    Ok(scene
        .reference_instances()
        .iter()
        .filter(|instance| {
            matches!(
                document.get(instance.id).map(|entity| &entity.data),
                Some(EntityData::Part { .. })
            )
        })
        .take(2)
        .count())
}
fn current(document: &Document, revision: u64, scene: &Scene) -> Result<(), String> {
    if !scene.is_full_model() {
        return Err("Choose bodies from a full current native model".into());
    }
    if scene.stamp().document != document.id() || scene.stamp().revision != revision {
        return Err("Wait for current native bodies before choosing body inputs".into());
    }
    Ok(())
}
pub(super) fn choices(
    document: &Document,
    revision: u64,
    scene: &Scene,
) -> Result<Vec<BodyChoice>, String> {
    current(document, revision, scene)?;
    let index = Index::new(document).map_err(|error| error.to_string())?;
    let mut choices = vec![];
    for instance in scene.reference_instances() {
        let Some(entity) = document.get(instance.id) else {
            return Err("A native body has no canonical source; refresh the model".into());
        };
        let EntityData::Part { features, .. } = &entity.data else {
            continue;
        };
        let mut head = None;
        for id in features {
            let Some(EntityData::Feature { suppressed, .. }) = document.get(*id).map(|e| &e.data)
            else {
                return Err("A body history contains a missing or invalid producer; repair it before modeling".into());
            };
            if !suppressed {
                head = Some(*id);
            }
        }
        if head != Some(instance.geometry) {
            return Err("The displayed body producer differs from its current history; refresh the native model".into());
        }
        let input = BodyInput {
            body: instance.id,
            producer: instance.geometry,
        };
        input
            .resolve_producer(document)
            .map_err(|error| error.to_string())?;
        let owner = index
            .parent(instance.id)
            .map_err(|error| error.to_string())?;
        let path = crate::model_tree::container_path(document, &index, owner);
        let visible = index
            .effective_visibility(instance.id)
            .map_err(|error| error.to_string())?;
        let label = format!(
            "{} · {}{}",
            entity.name,
            path,
            if visible { "" } else { " · hidden" }
        );
        let producer = document
            .get(instance.geometry)
            .expect("resolved producer")
            .name
            .clone();
        choices.push(BodyChoice {
            input,
            label,
            path,
            producer,
        });
    }
    choices.sort_by(|a, b| a.label.cmp(&b.label).then(a.input.body.cmp(&b.input.body)));
    Ok(choices)
}
