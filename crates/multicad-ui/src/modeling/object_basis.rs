//! Authored coordinate frames, independent of mesh tessellation and face order.
use super::GizmoBasis;
use crate::viewport::Scene;
use multicad_core::{
    constraint_sketch::source::Source,
    document::{Document, EntityData, EntityId},
    model::{self, PrimitiveParameters},
    parameters::EvaluatedParameters,
    project_file::ContentHash,
    references::{self, ReferenceReport, TopologySnapshot},
    sketch::{self, Frame},
};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ObjectGizmoBasis {
    /// The smallest explicit selected identity; selection order cannot silently
    /// change the basis or fall through to a different compatible object.
    pub primary: EntityId,
    pub basis: GizmoBasis,
}
pub fn object_gizmo_basis(
    document: &Document,
    revision: u64,
    scene: &Scene,
    selected: &BTreeSet<EntityId>,
) -> Result<ObjectGizmoBasis, String> {
    if !super::placement::available(document, revision, scene, selected) {
        return Err("Object coordinates require selected Parts or Instances in the complete current native model".into());
    }
    if scene
        .model_evaluation()
        .is_none_or(|info| ContentHash::document(document).ok() != Some(info.source_hash))
    {
        return Err("Object coordinates require the current source-bound native model".into());
    }
    let primary = *selected.first().expect("nonempty selection checked");
    let occurrence = scene
        .reference_instances()
        .iter()
        .find(|i| i.id == primary)
        .ok_or("The primary selected object is absent from the native model")?;
    let parameters = multicad_core::parameters::evaluate(document).map_err(|e| e.to_string())?;
    let snapshots = scene.reference_snapshots();
    let basis = match document.get(primary).map(|e| &e.data) {
        Some(EntityData::Instance { component, .. }) => {
            let pose = model::resolve_instance_pose_with_snapshots(
                document,
                primary,
                &parameters,
                revision,
                &snapshots,
            )
            .map_err(|e| e.to_string())?;
            if active_producer(document, *component)? != occurrence.geometry
                || pose
                    .translation_mm
                    .into_iter()
                    .zip(occurrence.transform.point([0.; 3]))
                    .any(|(a, b)| (a - b).abs() > 1e-8)
            {
                return Err("The native Instance producer or pose is stale".into());
            }
            let axes = GizmoBasis::default()
                .axes
                .map(|axis| occurrence.transform.vector(axis));
            for (axis, actual) in GizmoBasis::default().axes.into_iter().zip(axes) {
                if pose
                    .vector(axis)
                    .into_iter()
                    .zip(actual)
                    .any(|(a, b)| (a - b).abs() > 1e-9)
                {
                    return Err(
                        "The native Instance orientation disagrees with its canonical pose".into(),
                    );
                }
            }
            GizmoBasis { axes }
        }
        Some(EntityData::Part { .. }) => {
            let producer = active_producer(document, primary)?;
            if occurrence.geometry != producer {
                return Err("The primary Part producer is stale".into());
            }
            let report = references::inspect(document, revision, &snapshots);
            Resolver {
                document,
                revision,
                parameters: &parameters,
                snapshots: &snapshots,
                report: &report,
            }
            .feature(producer, &mut BTreeSet::new())?
        }
        _ => return Err("Choose a Part or explicit Instance for Object coordinates".into()),
    };
    basis.validate()?;
    Ok(ObjectGizmoBasis { primary, basis })
}
fn active_producer(document: &Document, body: EntityId) -> Result<EntityId, String> {
    let Some(EntityData::Part { features, .. }) = document.get(body).map(|e| &e.data) else {
        return Err("Object coordinates currently require a Part component".into());
    };
    features
        .iter()
        .rev()
        .find(|id| {
            matches!(
                document.get(**id).map(|e| &e.data),
                Some(EntityData::Feature {
                    suppressed: false,
                    ..
                })
            )
        })
        .copied()
        .ok_or_else(|| "The selected Part has no active producer".into())
}
struct Resolver<'a> {
    document: &'a Document,
    revision: u64,
    parameters: &'a EvaluatedParameters,
    snapshots: &'a BTreeMap<EntityId, TopologySnapshot>,
    report: &'a ReferenceReport,
}
impl Resolver<'_> {
    fn feature(
        &self,
        feature: EntityId,
        seen: &mut BTreeSet<EntityId>,
    ) -> Result<GizmoBasis, String> {
        if seen.len() >= 128 || !seen.insert(feature) {
            return Err(
                "Object coordinate history is cyclic or exceeds the supported depth".into(),
            );
        }
        let Some(EntityData::Feature { definition, .. }) =
            self.document.get(feature).map(|e| &e.data)
        else {
            return Err("Object coordinate source feature is missing".into());
        };
        let error = |e: multicad_core::document::DocumentError| e.to_string();
        match definition.type_id.as_str() {
            model::PRIMITIVE_TYPE => {
                let source = model::read_primitive(self.document, feature).map_err(error)?;
                let frame = match source.parameters {
                    PrimitiveParameters::Box { frame, .. }
                    | PrimitiveParameters::Cylinder { frame, .. } => frame,
                }
                .resolve(self.parameters)
                .map_err(error)?;
                Ok(frame_basis(frame))
            }
            model::TRANSFORM_TYPE => {
                let source = model::read_transform(self.document, feature).map_err(error)?;
                let upstream = self.feature(source.input.producer, seen)?;
                let pose = source.parameters.resolve(self.parameters).map_err(error)?;
                Ok(GizmoBasis {
                    axes: upstream.axes.map(|axis| pose.vector(axis)),
                })
            }
            model::ALIGN_TYPE => {
                let (source, references) =
                    model::read_alignment(self.document, feature).map_err(error)?;
                let upstream = self.feature(source.input.producer, seen)?;
                let pose = model::resolve_alignment(
                    self.document,
                    self.revision,
                    self.parameters,
                    &source.parameters,
                    &references,
                    self.snapshots,
                    Default::default(),
                )
                .map_err(error)?;
                Ok(GizmoBasis {
                    axes: upstream.axes.map(|axis| pose.vector(axis)),
                })
            }
            model::BOOLEAN_TYPE => {
                let source = model::read_boolean(self.document, feature).map_err(error)?;
                self.feature(source.target.producer, seen)
            }
            model::SPLIT_TYPE => {
                let source = model::read_split(self.document, feature).map_err(error)?;
                self.feature(source.target.producer, seen)
            }
            model::EXTRUDE_TYPE => {
                let source = model::read_extrude(self.document, feature).map_err(error)?;
                if source.operation != model::Operation::NewBody {
                    let target = source
                        .targets
                        .iter()
                        .find(|t| t.body == source.output.body)
                        .ok_or("Extrusion has no unique target coordinate frame")?;
                    self.feature(target.producer, seen)
                } else {
                    self.sketch(source.profile.sketch)
                }
            }
            model::REVOLVE_TYPE => {
                let source = model::read_revolve(self.document, feature).map_err(error)?;
                self.sketch(source.profile.sketch)
            }
            provider => Err(format!(
                "Object coordinates are unavailable for feature provider {provider}; choose World or a custom datum"
            )),
        }
    }
    fn sketch(&self, id: EntityId) -> Result<GizmoBasis, String> {
        let Some(EntityData::Sketch { definition, .. }) = self.document.get(id).map(|e| &e.data)
        else {
            return Err("The authored sketch coordinate frame is missing".into());
        };
        let frame = if definition.type_id == multicad_core::constraint_sketch::source::TYPE {
            Source::read_for_evaluation(self.document, id, self.revision, Some(self.report))
                .map_err(|e| e.to_string())?
                .1
        } else {
            sketch::Sketch::read_evaluated(self.document, id, self.revision, self.report)
                .map_err(|e| e.to_string())?
                .1
        };
        Ok(frame_basis(frame))
    }
}
fn frame_basis(frame: Frame) -> GizmoBasis {
    GizmoBasis {
        axes: [frame.u, frame.v, frame.n],
    }
}

#[cfg(test)]
mod tests;
