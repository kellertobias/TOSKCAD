//! Saved print layouts. Each placement refers to a canonical finished Part or
//! occurrence and affects only the output copy.
use super::{
    AidPolicy, ErrorKind, ExportError, ExportStamp, MAX_PRINT_PARTS, MeshQuality, PartRef,
    PrintPart, PrintRequest, PrintRole, PrintSelection, PrintSource, print_candidates,
    resolve_occurrences,
};
use crate::{
    commands::{Actor, CommandError, Prepared, Request, Session},
    document::{Definition, Document, DocumentError, Entity, EntityData, EntityId, Link, Mutation},
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const PRINT_PLATE_TYPE: &str = "exchange.print_plate";
pub const PRINT_PLATE_CAPABILITY: &str = "exchange.print_plate.1";
const SCHEMA: u32 = 1;
const MAX_BED_MM: f64 = 100_000.;
const MAX_POSITION_MM: f64 = 1_000_000.;
const MAX_ROTATION_DEG: f64 = 360_000.;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrintPlatePlacement {
    pub source: PartRef,
    /// Minimum oriented native bounds in plate XY. Minimum Z is always zero.
    pub position_mm: [f64; 2],
    /// Degrees about X, then Y, then Z.
    pub rotation_deg: [f64; 3],
}

impl PrintPlatePlacement {
    /// Row-major rotation matrix, multiplied by column position vectors.
    pub fn rotation(&self) -> [[f64; 3]; 3] {
        let [(sx, cx), (sy, cy), (sz, cz)] = self
            .rotation_deg
            .map(|angle| angle.rem_euclid(360.).to_radians().sin_cos());
        [
            [cz * cy, cz * sy * sx - sz * cx, cz * sy * cx + sz * sx],
            [sz * cy, sz * sy * sx + cz * cx, sz * sy * cx - cz * sx],
            [-sy, cy * sx, cy * cx],
        ]
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrintPlateDefinition {
    pub bed_mm: [f64; 2],
    pub placements: Vec<PrintPlatePlacement>,
    pub quality: MeshQuality,
    pub stl_per_part: bool,
}

impl Default for PrintPlateDefinition {
    fn default() -> Self {
        Self {
            bed_mm: [220., 220.],
            placements: vec![],
            quality: MeshQuality::default(),
            stl_per_part: false,
        }
    }
}

fn invalid(message: impl Into<String>) -> ExportError {
    ExportError::new(ErrorKind::Invalid, None, message)
}

impl PrintPlateDefinition {
    pub fn validate(&self) -> Result<(), ExportError> {
        if self
            .bed_mm
            .iter()
            .any(|size| !(1. ..=MAX_BED_MM).contains(size))
        {
            return Err(invalid(format!("Bed dimensions must be 1–{MAX_BED_MM} mm")));
        }
        if self.placements.len() > MAX_PRINT_PARTS {
            return Err(ExportError::new(
                ErrorKind::Budget,
                None,
                format!("A print plate holds at most {MAX_PRINT_PARTS} Parts"),
            ));
        }
        PrintRequest {
            quality: self.quality.clone(),
            ..Default::default()
        }
        .validate()?;
        let mut seen = BTreeSet::new();
        for placement in &self.placements {
            let source = &placement.source;
            let path = &source.occurrence_path;
            if source.part.is_nil()
                || source.root_assembly.is_some_and(|id| id.is_nil())
                || (source.root_assembly.is_some() && path.is_empty())
                || path.len() > 32
                || path.iter().any(|id| id.is_nil() || *id == source.part)
                || path.iter().collect::<BTreeSet<_>>().len() != path.len()
                || !seen.insert((source.part, path))
            {
                return Err(invalid("Choose each exact Part or occurrence once"));
            }
            if placement
                .position_mm
                .iter()
                .any(|coordinate| !(-MAX_POSITION_MM..=MAX_POSITION_MM).contains(coordinate))
                || placement
                    .rotation_deg
                    .iter()
                    .any(|angle| !(-MAX_ROTATION_DEG..=MAX_ROTATION_DEG).contains(angle))
            {
                return Err(invalid(
                    "Print placement coordinates and rotation must be finite and within their supported range",
                ));
            }
        }
        Ok(())
    }

    /// Legacy package settings carried in the receipt. The saved placements
    /// and their order remain authoritative for this plate.
    pub fn summary_request(&self, document: &Document) -> PrintRequest {
        PrintRequest {
            source: PrintSource::Parts {
                parts: self.roots(document).into_iter().collect(),
            },
            aids: AidPolicy::ExcludeRemovable,
            quality: self.quality.clone(),
            stl_per_part: self.stl_per_part,
        }
    }

    fn roots(&self, document: &Document) -> BTreeSet<EntityId> {
        self.placements
            .iter()
            .map(|placement| source_root(document, &placement.source))
            .collect()
    }

    fn inputs(&self) -> Vec<Link> {
        let mut references = BTreeSet::new();
        for placement in &self.placements {
            let source = &placement.source;
            references.insert((source.part, "plate/part"));
            references.extend(
                source
                    .occurrence_path
                    .iter()
                    .map(|id| (*id, "plate/occurrence")),
            );
            if let Some(assembly) = source.root_assembly {
                references.insert((assembly, "plate/assembly"));
            }
        }
        references
            .into_iter()
            .map(|(target, role)| Link::new(target, role))
            .collect()
    }
}

pub fn print_plates(document: &Document) -> impl Iterator<Item = EntityId> + '_ {
    document.entities().filter_map(|entity| {
        matches!(&entity.data, EntityData::ExportSet { definition, .. } if definition.type_id == PRINT_PLATE_TYPE)
            .then_some(entity.id)
    })
}

pub fn read_print_plate(
    document: &Document,
    id: EntityId,
) -> Result<PrintPlateDefinition, ExportError> {
    let entity = document
        .get(id)
        .ok_or_else(|| ExportError::new(ErrorKind::Missing, Some(id), "Print plate is missing"))?;
    let EntityData::ExportSet { definition, inputs } = &entity.data else {
        return Err(invalid("Choose a saved print plate"));
    };
    if definition.type_id != PRINT_PLATE_TYPE || definition.schema_version != SCHEMA {
        return Err(ExportError::new(
            ErrorKind::Unsupported,
            Some(id),
            "Unsupported print plate provider or schema",
        ));
    }
    let plate: PrintPlateDefinition = serde_json::from_value(definition.data.clone())
        .map_err(|error| invalid(error.to_string()))?;
    plate.validate()?;
    let expected = plate.inputs();
    if inputs.len() != expected.len() || expected.iter().any(|link| !inputs.contains(link)) {
        return Err(invalid(
            "Print plate source and canonical input links disagree",
        ));
    }
    Ok(plate)
}

pub fn prepare_print_plate(
    session: &Session,
    revision: u64,
    actor: Actor,
    id: EntityId,
    name: &str,
    plate: &PrintPlateDefinition,
) -> Result<Prepared, CommandError> {
    let document = session.document();
    resolve_print_plate(document, revision, plate)
        .map_err(|error| DocumentError::Invalid(error.to_string()))?;
    let mut entity = Entity::new(
        name,
        EntityData::ExportSet {
            definition: Definition {
                type_id: PRINT_PLATE_TYPE.into(),
                schema_version: SCHEMA,
                data: serde_json::to_value(plate)
                    .map_err(|error| DocumentError::Invalid(error.to_string()))?,
            },
            inputs: plate.inputs(),
        },
    );
    entity.id = id;
    let edit = if let Some(existing) = document.get(id) {
        if !matches!(&existing.data, EntityData::ExportSet { definition, .. } if definition.type_id == PRINT_PLATE_TYPE)
        {
            return Err(DocumentError::Invalid(format!("{id} is not a print plate")).into());
        }
        entity.description.clone_from(&existing.description);
        Mutation::Replace(entity)
    } else {
        Mutation::Insert(entity)
    };
    session.prepare(Request {
        expected_revision: revision,
        actor,
        label: "Save print plate".into(),
        edits: vec![edit],
    })
}

fn source_root(document: &Document, reference: &PartRef) -> EntityId {
    let Some(&first) = reference.occurrence_path.first() else {
        return reference.part;
    };
    reference
        .root_assembly
        .filter(|assembly| {
            matches!(
                document.get(*assembly).map(|entity| &entity.data),
                Some(EntityData::Assembly { instances }) if instances.contains(&first)
            )
        })
        .unwrap_or(first)
}

/// Resolve the exact saved occurrences in placement order. Missing or
/// changed identities never expand the selection to their sibling Parts.
pub fn resolve_print_plate(
    document: &Document,
    revision: u64,
    plate: &PrintPlateDefinition,
) -> Result<PrintSelection, ExportError> {
    plate.validate()?;
    let stamp = ExportStamp::capture(document, revision)?;
    let mut parts = Vec::with_capacity(plate.placements.len());
    if !plate.placements.is_empty() {
        let roots = plate.roots(document);
        let candidates: BTreeSet<_> = print_candidates(document)?
            .into_iter()
            .map(|candidate| candidate.entity)
            .collect();
        let selected: BTreeSet<_> = plate
            .placements
            .iter()
            .map(|placement| &placement.source)
            .collect();
        let mut matched = BTreeMap::new();
        for root in roots.intersection(&candidates) {
            for part in resolve_occurrences(document, *root)? {
                if selected.contains(&part.reference) {
                    matched.insert(part.reference.clone(), part);
                }
            }
        }
        let aids = crate::model::aid_parts(document);
        for (index, placement) in plate.placements.iter().enumerate() {
            let source = &placement.source;
            let part = matched.remove(source).filter(|_| !aids.contains(&source.part)).ok_or_else(|| {
                ExportError::new(
                    ErrorKind::Missing,
                    Some(source.part),
                    format!("Placement {} no longer identifies an exact finished Part or occurrence. Remove it or select its current source.", index + 1),
                )
            })?;
            let name = if part.occurrence_names.is_empty() {
                part.name
            } else {
                format!("{} ({})", part.name, part.occurrence_names.join(" / "))
            };
            parts.push(PrintPart {
                part: source.part,
                occurrence_path: source.occurrence_path.clone(),
                name,
                role: PrintRole::Finished,
            });
        }
    }
    let mut counts = BTreeMap::new();
    for part in &parts {
        *counts.entry(part.name.clone()).or_insert(0usize) += 1;
    }
    let mut ordinal = BTreeMap::new();
    for part in &mut parts {
        if counts[&part.name] > 1 {
            let ordinal = ordinal.entry(part.name.clone()).or_insert(0usize);
            *ordinal += 1;
            part.name = format!("{} #{ordinal}", part.name);
        }
    }
    Ok(PrintSelection {
        stamp,
        parts,
        skipped: vec![],
        processes: vec![],
    })
}
