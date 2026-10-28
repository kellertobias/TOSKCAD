use super::*;
use crate::{
    commands::{Actor, CommandError, Request, Session},
    document::{Entity, Mutation},
    hierarchy,
    model::{
        CreatedBody, ModelPrepared,
        boolean::available,
        common::{error, revision},
    },
};
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EdgeTreatmentRequest {
    pub feature: EntityId,
    pub input: BodyInput,
    pub parameters: EdgeTreatmentParameters,
    pub edges: Vec<GeometryIntent>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chamfer: Option<ChamferSettings>,
}
impl EdgeTreatmentRequest {
    pub fn new(
        input: BodyInput,
        parameters: EdgeTreatmentParameters,
        edges: Vec<GeometryIntent>,
    ) -> Self {
        Self {
            feature: EntityId::new(),
            input,
            parameters,
            edges,
            chamfer: None,
        }
    }
    pub fn existing(
        document: &crate::document::Document,
        feature: EntityId,
    ) -> Result<Self, crate::document::DocumentError> {
        let (source, edges) = read_edge_treatment(document, feature)?;
        Ok(Self {
            feature,
            input: source.input,
            parameters: source.parameters,
            edges,
            chamfer: source.chamfer,
        })
    }
}
pub fn prepare_edge_treatment(
    session: &Session,
    expected_revision: u64,
    actor: Actor,
    request: &EdgeTreatmentRequest,
) -> Result<ModelPrepared, CommandError> {
    revision(session, expected_revision)?;
    if request.feature.is_nil()
        || request.feature == session.document().id()
        || session.document().get(request.feature).is_some()
    {
        return Err(error(
            "Edge-treatment output needs a fresh non-nil feature identity",
        ));
    }
    available(session.document(), &request.input, true)?;
    request.parameters.resolved_size_mm(session.parameters())?;
    if let Some(chamfer) = &request.chamfer {
        chamfer.resolve(session.parameters())?;
    }
    let source = EdgeTreatmentDefinition {
        input: request.input.clone(),
        output: BodyOutput {
            body: request.input.body,
        },
        parameters: request.parameters.clone(),
        edges: request
            .edges
            .iter()
            .filter(|e| e.kind == TopologyKind::Edge)
            .map(|e| e.id)
            .collect(),
        chamfer: request.chamfer.clone(),
    };
    let mut feature = Entity::new(
        "Fillet / Chamfer",
        EntityData::Feature {
            definition: source.definition()?,
            inputs: source.inputs(&request.edges)?,
            suppressed: false,
        },
    );
    feature.id = request.feature;
    let mut part = session
        .document()
        .get(request.input.body)
        .expect("validated Part")
        .clone();
    let EntityData::Part { features, .. } = &mut part.data else {
        unreachable!()
    };
    features.push(feature.id);
    let mut edits = vec![Mutation::Insert(feature), Mutation::Replace(part)];
    let index = hierarchy::Index::new(session.document()).map_err(|e| error(e.to_string()))?;
    if let Some(parent) = index
        .parent(request.input.body)
        .map_err(|e| error(e.to_string()))?
    {
        let mut hierarchy = session.document().hierarchy().clone();
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
        label: "Treat Part edges".into(),
        edits,
    })?;
    Ok(ModelPrepared {
        prepared,
        outputs: vec![CreatedBody {
            body: request.input.body,
            feature: request.feature,
        }],
    })
}
pub fn prepare_edge_treatment_edit(
    session: &Session,
    expected_revision: u64,
    actor: Actor,
    feature: EntityId,
    parameters: &EdgeTreatmentParameters,
    edges: &[GeometryIntent],
) -> Result<ModelPrepared, CommandError> {
    revision(session, expected_revision)?;
    parameters.resolved_size_mm(session.parameters())?;
    let mut request = EdgeTreatmentRequest::existing(session.document(), feature)?;
    request.parameters = parameters.clone();
    request.edges = edges.to_vec();
    if parameters.profile == EdgeTreatmentProfile::Fillet {
        request.chamfer = None;
    }
    prepare_edge_treatment_edit_request(session, expected_revision, actor, &request)
}
pub fn prepare_edge_treatment_edit_request(
    session: &Session,
    expected_revision: u64,
    actor: Actor,
    request: &EdgeTreatmentRequest,
) -> Result<ModelPrepared, CommandError> {
    revision(session, expected_revision)?;
    request.parameters.resolved_size_mm(session.parameters())?;
    if let Some(chamfer) = &request.chamfer {
        chamfer.resolve(session.parameters())?;
    }
    let feature = request.feature;
    let (mut source, _) = read_edge_treatment(session.document(), feature)?;
    if source.input != request.input {
        return Err(error("An edge edit must retain its frozen Part producer"));
    }
    source.parameters = request.parameters.clone();
    source.edges = request
        .edges
        .iter()
        .filter(|e| e.kind == TopologyKind::Edge)
        .map(|e| e.id)
        .collect();
    source.chamfer = request.chamfer.clone();
    let mut entity = session
        .document()
        .get(feature)
        .expect("existing edge treatment")
        .clone();
    let EntityData::Feature {
        definition,
        inputs,
        suppressed,
    } = &mut entity.data
    else {
        unreachable!()
    };
    if *suppressed {
        return Err(error(
            "Unsuppress the edge-treatment feature before editing it",
        ));
    }
    *definition = source.definition()?;
    *inputs = source.inputs(&request.edges)?;
    let prepared = session.prepare(Request {
        expected_revision,
        actor,
        label: "Edit Fillet / Chamfer".into(),
        edits: vec![Mutation::Replace(entity)],
    })?;
    Ok(ModelPrepared {
        prepared,
        outputs: vec![CreatedBody {
            body: source.output.body,
            feature,
        }],
    })
}
