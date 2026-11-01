//! Source recipe facts for browsing; never physical occurrence/BOM evidence.
use super::*;
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq)]
pub struct MechanicalPartCard {
    pub feature: EntityId,
    pub name: String,
    pub family: String,
    /// Present for an importable template. Dimensions are its default values,
    /// not the possibly edited authoring Part outside the immutable template.
    pub symbol: Option<(EntityId, String)>,
    pub dimensions_mm: BTreeMap<String, f64>,
    pub representation: String,
    pub designation: Option<String>,
    pub source_url: Option<String>,
    pub matches_reference_dimensions: bool,
    pub problem: Option<String>,
}

/// If reusable definitions exist, describe exactly those pinned templates.
/// Otherwise describe source recipes, without making them placeable symbols.
pub fn mechanical_part_cards(document: &Document) -> Vec<MechanicalPartCard> {
    let definitions = crate::symbols::definitions(document).unwrap_or_default();
    if definitions.is_empty() {
        cards(document, None)
    } else {
        definitions
            .into_iter()
            .flat_map(|(_, definition)| {
                cards(
                    &definition.template,
                    Some((definition.symbol, definition.revision)),
                )
            })
            .collect()
    }
}
fn cards(document: &Document, symbol: Option<(EntityId, String)>) -> Vec<MechanicalPartCard> {
    let evaluated = crate::parameters::evaluate(document).map_err(|e| e.to_string());
    document
        .entities()
        .filter_map(|entity| {
            let EntityData::Feature { definition, .. } = &entity.data else {
                return None;
            };
            if definition.type_id != MECHANICAL_PART_TYPE {
                return None;
            }
            let mut card = MechanicalPartCard {
                feature: entity.id,
                name: entity.name.clone(),
                family: "Unsupported hardware".into(),
                symbol: symbol.clone(),
                dimensions_mm: BTreeMap::new(),
                representation: String::new(),
                designation: None,
                source_url: None,
                matches_reference_dimensions: false,
                problem: None,
            };
            let result = (|| -> Result<(), String> {
                let source =
                    read_mechanical_part(document, entity.id).map_err(|e| e.to_string())?;
                card.family = source.parameters.label().into();
                let evaluated = evaluated.as_ref().map_err(Clone::clone)?;
                let resolved = source
                    .parameters
                    .resolve(evaluated)
                    .map_err(|e| e.to_string())?;
                let (values, representation) = match resolved {
                    ResolvedMechanicalPart::Washer {
                        bore_radius_mm,
                        outside_radius_mm,
                        thickness_mm,
                        ..
                    } => (
                        vec![
                            ("Bore", bore_radius_mm * 2.),
                            ("Outside diameter", outside_radius_mm * 2.),
                            ("Thickness", thickness_mm),
                        ],
                        "Plain annular solid".to_string(),
                    ),
                    ResolvedMechanicalPart::BearingEnvelope {
                        bore_radius_mm,
                        outside_radius_mm,
                        width_mm,
                        ..
                    } => (
                        vec![
                            ("Bore", bore_radius_mm * 2.),
                            ("Outside diameter", outside_radius_mm * 2.),
                            ("Width", width_mm),
                        ],
                        "Mounting envelope · no bearing internals".to_string(),
                    ),
                    ResolvedMechanicalPart::HexNut {
                        across_flats_mm,
                        thread,
                    } => (
                        vec![
                            (
                                "Nominal thread diameter",
                                thread.nominal_major_radius_mm * 2.,
                            ),
                            ("Pitch", thread.pitch_mm),
                            ("Height", thread.length_mm),
                            ("Across flats", across_flats_mm),
                        ],
                        match thread.representation {
                            super::super::ThreadRepresentation::Modeled => {
                                "Modeled internal thread"
                            }
                            super::super::ThreadRepresentation::CosmeticEnvelope => {
                                "Simplified thread envelope · model thread before manufacturing"
                            }
                        }
                        .into(),
                    ),
                };
                card.dimensions_mm = values
                    .into_iter()
                    .map(|(key, value)| (key.into(), value))
                    .collect();
                card.representation = representation;
                card.matches_reference_dimensions = source
                    .matches_reference_dimensions(evaluated)
                    .map_err(|e| e.to_string())?;
                if let Some(provenance) = source.provenance {
                    card.designation = Some(provenance.designation);
                    card.source_url = Some(provenance.source_url);
                }
                Ok(())
            })();
            if let Err(error) = result {
                card.problem = Some(error);
            }
            Some(card)
        })
        .collect()
}
