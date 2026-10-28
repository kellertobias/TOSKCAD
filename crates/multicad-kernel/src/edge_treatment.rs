//! Exact edge treatments. Keys are evaluation-local; caller-supplied origins
//! retain the authored selection identity through native generation history.
use crate::{GeometryError, GeometryErrorKind, GeometryPolicy, Solid, ffi};
use multicad_core::{
    document::EntityId,
    references::{GeometryIntent, Origin, TopologyKey, TopologyKind},
};
use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum EdgeTreatment {
    Fillet { radius_mm: f64 },
    Chamfer { distance_mm: f64 },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TreatmentEdge {
    pub key: TopologyKey,
    pub origin: Origin,
}

fn failure(kind: GeometryErrorKind, message: impl Into<String>) -> GeometryError {
    GeometryError {
        operation: "edge treatment",
        kind,
        message: message.into(),
    }
}

impl Solid {
    /// Constant-radius fillets and symmetric-distance chamfers only. Invalid
    /// treatment results never replace or mutate the input B-rep.
    pub fn treat_edges_named(
        &self,
        edges: &[TreatmentEdge],
        treatment: EdgeTreatment,
        feature: EntityId,
        policy: GeometryPolicy,
    ) -> Result<Self, GeometryError> {
        policy.validate()?;
        let (mode, size) = match treatment {
            EdgeTreatment::Fillet { radius_mm } => (0, radius_mm),
            EdgeTreatment::Chamfer { distance_mm } => (1, distance_mm),
        };
        policy.length(size)?;
        if feature.is_nil() || edges.is_empty() || edges.len() > 128 {
            return Err(failure(
                GeometryErrorKind::Input,
                "Edge treatment needs a feature identity and between 1 and 128 selected edges",
            ));
        }
        if self.solid_count()? != 1 {
            return Err(failure(
                GeometryErrorKind::Input,
                "Edge treatment requires one connected solid",
            ));
        }
        let snapshot = self.topology_snapshot(feature, 0)?;
        let available: BTreeSet<_> = snapshot.elements.iter().map(|e| e.key).collect();
        let mut keys = BTreeSet::new();
        let mut origins = BTreeSet::new();
        for edge in edges {
            if edge.key.kind != TopologyKind::Edge
                || !available.contains(&edge.key)
                || !keys.insert(edge.key)
                || edge.origin.source != feature
                || !origins.insert(edge.origin.clone())
            {
                return Err(failure(
                    GeometryErrorKind::Input,
                    "Selected edges must be present, distinct edges with distinct treatment-owned origins",
                ));
            }
            GeometryIntent {
                id: feature,
                kind: TopologyKind::Face,
                origins: BTreeSet::from([edge.origin.clone()]),
                boundary: None,
                attachment: None,
            }
            .validate()
            .map_err(|e| failure(GeometryErrorKind::Input, e))?;
            if snapshot
                .elements
                .iter()
                .any(|e| e.origins.contains(&edge.origin))
            {
                return Err(failure(
                    GeometryErrorKind::Input,
                    "Treatment origin collides with existing source provenance",
                ));
            }
        }
        let indices: Vec<_> = edges.iter().map(|e| e.key.ordinal).collect();
        let handle = ffi::treat_edges(&self.native.0, &indices, mode, size)
            .map_err(|e| failure(GeometryErrorKind::Native, e.to_string()))?;
        let mut result = Self::adopt(handle, policy, "edge treatment")?;
        if result.solid_count()? != 1 {
            return Err(failure(
                GeometryErrorKind::InvalidResult,
                "Edge treatment did not produce one connected solid",
            ));
        }
        result.inherit_treatment_provenance(self, edges, feature)?;
        Ok(result)
    }
}
