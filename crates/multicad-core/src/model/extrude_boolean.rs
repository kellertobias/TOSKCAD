//! One original-session transaction for new extrusion tools and explicit body
//! operations. Exact native evaluation is required before applying this source.
use super::{
    BodyInput, BooleanRequest, ExtrudeRequest, ModelPrepared, Operation,
    common::{error, revision},
    prepare_booleans, prepare_extrude,
};
use crate::{
    commands::{Actor, CommandError, Request, Session},
    document::{Document, Mutation},
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExtrudeBooleanRequest {
    pub extrusion: ExtrudeRequest,
    pub booleans: Vec<BooleanRequest>,
}
impl ExtrudeBooleanRequest {
    /// Allocate once when the tool begins. Repeated previews preserve both
    /// extrusion and final Boolean producer identities.
    pub fn new(
        extrusion: ExtrudeRequest,
        operation: Operation,
        targets: Vec<BodyInput>,
        keep_tools: bool,
    ) -> Self {
        let tools: Vec<_> = extrusion
            .outputs
            .iter()
            .map(|output| BodyInput {
                body: output.body,
                producer: output.feature,
            })
            .collect();
        let booleans = targets
            .into_iter()
            .enumerate()
            .map(|(index, target)| {
                let mut request = BooleanRequest::new(operation, target, tools.clone());
                request.keep_tools = keep_tools;
                request.name = format!("{operation:?} extrusion {}", index + 1);
                request
            })
            .collect();
        Self {
            extrusion,
            booleans,
        }
    }
    fn validate(&self) -> Result<(), CommandError> {
        if self.extrusion.operation != Operation::NewBody
            || !self.extrusion.targets.is_empty()
            || self.extrusion.keep_tools
        {
            return Err(error(
                "Composite extrusion tools must use NewBody without targets or keep-tools",
            ));
        }
        let Some(first) = self.booleans.first() else {
            return Err(error("Choose explicit extrusion Boolean targets"));
        };
        if !matches!(
            first.operation,
            Operation::Join | Operation::Cut | Operation::Intersect
        ) {
            return Err(error("Composite extrusion requires Join, Cut or Intersect"));
        }
        let mut ids = BTreeSet::new();
        let tools: BTreeSet<_> = self
            .extrusion
            .outputs
            .iter()
            .map(|o| (o.body, o.feature))
            .collect();
        for output in &self.extrusion.outputs {
            for id in [output.feature, output.body] {
                if id.is_nil() || !ids.insert(id) {
                    return Err(error(
                        "Composite extrusion identities must be distinct and non-nil",
                    ));
                }
            }
        }
        for boolean in &self.booleans {
            if boolean.feature.is_nil() || !ids.insert(boolean.feature) {
                return Err(error(
                    "Composite Boolean identities must be distinct from all tool identities",
                ));
            }
            if boolean.operation != first.operation || boolean.keep_tools != first.keep_tools {
                return Err(error(
                    "Composite Boolean operation and keep-tools must agree for every target",
                ));
            }
            if boolean.tools.len() != tools.len()
                || boolean
                    .tools
                    .iter()
                    .map(|t| (t.body, t.producer))
                    .collect::<BTreeSet<_>>()
                    != tools
            {
                return Err(error(
                    "Each composite target must use exactly the newly extruded tool bodies and producers",
                ));
            }
        }
        Ok(())
    }
}
fn delta(original: &Document, candidate: &Document) -> Result<Vec<Mutation>, CommandError> {
    if original.id() != candidate.id() || original.name() != candidate.name() {
        return Err(error(
            "Composite modeling must retain the original document identity and name",
        ));
    }
    let mut edits = vec![];
    for entity in original.entities() {
        if candidate.get(entity.id).is_none() {
            edits.push(Mutation::Remove(entity.id));
        }
    }
    for entity in candidate.entities() {
        match original.get(entity.id) {
            None => edits.push(Mutation::Insert(entity.clone())),
            Some(existing) if existing != entity => edits.push(Mutation::Replace(entity.clone())),
            Some(_) => {}
        }
    }
    if original.hierarchy() != candidate.hierarchy() {
        edits.push(Mutation::SetHierarchy(candidate.hierarchy().clone()));
    }
    Ok(edits)
}
/// Prepare a single undoable, attributable candidate on the caller's Session.
/// Tool Parts remain canonical; outputs identify the final target producers.
/// No private Session is committed and no private Prepared escapes this call.
pub fn prepare_extrude_boolean(
    session: &Session,
    expected_revision: u64,
    actor: Actor,
    request: &ExtrudeBooleanRequest,
) -> Result<ModelPrepared, CommandError> {
    revision(session, expected_revision)?;
    request.validate()?;
    let extrusion = prepare_extrude(
        session,
        expected_revision,
        actor.clone(),
        &request.extrusion,
    )?;
    let derived = Session::new(extrusion.prepared.document().clone())?;
    let booleans = prepare_booleans(
        &derived,
        derived.revision(),
        actor.clone(),
        &request.booleans,
    )?;
    let candidate = booleans.prepared.document();
    let prepared = session.prepare(Request {
        expected_revision,
        actor,
        label: format!("Extrude and {:?}", request.booleans[0].operation),
        edits: delta(session.document(), candidate)?,
    })?;
    if prepared.document() != candidate {
        return Err(error(
            "Composite source preparation did not preserve the exact derived candidate",
        ));
    }
    Ok(ModelPrepared {
        prepared,
        outputs: booleans.outputs,
    })
}
