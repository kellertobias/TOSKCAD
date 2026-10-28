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
    let hash = ContentHash::document(document)?;
    let info = scene
        .transfer_evaluation()
        .ok_or("Transfer Apply requires native source-profile count receipts")?;
    info.validate().map_err(|e| e.to_string())?;
    if info.source_hash != hash
        || outputs
            .iter()
            .any(|output| !info.counts.contains_key(&output.feature))
    {
        return Err(
            "Native transfer counts belong to different source or omit a receiving feature".into(),
        );
    }
    let assembly = scene
        .assembly_evaluation()
        .ok_or("Transfer Apply requires complete native physical-occurrence receipts")?;
    assembly.validate().map_err(|e| e.to_string())?;
    let expected = multicad_core::model::transfer_occurrence_catalog(document)
        .map_err(|e| e.to_string())?
        .into_iter()
        .map(|item| (item.path, (item.component, item.producer)))
        .collect::<std::collections::BTreeMap<_, _>>();
    let actual = assembly
        .leaves
        .iter()
        .map(|item| (item.path.clone(), (item.component, item.producer)))
        .collect::<std::collections::BTreeMap<_, _>>();
    if assembly.source_hash != hash || expected != actual {
        return Err("Native transfer preview omitted, added or replaced a physical Part occurrence/current producer".into());
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
