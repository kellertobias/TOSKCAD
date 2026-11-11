//! Exact edge treatments. Keys are evaluation-local; caller-supplied origins
//! retain the authored selection identity through native generation history.
use crate::{GeometryError, GeometryErrorKind, GeometryPolicy, Solid, ffi};
use multicad_core::{
    document::EntityId,
    references::{
        AnalyticGeometry, GeometryIntent, Origin, TopologyKey, TopologyKind, TopologySnapshot,
    },
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SidedTreatmentEdge {
    pub edge: TreatmentEdge,
    /// Evaluation-local face resolved from an explicitly captured side intent.
    pub reference_face: TopologyKey,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SidedChamfer {
    /// First distance is measured on the explicitly selected reference face.
    Distances { first_mm: f64, second_mm: f64 },
    /// Distance is measured on that face; angle is between it and the chamfer.
    DistanceAngle { distance_mm: f64, angle_rad: f64 },
}

fn failure(kind: GeometryErrorKind, message: impl Into<String>) -> GeometryError {
    GeometryError {
        operation: "edge treatment",
        kind,
        message: message.into(),
    }
}

impl Solid {
    pub fn chamfer_edges_sided_named(
        &self,
        edges: &[SidedTreatmentEdge],
        treatment: SidedChamfer,
        feature: EntityId,
        policy: GeometryPolicy,
    ) -> Result<Self, GeometryError> {
        policy.validate()?;
        let (mode, first, second) = match treatment {
            SidedChamfer::Distances {
                first_mm,
                second_mm,
            } => {
                policy.length(first_mm)?;
                policy.length(second_mm)?;
                (2, first_mm, second_mm)
            }
            SidedChamfer::DistanceAngle {
                distance_mm,
                angle_rad,
            } => {
                policy.length(distance_mm)?;
                if !angle_rad.is_finite()
                    || !(0. ..std::f64::consts::FRAC_PI_2).contains(&angle_rad)
                    || angle_rad == 0.
                {
                    return Err(failure(
                        GeometryErrorKind::Input,
                        "Chamfer angle must lie strictly between zero and 90 degrees",
                    ));
                }
                (3, distance_mm, angle_rad)
            }
        };
        let selections: Vec<_> = edges.iter().map(|edge| edge.edge.clone()).collect();
        let (indices, topology) = self.validate_treatment_edges(&selections, feature)?;
        let available: BTreeSet<_> = topology.elements.iter().map(|e| e.key).collect();
        if edges.iter().any(|edge| {
            edge.reference_face.kind != TopologyKind::Face
                || !available.contains(&edge.reference_face)
        }) {
            return Err(failure(
                GeometryErrorKind::Input,
                "Each asymmetric chamfer edge needs a current explicitly selected reference face",
            ));
        }
        if self.uses_rust_geometry() {
            let distance = if mode == 3 {
                first * second.tan()
            } else {
                second
            };
            let faces: Vec<_> = edges
                .iter()
                .map(|selection| selection.reference_face)
                .collect();
            if let Some(result) = self.chamfer_cylinder_rims(
                &selections,
                Some(&faces),
                [first, distance],
                feature,
                policy,
                &topology,
            ) {
                return result;
            }
            let [selection] = edges else {
                return Err(failure(
                    GeometryErrorKind::Unsupported,
                    "Rust sided chamfer requires one straight edge",
                ));
            };
            return self.rust_chamfer_box_edge(
                &selection.edge,
                Some(selection.reference_face),
                [first, distance],
                feature,
                policy,
                topology,
            );
        }
        let sides: Vec<_> = edges
            .iter()
            .map(|edge| edge.reference_face.ordinal)
            .collect();
        let handle = ffi::treat_edges_sided(self.occt()?, &indices, &sides, mode, first, second)
            .map_err(|e| failure(GeometryErrorKind::Native, e.to_string()))?;
        let mut result = Self::adopt(handle, policy, "sided chamfer")?;
        if result.solid_count()? != 1 {
            return Err(failure(
                GeometryErrorKind::InvalidResult,
                "Chamfer did not produce one connected solid",
            ));
        }
        result.inherit_treatment_provenance(self, &selections, feature)?;
        Ok(result)
    }
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
        let (indices, topology) = self.validate_treatment_edges(edges, feature)?;
        if self.uses_rust_geometry() {
            if let EdgeTreatment::Chamfer { distance_mm } = treatment
                && let Some(result) = self.chamfer_cylinder_rims(
                    edges,
                    None,
                    [distance_mm; 2],
                    feature,
                    policy,
                    &topology,
                )
            {
                return result;
            }
            return match treatment {
                EdgeTreatment::Chamfer { distance_mm } => match edges {
                    [edge]
                        if topology.elements.iter().any(|element| {
                            element.key == edge.key
                                && matches!(element.geometry, Some(AnalyticGeometry::Line { .. }))
                        }) =>
                    {
                        self.rust_chamfer_box_edge(
                            edge,
                            None,
                            [distance_mm; 2],
                            feature,
                            policy,
                            topology,
                        )
                    }
                    _ => self.chamfer_polygon_bores(edges, distance_mm, policy),
                },
                EdgeTreatment::Fillet { radius_mm } => {
                    self.rust_fillet_box_edges(edges, radius_mm, feature, policy, &topology)
                }
            };
        }
        let handle = ffi::treat_edges(self.occt()?, &indices, mode, size)
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
    fn validate_treatment_edges(
        &self,
        edges: &[TreatmentEdge],
        feature: EntityId,
    ) -> Result<(Vec<u32>, TopologySnapshot), GeometryError> {
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
        Ok((edges.iter().map(|e| e.key.ordinal).collect(), snapshot))
    }
}
