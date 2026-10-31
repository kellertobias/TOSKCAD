//! Native supporting-face boundaries, published as owned sketch receipts.
use crate::{ModelError, error, external::Sources};
use multicad_core::{
    document::EntityId,
    project_file::ContentHash,
    references::{GeometryIntent, ReferenceAddress, ReferenceReport, Resolution},
    sketch_regions::{SketchSupport, supporting_face},
};
use multicad_kernel::PlaneFrame;
use std::collections::BTreeSet;

impl Sources<'_> {
    fn support(
        &self,
        sketch: EntityId,
        report: &ReferenceReport,
        source_hash: ContentHash,
    ) -> Result<SketchSupport, ModelError> {
        let failure = |e: String| error(Some(sketch), e);
        let (plane, link) = supporting_face(self.document, sketch)
            .ok_or_else(|| failure("Sketch has no supporting face".into()))?;
        let intent = link.geometry.as_ref().expect("face intent");
        let entry = report
            .get(ReferenceAddress {
                owner: plane,
                reference: intent.id,
            })
            .ok_or_else(|| failure("Supporting face has no current resolution".into()))?;
        let Resolution::Resolved { key, .. } = entry.resolution else {
            return Err(failure(
                "Supporting face is missing or ambiguous; repair the sketch plane".into(),
            ));
        };
        let solid = self.solids.get(&link.target).ok_or_else(|| {
            failure("Supporting face producer has no current native solid".into())
        })?;
        let topology = self
            .topologies
            .get(&link.target)
            .ok_or_else(|| failure("Supporting face has no native topology".into()))?;
        let frame = self.frame(sketch, report)?;
        let face = solid
            .faces()
            .map_err(|e| failure(e.to_string()))?
            .into_iter()
            .find(|f| f.index == key.ordinal)
            .ok_or_else(|| failure("Supporting native face is missing".into()))?;
        let (origin, normal) = face
            .plane
            .ok_or_else(|| failure("Sketch region support must be planar".into()))?;
        let dot = |a: [f64; 3], b: [f64; 3]| a.into_iter().zip(b).map(|(a, b)| a * b).sum::<f64>();
        if dot(normal, frame.n).abs() < 1. - 1e-8 {
            return Err(failure(
                "Supporting face and sketch plane are no longer parallel".into(),
            ));
        }
        // Offset face planes retain the same finite UV perimeter. Native wire
        // extraction still occurs on the actual face, never on display edges.
        let offset = dot(std::array::from_fn(|i| frame.origin[i] - origin[i]), normal);
        let native_frame = PlaneFrame {
            origin: std::array::from_fn(|i| frame.origin[i] - normal[i] * offset),
            u: frame.u,
            v: frame.v,
        };
        let mut sourced = solid
            .planar_face_contours_with_sources(key.ordinal, native_frame)
            .map_err(|e| failure(format!("Supporting face boundary unavailable: {e}")))?;
        sourced.contours.frame.origin_mm = frame.origin;
        let mut seen = BTreeSet::new();
        let edges = sourced
            .edge_keys
            .iter()
            .map(|keys| {
                keys.iter()
                    .map(|key| {
                        let mut intent =
                            GeometryIntent::capture(topology, *key).map_err(failure)?;
                        // GeometryIntent::capture allocates a reference UUID. Naming a
                        // derived edge instead hashes its plane and semantic selector.
                        intent.id = EntityId::nil();
                        let bytes = serde_json::to_vec(&(plane, intent))
                            .map_err(|e| failure(e.to_string()))?;
                        let hash = ContentHash::of(&bytes).to_string();
                        let id = EntityId::parse(&format!(
                            "{}-{}-{}-{}-{}",
                            &hash[..8],
                            &hash[8..12],
                            &hash[12..16],
                            &hash[16..20],
                            &hash[20..32]
                        ))
                        .map_err(|e| failure(e.to_string()))?;
                        if !seen.insert(id) {
                            return Err(failure(
                                "Supporting boundary edges have ambiguous semantic identities"
                                    .into(),
                            ));
                        }
                        Ok(id)
                    })
                    .collect::<Result<Vec<_>, ModelError>>()
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(SketchSupport {
            document: self.document.id(),
            revision: self.revision,
            source_hash,
            contours: sourced.contours,
            edges,
        })
    }

    pub(crate) fn populate_supports(
        &self,
        report: &mut ReferenceReport,
        only: Option<EntityId>,
        interrupt: Option<&dyn Fn() -> bool>,
    ) -> Result<(), ModelError> {
        let ids: Vec<_> = match only {
            Some(id) => vec![id],
            None => self.document.entities().map(|e| e.id).collect(),
        }
        .into_iter()
        .filter(|id| supporting_face(self.document, *id).is_some())
        .collect();
        if ids.is_empty() {
            return Ok(());
        }
        let source_hash = ContentHash::document(self.document).map_err(|e| error(None, e))?;
        for id in ids {
            if interrupt.is_some_and(|f| f()) {
                return Err(error(Some(id), "Sketch support evaluation cancelled"));
            }
            let result = self
                .support(id, report, source_hash)
                .map_err(|e| e.to_string());
            report.sketch_supports.insert(id, result);
        }
        Ok(())
    }
}
