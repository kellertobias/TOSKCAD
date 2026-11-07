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
    pub recipe_facets: MechanicalRecipeFacets,
    /// Non-length configured profile facts with explicit units; not catalog certification.
    pub profile_details: Vec<String>,
    pub representation: String,
    pub designation: Option<String>,
    pub source_url: Option<String>,
    pub catalog_facts: Option<MechanicalPartCatalogFacts>,
    pub matches_reference_dimensions: bool,
    pub problem: Option<String>,
}

/// Applicable recipe defaults, projected without parsing display text or
/// changing the retained catalog source. Washer thickness and nut height are
/// not screw lengths.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MechanicalRecipeFacets {
    pub length: Option<MechanicalRecipeLength>,
    pub pitch: Option<MechanicalRecipePitch>,
    pub head: Option<MechanicalHeadForm>,
    pub drive: Option<MechanicalDriveForm>,
}

#[derive(Clone, Copy, Debug, PartialEq, PartialOrd)]
pub enum MechanicalRecipeLength {
    Underhead(f64),
    Overall(f64),
}
impl MechanicalRecipeLength {
    fn new(mm: f64, convention: ScrewLengthConvention) -> Self {
        match convention {
            ScrewLengthConvention::Underhead => Self::Underhead(mm),
            ScrewLengthConvention::Overall => Self::Overall(mm),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, PartialOrd)]
pub enum MechanicalRecipePitch {
    Thread(f64),
    WoodReference(f64),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum MechanicalHeadForm {
    SocketCap,
    ExternalHex,
    Pan,
    Button,
    Countersunk,
}
impl From<&ResolvedScrewHead> for MechanicalHeadForm {
    fn from(head: &ResolvedScrewHead) -> Self {
        match head {
            ResolvedScrewHead::SocketCap { .. } => Self::SocketCap,
            ResolvedScrewHead::ExternalHex { .. } => Self::ExternalHex,
            ResolvedScrewHead::Pan { .. } => Self::Pan,
            ResolvedScrewHead::Button { .. } => Self::Button,
            ResolvedScrewHead::Countersunk { .. } => Self::Countersunk,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum MechanicalDriveForm {
    HexSocket,
    ExternalHex,
    Hexalobular,
    PhillipsH,
}
impl From<&ResolvedScrewDrive> for MechanicalDriveForm {
    fn from(drive: &ResolvedScrewDrive) -> Self {
        match drive {
            ResolvedScrewDrive::HexSocket { .. } => Self::HexSocket,
            ResolvedScrewDrive::ExternalHex => Self::ExternalHex,
            ResolvedScrewDrive::Hexalobular { .. } => Self::Hexalobular,
            ResolvedScrewDrive::PhillipsH { .. } => Self::PhillipsH,
        }
    }
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
            .flat_map(|(_, definition)| mechanical_part_definition_cards(&definition))
            .collect()
    }
}

/// Use the same template defaults when listing a file and when choosing a
/// freshly loaded definition from that file.
pub fn mechanical_part_definition_cards(
    definition: &crate::symbols::SymbolDefinition,
) -> Vec<MechanicalPartCard> {
    cards(
        &definition.template,
        Some((definition.symbol, definition.revision.clone())),
    )
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
                recipe_facets: Default::default(),
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
                    ResolvedMechanicalPart::CustomConnection {parameters:p} => {
                        card.profile_details.push("Explicit authored thread profiles and flat-cut endpoints; no proprietary wood profile, installation pilot, strength or purchased standard is inferred".into());
                        let values=match *p {
                            ResolvedCustomConnectionHardware::WoodScrew{wood_thread,plain_length_mm,plain_radius_mm,head,..}=>vec![("Wood thread diameter",wood_thread.major_radius_mm*2.),("Wood pitch",wood_thread.pitch_mm),("Actual wood thread length",wood_thread.length_mm),("Plain length",plain_length_mm),("Plain diameter",plain_radius_mm*2.),("Head height",head.height_mm())],
                            ResolvedCustomConnectionHardware::WoodInsert{outer_thread,inner_thread,..}=>vec![("Outer diameter",outer_thread.major_radius_mm*2.),("Actual outer length",outer_thread.length_mm),("Actual inner length",inner_thread.length_mm),("Inner nominal diameter",inner_thread.nominal_major_radius_mm*2.)],
                            ResolvedCustomConnectionHardware::HangerBolt{wood_thread,metric_thread,plain_length_mm,plain_radius_mm,..}=>vec![("Wood diameter",wood_thread.major_radius_mm*2.),("Actual wood length",wood_thread.length_mm),("Metric nominal diameter",metric_thread.nominal_major_radius_mm*2.),("Actual metric length",metric_thread.length_mm),("Plain length",plain_length_mm),("Plain diameter",plain_radius_mm*2.)],
                        };
                        (values,"Custom modeled engineering geometry · flat ends · strength unverified".into())
                    },
                    ResolvedMechanicalPart::WoodScrew {parameters:p} => {
                        card.recipe_facets = MechanicalRecipeFacets {
                            length: Some(MechanicalRecipeLength::new(p.authored_length_mm, p.length_convention)),
                            pitch: Some(MechanicalRecipePitch::WoodReference(p.reference_profile.pitch_mm)),
                            head: Some((&p.head).into()),
                            drive: Some((&p.drive).into()),
                        };
                        card.profile_details.extend([
                            format!("Authored length convention: {}",match p.length_convention {ScrewLengthConvention::Underhead=>"underhead",ScrewLengthConvention::Overall=>"overall"}),
                            format!("Reference V included angle: {} degrees",p.reference_profile.included_angle_deg),
                            "Minimum tip-inclusive extent is a source lower bound, not the actual thread length or start".into(),
                            "Manufacturer point detail unspecified; nominal stock ends at a flat envelope plane".into(),
                            super::super::ManufacturingLimitationKind::SimplifiedWoodScrew.message().into(),
                        ]);
                        let mut values=vec![("Authored length",p.authored_length_mm),("Nominal wood diameter",p.nominal_diameter_mm),("Underhead stock-envelope length",p.underhead_length_mm),("Overall stock-envelope length",p.overall_length_mm),("Head height",p.head.height_mm()),("Reference V pitch",p.reference_profile.pitch_mm),("Reference core diameter",p.reference_profile.core_diameter_mm),("Minimum tip-inclusive thread extent",p.minimum_tip_inclusive_thread_extent_mm),("Maximum possible plain length",p.maximum_possible_plain_length_mm)];
                        if let ResolvedScrewHead::ExternalHex {across_flats_mm,..}=p.head {values.push(("Head across flats",across_flats_mm));}
                        (values,"Simplified wood stock envelope · thread and point detail unspecified · manufacturing unavailable".into())
                    },
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
                        card.recipe_facets = MechanicalRecipeFacets {
                            length: Some(MechanicalRecipeLength::new(authored_length_mm, length_convention)),
                            pitch: Some(MechanicalRecipePitch::Thread(thread.pitch_mm)),
                            head: Some((&head).into()),
                            drive: Some((&drive).into()),
                        };
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
                    } => {
                        card.recipe_facets.pitch = Some(MechanicalRecipePitch::Thread(thread.pitch_mm));
                        (
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
                        )
                    },
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
