//! Compile a semantic partition only after the entire exact pair is valid.
use crate::{ModelError, compiler::error};
use multicad_core::{
    document::{Document, EntityId},
    model::{SplitDefinition, SplitSide},
};
use multicad_kernel::{GeometryPolicy, Solid};
use std::collections::BTreeMap;
pub(crate) fn build(
    document: &Document,
    feature: EntityId,
    source: &SplitDefinition,
    solids: &BTreeMap<EntityId, Solid>,
    policy: GeometryPolicy,
    interrupt: Option<&dyn Fn() -> bool>,
) -> Result<Solid, ModelError> {
    source
        .target
        .resolve_producer(document)
        .map_err(|e| error(Some(feature), e))?;
    source
        .tool
        .resolve_producer(document)
        .map_err(|e| error(Some(feature), e))?;
    let target = solids
        .get(&source.target.producer)
        .ok_or_else(|| error(Some(feature), "Split target has no current exact solid"))?;
    let tool = solids
        .get(&source.tool.producer)
        .ok_or_else(|| error(Some(feature), "Split tool has no current exact solid"))?;
    let pair = target
        .split_by_solid_interruptible(tool, policy, interrupt)
        .map_err(|e| error(Some(feature), e))?;
    Ok(match source.side {
        SplitSide::InsideTool => pair.inside,
        SplitSide::OutsideTool => pair.outside,
    })
}
