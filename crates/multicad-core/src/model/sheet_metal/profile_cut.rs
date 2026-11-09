//! A through-sheet cut driven by a live sketch region. The feature depends
//! on its preceding solid rather than changing the forming feature that may
//! already host the sketch.
use super::{
    BodyOutput, CutoutFace, Definition, Document, DocumentError, Entity, EntityData, EntityId,
    Link, ProfileSource, SHEET_SCHEMA, SHEET_TYPE, invalid, read_sheet, sheet_feature,
    validate_region,
};
use crate::{
    commands::{Actor, CommandError, Request, Session},
    document::Mutation,
    model::{
        BodyInput, CreatedBody, ModelPrepared,
        common::{error, revision},
    },
};
use serde::{Deserialize, Serialize};

pub const SHEET_PROFILE_CUT_TYPE: &str = "model.sheet_profile_cut";
pub const SHEET_PROFILE_CUT_SCHEMA: u32 = 1;
pub const SHEET_PROFILE_CUT_CAPABILITY: &str = "model.sheet_profile_cut.1";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SheetProfileCutDefinition {
    pub output: BodyOutput,
    pub input: BodyInput,
    pub profile: ProfileSource,
    pub face: CutoutFace,
}

impl SheetProfileCutDefinition {
    fn validate(&self) -> Result<(), DocumentError> {
        if self.output.body != self.input.body
            || [self.input.body, self.input.producer, self.profile.sketch]
                .iter()
                .any(EntityId::is_nil)
            || self.input.body == self.input.producer
        {
            return Err(invalid(
                "A sheet profile cut requires an upstream producer and a sketch and keeps the same Part",
            ));
        }
        validate_region(&self.profile.region)
    }

    pub fn definition(&self) -> Result<Definition, DocumentError> {
        self.validate()?;
        Ok(Definition {
            type_id: SHEET_PROFILE_CUT_TYPE.into(),
            schema_version: SHEET_PROFILE_CUT_SCHEMA,
            data: serde_json::to_value(self).map_err(|error| invalid(error.to_string()))?,
        })
    }

    pub fn from_definition(definition: &Definition) -> Result<Self, DocumentError> {
        if definition.type_id != SHEET_PROFILE_CUT_TYPE
            || definition.schema_version != SHEET_PROFILE_CUT_SCHEMA
        {
            return Err(invalid("Choose a supported sheet profile cut feature"));
        }
        let value: Self = serde_json::from_value(definition.data.clone())
            .map_err(|error| invalid(error.to_string()))?;
        value.validate()?;
        Ok(value)
    }

    pub fn inputs(&self) -> [Link; 2] {
        [
            Link::new(self.input.producer, "source"),
            Link::new(self.profile.sketch, "profile"),
        ]
    }

    fn validate_sources(&self, document: &Document) -> Result<(), DocumentError> {
        self.input.resolve_producer(document)?;
        if !compatible_producer(document, self.input.producer) {
            return Err(invalid(
                "A sheet profile cut needs a sheet or an earlier sheet profile cut",
            ));
        }
        if !matches!(
            document.get(self.profile.sketch).map(|entity| &entity.data),
            Some(EntityData::Sketch { .. })
        ) {
            return Err(invalid(
                "Choose an existing sketch profile for the sheet cut",
            ));
        }
        let forming = sheet_feature(document, self.input.body)
            .ok_or_else(|| invalid("Choose an existing sheet-metal Part"))?;
        let sheet = read_sheet(document, forming)?;
        if matches!(self.face, CutoutFace::Flange { index } if index as usize >= sheet.flanges.len())
        {
            return Err(invalid("The cut's selected flange no longer exists"));
        }
        Ok(())
    }
}

fn compatible_producer(document: &Document, feature: EntityId) -> bool {
    matches!(
        document.get(feature).map(|entity| &entity.data),
        Some(EntityData::Feature { definition, .. })
            if matches!(
                (definition.type_id.as_str(), definition.schema_version),
                (SHEET_TYPE, SHEET_SCHEMA) | (SHEET_PROFILE_CUT_TYPE, SHEET_PROFILE_CUT_SCHEMA)
            )
    )
}

