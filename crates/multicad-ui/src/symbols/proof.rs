use crate::viewport::{Scene, SceneStamp};
use multicad_core::{document::Document, project_file::ContentHash, references, symbols};
use std::collections::BTreeMap;
pub(super) fn validate(
    document: &Document,
    stamp: SceneStamp,
    scene: &Scene,
) -> Result<(), String> {
    let hash = ContentHash::document(document)?;
    if !scene.is_full_model()
        || scene.stamp() != stamp
        || scene
            .model_evaluation()
            .is_none_or(|i| i.source_hash != hash)
    {
        return Err(
            "Symbol Apply requires a complete native evaluation of these exact inputs".into(),
        );
    }
    let report = scene
        .native_reference_report(hash)
        .cloned()
        .unwrap_or_else(|| {
            references::inspect(document, stamp.revision, &scene.reference_snapshots())
        });
    if !report
        .for_model(document)
        .map_err(|e| e.to_string())?
        .all_resolved()
    {
        return Err(
            "Resolve the candidate's driving native references before applying this symbol".into(),
        );
    }
    crate::assembly::validate_scene(document, stamp.revision, scene)?;
    // Validate the pin and owned maps against the same candidate source whose
    // real leaf geometry was checked above, including definitions without placements.
    let definitions = symbols::definitions(document).map_err(|e| e.to_string())?;
    let pins = definitions
        .iter()
        .map(|(id, d)| d.pin().map(|pin| (*id, pin)).map_err(|e| e.to_string()))
        .collect::<Result<BTreeMap<_, _>, _>>()?;
    for (definition, _) in definitions {
        for (_, instance) in symbols::instances(document, definition).map_err(|e| e.to_string())? {
            if pins.get(&instance.definition) != Some(&instance.pin) {
                return Err(
                    "The symbol instance pin does not match its embedded definition".into(),
                );
            }
            for owned in instance.ids.values() {
                if document.get(*owned).is_none() {
                    return Err("The candidate omits owned editable symbol content".into());
                }
            }
        }
    }
    Ok(())
}
