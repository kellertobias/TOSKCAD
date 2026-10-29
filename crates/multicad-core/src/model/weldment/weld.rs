//! Weld definitions and authored weld sequences. A weld is an extension
//! attached to one member connection, with a unique mark (its ID on
//! drawings and schedules), a symbol, size and process annotations, and
//! access or orientation notes. A sequence lists welds in the order the
//! fabricator should make them. Both are documentation: nothing here
//! computes distortion, strength, heat input or structural safety.
use super::*;
use crate::drawings::{WeldSide, WeldSymbol};

pub const WELD_KIND: &str = "weldment.weld";
pub const WELD_CAPABILITY: &str = "weldment.weld.1";
pub const SEQUENCE_KIND: &str = "weldment.sequence";
pub const SEQUENCE_CAPABILITY: &str = "weldment.sequence.1";
pub const MAX_SEQUENCE_STEPS: usize = 512;
const MAX_TEXT: usize = 1000;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WeldDefinition {
    /// The member connection this weld joins.
    pub connection: EntityId,
    /// Weld ID shown on drawings and schedules, unique in the document.
    pub mark: String,
    pub symbol: WeldSymbol,
    #[serde(default = "arrow_side")]
    pub side: WeldSide,
    /// Authored size, for example a fillet leg.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<Dimension>,
    /// Authored weld length; none for the full joint.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub length: Option<Dimension>,
    #[serde(default)]
    pub all_around: bool,
    /// Made on site rather than in the shop.
    #[serde(default)]
    pub field: bool,
    /// Process annotation, for example "135 MAG".
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub process: String,
    /// Access, position and orientation notes.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub notes: String,
}
fn arrow_side() -> WeldSide {
    WeldSide::Arrow
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SequenceStep {
    pub weld: EntityId,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub note: String,
}
/// Welds in authored order. Reordering keeps each weld and its connection.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SequenceDefinition {
    pub steps: Vec<SequenceStep>,
}

fn text_ok(value: &str, what: &str, max: usize) -> Result<(), DocumentError> {
    if value.len() > max || value.chars().any(|c| c.is_control() && c != '\n') {
        return Err(invalid(format!(
            "{what} is longer than {max} bytes or contains control characters"
        )));
    }
    Ok(())
}
impl WeldDefinition {
    pub fn validate(&self) -> Result<(), DocumentError> {
        if self.connection.is_nil() {
            return Err(invalid("A weld needs its member connection"));
        }
        if self.mark.trim().is_empty() {
            return Err(invalid("A weld needs a mark, for example W1"));
        }
        text_ok(&self.mark, "The weld mark", 32)?;
        text_ok(&self.process, "The weld process", 256)?;
        text_ok(&self.notes, "The weld notes", MAX_TEXT)?;
        for d in self.size.iter().chain(&self.length) {
            check_dimension(d, Unit::Millimeter)?;
        }
        Ok(())
    }
    fn inputs(&self) -> Vec<Link> {
        let mut links = parameter_links(self.size.iter().chain(&self.length).collect());
        links.push(Link::new(self.connection, "connection"));
        links
    }
    pub(crate) fn entity_data(&self) -> Result<EntityData, DocumentError> {
        self.validate()?;
        Ok(EntityData::Extension {
            definition: definition(WELD_KIND, self)?,
            inputs: self.inputs(),
        })
    }
}
impl SequenceDefinition {
    pub fn validate(&self) -> Result<(), DocumentError> {
        if self.steps.len() > MAX_SEQUENCE_STEPS {
            return Err(invalid(format!(
                "A weld sequence holds at most {MAX_SEQUENCE_STEPS} steps"
            )));
        }
        let mut seen = BTreeSet::new();
        for step in &self.steps {
            if step.weld.is_nil() || !seen.insert(step.weld) {
                return Err(invalid("Each weld appears once in a sequence"));
            }
            text_ok(&step.note, "A sequence note", MAX_TEXT)?;
        }
        Ok(())
    }
    fn inputs(&self) -> Vec<Link> {
        self.steps
            .iter()
            .enumerate()
            .map(|(i, s)| Link::new(s.weld, format!("step/{i}")))
            .collect()
    }
    pub(crate) fn entity_data(&self) -> Result<EntityData, DocumentError> {
        self.validate()?;
        Ok(EntityData::Extension {
            definition: definition(SEQUENCE_KIND, self)?,
            inputs: self.inputs(),
        })
    }
}

fn of_kind<'a>(document: &'a Document, kind: &'a str) -> impl Iterator<Item = &'a Entity> {
    document.entities().filter(
        move |e| matches!(&e.data, EntityData::Extension { definition, .. } if definition.type_id == kind),
    )
}
pub fn welds(document: &Document) -> impl Iterator<Item = &Entity> {
    of_kind(document, WELD_KIND)
}
pub fn sequences(document: &Document) -> impl Iterator<Item = &Entity> {
    of_kind(document, SEQUENCE_KIND)
}
fn extension<'a>(
    document: &'a Document,
    id: EntityId,
    kind: &str,
) -> Result<&'a Definition, DocumentError> {
    match document.get(id).map(|e| &e.data) {
        Some(EntityData::Extension { definition, .. }) if definition.type_id == kind => {
            Ok(definition)
        }
        Some(_) => Err(invalid(format!("{id} is not a {kind}"))),
        None => Err(DocumentError::Missing(id)),
    }
}
pub fn read_weld(document: &Document, id: EntityId) -> Result<WeldDefinition, DocumentError> {
    let value: WeldDefinition = parse(extension(document, id, WELD_KIND)?, WELD_KIND)?;
    value.validate()?;
    Ok(value)
}
pub fn read_sequence(
    document: &Document,
    id: EntityId,
) -> Result<SequenceDefinition, DocumentError> {
    let value: SequenceDefinition = parse(extension(document, id, SEQUENCE_KIND)?, SEQUENCE_KIND)?;
    value.validate()?;
    Ok(value)
}
pub(super) fn validate_welds(document: &Document) -> Result<(), DocumentError> {
    let mut marks = BTreeSet::new();
    for e in welds(document) {
        let w = read_weld(document, e.id)?;
        if document.get(w.connection).is_some() {
            read_connection(document, w.connection)?;
        }
        if !marks.insert(w.mark.trim().to_owned()) {
            return Err(invalid(format!(
                "Weld mark {} is used twice; marks identify welds on drawings",
                w.mark
            )));
        }
    }
    let mut sequenced = BTreeSet::new();
    for e in sequences(document) {
        let s = read_sequence(document, e.id)?;
        for step in &s.steps {
            if document.get(step.weld).is_some() {
                read_weld(document, step.weld)?;
            }
            if !sequenced.insert(step.weld) {
                return Err(invalid(format!(
                    "A weld appears in more than one sequence; {} repeats it",
                    e.name
                )));
            }
        }
    }
    Ok(())
}
