//! Display and planar queries on Rust solid topology.
use crate::{
    DisplayEdge, DisplayVertex, GeometryError, GeometryErrorKind, GeometryPolicy, PlaneFrame,
    Solid, ffi, planar_contours::SourcedPlanarContours,
};
use multicad_core::{
    manufacturing::{PlanarContours, PlanarEdge, PlanarLoop},
    references::{AnalyticGeometry, ReferenceFrame, TopologyKey, TopologyKind, TopologySnapshot},
};
use multicad_rust_kernel::display::{DisplayMesh, MeshError};
use std::collections::BTreeMap;

fn error(
    operation: &'static str,
    kind: GeometryErrorKind,
    message: impl Into<String>,
) -> GeometryError {
    GeometryError {
        operation,
        kind,
        message: message.into(),
    }
}

pub(crate) fn mesh(result: Result<DisplayMesh, MeshError>) -> Result<ffi::Mesh, GeometryError> {
    let mesh = result.map_err(|cause| {
        let (kind, message) = match cause {
            MeshError::InvalidOptions => (
                GeometryErrorKind::Input,
                "Invalid Rust tessellation options",
            ),
            MeshError::BudgetExceeded => (
                GeometryErrorKind::Limit,
                "Rust tessellation exceeds the triangle budget",
            ),
            MeshError::UnresolvedBoundary => (
                GeometryErrorKind::InvalidResult,
                "Rust tessellation could not resolve the sampled face boundary",
            ),
        };
        error("mesh", kind, message)
    })?;
    Ok(ffi::Mesh {
        vertices: mesh.vertices,
        indices: mesh.indices,
        face_indices: mesh.face_indices,
    })
}

fn point_on_arc(frame: ReferenceFrame, radius: f64, angle: f64) -> [f64; 3] {
    let (sin, cos) = angle.sin_cos();
    std::array::from_fn(|i| frame.origin_mm[i] + radius * (cos * frame.u[i] + sin * frame.v[i]))
}

pub(crate) fn edges(
    topology: TopologySnapshot,
    deflection: f64,
) -> Result<Vec<DisplayEdge>, GeometryError> {
    const MAX_EDGE_POINTS: usize = 5_000_000;
    let mut total_points = 0;
    topology
        .elements
        .into_iter()
        .filter(|e| e.key.kind == TopologyKind::Edge)
        .map(|e| {
            let points_mm = match e.geometry {
                Some(AnalyticGeometry::Line { a_mm, b_mm }) => {
                    total_points += 2;
                    vec![a_mm, b_mm]
                }
                Some(AnalyticGeometry::CircularArc {
                    frame,
                    radius_mm,
                    start_rad,
                    sweep_rad,
                }) => {
                    let sagitta_angle = 4. * (deflection / (2. * radius_mm)).sqrt().min(1.).asin();
                    let segments = (sweep_rad.abs() / sagitta_angle.min(0.3)).ceil() as usize;
                    total_points += segments + 1;
                    if total_points > MAX_EDGE_POINTS {
                        return Err(error(
                            "display edges",
                            GeometryErrorKind::Limit,
                            "Rust edge sampling exceeds the point budget",
                        ));
                    }
                    (0..=segments)
                        .map(|i| {
                            point_on_arc(
                                frame,
                                radius_mm,
                                start_rad + sweep_rad * i as f64 / segments as f64,
                            )
                        })
                        .collect()
                }
                _ => {
                    return Err(error(
                        "display edges",
                        GeometryErrorKind::InvalidResult,
                        "Rust topology is missing analytic edge geometry",
                    ));
                }
            };
            Ok(DisplayEdge {
                index: e.key.ordinal,
                points_mm,
            })
        })
        .collect()
}

