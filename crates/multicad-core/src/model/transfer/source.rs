use super::*;
use crate::{
    document::{Definition, Document, DocumentError, EntityData, EntityId, Link},
    model::{AlignmentInput, common::parameter_links, source::invalid},
    references::TopologyKind,
};
use std::collections::BTreeSet;
pub const TRANSFER_POSE_ROLE: &str = "transfer/pose/";
fn path_valid(path: &crate::model::OccurrencePath) -> Result<(), DocumentError> {
    if path.root.is_nil()
        || path.members.len() > 32
        || path.members.iter().any(|id| id.is_nil())
        || path.members.iter().collect::<BTreeSet<_>>().len() != path.members.len()
    {
        return Err(invalid(
            "Transfer occurrence paths need a valid root and at most 32 distinct canonical member IDs",
        ));
    }
    Ok(())
}
impl TransferDefinition {
    pub fn validate(&self) -> Result<(), DocumentError> {
        if self.source.component.is_nil() {
            return Err(invalid("Transfer source needs an explicit Part definition"));
        }
        path_valid(&self.source.path)?;
        let captures = self.source.selection.captures();
        if captures.is_empty()
            || captures.len() > MAX_TRANSFER_SOURCES
            || captures.iter().any(|id| id.is_nil())
            || captures.iter().collect::<BTreeSet<_>>().len() != captures.len()
        {
            return Err(invalid("Select 1..256 distinct source captures"));
        }
        if self.targets.is_empty() || self.targets.len() > MAX_TRANSFER_TARGETS {
            return Err(invalid("Choose 1..64 explicit receiving Parts"));
        }
        let mut rows = BTreeSet::new();
        let mut bodies = BTreeSet::new();
        let mut features = BTreeSet::new();
        let mut refs = captures.into_iter().collect::<BTreeSet<_>>();
        for target in &self.targets {
            path_valid(&target.path)?;
            target.parameters.validate()?;
            if target.id.is_nil()
                || target.feature.is_nil()
                || target.id == target.feature
                || target.input.body.is_nil()
                || target.input.producer.is_nil()
                || target.input.body == target.input.producer
                || target.input.body == self.source.component
                || !rows.insert(target.id)
                || !bodies.insert(target.input.body)
                || !features.insert(target.feature)
                || target.face.is_nil()
                || !refs.insert(target.face)
            {
                return Err(invalid(
                    "Transfer targets need distinct row/feature/Part/face identities and independent source hardware; repeated target definitions are unsupported",
                ));
            }
            if matches!(
                self.source.selection,
                TransferSelection::FaceBoundary { .. }
            ) && target.parameters.diameter.is_some()
            {
                return Err(invalid(
                    "Diameter overrides apply to holes; cutout boundaries use radial clearance only",
                ));
            }
        }
        Ok(())
    }
    pub fn definition(&self) -> Result<Definition, DocumentError> {
        self.validate()?;
        Ok(Definition {
            type_id: TRANSFER_GROUP_TYPE.into(),
            schema_version: 1,
            data: serde_json::to_value(self).map_err(|e| invalid(e.to_string()))?,
        })
    }
    pub fn from_definition(definition: &Definition) -> Result<Self, DocumentError> {
        if definition.type_id != TRANSFER_GROUP_TYPE || definition.schema_version != 1 {
            return Err(invalid("Unsupported transfer group provider/version"));
        }
        let s: Self =
            serde_json::from_value(definition.data.clone()).map_err(|e| invalid(e.to_string()))?;
        s.validate()?;
        Ok(s)
    }
    pub fn inputs(&self, references: &[AlignmentInput]) -> Result<Vec<Link>, DocumentError> {
        self.validate()?;
        let mut links = parameter_links(
            self.targets
                .iter()
                .flat_map(|t| t.parameters.dimensions())
                .collect::<Vec<_>>(),
        );
        let mut seen = BTreeSet::new();
        for id in self.source.selection.captures() {
            let input = references
                .iter()
                .find(|r| r.intent.id == id)
                .ok_or_else(|| invalid("A selected source capture is missing"))?;
            input.intent.validate().map_err(invalid)?;
            if input.intent.attachment.is_some() {
                return Err(invalid(
                    "Transfer source captures use exact native geometry; attachment offsets/roll are unsupported",
                ));
            }
            if input.source != self.source.component {
                return Err(invalid(
                    "Source captures must bind the source Part alias, not an occurrence/world recapture",
                ));
            }
            if !matches!(self.source.selection, TransferSelection::Circles { .. })
                && input.intent.kind != TopologyKind::Face
            {
                return Err(invalid("Face-driven transfer needs a captured source face"));
            }
            if matches!(self.source.selection, TransferSelection::Circles { .. })
                && !matches!(input.intent.kind, TopologyKind::Edge | TopologyKind::Face)
            {
                return Err(invalid(
                    "Explicit holes need circular rim edges or analytic cylinder faces",
                ));
            }
            seen.insert(id);
            links.push(Link {
                target: input.source,
                role: format!("transfer/source/{id}"),
                geometry: Some(input.intent.clone()),
            });
        }
        for target in &self.targets {
            let input = references
                .iter()
                .find(|r| r.intent.id == target.face)
                .ok_or_else(|| invalid("A receiving face capture is missing"))?;
            input.intent.validate().map_err(invalid)?;
            if input.intent.attachment.is_some() {
                return Err(invalid(
                    "Transfer receiving faces use their native plane; attachment offsets/roll are unsupported",
                ));
            }
            if input.source != target.input.producer || input.intent.kind != TopologyKind::Face {
                return Err(invalid(
                    "Receiving face captures must bind the frozen prior solid producer",
                ));
            }
            seen.insert(target.face);
            links.push(Link {
                target: input.source,
                role: format!("transfer/target/{}/face", target.id),
                geometry: Some(input.intent.clone()),
            });
        }
        if references.len() != seen.len() || references.iter().any(|r| !seen.contains(&r.intent.id))
        {
            return Err(invalid(
                "Transfer capture inputs must exactly match unique source and receiving references",
            ));
        }
        for (role, path) in std::iter::once(("source".to_owned(), &self.source.path)).chain(
            self.targets
                .iter()
                .map(|t| (format!("target/{}", t.id), &t.path)),
        ) {
            for (index, id) in std::iter::once(path.root)
                .chain(path.members.iter().copied())
                .enumerate()
            {
                links.push(Link::new(id, format!("{TRANSFER_POSE_ROLE}{role}/{index}")));
            }
        }
        Ok(links)
    }
}
pub fn transfer_group(document: &Document, id: EntityId) -> Result<EntityId, DocumentError> {
    match document.get(id).map(|e| &e.data) {
        Some(EntityData::Extension { definition, .. })
            if definition.type_id == TRANSFER_GROUP_TYPE =>
        {
            Ok(id)
        }
        Some(EntityData::Feature { definition, .. }) if definition.type_id == TRANSFER_TYPE => {
            Ok(TransferFeatureDefinition::from_definition(definition)?.group)
        }
        _ => Err(invalid(
            "Choose a transfer group or its receiving output feature",
        )),
    }
}
/// Stored bindings stay readable when an upstream occurrence/member disappears;
/// binding resolution belongs to native preview and repair, not file loading.
pub fn read_transfer(
    document: &Document,
    id: EntityId,
) -> Result<(TransferDefinition, Vec<AlignmentInput>), DocumentError> {
    let group = transfer_group(document, id)?;
    let Some(EntityData::Extension { definition, inputs }) = document.get(group).map(|e| &e.data)
    else {
        return Err(invalid("Transfer group is missing"));
    };
    let s = TransferDefinition::from_definition(definition)?;
    let refs = inputs
        .iter()
        .filter_map(|l| {
            l.geometry.as_ref().map(|intent| AlignmentInput {
                source: l.target,
                intent: intent.clone(),
            })
        })
        .collect::<Vec<_>>();
    let expected = s.inputs(&refs)?;
    if inputs.len() != expected.len() || expected.iter().any(|l| !inputs.contains(l)) {
        return Err(invalid(
            "Stored transfer payload and canonical reference/pose links disagree",
        ));
    }
    Ok((s, refs))
}
impl TransferFeatureDefinition {
    pub fn definition(&self) -> Result<Definition, DocumentError> {
        if self.group.is_nil() || self.target.is_nil() {
            return Err(invalid("Transfer output needs group and row identities"));
        }
        Ok(Definition {
            type_id: TRANSFER_TYPE.into(),
            schema_version: 1,
            data: serde_json::to_value(self).map_err(|e| invalid(e.to_string()))?,
        })
    }
    pub fn from_definition(definition: &Definition) -> Result<Self, DocumentError> {
        if definition.type_id != TRANSFER_TYPE || definition.schema_version != 1 {
            return Err(invalid("Unsupported transfer output provider/version"));
        }
        let source: Self =
            serde_json::from_value(definition.data.clone()).map_err(|e| invalid(e.to_string()))?;
        source.definition()?;
        Ok(source)
    }
}
pub fn read_transfer_feature(
    document: &Document,
    id: EntityId,
) -> Result<
    (
        EntityId,
        TransferDefinition,
        TransferTarget,
        Vec<AlignmentInput>,
    ),
    DocumentError,
