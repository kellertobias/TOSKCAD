use crate::viewport::Scene;
use multicad_core::{
    document::{Document, EntityData},
    model::CreatedBody,
    project_file::ContentHash,
};
pub(super) fn validate(
    document: &Document,
    scene: &Scene,
    outputs: &[CreatedBody],
) -> Result<(), String> {
    if !scene.is_full_model()
        || scene
            .model_evaluation()
            .is_none_or(|i| Some(i.source_hash) != ContentHash::document(document).ok())
    {
        return Err(
            "Transfer Apply requires a complete native evaluation of this exact candidate source"
                .into(),
        );
    }
    for output in outputs {
        if !matches!(document.get(output.body).map(|e|&e.data),Some(EntityData::Part{features,..})if features.contains(&output.feature))
            || !scene
                .geometries()
                .get(&output.feature)
                .is_some_and(|g| !g.vertices().is_empty() && !g.indices().is_empty())
            || scene
                .geometry_snapshot(output.feature)
                .is_none_or(|s| s.elements.is_empty())
        {
            return Err(
                "Native transfer preview omitted a receiving feature or its named geometry".into(),
            );
        }
        let info = scene.model_evaluation().expect("checked native metadata");
        if !info.consumed_bodies.contains(&output.body) {
            let Some(EntityData::Part { features, .. }) =
                document.get(output.body).map(|e| &e.data)
            else {
                unreachable!()
            };
            let head = features
                .iter()
                .rev()
                .find(|id| {
                    matches!(
                        document.get(**id).map(|e| &e.data),
                        Some(EntityData::Feature {
                            suppressed: false,
                            ..
                        })
                    )
                })
                .ok_or("Receiving Part has no active final producer")?;
            if !scene
                .reference_instances()
                .iter()
                .any(|i| i.id == output.body && i.geometry == *head)
            {
                let owned = multicad_core::model::assembly_definition_parts(document)
                    .map_err(|e| e.to_string())?
                    .contains(&output.body)
                    || multicad_core::model::scale_component_owner(document, output.body)
                        .map_err(|e| e.to_string())?
                        .is_some();
                if !owned {
                    return Err("Native transfer preview did not retain the receiving Part's current final head".into());
                }
            }
        }
    }
    Ok(())
}
