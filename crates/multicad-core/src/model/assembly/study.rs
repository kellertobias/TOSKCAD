//! Source for a saved, runnable assembly inspection range.
use super::{JointCoordinate, read_joint};
use crate::{
    assembly_inspection::{
        InspectionStamp, MAX_INSPECTION_PAIRS, MotionRangeQuery, OccurrencePair,
    },
    commands::Session,
    constraint_sketch::Dimension,
    document::{Definition, Document, Entity, EntityData, EntityId, Link},
    model::{common::parameter_links, source::check_dimension},
    parameters::Unit,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub const MOTION_STUDY_TYPE: &str = "model.motion_study";
pub const MOTION_STUDY_CAPABILITY: &str = "model.motion_study.1";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SavedMotionStudy {
    pub pairs: Vec<OccurrencePair>,
    pub joint: EntityId,
    pub coordinate: JointCoordinate,
    pub start: Dimension,
    pub end: Dimension,
    pub step: Dimension,
    pub contact_tolerance_mm: f64,
    pub required_clearance_mm: f64,
}

impl SavedMotionStudy {
    pub fn from_definition(definition: &Definition) -> Result<Self, String> {
        if definition.type_id != MOTION_STUDY_TYPE || definition.schema_version != 1 {
            return Err("Choose a supported saved motion study".into());
        }
        let study: Self =
            serde_json::from_value(definition.data.clone()).map_err(|error| error.to_string())?;
        study.links()?;
        Ok(study)
    }

    pub fn unit(&self) -> Unit {
        match self.coordinate {
            JointCoordinate::Rotation => Unit::Radian,
            _ => Unit::Millimeter,
        }
    }

    fn links(&self) -> Result<Vec<Link>, String> {
        if self.joint.is_nil()
            || self.pairs.is_empty()
            || self.pairs.len() > MAX_INSPECTION_PAIRS
            || !(1e-7..=1e-3).contains(&self.contact_tolerance_mm)
            || !self.required_clearance_mm.is_finite()
            || self.required_clearance_mm < 0.
        {
            return Err(
                "A motion study needs a joint, occurrence pairs and valid clearance settings"
                    .into(),
            );
        }
        for dimension in [&self.start, &self.end, &self.step] {
            check_dimension(dimension, self.unit()).map_err(|error| error.to_string())?;
        }
        let mut members = BTreeSet::new();
        for pair in &self.pairs {
            pair.validate()?;
            for path in [&pair.a, &pair.b] {
                members.insert(path.root);
                members.extend(path.members.iter().copied());
            }
        }
        let mut links = parameter_links(vec![&self.start, &self.end, &self.step]);
        links.push(Link::new(self.joint, "study/joint"));
        links.extend(
            members
                .into_iter()
                .map(|id| Link::new(id, "study/occurrence")),
        );
        Ok(links)
    }

    pub fn entity(&self, id: EntityId, name: impl Into<String>) -> Result<Entity, String> {
        let mut entity = Entity::new(
            name,
            EntityData::Extension {
                definition: Definition {
                    type_id: MOTION_STUDY_TYPE.into(),
                    schema_version: 1,
                    data: serde_json::to_value(self).map_err(|error| error.to_string())?,
                },
                inputs: self.links()?,
            },
        );
        entity.id = id;
        Ok(entity)
    }

    pub fn query(&self, session: &Session) -> Result<MotionRangeQuery, String> {
        let (joint, _) =
            read_joint(session.document(), self.joint).map_err(|error| error.to_string())?;
        if !joint.parameters.kind.allows(self.coordinate) {
            return Err("The saved motion study coordinate is unavailable on its joint".into());
        }
        let value = |dimension: &Dimension| {
            dimension
                .value_in(self.unit(), Some(session.parameters()))
                .map_err(|error| error.to_string())
        };
        let query = MotionRangeQuery {
            expected: InspectionStamp::capture(session.document(), session.revision())?,
            pairs: self.pairs.clone(),
            contact_tolerance_mm: self.contact_tolerance_mm,
            joint: self.joint,
            coordinate: self.coordinate,
            start: value(&self.start)?,
            end: value(&self.end)?,
            step: value(&self.step)?,
        };
        query.validate()?;
        Ok(query)
    }
}

pub fn read_saved_motion_study(
    document: &Document,
    id: EntityId,
) -> Result<SavedMotionStudy, String> {
    let Some(EntityData::Extension { definition, inputs }) =
        document.get(id).map(|entity| &entity.data)
    else {
        return Err("Choose a saved motion study".into());
    };
    let study = SavedMotionStudy::from_definition(definition)?;
    let links = study.links()?;
    if links.len() != inputs.len() || links.iter().any(|link| !inputs.contains(link)) {
        return Err("Motion study inputs disagree with its saved source".into());
    }
    let (joint, _) = read_joint(document, study.joint).map_err(|error| error.to_string())?;
    if !joint.parameters.kind.allows(study.coordinate) {
        return Err("The saved motion study coordinate is unavailable on its joint".into());
    }
    Ok(study)
}
