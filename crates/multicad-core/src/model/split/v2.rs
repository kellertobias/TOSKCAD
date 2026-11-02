//! Explicit plane/body Split roles and atomic multi-target preparation.
use super::*;
use crate::{
    constraint_sketch::Dimension,
    model::{
        PrimitiveFrame,
        common::{number, parameter_links},
        source::check_dimension,
    },
    parameters::{EvaluatedParameters, Unit},
    sketch,
};

pub const SPLIT_V2_SCHEMA: u32 = 2;
pub const SPLIT_V2_CAPABILITY: &str = "model.split.2";
pub const MAX_SPLIT_TARGETS: usize = 128;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum SplitPlane {
    Authored { frame: PrimitiveFrame },
    Existing { plane: EntityId, offset: Dimension },
}
impl SplitPlane {
    pub fn validate(&self) -> Result<(), DocumentError> {
        match self {
            Self::Authored { frame } => frame.validate(),
            Self::Existing { plane, offset } => {
                if plane.is_nil() {
                    return Err(invalid("Split needs a non-nil construction plane"));
                }
                check_dimension(offset, Unit::Millimeter)
            }
        }
    }
    fn inputs(&self) -> Vec<Link> {
        match self {
            Self::Authored { frame } => parameter_links(frame.origin.iter().collect()),
            Self::Existing { plane, offset } => {
                let mut inputs = vec![Link::new(*plane, "splitting_plane")];
                inputs.extend(parameter_links(vec![offset]));
                inputs
            }
        }
    }
    pub fn resolve(
        &self,
        document: &Document,
        revision: u64,
        parameters: &EvaluatedParameters,
        report: Option<&crate::references::ReferenceReport>,
    ) -> Result<sketch::Frame, DocumentError> {
        self.validate()?;
        match self {
            Self::Authored { frame } => frame.resolve(parameters),
            Self::Existing { plane, offset } => {
                let mut frame = sketch::inspect_plane(document, *plane, revision, report)?;
                let offset = number(offset, Unit::Millimeter, parameters)?;
                frame.origin = std::array::from_fn(|i| frame.origin[i] + frame.n[i] * offset);
                Ok(frame)
            }
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum SplitTool {
    Body { input: BodyInput },
    Plane { source: SplitPlane },
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SplitSideV2 {
    InsideTool,
    OutsideTool,
    PositivePlane,
    NegativePlane,
}
impl SplitTool {
    pub fn sides(&self) -> [SplitSideV2; 2] {
        match self {
            Self::Body { .. } => [SplitSideV2::InsideTool, SplitSideV2::OutsideTool],
            Self::Plane { .. } => [SplitSideV2::PositivePlane, SplitSideV2::NegativePlane],
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SplitDefinitionV2 {
    pub output: BodyOutput,
    pub target: BodyInput,
    pub tool: SplitTool,
    pub side: SplitSideV2,
    pub keep_tools: bool,
}
impl SplitDefinitionV2 {
    pub fn validate(&self) -> Result<(), DocumentError> {
        let mut ids = vec![self.output.body, self.target.body, self.target.producer];
        match &self.tool {
            SplitTool::Body { input } => ids.extend([input.body, input.producer]),
            SplitTool::Plane { source } => {
                source.validate()?;
                if self.keep_tools {
                    return Err(invalid("A Split plane has no tool body to keep"));
                }
            }
        }
        if ids.iter().any(EntityId::is_nil)
            || ids.iter().collect::<BTreeSet<_>>().len() != ids.len()
        {
            return Err(invalid(
                "Split output, target and tool need distinct non-nil body/producer identities",
            ));
        }
        if !self.tool.sides().contains(&self.side) {
            return Err(invalid("Split side does not match its explicit tool kind"));
        }
        Ok(())
    }
    pub fn inputs(&self) -> Result<Vec<Link>, DocumentError> {
        self.validate()?;
        let mut inputs = vec![Link::new(
            self.target.producer,
            format!("target/{}/producer", self.target.body),
        )];
        match &self.tool {
            SplitTool::Body { input } => inputs.push(Link::new(
                input.producer,
                format!("tool/{}/producer", input.body),
            )),
            SplitTool::Plane { source } => inputs.extend(source.inputs()),
        }
        Ok(inputs)
    }
    pub fn definition(&self) -> Result<Definition, DocumentError> {
        self.validate()?;
        Ok(Definition {
            type_id: SPLIT_TYPE.into(),
            schema_version: SPLIT_V2_SCHEMA,
            data: serde_json::to_value(self).map_err(|e| invalid(e.to_string()))?,
        })
    }
    pub fn from_definition(definition: &Definition) -> Result<Self, DocumentError> {
        if definition.type_id != SPLIT_TYPE || definition.schema_version != SPLIT_V2_SCHEMA {
            return Err(invalid("Unsupported explicit Split provider/version"));
        }
        let source: Self =
            serde_json::from_value(definition.data.clone()).map_err(|e| invalid(e.to_string()))?;
        source.validate()?;
        Ok(source)
    }
    pub fn consumed_inputs(&self) -> Vec<BodyInput> {
        let mut inputs = vec![self.target.clone()];
        if !self.keep_tools
            && let SplitTool::Body { input } = &self.tool
        {
            inputs.push(input.clone());
        }
        inputs
    }
}
pub fn read_split_v2(
    document: &Document,
    feature: EntityId,
) -> Result<SplitDefinitionV2, DocumentError> {
    let Some(EntityData::Feature {
        definition, inputs, ..
    }) = document.get(feature).map(|e| &e.data)
    else {
        return Err(invalid("Choose an explicit Split feature"));
    };
    let source = SplitDefinitionV2::from_definition(definition)?;
    let expected = source.inputs()?;
    if inputs.len() != expected.len() || expected.iter().any(|link| !inputs.contains(link)) {
        return Err(invalid("Split source and canonical input links disagree"));
    }
    validate_body(document, &source.target)?;
    match &source.tool {
        SplitTool::Body { input } => validate_body(document, input)?,
        SplitTool::Plane {
            source: SplitPlane::Existing { plane, .. },
        } => {
            if !matches!(
                document.get(*plane).map(|e| &e.data),
                Some(EntityData::Plane { .. })
            ) {
                return Err(invalid(
                    "Split construction plane is missing or has the wrong kind",
                ));
            }
        }
        _ => {}
    }
    if let Some(entity) = document.get(source.output.body) {
        let EntityData::Part { features, .. } = &entity.data else {
            return Err(invalid("Split output must identify a Part"));
        };
        if !features.contains(&feature) {
            return Err(invalid(
                "Split output Part must contain its producing feature",
            ));
        }
    }
    Ok(source)
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SplitOutputV2 {
    pub feature: EntityId,
    pub body: EntityId,
    pub name: String,
    pub side: SplitSideV2,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SplitTarget {
    pub input: BodyInput,
    pub outputs: [SplitOutputV2; 2],
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SplitOperationRequest {
    pub targets: Vec<SplitTarget>,
    pub tool: SplitTool,
    pub keep_tools: bool,
}
impl SplitOperationRequest {
    pub fn new(targets: Vec<BodyInput>, tool: SplitTool, keep_tools: bool) -> Self {
        let sides = tool.sides();
        Self {
            targets: targets
                .into_iter()
                .map(|input| SplitTarget {
                    input,
                    outputs: sides.map(|side| SplitOutputV2 {
                        feature: EntityId::new(),
                        body: EntityId::new(),
                        name: match side {
                            SplitSideV2::InsideTool => "Split inside",
                            SplitSideV2::OutsideTool => "Split outside",
                            SplitSideV2::PositivePlane => "Split positive",
                            SplitSideV2::NegativePlane => "Split negative",
                        }
                        .into(),
                        side,
                    }),
                })
                .collect(),
            tool,
            keep_tools,
        }
    }
}
/// Source-only preparation; the native candidate gate must validate every side.
pub fn prepare_split_operation(
    session: &Session,
    expected_revision: u64,
    actor: Actor,
    request: &SplitOperationRequest,
) -> Result<ModelPrepared, CommandError> {
    revision(session, expected_revision)?;
    if request.targets.is_empty() || request.targets.len() > MAX_SPLIT_TARGETS {
        return Err(error("Select between 1 and 128 explicit Split targets"));
    }
    let targets: BTreeSet<_> = request
        .targets
        .iter()
        .map(|target| target.input.body)
        .collect();
    if targets.len() != request.targets.len() {
        return Err(error("Each Split target must be a distinct Part"));
    }
    match &request.tool {
        SplitTool::Body { input } => {
            if targets.contains(&input.body) {
                return Err(error("A Split target cannot also be the splitting tool"));
            }
            available(session.document(), input, true)?;
        }
        SplitTool::Plane { source } => {
            source.validate()?;
            if let SplitPlane::Existing { plane, .. } = source
                && !matches!(
                    session.document().get(*plane).map(|entity| &entity.data),
                    Some(EntityData::Plane { .. })
                )
            {
                return Err(error("Choose an existing canonical construction plane"));
            }
        }
    }
    let index = hierarchy::Index::new(session.document()).map_err(|e| error(e.to_string()))?;
    let mut hierarchy = session.document().hierarchy().clone();
    let mut ids = BTreeSet::new();
    let mut edits = vec![];
    let mut outputs = vec![];
    for target in &request.targets {
        available(session.document(), &target.input, true)?;
        let sides: BTreeSet<_> = target.outputs.iter().map(|output| output.side).collect();
        if sides != BTreeSet::from(request.tool.sides()) {
            return Err(error(
                "Each Split target needs its two explicit semantic output sides",
            ));
        }
        let parent = index
            .parent(target.input.body)
            .map_err(|e| error(e.to_string()))?;
        for output in &target.outputs {
            for id in [output.feature, output.body] {
                if id.is_nil()
                    || id == session.document().id()
                    || session.document().get(id).is_some()
                    || !ids.insert(id)
                {
                    return Err(error(
                        "Split outputs require new, distinct non-nil identities",
                    ));
                }
            }
            if output.name.trim().is_empty() || output.name.len() > 1000 {
                return Err(error("Split output name must contain 1–1000 bytes"));
            }
            let source = SplitDefinitionV2 {
                output: BodyOutput { body: output.body },
                target: target.input.clone(),
                tool: request.tool.clone(),
                side: output.side,
                keep_tools: request.keep_tools,
            };
            let mut feature = Entity::new(
                &output.name,
                EntityData::Feature {
                    definition: source.definition()?,
                    inputs: source.inputs()?,
                    suppressed: false,
                },
            );
            feature.id = output.feature;
            let mut part = session
                .document()
                .get(target.input.body)
                .expect("validated target")
                .clone();
            part.id = output.body;
            part.name = output.name.clone();
            let EntityData::Part { features, .. } = &mut part.data else {
                unreachable!()
            };
            *features = vec![output.feature];
            edits.extend([Mutation::Insert(feature), Mutation::Insert(part)]);
            if let Some(parent) = parent {
                hierarchy
                    .members
                    .entry(parent)
                    .or_default()
                    .extend([output.feature, output.body]);
            }
            outputs.push(CreatedBody {
                feature: output.feature,
                body: output.body,
            });
        }
    }
    if hierarchy != *session.document().hierarchy() {
        edits.push(Mutation::SetHierarchy(hierarchy));
    }
    let prepared = session.prepare(Request {
        expected_revision,
        actor,
        label: "Split bodies".into(),
        edits,
    })?;
    Ok(ModelPrepared { prepared, outputs })
}
