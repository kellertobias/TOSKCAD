//! Canonical circular helical cut on one retained axial cylinder Part.
use super::{
    BodyInput, BodyOutput, CreatedBody, ModelPrepared,
    boolean::available,
    common::{error, number, parameter_links, prepare_edit, revision},
    source::{check_dimension, invalid},
};
use crate::{
    commands::{Actor, CommandError, Request, Session},
    constraint_sketch::Dimension,
    document::{Definition, Document, DocumentError, Entity, EntityData, EntityId, Link, Mutation},
    hierarchy,
    parameters::{EvaluatedParameters, Unit},
};
use serde::{Deserialize, Serialize};

pub const HELICAL_TUBE_CUT_TYPE: &str = "model.helical_tube_cut";
pub const HELICAL_TUBE_CUT_SCHEMA: u32 = 1;
pub const HELICAL_TUBE_CUT_CAPABILITY: &str = "model.helical_tube_cut.1";

/// The right-handed tool's circular section is normal to its path. The
/// centerline uses the source cylinder's outer radius and local coordinates.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HelicalTubeCutParameters {
    pub radius: Dimension,
    pub pitch: Dimension,
    pub path_height: Dimension,
    pub start_height: Dimension,
    pub start_angle: Dimension,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ResolvedHelicalTubeCut {
    pub radius_mm: f64,
    pub pitch_mm: f64,
    pub path_height_mm: f64,
    pub start_height_mm: f64,
    pub start_angle_rad: f64,
}

impl HelicalTubeCutParameters {
    fn lengths(&self) -> [&Dimension; 4] {
        [
            &self.radius,
            &self.pitch,
            &self.path_height,
            &self.start_height,
        ]
    }

    fn dimensions(&self) -> Vec<&Dimension> {
        self.lengths()
            .into_iter()
            .chain([&self.start_angle])
            .collect()
    }

    fn validate(&self) -> Result<(), DocumentError> {
        for dimension in self.lengths() {
            check_dimension(dimension, Unit::Millimeter)?;
        }
        check_dimension(&self.start_angle, Unit::Radian)
    }

