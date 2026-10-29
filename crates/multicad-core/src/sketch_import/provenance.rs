//! Saved import provenance. The record describes how curves were created; it
//! does not own them. Later edits may change or delete those curves, so the
//! recorded curve list is historical and never drives evaluation.
use super::{Diagnostic, Format, UnitChoice};
use crate::document::{Definition, Document, DocumentError, Entity, EntityData, EntityId, Link};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const PROVENANCE_TYPE: &str = "core.sketch_import";
pub const CAPABILITY: &str = "core.sketch_import.1";
const SKETCH_ROLE: &str = "sketch";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Provenance {
    pub format: Format,
    /// File name only; absolute paths are not stored in the project.
    pub file_name: String,
    /// Lowercase hexadecimal SHA-256 of the exact imported bytes.
    pub sha256: String,
    pub byte_length: u64,
    pub units: UnitChoice,
    pub millimeters_per_unit: f64,
    pub scale: f64,
    pub insertion_mm: [f64; 2],
    pub rotation_deg: f64,
    pub weld_tolerance_mm: f64,
    /// Imported layers or groups, in file order.
    pub layers: Vec<String>,
    pub curves: Vec<EntityId>,
    pub unsupported: Vec<Diagnostic>,
}

fn invalid(message: impl Into<String>) -> DocumentError {
    DocumentError::Invalid(message.into())
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

impl Provenance {
    pub fn validate(&self) -> Result<(), DocumentError> {
        if self.file_name.trim().is_empty()
            || self.file_name.len() > 255
            || self.file_name.contains(['/', '\\'])
        {
            return Err(invalid("Import provenance needs a bare file name"));
        }
        if self.sha256.len() != 64
            || !self
                .sha256
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(invalid("Import provenance has an invalid SHA-256"));
        }
        let finite = |v: f64| v.is_finite() && v.abs() <= 1e9;
        if !(finite(self.millimeters_per_unit) && self.millimeters_per_unit > 0.)
            || !(finite(self.scale) && self.scale > 0.)
            || !self.insertion_mm.iter().all(|v| finite(*v))
            || !finite(self.rotation_deg)
            || !(finite(self.weld_tolerance_mm) && self.weld_tolerance_mm > 0.)
        {
            return Err(invalid("Import provenance placement is invalid"));
        }
        if self.layers.len() > 4096
            || self.curves.len() > 512
            || self.unsupported.len() > super::MAX_DIAGNOSTIC_KINDS + 1
            || self.curves.iter().any(|c| c.is_nil())
        {
            return Err(invalid("Import provenance exceeds its record limits"));
        }
        Ok(())
    }
    pub fn definition(&self) -> Result<Definition, DocumentError> {
        self.validate()?;
        Ok(Definition {
            type_id: PROVENANCE_TYPE.into(),
            schema_version: 1,
            data: serde_json::to_value(self).map_err(|e| invalid(e.to_string()))?,
        })
    }
    pub fn entity(&self, sketch: EntityId) -> Result<Entity, DocumentError> {
        Ok(Entity::new(
            format!("Import {}", self.file_name),
            EntityData::Extension {
                definition: self.definition()?,
                inputs: vec![Link::new(sketch, SKETCH_ROLE)],
            },
        ))
    }
    pub fn read(document: &Document, id: EntityId) -> Result<(Self, EntityId), DocumentError> {
        let Some(EntityData::Extension { definition, inputs }) = document.get(id).map(|e| &e.data)
        else {
            return Err(invalid("Import provenance does not exist"));
        };
        if definition.type_id != PROVENANCE_TYPE || definition.schema_version != 1 {
            return Err(invalid("Unsupported import provenance version"));
        }
        let record: Self = serde_json::from_value(definition.data.clone())
            .map_err(|e| invalid(format!("Invalid import provenance: {e}")))?;
        record.validate()?;
        let [link] = inputs.as_slice() else {
            return Err(invalid("Import provenance needs exactly one sketch link"));
        };
        if link.role != SKETCH_ROLE || link.geometry.is_some() {
            return Err(invalid("Import provenance needs exactly one sketch link"));
        }
        if !matches!(
            document.get(link.target).map(|e| &e.data),
            Some(EntityData::Sketch { .. })
        ) {
            return Err(invalid("Import provenance must link to a sketch"));
        }
        Ok((record, link.target))
    }
}

pub fn records(document: &Document) -> impl Iterator<Item = EntityId> + '_ {
    document.entities().filter_map(|e| {
        matches!(&e.data, EntityData::Extension { definition, .. } if definition.type_id == PROVENANCE_TYPE)
            .then_some(e.id)
    })
}

/// Records for one sketch in document order.
pub fn for_sketch(document: &Document, sketch: EntityId) -> Vec<(EntityId, Provenance)> {
    records(document)
        .filter_map(|id| {
            Provenance::read(document, id)
                .ok()
                .filter(|(_, s)| *s == sketch)
                .map(|(p, _)| (id, p))
        })
        .collect()
}

pub fn required_capability(document: &Document) -> bool {
    records(document).next().is_some()
}

pub(crate) fn validate(document: &Document) -> Result<(), DocumentError> {
    for id in records(document) {
        Provenance::read(document, id)?;
    }
    Ok(())
}
