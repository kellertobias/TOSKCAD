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
    /// Non-length configured profile facts with explicit units; not catalog certification.
    pub profile_details: Vec<String>,
    pub representation: String,
    pub designation: Option<String>,
    pub source_url: Option<String>,
    pub catalog_facts: Option<MechanicalPartCatalogFacts>,
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
                profile_details: Vec::new(),
                representation: String::new(),
                designation: None,
                source_url: None,
                catalog_facts: None,
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
                    ResolvedMechanicalPart::MachineScrew {
                        underhead_length_mm,
                        authored_length_mm,
                        length_convention,
                        shank_radius_mm,
                        head,
                        drive,
                        thread,
                        ..
                    } => {
                        let mut values = vec![
                            (
                                "Nominal thread diameter",
                                thread.nominal_major_radius_mm * 2.,
                            ),
                            ("Pitch", thread.pitch_mm),
                            ("Underhead length", underhead_length_mm),
                            ("Thread length", thread.length_mm),
                            ("Plain shank length", underhead_length_mm - thread.length_mm),
                            ("Shank diameter", shank_radius_mm * 2.),
                            ("Head height", head.height_mm()),
                        ];
                        if length_convention == ScrewLengthConvention::Overall {
                            values.push(("Overall length", authored_length_mm));
                        }
                        for (label, value) in head
                            .additional_profile_values()
                            .into_iter()
                            .chain(drive.additional_profile_values())
                        {
                            if label.contains("(degrees)") {
                                card.profile_details.push(format!(
                                    "{}: {value} degrees",
                                    label.replace(" (degrees)", "")
                                ));
                            } else if label == "Phillips H size" {
                                card.profile_details.push(format!("Phillips H{value:.0}"));
                            } else {
                                values.push((label, value));
                            }
                        }
                        let head_label = match head {
                            ResolvedScrewHead::Pan { radius_mm, .. } => {
                                values.push(("Head diameter", radius_mm * 2.));
                                "Pan head"
                            }
                            ResolvedScrewHead::Button { radius_mm, .. } => {
                                values.push(("Head diameter", radius_mm * 2.));
                                "Button head"
                            }
                            ResolvedScrewHead::Countersunk { radius_mm, .. } => {
                                values.push(("Head diameter", radius_mm * 2.));
                                "Countersunk head · neck construction datum"
                            }
                            ResolvedScrewHead::SocketCap { radius_mm, .. } => {
                                values.push(("Head diameter", radius_mm * 2.));
                                "Socket-cap head"
                            }
                            ResolvedScrewHead::ExternalHex {
                                across_flats_mm, ..
                            } => {
                                values.push(("Head across flats", across_flats_mm));
                                "External-hex head"
                            }
                        };
                        let drive_label = match drive {
                            ResolvedScrewDrive::HexSocket {
                                across_flats_mm,
                                depth_mm,
                            } => {
                                values.push(("Socket across flats", across_flats_mm));
                                values.push(("Socket depth", depth_mm));
                                "flat-floor hex socket"
                            }
                            ResolvedScrewDrive::Hexalobular { .. } => "hexalobular recess",
                            ResolvedScrewDrive::PhillipsH { .. } => "Phillips H recess",
                            ResolvedScrewDrive::ExternalHex => "external hex flats",
                        };
                        let profile = match thread.representation {
                            super::super::ThreadRepresentation::Modeled => "modeled thread",
                            super::super::ThreadRepresentation::CosmeticEnvelope => {
                                "cosmetic thread envelope; model before manufacturing"
                            }
                        };
                        (values, format!("{head_label} · {drive_label} · {profile}"))
                    }
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
                    card.catalog_facts = provenance.catalog_facts;
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