> {
    let Some(EntityData::Feature {
        definition, inputs, ..
    }) = document.get(id).map(|e| &e.data)
    else {
        return Err(invalid("Transfer receiving output is missing"));
    };
    let feature = TransferFeatureDefinition::from_definition(definition)?;
    let (group, refs) = read_transfer(document, feature.group)?;
    let target = group
        .targets
        .iter()
        .find(|t| t.id == feature.target && t.feature == id)
        .ok_or_else(|| invalid("Transfer output no longer matches its stable group row"))?
        .clone();
    let expected = [
        Link::new(feature.group, "transfer_group"),
        Link::new(target.input.producer, "prior_solid"),
    ];
    if inputs.len() != expected.len() || expected.iter().any(|l| !inputs.contains(l)) {
        return Err(invalid(
            "Transfer output and canonical group/prior-solid links disagree",
        ));
    }
    if let Some(EntityData::Part { features, .. }) =
        document.get(target.input.body).map(|e| &e.data)
    {
        let prior = features.iter().position(|id| *id == target.input.producer);
        let output = features.iter().position(|feature| *feature == id);
        if document.get(target.input.producer).is_some() && prior.is_none() {
            return Err(invalid(
                "Frozen transfer producer is not in its receiving Part history",
            ));
        }
        if output.is_none() || matches!((prior,output),(Some(prior),Some(output))if prior>=output) {
            return Err(invalid(
                "Transfer output must follow its frozen producer in the same receiving Part history",
            ));
        }
    }
    Ok((feature.group, group, target, refs))
}
impl TransferRequest {
    pub fn existing(document: &Document, id: EntityId) -> Result<Self, DocumentError> {
        let group = transfer_group(document, id)?;
        let (source, references) = read_transfer(document, group)?;
        Ok(Self {
            group,
            name: document.get(group).expect("read transfer").name.clone(),
            source: source.source,
            targets: source.targets,
            references,
        })
    }
    pub fn source(&self) -> Result<TransferDefinition, DocumentError> {
        let s = TransferDefinition {
            source: self.source.clone(),
            targets: self.targets.clone(),
        };
        s.inputs(&self.references)?;
        Ok(s)
    }
}
pub(crate) fn validate_document(document: &Document) -> Result<(), DocumentError> {
    for entity in document.entities() {
        match &entity.data {
            EntityData::Extension { definition, .. }
                if definition.type_id == TRANSFER_GROUP_TYPE =>
            {
                read_transfer(document, entity.id)?;
            }
            EntityData::Feature { definition, .. } if definition.type_id == TRANSFER_TYPE => {
                read_transfer_feature(document, entity.id)?;
            }
            _ => {}
        }
    }
    Ok(())
}