    pub fn resolve(
        &self,
        parameters: &EvaluatedParameters,
    ) -> Result<ResolvedHelicalTubeCut, DocumentError> {
        let [radius, pitch, path, start] = self
            .lengths()
            .map(|dimension| number(dimension, Unit::Millimeter, parameters));
        let [radius_mm, pitch_mm, path_height_mm, start_height_mm] =
            [radius?, pitch?, path?, start?];
        let start_angle_rad = number(&self.start_angle, Unit::Radian, parameters)?;
        if [radius_mm, pitch_mm, path_height_mm, start_height_mm]
            .into_iter()
            .any(|value| value <= 0.)
            || start_angle_rad.abs() > std::f64::consts::TAU
        {
            return Err(invalid(
                "Helical tube cut needs positive lengths and a start angle within one turn",
            ));
        }
        Ok(ResolvedHelicalTubeCut {
            radius_mm,
            pitch_mm,
            path_height_mm,
            start_height_mm,
            start_angle_rad,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HelicalTubeCutDefinition {
    pub input: BodyInput,
    pub output: BodyOutput,
    pub parameters: HelicalTubeCutParameters,
}

impl HelicalTubeCutDefinition {
    fn validate(&self) -> Result<(), DocumentError> {
        if self.input.body.is_nil()
            || self.input.producer.is_nil()
            || self.input.body == self.input.producer
            || self.output.body != self.input.body
        {
            return Err(invalid(
                "Helical tube cut needs a frozen source producer and retains its Part",
            ));
        }
        self.parameters.validate()
    }

    fn definition(&self) -> Result<Definition, DocumentError> {
        self.validate()?;
        Ok(Definition {
            type_id: HELICAL_TUBE_CUT_TYPE.into(),
            schema_version: HELICAL_TUBE_CUT_SCHEMA,
            data: serde_json::to_value(self).map_err(|e| invalid(e.to_string()))?,
        })
    }

    fn inputs(&self) -> Vec<Link> {
        let mut links = vec![Link::new(self.input.producer, "source")];
        links.extend(parameter_links(self.parameters.dimensions()));
        links
    }
}

pub fn read_helical_tube_cut(
    document: &Document,
    feature: EntityId,
) -> Result<HelicalTubeCutDefinition, DocumentError> {
    let Some(EntityData::Feature {
        definition, inputs, ..
    }) = document.get(feature).map(|entity| &entity.data)
    else {
        return Err(invalid("Choose a helical tube cut feature"));
    };
    if definition.type_id != HELICAL_TUBE_CUT_TYPE
        || definition.schema_version != HELICAL_TUBE_CUT_SCHEMA
    {
        return Err(invalid("Unsupported helical tube cut provider or schema"));
    }
    let source: HelicalTubeCutDefinition =
        serde_json::from_value(definition.data.clone()).map_err(|e| invalid(e.to_string()))?;
    source.validate()?;
    let expected = source.inputs();
    if inputs.len() != expected.len() || expected.iter().any(|link| !inputs.contains(link)) {
        return Err(invalid(
            "Helical tube cut source and canonical inputs disagree",
        ));
    }
    for link in inputs {
        if let Some(entity) = document.get(link.target) {
            let correct = if link.role == "source" {
                matches!(entity.data, EntityData::Feature { .. })
            } else {
                matches!(entity.data, EntityData::Parameter { .. })
            };
            if !correct {
                return Err(invalid("Helical tube cut input has the wrong entity kind"));
            }
        }
    }
    if let Some(part) = document.get(source.output.body) {
        let EntityData::Part { features, .. } = &part.data else {
            return Err(invalid("Helical tube cut output must be a Part"));
        };
        let output = features
            .iter()
            .position(|id| *id == feature)
            .ok_or_else(|| invalid("Helical tube cut is missing from its Part history"))?;
        if !features[..output].contains(&source.input.producer) {
            return Err(invalid(
                "Helical tube cut input must precede the cut in Part history",
            ));
        }
    }
    Ok(source)
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HelicalTubeCutRequest {
    pub feature: EntityId,
    pub input: BodyInput,
    pub parameters: HelicalTubeCutParameters,
}

impl HelicalTubeCutRequest {
    pub fn new(input: BodyInput, parameters: HelicalTubeCutParameters) -> Self {
        Self {
            feature: EntityId::new(),
            input,
            parameters,
        }
    }
}

pub fn prepare_helical_tube_cut(
    session: &Session,
    expected_revision: u64,
    actor: Actor,
    request: &HelicalTubeCutRequest,
) -> Result<ModelPrepared, CommandError> {
    revision(session, expected_revision)?;
    let document = session.document();
    if request.feature.is_nil()
        || request.feature == document.id()
        || document.get(request.feature).is_some()
    {
        return Err(error("Helical tube cut needs a fresh feature identity"));
    }
    available(document, &request.input, true)?;
    request.parameters.resolve(session.parameters())?;
    let source = HelicalTubeCutDefinition {
        input: request.input.clone(),
        output: BodyOutput {
            body: request.input.body,
        },
        parameters: request.parameters.clone(),
    };
    let mut feature = Entity::new(
        "Helical tube cut",
        EntityData::Feature {
            definition: source.definition()?,
            inputs: source.inputs(),
            suppressed: false,
        },
    );
    feature.id = request.feature;
    let mut part = document
        .get(request.input.body)
        .expect("validated Part")
        .clone();
    let EntityData::Part { features, .. } = &mut part.data else {
        unreachable!()
    };
    features.push(feature.id);
    let mut edits = vec![Mutation::Insert(feature), Mutation::Replace(part)];
    let index = hierarchy::Index::new(document).map_err(|e| error(e.to_string()))?;
    if let Some(parent) = index
        .parent(request.input.body)
        .map_err(|e| error(e.to_string()))?
    {
        let mut hierarchy = document.hierarchy().clone();
        hierarchy
            .members
            .entry(parent)
            .or_default()
            .push(request.feature);
        edits.push(Mutation::SetHierarchy(hierarchy));
    }
    let prepared = session.prepare(Request {
        expected_revision,
        actor,
        label: "Cut a helical tube".into(),
        edits,
    })?;
    Ok(ModelPrepared {
        prepared,
        outputs: vec![CreatedBody {
            feature: request.feature,
            body: request.input.body,
        }],
    })
}

pub fn prepare_helical_tube_cut_edit(
    session: &Session,
    expected_revision: u64,
    actor: Actor,
    feature: EntityId,
    parameters: &HelicalTubeCutParameters,
) -> Result<ModelPrepared, CommandError> {
    revision(session, expected_revision)?;
    let mut source = read_helical_tube_cut(session.document(), feature)?;
    available(session.document(), &source.input, false)?;
    parameters.resolve(session.parameters())?;
    source.parameters = parameters.clone();
    prepare_edit(
        session,
        expected_revision,
        actor,
        "Edit helical tube cut",
        CreatedBody {
            feature,
            body: source.output.body,
        },
        source.definition()?,
        source.inputs(),
    )
}
