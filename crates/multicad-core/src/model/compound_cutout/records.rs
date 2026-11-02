use super::super::{
    BodyInput, BodyOutput, PlanRectangle,
    common::parameter_links,
    source::{check_dimension, invalid},
};
use crate::{
    constraint_sketch::Dimension,
    document::{Definition, Document, DocumentError, EntityData, EntityId, Link},
    parameters::Unit,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub const COMPOUND_CUTOUT_TYPE: &str = "architecture.compound_cutout";
pub const COMPOUND_CUTOUT_RECEIVER_TYPE: &str = "architecture.compound_cutout_receiver";
pub const COMPOUND_CUTOUT_SCHEMA: u32 = 1;
pub const COMPOUND_CUTOUT_CAPABILITY: &str = "architecture.compound_cutout.1";
pub const COMPOUND_CUTOUT_RECEIVER_CAPABILITY: &str = "architecture.compound_cutout_receiver.1";
pub const MAX_COMPOUND_CUTOUT_BOXES: usize = 32;
pub const MAX_COMPOUND_CUTOUT_RECEIVERS: usize = 64;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CutoutZRange {
    pub bottom: Dimension,
    pub top: Dimension,
}
impl CutoutZRange {
    pub(super) fn dimensions(&self) -> Vec<&Dimension> {
        vec![&self.bottom, &self.top]
    }
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "range",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum CompoundCutoutExtent {
    #[default]
    ThroughReceivers,
    Fixed(CutoutZRange),
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompoundCutoutBox {
    pub id: EntityId,
    pub rectangle: PlanRectangle,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vertical: Option<CutoutZRange>,
}
impl CompoundCutoutBox {
    pub fn new(rectangle: PlanRectangle) -> Self {
        Self {
            id: EntityId::new(),
            rectangle,
            vertical: None,
        }
    }
    pub(super) fn dimensions(&self) -> Vec<&Dimension> {
        let r = &self.rectangle;
        let mut result = vec![&r.center[0], &r.center[1], &r.width, &r.depth];
        if let Some(z) = &self.vertical {
            result.extend(z.dimensions());
        }
        result
    }
}
/// Allocate `feature` once for each receiving Part, including before Preview.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompoundCutoutReceiver {
    pub target: BodyInput,
    pub feature: EntityId,
}
impl CompoundCutoutReceiver {
    pub fn new(target: BodyInput) -> Self {
        Self {
            target,
            feature: EntityId::new(),
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CutoutLevelRange {
    pub from: EntityId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to: Option<EntityId>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompoundCutoutDefinition {
    pub boxes: Vec<CompoundCutoutBox>,
    pub receivers: Vec<CompoundCutoutReceiver>,
    #[serde(default)]
    pub extent: CompoundCutoutExtent,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub levels: Option<CutoutLevelRange>,
}
impl CompoundCutoutDefinition {
    pub fn validate(&self) -> Result<(), DocumentError> {
        if self.boxes.is_empty() || self.boxes.len() > MAX_COMPOUND_CUTOUT_BOXES {
            return Err(invalid("A compound cutout needs 1–32 boxes"));
        }
        if self.receivers.is_empty() || self.receivers.len() > MAX_COMPOUND_CUTOUT_RECEIVERS {
            return Err(invalid("Choose 1–64 explicit compound cutout receivers"));
        }
        let mut boxes = BTreeSet::new();
        for item in &self.boxes {
            if item.id.is_nil() || !boxes.insert(item.id) {
                return Err(invalid("Cutout box identities must be non-nil and unique"));
            }
            if !item.rectangle.rotation_deg.is_finite() {
                return Err(invalid("Cutout box rotation must be finite"));
            }
            for dimension in item.dimensions() {
                check_dimension(dimension, Unit::Millimeter)?;
            }
        }
        if let CompoundCutoutExtent::Fixed(z) = &self.extent {
            for dimension in z.dimensions() {
                check_dimension(dimension, Unit::Millimeter)?;
            }
        }
        let mut bodies = BTreeSet::new();
        let mut features = BTreeSet::new();
        for receiver in &self.receivers {
            if receiver.feature.is_nil()
                || receiver.target.body.is_nil()
                || receiver.target.producer.is_nil()
                || receiver.feature == receiver.target.body
                || receiver.feature == receiver.target.producer
                || receiver.target.body == receiver.target.producer
                || !bodies.insert(receiver.target.body)
                || !features.insert(receiver.feature)
            {
                return Err(invalid(
                    "Compound cutout receivers require distinct explicit Parts and generated feature identities",
                ));
            }
        }
        if self.receivers.iter().any(|receiver| {
            boxes.contains(&receiver.feature)
                || boxes.contains(&receiver.target.body)
                || boxes.contains(&receiver.target.producer)
                || features.contains(&receiver.target.body)
                || features.contains(&receiver.target.producer)
        }) {
            return Err(invalid(
                "Cutout boxes, generated cuts and receiver inputs need distinct identities",
            ));
        }
        if self
            .levels
            .as_ref()
            .is_some_and(|range| range.from.is_nil() || range.to.is_some_and(|id| id.is_nil()))
        {
            return Err(invalid("Cutout levels need explicit non-nil identities"));
        }
        Ok(())
    }
    pub fn definition(&self) -> Result<Definition, DocumentError> {
        self.validate()?;
        encode(COMPOUND_CUTOUT_TYPE, self)
    }
    pub fn from_definition(definition: &Definition) -> Result<Self, DocumentError> {
        let source: Self = decode(COMPOUND_CUTOUT_TYPE, definition)?;
        source.validate()?;
        Ok(source)
    }
    pub fn inputs(&self) -> Result<Vec<Link>, DocumentError> {
        self.validate()?;
        let mut dimensions: Vec<_> = self
            .boxes
            .iter()
            .flat_map(CompoundCutoutBox::dimensions)
            .collect();
        if let CompoundCutoutExtent::Fixed(z) = &self.extent {
            dimensions.extend(z.dimensions());
        }
        let mut links = parameter_links(dimensions);
        links.extend(self.receivers.iter().map(|receiver| {
            Link::new(
                receiver.target.producer,
                format!("receiver/{}/producer", receiver.target.body),
            )
        }));
        if let Some(range) = &self.levels {
            links.push(Link::new(range.from, "from_level"));
            if let Some(to) = range.to {
                links.push(Link::new(to, "to_level"));
            }
        }
        Ok(links)
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompoundCutoutReceiverDefinition {
    pub source: EntityId,
    pub target: BodyInput,
    pub output: BodyOutput,
}
impl CompoundCutoutReceiverDefinition {
    pub fn definition(&self) -> Result<Definition, DocumentError> {
        if self.source.is_nil()
            || self.target.body.is_nil()
            || self.target.producer.is_nil()
            || self.target.body == self.target.producer
            || self.output.body != self.target.body
            || self.source == self.target.producer
            || self.source == self.target.body
        {
            return Err(invalid(
                "A generated cut must retain its receiving Part and explicit compound source",
            ));
        }
        encode(COMPOUND_CUTOUT_RECEIVER_TYPE, self)
    }
    pub fn from_definition(definition: &Definition) -> Result<Self, DocumentError> {
        let source: Self = decode(COMPOUND_CUTOUT_RECEIVER_TYPE, definition)?;
        source.definition()?;
        Ok(source)
    }
    pub fn inputs(&self) -> Vec<Link> {
        vec![
            Link::new(self.source, "compound_cutout"),
            Link::new(self.target.producer, "receiver_producer"),
        ]
    }
}
fn encode(kind: &str, value: &impl Serialize) -> Result<Definition, DocumentError> {
    Ok(Definition {
        type_id: kind.into(),
        schema_version: COMPOUND_CUTOUT_SCHEMA,
        data: serde_json::to_value(value).map_err(|e| invalid(e.to_string()))?,
    })
}
fn decode<T: serde::de::DeserializeOwned>(
    kind: &str,
    definition: &Definition,
) -> Result<T, DocumentError> {
    if definition.type_id != kind || definition.schema_version != COMPOUND_CUTOUT_SCHEMA {
        return Err(invalid("Unsupported compound cutout source/version"));
    }
    serde_json::from_value(definition.data.clone()).map_err(|e| invalid(e.to_string()))
}
pub(super) fn feature(
    document: &Document,
    id: EntityId,
) -> Result<(&Definition, &[Link]), DocumentError> {
    match document.get(id).map(|entity| &entity.data) {
        Some(EntityData::Feature {
            definition,
            inputs,
            suppressed: false,
        }) => Ok((definition, inputs)),
        _ => Err(invalid(format!(
            "Compound cutout source {id} is missing, suppressed or invalid; repair it"
        ))),
    }
}
fn links_match(actual: &[Link], expected: &[Link]) -> bool {
    actual.len() == expected.len() && expected.iter().all(|link| actual.contains(link))
}
pub fn compound_cutouts(document: &Document) -> impl Iterator<Item = &crate::document::Entity> {
    document.entities().filter(|entity|matches!(&entity.data,EntityData::Feature {definition,..} if definition.type_id==COMPOUND_CUTOUT_TYPE))
}
pub fn read_compound_cutout(
    document: &Document,
    id: EntityId,
) -> Result<CompoundCutoutDefinition, DocumentError> {
    let (definition, inputs) = feature(document, id)?;
    let source = CompoundCutoutDefinition::from_definition(definition)?;
    if !links_match(inputs, &source.inputs()?) {
        return Err(invalid(
            "Compound cutout source and dependency links disagree; repair the source",
        ));
    }
    if source.boxes.iter().any(|item| item.id == id)
        || source.receivers.iter().any(|receiver| {
            [
                receiver.feature,
                receiver.target.body,
                receiver.target.producer,
            ]
            .contains(&id)
        })
    {
        return Err(invalid(
            "Compound cutout identity collides with a box or receiver",
        ));
    }
    if document.entities().any(
        |entity| matches!(&entity.data,EntityData::Part {features,..}if features.contains(&id)),
    ) {
        return Err(invalid(
            "Compound cutter source must not be an independent final Part",
        ));
    }
    let members: BTreeSet<_> = source
        .receivers
        .iter()
        .map(|receiver| receiver.feature)
        .collect();
    for receiver in &source.receivers {
        receiver.target.resolve_producer(document)?;
        super::resolve::receiver_base(document, &receiver.target)?;
        let (definition, inputs) = feature(document, receiver.feature)?;
        let member = CompoundCutoutReceiverDefinition::from_definition(definition)?;
        if member.source != id
            || member.target != receiver.target
            || member.output.body != receiver.target.body
            || !links_match(inputs, &member.inputs())
        {
            return Err(invalid(format!(
                "Compound cutout receiver {} is inconsistent; repair its generated cut",
                receiver.target.body
            )));
        }
        let Some(EntityData::Part { features, .. }) = document
            .get(receiver.target.body)
            .map(|entity| &entity.data)
        else {
            unreachable!("resolved Part")
        };
        let before = features
            .iter()
            .position(|feature| *feature == receiver.target.producer);
        let after = features
            .iter()
            .position(|feature| *feature == receiver.feature);
        if !matches!((before,after),(Some(before),Some(after))if before<after) {
            return Err(invalid(
                "Generated cut must follow its frozen input in the receiving Part history",
            ));
        }
        if features[before.expect("checked producer") + 1..after.expect("checked member")]
            .iter()
            .any(|id| {
                matches!(
                    document.get(*id).map(|entity| &entity.data),
                    Some(EntityData::Feature {
                        suppressed: false,
                        ..
                    })
                )
            })
        {
            return Err(invalid(
                "Generated cut bypasses an intervening producer; repair its frozen receiver history",
            ));
        }
        if document.entities().any(|entity|entity.id!=receiver.target.body&&matches!(&entity.data,EntityData::Part {features,..}if features.contains(&receiver.feature))) {
            return Err(invalid("A generated receiver cut is attached to another Part; repair its ownership"));
        }
    }
    for entity in document.entities() {
        if let EntityData::Feature { definition, .. } = &entity.data
            && definition.type_id == COMPOUND_CUTOUT_RECEIVER_TYPE
        {
            let member = CompoundCutoutReceiverDefinition::from_definition(definition)?;
            if member.source == id && !members.contains(&entity.id) {
                return Err(invalid(
                    "Compound cutout has an unregistered generated receiver; repair its source",
                ));
            }
        }
    }
    Ok(source)
}
pub fn read_compound_cutout_receiver(
    document: &Document,
    id: EntityId,
) -> Result<CompoundCutoutReceiverDefinition, DocumentError> {
    let (definition, inputs) = feature(document, id)?;
    let member = CompoundCutoutReceiverDefinition::from_definition(definition)?;
    if !links_match(inputs, &member.inputs()) {
        return Err(invalid(
            "Generated cut dependency links disagree with its source",
        ));
    }
    let source = read_compound_cutout(document, member.source)?;
    if !source
        .receivers
        .iter()
        .any(|receiver| receiver.feature == id && receiver.target == member.target)
    {
        return Err(invalid(
            "Generated cut is absent from its compound definition; repair the source",
        ));
    }
    Ok(member)
}
pub fn compound_cutout_source(document: &Document, id: EntityId) -> Option<EntityId> {
    let (definition, _) = feature(document, id).ok()?;
    if definition.type_id == COMPOUND_CUTOUT_TYPE {
        read_compound_cutout(document, id).ok().map(|_| id)
    } else if definition.type_id == COMPOUND_CUTOUT_RECEIVER_TYPE {
        read_compound_cutout_receiver(document, id)
            .ok()
            .map(|member| member.source)
    } else {
        None
    }
}
