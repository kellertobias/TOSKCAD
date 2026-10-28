use super::*;
use crate::advanced::{Path, ProfileInputs};
use multicad_core::{
    constraint_sketch::CurveKind,
    model::{AdvancedDefinition, AdvancedParameters, SketchPathSource},
    sketch::Geometry,
};
impl Profiles<'_> {
    pub(super) fn advanced(
        &mut self,
        source: &AdvancedDefinition,
        feature: EntityId,
        solids: &BTreeMap<EntityId, Solid>,
        topologies: &BTreeMap<EntityId, TopologySnapshot>,
        interrupt: Option<&dyn Fn() -> bool>,
    ) -> Result<ProfileInputs, ModelError> {
        let mut prepared = ProfileInputs::default();
        for (index, selection) in source.profiles.iter().enumerate() {
            checkpoint(interrupt)?;
            let (mut profile, mut origins) = self.get(
                selection.sketch,
                &selection.region,
                feature,
                solids,
                topologies,
                interrupt,
            )?;
            if let AdvancedParameters::Loft(parameters) = &source.parameters {
                if !profile.holes.is_empty() {
                    return Err(error(
                        Some(feature),
                        "Loft sections with holes are unsupported; choose one closed outer boundary",
                    ));
                }
                let seam = parameters.seams[index];
                let starts = origins
                    .iter()
                    .enumerate()
                    .filter(|(_, o)| o.source == seam)
                    .map(|(i, _)| i)
                    .collect::<Vec<_>>();
                if starts.len() != 1 {
                    return Err(error(
                        Some(feature),
                        "The explicit loft seam curve is missing or split; choose an unambiguous authored boundary curve",
                    ));
                }
                profile.outer.rotate_left(starts[0]);
                origins.rotate_left(starts[0]);
            }
            prepared.sections.push((profile, origins));
        }
        if let Some(path) = &source.path {
            prepared.path = Some(self.advanced_path(path, feature, solids, topologies, interrupt)?);
        }
        Ok(prepared)
    }
    fn advanced_path(
        &self,
        path: &SketchPathSource,
        feature: EntityId,
        solids: &BTreeMap<EntityId, Solid>,
        topologies: &BTreeMap<EntityId, TopologySnapshot>,
        interrupt: Option<&dyn Fn() -> bool>,
    ) -> Result<Path, ModelError> {
        let mut report = references::inspect(
            self.context.document(),
            self.context.stamp().revision,
            topologies,
        );
        crate::external::Sources {
            document: self.context.document(),
            revision: self.context.stamp().revision,
            parameters: self.context.parameters(),
            body_heads: self.body_heads,
            solids,
            topologies,
            policy: self.policy,
        }
        .populate(&mut report, Some(path.sketch), interrupt)?;
        let evaluated = self
            .context
            .evaluate(path.sketch, Some(&report), interrupt)
            .map_err(|e| error(Some(feature), e))?;
        let geometry = evaluated.geometry().map_err(|e| error(Some(feature), e))?;
        let mut points = vec![];
        let mut origins = vec![];
        let mut previous = None;
        let mut first = None;
        for id in &path.curves {
            let curve = evaluated
                .canonical()
                .sketch
                .curves
                .iter()
                .find(|c| c.id == *id)
                .ok_or_else(|| {
                    error(
                        Some(feature),
                        "An authored sweep path curve disappeared; select the ordered path again",
                    )
                })?;
            if curve.construction {
                return Err(error(
                    Some(feature),
                    "Sweep path contains a construction curve; choose authored non-construction lines or restore the path curve",
                ));
            }
            let CurveKind::Line { start, end } = curve.kind else {
                return Err(error(
                    Some(feature),
                    "This sweep provider supports an open ordered polyline; arcs and splines are unsupported",
                ));
            };
            if previous.is_some_and(|previous| previous != start) {
                return Err(error(
                    Some(feature),
                    "Sweep path lines must share authored endpoints in the selected forward order",
                ));
            }
            let Some(Geometry::Line { a, b, .. }) = geometry.iter().find(|g| g.id() == *id) else {
                return Err(error(
                    Some(feature),
                    "Sweep path has no current solved straight geometry",
                ));
            };
            if points.is_empty() {
                points.push(evaluated.frame.point(*a));
                first = Some(start);
            }
            points.push(evaluated.frame.point(*b));
            previous = Some(end);
            origins.push(Origin {
                source: *id,
                role: "path.edge".into(),
            });
        }
        if first == previous {
            return Err(error(Some(feature), "Sweep path must remain open"));
        }
        Ok((points, origins))
    }
}
