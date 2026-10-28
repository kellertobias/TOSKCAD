use super::*;
use crate::{
    document::Link,
    references::{GeometryIntent, Origin, ReferenceFrame, TopologyKind},
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub const PLANAR_CAPABILITY: &str = "exchange.planar_faces.1";
pub const MAX_PLANAR_BINDINGS: usize = 128;
pub const MAX_PLANAR_FACE_CHOICES: usize = 4096;
pub const PLANAR_FACE_ROLE: &str = "planar_face";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanarAxis {
    WorldX,
    WorldY,
    WorldZ,
}
impl PlanarAxis {
    /// Project the explicitly chosen world direction onto the current face.
    /// The origin is the world origin projected onto that plane. This policy
    /// keeps world placements observable and does not depend on native UV axes.
    pub fn frame(self, face: ReferenceFrame) -> Result<ReferenceFrame, String> {
        face.validate()?;
        let n = face.normal();
        let axis = match self {
            Self::WorldX => [1., 0., 0.],
            Self::WorldY => [0., 1., 0.],
            Self::WorldZ => [0., 0., 1.],
        };
        let dot = |a: [f64; 3], b: [f64; 3]| a.into_iter().zip(b).map(|(a, b)| a * b).sum::<f64>();
        let d = dot(axis, n);
        let mut u = std::array::from_fn::<_, 3, _>(|i| axis[i] - d * n[i]);
        let length = dot(u, u).sqrt();
        if length < 1e-9 {
            return Err(
                "Chosen horizontal world axis is normal to this face; choose another axis".into(),
            );
        }
        u = u.map(|v| v / length);
        let v = [
            n[1] * u[2] - n[2] * u[1],
            n[2] * u[0] - n[0] * u[2],
            n[0] * u[1] - n[1] * u[0],
        ];
        let offset = dot(face.origin_mm, n);
        let result = ReferenceFrame {
            origin_mm: n.map(|v| v * offset),
            u,
            v,
        };
        result.validate()?;
        Ok(result)
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanarFace {
    pub recipe: EntityId,
    pub subject: PartRef,
    pub source: EntityId,
    pub reference: EntityId,
    pub horizontal: PlanarAxis,
}
impl PlanarFace {
    pub fn validate(&self) -> Result<(), DocumentError> {
        let expected = self
            .subject
            .occurrence_path
            .first()
            .copied()
            .unwrap_or(self.subject.part);
        if self.recipe.is_nil()
            || self.reference.is_nil()
            || self.subject.part.is_nil()
            || self.source != expected
            || self.subject.occurrence_path.len() > 32
        {
            return Err(DocumentError::Invalid(
                "Invalid explicit planar face binding".into(),
            ));
        }
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanarFaceChoice {
    pub source: EntityId,
    pub intent: GeometryIntent,
    pub boundary: BTreeSet<Origin>,
    pub plane: ReferenceFrame,
    pub area_mm2: f64,
    pub label: String,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanarPartChoices {
    pub subject: PartRef,
    pub name: String,
    pub faces: Vec<PlanarFaceChoice>,
    pub diagnostics: Vec<String>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanarCatalog {
    pub stamp: ExportStamp,
    pub scope: Scope,
    pub parts: Vec<PlanarPartChoices>,
}
/// Native developed-sheet modules may return this same exact contour contract.
/// This is evaluated output, never a claim that an unfolding provider exists.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StampedPlanarContours {
    pub stamp: ExportStamp,
    pub source: EntityId,
    pub contours: crate::manufacturing::PlanarContours,
}

pub fn geometry_links(document: &Document, id: EntityId) -> Vec<Link> {
    match document.get(id).map(|e| &e.data) {
        Some(EntityData::ExportSet { inputs, .. }) => inputs
            .iter()
            .filter(|l| l.role == PLANAR_FACE_ROLE)
            .cloned()
            .collect(),
        _ => vec![],
    }
}
/// A valid saved planar-output reference drives only its export, never a solid.
/// Publication gates may omit it while retaining its complete repair diagnostic.
pub fn is_non_driving_planar_reference(
    document: &Document,
    diagnostic: &crate::references::Diagnostic,
) -> bool {
    if diagnostic.role != PLANAR_FACE_ROLE {
        return false;
    }
    let Ok(definition) = read(document, diagnostic.address.owner) else {
        return false;
    };
    definition
        .planar_faces
        .iter()
        .any(|f| f.reference == diagnostic.address.reference && f.source == diagnostic.source)
        && geometry_links(document, diagnostic.address.owner)
            .iter()
            .any(|l| {
                l.target == diagnostic.source && l.geometry.as_ref() == Some(&diagnostic.intent)
            })
}
pub(super) fn checked_inputs(
    definition: &SetDefinition,
    geometry: &[Link],
) -> Result<Vec<Link>, DocumentError> {
    let refs: BTreeSet<_> = definition
        .planar_faces
        .iter()
        .map(|f| f.reference)
        .collect();
    if geometry.len() != refs.len()
        || geometry
            .iter()
            .map(|l| l.geometry.as_ref().map(|g| g.id))
            .collect::<BTreeSet<_>>()
            .len()
            != geometry.len()
    {
        return Err(DocumentError::Invalid(
            "Planar face reference links must match the explicit recipe bindings".into(),
        ));
    }
    let mut result = definition.inputs();
    for binding in &definition.planar_faces {
        let link = geometry
            .iter()
            .find(|l| {
                l.geometry
                    .as_ref()
                    .is_some_and(|g| g.id == binding.reference)
            })
            .ok_or_else(|| DocumentError::Invalid("Planar face reference is missing".into()))?;
        if link.target != binding.source
            || link.role != PLANAR_FACE_ROLE
            || link.geometry.as_ref().is_none_or(|g| {
                g.kind != TopologyKind::Face || g.attachment.is_some() || g.validate().is_err()
            })
        {
            return Err(DocumentError::Invalid(
                "Planar recipe requires a matching persistent face intent".into(),
            ));
        }
        result.push(link.clone());
    }
    Ok(result)
}
