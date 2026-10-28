//! Selected native finite face wires become exact manufacturing contours.
use crate::{GeometryError, GeometryErrorKind, GeometryPolicy, PlaneFrame, Solid, ffi};
use multicad_core::{
    manufacturing::{PlanarContours, PlanarEdge, PlanarLoop},
    references::{ReferenceFrame, TopologyKey, TopologyKind},
};
/// Exact contours with evaluation-local native edge keys in the same loop and
/// segment order. Capture semantic origins from the current topology snapshot;
/// these keys are never persistent references.
#[derive(Clone, Debug, PartialEq)]
pub struct SourcedPlanarContours {
    pub contours: PlanarContours,
    pub edge_keys: Vec<Vec<TopologyKey>>,
}
impl Solid {
    pub fn planar_face_contours_with_sources(
        &self,
        face: u32,
        frame: PlaneFrame,
    ) -> Result<SourcedPlanarContours, GeometryError> {
        extract_sourced(self, face, frame, self.geometry_policy())
    }
}
fn error(kind: GeometryErrorKind, message: impl ToString) -> GeometryError {
    GeometryError {
        operation: "planar face contours",
        kind,
        message: message.to_string(),
    }
}
pub(crate) fn extract(
    solid: &Solid,
    face: u32,
    frame: PlaneFrame,
    policy: GeometryPolicy,
) -> Result<PlanarContours, GeometryError> {
    Ok(extract_sourced(solid, face, frame, policy)?.contours)
}
fn extract_sourced(
    solid: &Solid,
    face: u32,
    frame: PlaneFrame,
    policy: GeometryPolicy,
) -> Result<SourcedPlanarContours, GeometryError> {
    let native = frame.checked(policy)?;
    let wires =
        ffi::planar_face_contours(&solid.native.0, face, &native, policy.linear_tolerance_mm)
            .map_err(|e| error(GeometryErrorKind::Native, e))?;
    let mut loops = Vec::with_capacity(wires.len());
    let mut edge_keys = Vec::with_capacity(wires.len());
    for wire in wires {
        let mut edges = Vec::with_capacity(wire.segments.len());
        let mut keys = Vec::with_capacity(wire.segments.len());
        for e in wire.segments {
            keys.push(TopologyKey {
                kind: TopologyKind::Edge,
                ordinal: e.edge_index,
            });
            let a = [e.a.x, e.a.y];
            let b = [e.b.x, e.b.y];
            edges.push(match e.kind {
                0 => PlanarEdge::Line { a, b },
                1 => PlanarEdge::Arc {
                    a,
                    b,
                    center: [e.center.x, e.center.y],
                    sweep_rad: e.sweep,
                },
                _ => {
                    return Err(error(
                        GeometryErrorKind::InvalidResult,
                        "Unknown native planar curve family",
                    ));
                }
            });
        }
        let mut wire = PlanarLoop {
            outer: wire.outer,
            edges,
        };
        if (wire.signed_area() > 0.) != wire.outer {
            keys.reverse();
            wire.normalize_winding();
        }
        edge_keys.push(keys);
        loops.push(wire);
    }
    let contours = PlanarContours {
        frame: ReferenceFrame {
            origin_mm: frame.origin,
            u: frame.u,
            v: frame.v,
        },
        loops,
    };
    contours
        .validate(policy.linear_tolerance_mm)
        .map_err(|e| error(GeometryErrorKind::InvalidResult, e))?;
    Ok(SourcedPlanarContours {
        contours,
        edge_keys,
    })
}
