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

pub(crate) struct ExplicitSplit<'a> {
    pub document: &'a Document,
    pub revision: u64,
    pub parameters: &'a multicad_core::parameters::EvaluatedParameters,
    pub solids: &'a BTreeMap<EntityId, Solid>,
    pub topologies: &'a BTreeMap<EntityId, multicad_core::references::TopologySnapshot>,
    pub policy: GeometryPolicy,
    pub interrupt: Option<&'a dyn Fn() -> bool>,
}
impl ExplicitSplit<'_> {
    pub fn build(
        &self,
        feature: EntityId,
        source: &multicad_core::model::SplitDefinitionV2,
    ) -> Result<Solid, ModelError> {
        use multicad_core::model::{SplitSideV2, SplitTool};
        source
            .target
            .resolve_producer(self.document)
            .map_err(|e| error(Some(feature), e))?;
        let target = self
            .solids
            .get(&source.target.producer)
            .ok_or_else(|| error(Some(feature), "Split target has no current exact solid"))?;
        match &source.tool {
            SplitTool::Body { input } => {
                input
                    .resolve_producer(self.document)
                    .map_err(|e| error(Some(feature), e))?;
                let tool = self.solids.get(&input.producer).ok_or_else(|| {
                    error(
                        Some(feature),
                        "Split tool has no current exact closed solid",
                    )
                })?;
                let pair = target
                    .split_by_solid_interruptible(tool, self.policy, self.interrupt)
                    .map_err(|e| error(Some(feature), e))?;
                match source.side {
                    SplitSideV2::InsideTool => Ok(pair.inside),
                    SplitSideV2::OutsideTool => Ok(pair.outside),
                    _ => Err(error(
                        Some(feature),
                        "Body splitter has a plane-side source",
                    )),
                }
            }
            SplitTool::Plane { source: plane } => {
                if target.uses_rust_geometry() {
                    return Err(error(
                        Some(feature),
                        "Rust Split currently requires a closed tool body",
                    ));
                }
                let report = multicad_core::references::inspect(
                    self.document,
                    self.revision,
                    self.topologies,
                );
                let frame = plane
                    .resolve(self.document, self.revision, self.parameters, Some(&report))
                    .map_err(|e| error(Some(feature), e))?;
                let pair = target
                    .split_by_plane_named(
                        multicad_kernel::PlaneFrame {
                            origin: frame.origin,
                            u: frame.u,
                            v: frame.v,
                        },
                        feature,
                        self.policy,
                        self.interrupt,
                    )
                    .map_err(|e| error(Some(feature), e))?;
                match source.side {
                    SplitSideV2::PositivePlane => Ok(pair.positive),
                    SplitSideV2::NegativePlane => Ok(pair.negative),
                    _ => Err(error(
                        Some(feature),
                        "Plane splitter has a tool-membership source",
                    )),
                }
            }
        }
    }
}
