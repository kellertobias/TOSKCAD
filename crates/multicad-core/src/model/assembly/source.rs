use super::*;
use crate::{
    document::{Definition, Document, DocumentError, EntityData, EntityId, Link},
    model::{AlignmentInput, PlacementBase, common::parameter_links, source::invalid},
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JointDefinition {
    pub owner: EntityId,
    pub component: EntityId,
    pub base: PlacementBase,
    pub parameters: JointParameters,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssemblyDefinition {
    pub assembly: EntityId,
    pub definition_parts: Vec<EntityId>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JointPoseValue {
    pub joint: EntityId,
    pub values: JointValues,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NamedPoseDefinition {
    pub values: Vec<JointPoseValue>,
}
fn definition<T: Serialize>(kind: &str, source: &T) -> Result<Definition, DocumentError> {
    Ok(Definition {
        type_id: kind.into(),
        schema_version: 1,
        data: serde_json::to_value(source).map_err(|e| invalid(e.to_string()))?,
    })
}
fn decode<T: for<'de> Deserialize<'de>>(d: &Definition, kind: &str) -> Result<T, DocumentError> {
    if d.type_id != kind || d.schema_version != 1 {
        return Err(invalid("Unsupported assembly provider/version"));
    }
    serde_json::from_value(d.data.clone()).map_err(|e| invalid(e.to_string()))
}
impl JointDefinition {
    pub fn definition(&self) -> Result<Definition, DocumentError> {
        self.parameters.validate()?;
        if self.owner.is_nil()
            || self.component.is_nil()
            || self.parameters.parent == Some(self.owner)
        {
            return Err(invalid(
                "Joint owner, component and parent must be valid and distinct",
            ));
        }
        match self.base {
            PlacementBase::Literal { pose } => pose.validate()?,
            PlacementBase::Previous { driver } if driver.is_nil() || driver == self.owner => {
                return Err(invalid("Joint preceding driver is invalid"));
            }
            _ => {}
        }
        definition(JOINT_TYPE, self)
    }
    pub fn from_definition(d: &Definition) -> Result<Self, DocumentError> {
        let source: Self = decode(d, JOINT_TYPE)?;
        source.definition()?;
        Ok(source)
    }
    pub fn inputs(&self, references: &[AlignmentInput]) -> Result<Vec<Link>, DocumentError> {
        self.definition()?;
        let mut links = parameter_links(self.parameters.dimensions());
        links.push(Link::new(self.component, "joint/component"));
        if let PlacementBase::Previous { driver } = self.base {
            links.push(Link::new(driver, "previous_pose"));
        }
        if let Some(parent) = self.parameters.parent {
            links.push(Link::new(parent, "joint/parent"));
        }
        for (anchor, role) in [
            (&self.parameters.moving, "moving"),
            (&self.parameters.target, "target"),
        ] {
            if let JointAnchor::GearPairFrame { pair, .. } = anchor {
                links.push(Link::new(*pair, format!("joint/gear_pair/{role}")));
            }
        }
        let wanted = [&self.parameters.moving, &self.parameters.target]
            .into_iter()
            .filter_map(|a| {
                if let JointAnchor::Geometry { reference } = a {
                    Some(*reference)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        if wanted.iter().collect::<BTreeSet<_>>().len() != wanted.len() {
            return Err(invalid(
                "Moving and target anchors need distinct capture identities",
            ));
        }
        let mut seen = BTreeSet::new();
        for input in references {
            input.intent.validate().map_err(invalid)?;
            if input.source.is_nil()
                || !wanted.contains(&input.intent.id)
                || !seen.insert(input.intent.id)
            {
                return Err(invalid(
                    "Joint inputs must exactly match their unique explicit anchor captures",
                ));
            }
            links.push(Link {
                target: input.source,
                role: format!("joint/reference/{}", input.intent.id),
                geometry: Some(input.intent.clone()),
            });
        }
        if seen.len() != wanted.len() {
            return Err(invalid("Joint anchor capture is missing"));
        }
        Ok(links)
    }
}
fn extension(
    document: &Document,
    id: EntityId,
) -> Result<(&Definition, &Vec<Link>), DocumentError> {
    match document.get(id).map(|e| &e.data) {
        Some(EntityData::Extension { definition, inputs }) => Ok((definition, inputs)),
        _ => Err(invalid("Assembly source is missing or is not an Extension")),
    }
}
fn exact_links(actual: &[Link], expected: &[Link]) -> Result<(), DocumentError> {
    if actual.len() != expected.len() || expected.iter().any(|l| !actual.contains(l)) {
        return Err(invalid(
            "Assembly payload and canonical dependency links disagree",
        ));
    }
    Ok(())
}
pub fn read_joint(
    document: &Document,
    id: EntityId,
) -> Result<(JointDefinition, Vec<AlignmentInput>), DocumentError> {
    let (d, links) = extension(document, id)?;
    let source = JointDefinition::from_definition(d)?;
    let references = links
        .iter()
        .filter_map(|l| {
            l.geometry.as_ref().map(|intent| AlignmentInput {
                source: l.target,
                intent: intent.clone(),
            })
        })
        .collect::<Vec<_>>();
    exact_links(links, &source.inputs(&references)?)?;
    for anchor in [&source.parameters.moving, &source.parameters.target] {
        if let JointAnchor::GearPairFrame { pair, .. } = anchor {
            let pair = crate::model::read_gear_pair(document, *pair)?;
            if !pair.outputs.joints.contains(&id) {
                return Err(invalid(
                    "A derived gear anchor belongs only to its pair-controlled relationship",
                ));
            }
        }
    }
    let Some(EntityData::Instance { component, .. }) = document.get(source.owner).map(|e| &e.data)
    else {
        return Err(invalid("Joint owner must be an existing Instance"));
    };
    if *component != source.component {
        return Err(invalid(
            "Joint component changed; explicitly repair its local anchor frame before continuing",
        ));
    }
    let parent_component = source
        .parameters
        .parent
        .map(|id| match document.get(id).map(|e| &e.data) {
            Some(EntityData::Instance { component, .. }) => Ok(*component),
            _ => Err(invalid("Joint parent must be an existing Instance")),
        })
        .transpose()?;
    if let Some(parent) = source.parameters.parent {
        let containing = |id| {
            document.entities().find_map(|e| {
                if let EntityData::Assembly { instances } = &e.data {
                    instances.contains(&id).then_some(e.id)
                } else {
                    None
                }
            })
        };
        if containing(parent) != containing(source.owner) {
            return Err(invalid(
                "Joint parent and moving Instance must share one assembly definition context",
            ));
        }
    }
    for (anchor, expected) in [
        (&source.parameters.moving, Some(source.component)),
        (&source.parameters.target, parent_component),
    ] {
        if let JointAnchor::Geometry { reference } = anchor {
            let input = references
                .iter()
                .find(|r| r.intent.id == *reference)
                .ok_or_else(|| invalid("Missing anchor input"))?;
            if expected.is_some_and(|id| id != input.source) {
                return Err(invalid(
                    "A local joint anchor must reference its own component topology",
                ));
            }
        }
    }
    Ok((source, references))
}
impl AssemblyDefinition {
    pub fn definition(&self) -> Result<Definition, DocumentError> {
        if self.assembly.is_nil()
            || self.definition_parts.len() > 4096
            || self.definition_parts.iter().any(|id| id.is_nil())
            || self.definition_parts.iter().collect::<BTreeSet<_>>().len()
                != self.definition_parts.len()
        {
            return Err(invalid(
                "Assembly definition Parts must be distinct, non-nil and bounded",
            ));
        }
        definition(ASSEMBLY_TYPE, self)
    }
    pub fn inputs(&self) -> Result<Vec<Link>, DocumentError> {
        self.definition()?;
        Ok(std::iter::once(Link::new(self.assembly, "assembly"))
            .chain(
                self.definition_parts
                    .iter()
                    .map(|id| Link::new(*id, "definition_part")),
            )
            .collect())
    }
}
pub fn read_assembly(
    document: &Document,
    id: EntityId,
) -> Result<AssemblyDefinition, DocumentError> {
    let (d, links) = extension(document, id)?;
    let source: AssemblyDefinition = decode(d, ASSEMBLY_TYPE)?;
    exact_links(links, &source.inputs()?)?;
    if !matches!(
        document.get(source.assembly).map(|e| &e.data),
        Some(EntityData::Assembly { .. })
    ) {
        return Err(invalid("Assembly definition must own an Assembly"));
    }
    for part in &source.definition_parts {
        if !matches!(
            document.get(*part).map(|e| &e.data),
            Some(EntityData::Part { .. })
        ) {
            return Err(invalid("Assembly definition source must be a Part"));
        }
    }
    let reachable = assembly_component_parts(document, source.assembly)?;
    if source
        .definition_parts
        .iter()
        .any(|id| !reachable.contains(id))
    {
        return Err(invalid(
            "Definition-only Parts must actually occur in this reusable Assembly",
        ));
    }
    Ok(source)
}
pub fn assembly_component_parts(
    document: &Document,
    component: EntityId,
) -> Result<BTreeSet<EntityId>, DocumentError> {
    let mut pending = vec![component];
    let mut visited = BTreeSet::new();
    let mut parts = BTreeSet::new();
    while let Some(id) = pending.pop() {
        if !visited.insert(id) {
            continue;
        }
        if visited.len() > 8192 {
            return Err(invalid(
                "Assembly component traversal exceeds its source budget",
            ));
        }
        match document.get(id).map(|e| &e.data) {
            Some(EntityData::Part { .. }) => {
                parts.insert(id);
            }
            Some(EntityData::Assembly { instances }) => {
                for member in instances {
                    let Some(EntityData::Instance { component, .. }) =
                        document.get(*member).map(|e| &e.data)
                    else {
                        return Err(invalid("Assembly member must be an existing Instance"));
                    };
                    pending.push(*component);
                }
            }
            _ => return Err(invalid("Choose an existing Part or Assembly component")),
        }
    }
    Ok(parts)
}
pub fn reusable_assemblies(document: &Document) -> Result<BTreeSet<EntityId>, DocumentError> {
    document.entities().filter(|e|matches!(&e.data,EntityData::Extension{definition,..}if definition.type_id==ASSEMBLY_TYPE)).map(|e|read_assembly(document,e.id).map(|s|s.assembly)).collect()
}
impl NamedPoseDefinition {
    pub fn definition(&self) -> Result<Definition, DocumentError> {
        if self.values.is_empty()
            || self.values.len() > 4096
            || self.values.iter().any(|v| v.joint.is_nil())
            || self
                .values
                .iter()
                .map(|v| v.joint)
                .collect::<BTreeSet<_>>()
                .len()
                != self.values.len()
        {
            return Err(invalid(
                "Named pose needs 1..4096 distinct joint identities",
            ));
        }
        for v in &self.values {
            v.values.validate()?;
        }
        definition(NAMED_POSE_TYPE, self)
    }
    pub fn inputs(&self) -> Result<Vec<Link>, DocumentError> {
        self.definition()?;
        let mut links = parameter_links(
            self.values
                .iter()
                .flat_map(|v| v.values.dimensions())
                .collect::<Vec<_>>(),
        );
        links.extend(self.values.iter().map(|v| Link::new(v.joint, "pose/joint")));
        Ok(links)
    }
}
pub fn read_named_pose(
    document: &Document,
    id: EntityId,
) -> Result<NamedPoseDefinition, DocumentError> {
    let (d, links) = extension(document, id)?;
    let source: NamedPoseDefinition = decode(d, NAMED_POSE_TYPE)?;
    exact_links(links, &source.inputs()?)?;
    for v in &source.values {
        read_joint(document, v.joint)?;
    }
    Ok(source)
}
pub fn assembly_definition_parts(document: &Document) -> Result<BTreeSet<EntityId>, DocumentError> {
    let mut parts = crate::model::screw_connection_definition_parts(document)?;
    let mut owners = BTreeSet::new();
    for entity in document.entities() {
        if matches!(&entity.data,EntityData::Extension{definition,..}if definition.type_id==ASSEMBLY_TYPE)
        {
            let source = read_assembly(document, entity.id)?;
            if !owners.insert(source.assembly) {
                return Err(invalid(
                    "An Assembly has more than one definition ownership record",
                ));
            }
            parts.extend(source.definition_parts);
        }
    }
    Ok(parts)
}
pub fn assembly_member_instances(document: &Document) -> Result<BTreeSet<EntityId>, DocumentError> {
    let mut members = BTreeSet::new();
    for entity in document.entities() {
        if let EntityData::Assembly { instances } = &entity.data {
            for member in instances {
                if !members.insert(*member) {
                    return Err(invalid(
                        "An Instance cannot belong to multiple reusable Assembly definitions",
                    ));
                }
            }
        }
    }
    Ok(members)
}
pub(crate) fn validate_document(document: &Document) -> Result<(), DocumentError> {
    assembly_definition_parts(document)?;
    if document.entities().any(|e|matches!(&e.data,EntityData::Extension{definition,..}if [ASSEMBLY_TYPE,JOINT_TYPE].contains(&definition.type_id.as_str()))){assembly_member_instances(document)?;}
    for entity in document.entities() {
        if let EntityData::Extension { definition, .. } = &entity.data {
            match definition.type_id.as_str() {
                JOINT_TYPE => {
                    let (source, _) = read_joint(document, entity.id)?;
                    validate_joint_parent_depth(document, source.owner, source.parameters.parent)?;
                }
                NAMED_POSE_TYPE => {
                    read_named_pose(document, entity.id)?;
                }
                _ => {}
            }
        }
    }
    if document.entities().any(
        |e| matches!(&e.data,EntityData::Extension{definition,..}if definition.type_id==JOINT_TYPE),
    ) {
        for entity in document.entities() {
            if let EntityData::Instance {
                placement_source, ..
            } = &entity.data
                && let Some(joint) = joint_driver(document, entity.id)?
                && *placement_source != Some(joint)
            {
                return Err(invalid(
                    "A joint must remain the active incoming placement driver; edit joint values instead of appending a free transform",
                ));
            }
        }
    }
    Ok(())
}
