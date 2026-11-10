//! Adapt exact Rust solid boundaries to the planar drawing projector.
use super::*;
use multicad_core::references::{AnalyticGeometry, TopologyKind};
use multicad_rust_kernel::planar_projection::{self, Body, Face, Options, ProjectionError};
use std::collections::BTreeMap;

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    std::array::from_fn(|i| a[(i + 1) % 3] * b[(i + 2) % 3] - a[(i + 2) % 3] * b[(i + 1) % 3])
}

fn unit(vector: [f64; 3]) -> [f64; 3] {
    let length = vector.into_iter().map(|v| v * v).sum::<f64>().sqrt();
    vector.map(|v| v / length)
}

pub(super) fn view_axes(frame: PlaneFrame) -> [[f64; 3]; 3] {
    // Match the native projector's normalized X and constructed Y axes.
    let u = unit(frame.u);
    let n = unit(cross(u, frame.v));
    [u, cross(n, u), n]
}

pub(super) fn project(
    solids: &[DrawingSolid<'_>],
    options: DrawingProjection,
) -> Result<Vec<ProjectedVector>, GeometryError> {
    let (bodies, tolerance) = prepare(solids, options.frame)?;
    project_bodies(&bodies, options, tolerance)
}

pub(super) fn project_bodies(
    bodies: &[Body],
    options: DrawingProjection,
    tolerance: f64,
) -> Result<Vec<ProjectedVector>, GeometryError> {
    let projected = planar_projection::project(
        bodies,
        Options {
            tolerance,
            hidden_lines: options.hidden_lines,
            tangent_edges: options.tangent_edges,
            detail: options.detail,
            segment_limit: (options.maximum_curves as usize)
                .min(options.maximum_points as usize / 2),
        },
    )
    .map_err(projection_error)?;
    Ok(projected
        .into_iter()
        .map(|segment| {
            line(
                segment.source,
                if segment.hidden {
                    VectorRole::Hidden
                } else {
                    VectorRole::Visible
                },
                segment.ends,
            )
        })
        .collect())
}

pub(super) fn line(
    source_index: usize,
    role: VectorRole,
    [a, b]: [[f64; 2]; 2],
) -> ProjectedVector {
    ProjectedVector {
        source_index,
        role,
        curve: VectorCurve::Line { a, b },
        preview_mm: vec![a, b],
    }
}

pub(super) fn projection_error(cause: ProjectionError) -> GeometryError {
    match cause {
        ProjectionError::Invalid => invalid_boundary(),
        ProjectionError::Limit => err(
            GeometryErrorKind::Limit,
            "Rust drawing projection exceeded its work or output budget",
        ),
    }
}

fn invalid_boundary() -> GeometryError {
    err(
        GeometryErrorKind::InvalidResult,
        "Rust drawing source has an invalid exact boundary",
    )
}

pub(super) fn prepare(
    solids: &[DrawingSolid<'_>],
    frame: PlaneFrame,
) -> Result<(Vec<Body>, f64), GeometryError> {
    let axes = view_axes(frame);
    let local_vector =
        |vector: [f64; 3]| axes.map(|axis| vector.into_iter().zip(axis).map(|(a, b)| a * b).sum());
    let local_point =
        |point: [f64; 3]| local_vector(std::array::from_fn(|i| point[i] - frame.origin[i]));
    let unsupported = || {
        err(
            GeometryErrorKind::Unsupported,
            "Rust planar drawing projection requires straight edges and planar faces",
        )
    };
    let tolerance = solids
        .iter()
        .map(|input| input.solid.geometry_policy().linear_tolerance_mm)
        .fold(GeometryPolicy::default().linear_tolerance_mm, f64::max);
    let mut bodies = Vec::with_capacity(solids.len());
    for input in solids {
        let solid = input.solid;
        let topology = solid.rust_topology().ok_or_else(unsupported)??;
        let mut indices = BTreeMap::new();
        let mut edges = Vec::new();
        for edge in topology
            .elements
            .into_iter()
            .filter(|element| element.key.kind == TopologyKind::Edge)
        {
            let Some(AnalyticGeometry::Line { a_mm, b_mm }) = edge.geometry else {
                return Err(unsupported());
            };
            indices.insert(edge.key.ordinal, edges.len());
            edges.push([local_point(a_mm), local_point(b_mm)]);
        }
        let faces = solid
            .faces()?
            .into_iter()
            .map(|face| {
                let (point, normal) = face.plane.ok_or_else(unsupported)?;
                let edges = crate::rust_display::planar_loops(solid, face.index)
                    .into_iter()
                    .flatten()
                    .map(|(edge, _)| indices.get(&edge).copied().ok_or_else(invalid_boundary))
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(Face {
                    point: local_point(point),
                    normal: local_vector(normal),
                    edges,
                })
            })
            .collect::<Result<Vec<_>, GeometryError>>()?;
        bodies.push(Body { faces, edges });
    }
    Ok((bodies, tolerance))
}
