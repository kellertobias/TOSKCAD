//! Selected native finite face wires become exact manufacturing contours.
use crate::{GeometryError, GeometryErrorKind, GeometryPolicy, PlaneFrame, Solid, ffi};
use multicad_core::{
    manufacturing::{PlanarContours, PlanarEdge, PlanarLoop},
    references::ReferenceFrame,
};
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
    let native = frame.checked(policy)?;
    let wires =
        ffi::planar_face_contours(&solid.native.0, face, &native, policy.linear_tolerance_mm)
            .map_err(|e| error(GeometryErrorKind::Native, e))?;
    let mut loops = Vec::with_capacity(wires.len());
    for wire in wires {
        let mut edges = Vec::with_capacity(wire.segments.len());
        for e in wire.segments {
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
        wire.normalize_winding();
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
    Ok(contours)
}