fn active_head(document: &Document, features: &[EntityId]) -> Option<EntityId> {
    features.iter().rev().copied().find(|feature| {
        matches!(
            document.get(*feature).map(|entity| &entity.data),
            Some(EntityData::Feature {
                suppressed: false,
                ..
            })
        )
    })
}

/// The current canonical sheet or through-cut producer, if the Part has no
/// later incompatible feature. The native evaluator still proves its solid.
pub fn sheet_profile_head(document: &Document, part: EntityId) -> Option<EntityId> {
    let EntityData::Part { features, .. } = &document.get(part)?.data else {
        return None;
    };
    let head = active_head(document, features)?;
    compatible_producer(document, head).then_some(head)
}

pub fn read_sheet_profile_cut(
    document: &Document,
    feature: EntityId,
) -> Result<SheetProfileCutDefinition, DocumentError> {
    let Some(EntityData::Feature {
        definition,
        inputs,
        suppressed,
    }) = document.get(feature).map(|entity| &entity.data)
    else {
        return Err(invalid("Choose a sheet profile cut feature"));
    };
    let source = SheetProfileCutDefinition::from_definition(definition)?;
    let expected = source.inputs();
    if inputs.len() != expected.len() || expected.iter().any(|link| !inputs.contains(link)) {
        return Err(invalid(
            "Sheet profile cut source and canonical input links disagree",
        ));
    }
    source.validate_sources(document)?;
    let Some(EntityData::Part { features, .. }) =
        document.get(source.output.body).map(|entity| &entity.data)
    else {
        return Err(invalid("Sheet profile cut output Part is missing"));
    };
    let position = features
        .iter()
        .position(|id| *id == feature)
        .ok_or_else(|| invalid("Sheet profile cut is absent from its Part history"))?;
    let earlier = &features[..position];
    if !earlier.contains(&source.input.producer)
        || (!suppressed && active_head(document, earlier) != Some(source.input.producer))
    {
        return Err(invalid(
            "Sheet profile cut must follow its captured sheet producer",
        ));
    }
    Ok(source)
}

/// Prepare a source-only continuation. Folded and developed geometry must
/// both pass the native candidate check before the caller commits it.
pub fn prepare_sheet_profile_cut(
    session: &Session,
    expected_revision: u64,
    actor: Actor,
    feature: EntityId,
    source: &SheetProfileCutDefinition,
) -> Result<ModelPrepared, CommandError> {
    revision(session, expected_revision)?;
    let document = session.document();
    if feature.is_nil() || feature == document.id() || document.get(feature).is_some() {
        return Err(error(
            "Choose a new feature identity for the sheet profile cut",
        ));
    }
    let definition = source.definition()?;
    source.validate_sources(document)?;
    let body = source.output.body;
    if sheet_profile_head(document, body) != Some(source.input.producer) {
        return Err(error(
            "The sheet's current producer changed; reopen Extrude Cut",
        ));
    }
    let mut part = document
        .get(body)
        .cloned()
        .ok_or_else(|| error("Sheet Part is missing"))?;
    let EntityData::Part { features, .. } = &mut part.data else {
        return Err(error("Choose a sheet-metal Part"));
    };
    features.push(feature);
    let mut cut = Entity::new(
        "Sheet profile cut",
        EntityData::Feature {
            definition,
            inputs: source.inputs().into(),
            suppressed: false,
        },
    );
    cut.id = feature;
    let prepared = session.prepare(Request {
        expected_revision,
        actor,
        label: "Extrude cut through sheet".into(),
        edits: vec![Mutation::Insert(cut), Mutation::Replace(part)],
    })?;
    Ok(ModelPrepared {
        prepared,
        outputs: vec![CreatedBody { feature, body }],
    })
}