pub(crate) fn vertices(topology: TopologySnapshot) -> Vec<DisplayVertex> {
    topology
        .elements
        .into_iter()
        .filter_map(|e| {
            let Some(AnalyticGeometry::Point { position_mm }) = e.geometry else {
                return None;
            };
            Some(DisplayVertex {
                index: e.key.ordinal,
                position_mm,
            })
        })
        .collect()
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

fn subtract(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    std::array::from_fn(|i| a[i] - b[i])
}

// Return analytic edge ordinals in loop order. These are the same ordinals
// published by the prism topology and are distinct from mesh indices.
fn planar_loops(solid: &Solid, face: u32) -> Vec<Vec<(u32, bool)>> {
    let Some(prism) = solid.rust_prism() else {
        return vec![vec![(face, false)]];
    };
    let holes = prism.inner_loops().len() as u32;
    let stride = 4 + holes;
    if face < 2 {
        let first = face * stride;
        return std::iter::once((first..first + 4).map(|edge| (edge, false)).collect())
            .chain((first + 4..first + stride).map(|edge| vec![(edge, false)]))
            .collect();
    }
    let side = face - 2;
    vec![vec![
        (side, false),
        (2 * stride + (side + 1) % 4, false),
        (stride + side, true),
        (2 * stride + side, true),
    ]]
}

pub(crate) fn planar_contours(
    solid: &Solid,
    topology: TopologySnapshot,
    face: u32,
    frame: PlaneFrame,
    policy: GeometryPolicy,
) -> Result<SourcedPlanarContours, GeometryError> {
    let input = |message| error("planar face contours", GeometryErrorKind::Input, message);
    let bad_result = |message: String| {
        error(
            "planar face contours",
            GeometryErrorKind::InvalidResult,
            message,
        )
    };
    let Some((origin, normal)) = solid
        .faces()?
        .into_iter()
        .find(|candidate| candidate.index == face)
        .and_then(|selected| selected.plane)
    else {
        return Err(input("Select a planar Rust solid face"));
    };
    let query_normal = frame.normal();
    if dot(normal, query_normal).abs() < 1. - 1e-9
        || dot(normal, subtract(frame.origin, origin)).abs() > policy.linear_tolerance_mm
    {
        return Err(input(
            "Contour frame must be coplanar with the selected face",
        ));
    }
    let project = |point| {
        let delta = subtract(point, frame.origin);
        if dot(delta, query_normal).abs() > policy.linear_tolerance_mm {
            return Err(input("Boundary is outside the contour plane tolerance"));
        }
        Ok([dot(delta, frame.u), dot(delta, frame.v)])
    };
    let geometry: BTreeMap<_, _> = topology
        .elements
        .into_iter()
        .filter(|e| e.key.kind == TopologyKind::Edge)
        .map(|e| (e.key.ordinal, e.geometry))
        .collect();
    let mut loops = vec![];
    let mut edge_keys = vec![];
    for (index, ring) in planar_loops(solid, face).into_iter().enumerate() {
        let mut edges = vec![];
        let mut keys = vec![];
        for (ordinal, reversed) in ring {
            let edge = match geometry.get(&ordinal).copied().flatten() {
                Some(AnalyticGeometry::Line { a_mm, b_mm }) => PlanarEdge::Line {
                    a: project(a_mm)?,
                    b: project(b_mm)?,
                },
                Some(AnalyticGeometry::CircularArc {
                    frame: support,
                    radius_mm,
                    start_rad,
                    sweep_rad,
                }) => {
                    let max_arc_offset =
                        dot(subtract(support.origin_mm, frame.origin), query_normal).abs()
                            + radius_mm
                                * dot(support.u, query_normal).hypot(dot(support.v, query_normal));
                    if max_arc_offset > policy.linear_tolerance_mm {
                        return Err(input("Arc is outside the contour plane tolerance"));
                    }
                    let a = project(point_on_arc(support, radius_mm, start_rad))?;
                    let b = if sweep_rad.abs() == std::f64::consts::TAU {
                        a
                    } else {
                        project(point_on_arc(support, radius_mm, start_rad + sweep_rad))?
                    };
                    PlanarEdge::Arc {
                        a,
                        b,
                        center: project(support.origin_mm)?,
                        sweep_rad: sweep_rad * dot(support.normal(), query_normal).signum(),
                    }
                }
                _ => return Err(bad_result("Rust face boundary edge is missing".into())),
            };
            edges.push(if reversed { edge.reversed() } else { edge });
            keys.push(TopologyKey {
                kind: TopologyKind::Edge,
                ordinal,
            });
        }
        let mut wire = PlanarLoop {
            outer: index == 0,
            edges,
        };
        if (wire.signed_area() > 0.) != wire.outer {
            keys.reverse();
            wire.normalize_winding();
        }
        loops.push(wire);
        edge_keys.push(keys);
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
        .map_err(bad_result)?;
    Ok(SourcedPlanarContours {
        contours,
        edge_keys,
    })
}
