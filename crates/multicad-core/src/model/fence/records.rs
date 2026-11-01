//! Versioned source records. A fence run owns its path and context; member
//! features refer to semantic layout keys rather than storing copied geometry.
use super::*;
use crate::{
    document::{Definition, Entity},
    model::{BodyOutput, length},
};
use serde::de::DeserializeOwned;

pub const FENCE_TYPE_KIND: &str = "architecture.fence_type";
pub const FENCE_TYPE_CAPABILITY: &str = "architecture.fence_type.1";
pub const FENCE_RUN_KIND: &str = "architecture.fence";
pub const FENCE_RUN_CAPABILITY: &str = "architecture.fence.1";
pub const FENCE_MEMBER_TYPE: &str = "architecture.fence_member";
pub const FENCE_MEMBER_CAPABILITY: &str = "architecture.fence_member.1";
pub const FENCE_SCHEMA: u32 = 1;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FenceInstanceOverrides {
    /// Post height only. Fixed physical infill dimensions are never scaled;
    /// shortening below an infill requires an explicit construction repair.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub post_height: Option<Dimension>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub post_spacing: Option<Dimension>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FenceRunDefinition {
    pub fence_type: EntityId,
    pub path: FencePath,
    pub group: EntityId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub level: Option<EntityId>,
    #[serde(default)]
    pub overrides: FenceInstanceOverrides,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub post_positions: Vec<FencePostPosition>,
}
impl FenceRunDefinition {
    pub fn new(fence_type: EntityId, path: FencePath) -> Self {
        Self {
            fence_type,
            path,
            group: EntityId::new(),
            level: None,
            overrides: FenceInstanceOverrides::default(),
            post_positions: Vec::new(),
        }
    }
    pub fn validate(&self) -> Result<(), DocumentError> {
        if self.fence_type.is_nil()
            || self.group.is_nil()
            || self.level.is_some_and(|id| id.is_nil())
        {
            return Err(invalid(
                "Fence type, group and optional level need nonempty identities",
            ));
        }
        self.path.validate()?;
        for dimension in self
            .overrides
            .post_height
            .iter()
            .chain(self.overrides.post_spacing.iter())
        {
            check_dimension(dimension, Unit::Millimeter)?;
            if matches!(dimension, Dimension::Literal { value, .. } if *value <= 0.) {
                return Err(invalid(
                    "Fence height and spacing overrides must be positive",
                ));
            }
        }
        if self.post_positions.len() > MAX_FENCE_MEMBERS {
            return Err(invalid("Too many fence post-position overrides"));
        }
        let mut keys = BTreeSet::new();
        for position in &self.post_positions {
            check_post_key(position.post)?;
            if !matches!(position.post, FencePostKey::Interior { .. })
                || !keys.insert(position.post)
            {
                return Err(invalid(
                    "Fence position overrides need distinct interior post keys",
                ));
            }
            check_dimension(&position.offset, Unit::Millimeter)?;
            if matches!(&position.offset, Dimension::Literal { value, .. } if *value <= 0.) {
                return Err(invalid("Fence post-position offsets must be positive"));
            }
        }
        Ok(())
    }
    pub fn inputs(&self) -> Vec<Link> {
        let mut links = self.path.inputs();
        for link in parameter_links(
            self.overrides
                .post_height
                .iter()
                .chain(self.overrides.post_spacing.iter())
                .chain(self.post_positions.iter().map(|p| &p.offset))
                .collect(),
        ) {
            if !links.contains(&link) {
                links.push(link);
            }
        }
        links.push(Link::new(self.fence_type, "fence_type"));
        if let Some(level) = self.level {
            links.push(Link::new(level, "level"));
        }
        links
    }
    pub(crate) fn entity_data(&self) -> Result<EntityData, DocumentError> {
        self.validate()?;
        Ok(EntityData::Extension {
            definition: definition(FENCE_RUN_KIND, self)?,
            inputs: self.inputs(),
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum FenceMemberKey {
    Post { post: FencePostKey },
    Infill { infill: FenceInfillKey },
}
impl FenceMemberKey {
    fn validate(self) -> Result<(), DocumentError> {
        match self {
            Self::Post { post } => check_post_key(post),
            Self::Infill { infill } => {
                check_post_key(infill.bay.start)?;
                check_post_key(infill.bay.end)?;
                if infill.recipe.is_nil()
                    || infill.bay.start == infill.bay.end
                    || infill.ordinal as usize >= MAX_FENCE_MEMBERS
                {
                    return Err(invalid("Fence infill key is invalid"));
                }
                Ok(())
            }
        }
    }
}

fn check_post_key(key: FencePostKey) -> Result<(), DocumentError> {
    match key {
        FencePostKey::Vertex { vertex } if !vertex.is_nil() => Ok(()),
        FencePostKey::Interior {
            start,
            end,
            ordinal,
        } if !start.is_nil()
            && !end.is_nil()
            && start != end
            && ordinal > 0
            && (ordinal as usize) < MAX_FENCE_MEMBERS =>
        {
            Ok(())
        }
        _ => Err(invalid("Fence post key is invalid")),
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FenceMemberDefinition {
    pub run: EntityId,
    pub key: FenceMemberKey,
    pub output: BodyOutput,
}
impl FenceMemberDefinition {
    pub fn definition(&self) -> Result<Definition, DocumentError> {
        self.key.validate()?;
        if self.run.is_nil() || self.output.body.is_nil() || self.run == self.output.body {
            return Err(invalid(
                "Fence member needs distinct run and output identities",
            ));
        }
        definition(FENCE_MEMBER_TYPE, self)
    }
    pub fn inputs(&self) -> Vec<Link> {
        vec![Link::new(self.run, "fence_run")]
    }
}

pub(super) fn definition<T: Serialize>(kind: &str, value: &T) -> Result<Definition, DocumentError> {
    Ok(Definition {
        type_id: kind.into(),
        schema_version: FENCE_SCHEMA,
        data: serde_json::to_value(value).map_err(|e| invalid(e.to_string()))?,
    })
}
fn parse<T: DeserializeOwned>(definition: &Definition, kind: &str) -> Result<T, DocumentError> {
    if definition.type_id != kind || definition.schema_version != FENCE_SCHEMA {
        return Err(invalid("Unsupported fence source kind or schema"));
    }
    serde_json::from_value(definition.data.clone()).map_err(|e| invalid(e.to_string()))
}
fn check_links(actual: &[Link], expected: &[Link]) -> Result<(), DocumentError> {
    if actual.len() != expected.len() || expected.iter().any(|link| !actual.contains(link)) {
        return Err(invalid("Fence source and canonical input links disagree"));
    }
    Ok(())
}
pub fn fence_types(document: &Document) -> impl Iterator<Item = &Entity> {
    document.entities().filter(|e| matches!(&e.data, EntityData::Extension { definition, .. } if definition.type_id == FENCE_TYPE_KIND))
}
pub fn fence_runs(document: &Document) -> impl Iterator<Item = &Entity> {
    document.entities().filter(|e| matches!(&e.data, EntityData::Extension { definition, .. } if definition.type_id == FENCE_RUN_KIND))
}
pub fn read_fence_type(
    document: &Document,
    id: EntityId,
) -> Result<FenceConstruction, DocumentError> {
    let Some(EntityData::Extension { definition, inputs }) = document.get(id).map(|e| &e.data)
    else {
        return Err(invalid("Choose a fence construction type"));
    };
    let value: FenceConstruction = parse(definition, FENCE_TYPE_KIND)?;
    value.validate()?;
    check_links(inputs, &value.inputs())?;
    for material in
        std::iter::once(value.post_material).chain(value.infills.iter().map(|i| i.material))
    {
        if !matches!(
            document.get(material).map(|e| &e.data),
            Some(EntityData::Material { .. })
        ) {
            return Err(invalid(
                "Fence construction material is missing or has the wrong entity kind",
            ));
        }
    }
    Ok(value)
}
pub fn read_fence_run(
    document: &Document,
    id: EntityId,
) -> Result<FenceRunDefinition, DocumentError> {
    let value = read_run_source(document, id)?;
    read_fence_type(document, value.fence_type)?;
    Ok(value)
}
fn read_run_source(document: &Document, id: EntityId) -> Result<FenceRunDefinition, DocumentError> {
    let Some(EntityData::Extension { definition, inputs }) = document.get(id).map(|e| &e.data)
    else {
        return Err(invalid("Choose an editable fence run"));
    };
    let value: FenceRunDefinition = parse(definition, FENCE_RUN_KIND)?;
    value.validate()?;
    check_links(inputs, &value.inputs())?;
    if !matches!(
        document.get(value.group).map(|e| &e.data),
        Some(EntityData::Group {})
    ) {
        return Err(invalid(
            "Fence run group is missing; repair the run before editing",
        ));
    }
    if let Some(level) = value.level {
        super::super::read_level(document, level)?;
    }
    Ok(value)
}
pub fn read_fence_member(
    document: &Document,
    id: EntityId,
) -> Result<FenceMemberDefinition, DocumentError> {
    let Some(EntityData::Feature {
        definition, inputs, ..
    }) = document.get(id).map(|e| &e.data)
    else {
        return Err(invalid("Choose a fence member feature"));
    };
    let value: FenceMemberDefinition = parse(definition, FENCE_MEMBER_TYPE)?;
    value.definition()?;
    check_links(inputs, &value.inputs())?;
    if value.run == id || value.output.body == id {
        return Err(invalid("Fence member identities must be distinct"));
    }
    if let Some(entity) = document.get(value.output.body) {
        if !matches!(&entity.data, EntityData::Part { features, .. } if features.contains(&id)) {
            return Err(invalid("Fence output Part must contain its member feature"));
        }
    } else {
        return Err(invalid("Fence member output Part is missing"));
    }
    Ok(value)
}
pub fn fence_members(
    document: &Document,
    run: EntityId,
) -> Result<Vec<(FenceMemberKey, EntityId, EntityId)>, DocumentError> {
    let mut result = Vec::new();
    for entity in document.entities() {
        if matches!(&entity.data, EntityData::Feature { definition, .. } if definition.type_id == FENCE_MEMBER_TYPE)
        {
            let value = read_fence_member(document, entity.id)?;
            if value.run == run {
                result.push((value.key, entity.id, value.output.body));
            }
        }
    }
    result.sort();
    Ok(result)
}
pub fn resolve_fence_run(
    document: &Document,
    parameters: &EvaluatedParameters,
    value: &FenceRunDefinition,
) -> Result<FenceLayout, DocumentError> {
    value.validate()?;
    let mut construction = read_fence_type(document, value.fence_type)?;
    if let Some(height) = &value.overrides.post_height {
        construction.height = height.clone();
    }
    if let Some(spacing) = &value.overrides.post_spacing {
        construction.preferred_post_spacing = spacing.clone();
    }
    let mut path = value.path.clone();
    if let Some(level) = value.level {
        let base = super::super::resolve_level(document, parameters, level)?.elevation_mm
            + number(&path.base_z, Unit::Millimeter, parameters)?;
        path.base_z = length(base);
    }
    plan_fence(
        document,
        parameters,
        &construction,
        &path,
        &value.post_positions,
    )
}

pub(super) fn validate_document(document: &Document) -> Result<(), DocumentError> {
    let mut types = BTreeSet::new();
    for entity in fence_types(document) {
        read_fence_type(document, entity.id)?;
        types.insert(entity.id);
    }
    let mut groups = BTreeSet::new();
    let mut runs = BTreeSet::new();
    for entity in fence_runs(document) {
        let value = read_run_source(document, entity.id)?;
        if !types.contains(&value.fence_type) {
            return Err(invalid("Fence run construction type is missing"));
        }
        runs.insert(entity.id);
        if !groups.insert(value.group) {
            return Err(invalid("Two fence runs cannot own the same group"));
        }
    }
    let mut members = BTreeSet::new();
    let mut bodies = BTreeSet::new();
    for entity in document.entities() {
        if matches!(&entity.data, EntityData::Feature { definition, .. } if definition.type_id == FENCE_MEMBER_TYPE)
        {
            let value = read_fence_member(document, entity.id)?;
            if !runs.contains(&value.run) {
                return Err(invalid("Fence member run is missing"));
            }
            if !members.insert((value.run, value.key)) || !bodies.insert(value.output.body) {
                return Err(invalid("Fence member keys and output Parts must be unique"));
            }
        }
    }
    Ok(())
}
