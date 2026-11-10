//! Adapt exact Rust topology to bounded finite closest-point measurements.
use super::{BRepSelection, DistanceWitness, MinimumDistance, error};
use crate::{GeometryError, GeometryErrorKind, Solid};
use multicad_core::references::{AnalyticGeometry, TopologyKind};
use multicad_rust_kernel::planar_distance::{DistanceError, Face, Shape, minimum};
use std::collections::BTreeMap;

type Point = [f64; 3];
type Segment = [Point; 2];
const MAX_SOLIDS: u32 = 128;

pub(super) fn measure(
    a: &Solid,
    selection_a: BRepSelection,
    b: &Solid,
    selection_b: BRepSelection,
    tolerance: f64,
) -> Result<MinimumDistance, GeometryError> {
    let left = prepare(a, selection_a)?;
    let right = prepare(b, selection_b)?;
    let result = minimum(&left, &right, tolerance).map_err(|cause| match cause {
        DistanceError::Invalid => error(
            GeometryErrorKind::InvalidResult,
            "Rust distance requires valid exact planar boundaries",
        ),
        DistanceError::Limit => error(
            GeometryErrorKind::Limit,
            "Rust distance exceeded its work or witness budget",
        ),
        DistanceError::Unresolved => error(
            GeometryErrorKind::InvalidResult,
            "Rust distance could not resolve the exact material or a finite witness",
        ),
    })?;
    let witnesses_complete = result.total_solutions as usize == result.witnesses.len();
    let witnesses: Vec<_> = result
        .witnesses
        .into_iter()
        .map(|[a_mm, b_mm]| DistanceWitness { a_mm, b_mm })
        .collect();
    if witnesses.iter().any(|witness| {
        let distance = (0..3)
            .map(|i| (witness.a_mm[i] - witness.b_mm[i]).powi(2))
            .sum::<f64>()
            .sqrt();
        !distance.is_finite() || (distance - result.distance).abs() > tolerance * 4.
    }) {
        return Err(error(
            GeometryErrorKind::InvalidResult,
            "Rust distance witnesses disagree with the measured scalar",
        ));
    }
    Ok(MinimumDistance {
        distance_mm: result.distance,
        witnesses,
        witnesses_complete,
        total_solutions: result.total_solutions,
        inner_solution: result.inner_solution,
    })
}

fn missing() -> GeometryError {
    error(
        GeometryErrorKind::Input,
        "Selected Rust B-rep topology is missing",
    )
}

fn curved() -> GeometryError {
    error(
        GeometryErrorKind::Unsupported,
        "Rust distance supports points, straight edges and straight-edged planar faces",
    )
}

fn point(geometry: Option<AnalyticGeometry>) -> Result<Point, GeometryError> {
    match geometry {
        Some(AnalyticGeometry::Point { position_mm }) => Ok(position_mm),
        _ => Err(curved()),
    }
}

fn line(geometry: Option<AnalyticGeometry>) -> Result<Segment, GeometryError> {
    match geometry {
        Some(AnalyticGeometry::Line { a_mm, b_mm }) => Ok([a_mm, b_mm]),
        _ => Err(curved()),
    }
}

fn prepare(solid: &Solid, selection: BRepSelection) -> Result<Vec<Shape>, GeometryError> {
    if selection == BRepSelection::Whole && solid.solid_count()? > MAX_SOLIDS {
        return Err(error(
            GeometryErrorKind::Limit,
            "Whole-solid distance supports at most 128 solids per Part",
        ));
    }
    let mut shapes = Vec::new();
    add_shape(&mut shapes, solid, selection)?;
    Ok(shapes)
}

fn add_shape(
    shapes: &mut Vec<Shape>,
    solid: &Solid,
    selection: BRepSelection,
) -> Result<(), GeometryError> {
    if selection == BRepSelection::Whole
        && let Some(compound) = solid.rust_compound()
    {
        for member in compound.members() {
            add_shape(shapes, member, selection)?;
        }
        return Ok(());
    }
    let topology = solid.rust_topology().ok_or_else(curved)??;
    let find = |kind, ordinal| {
        topology
            .elements
            .iter()
            .find(|element| element.key.kind == kind && element.key.ordinal == ordinal)
            .map(|element| element.geometry)
            .ok_or_else(missing)
    };
    let shape = match selection {
        BRepSelection::Vertex(index) => Shape {
            points: vec![point(find(TopologyKind::Vertex, index)?)?],
            ..Shape::default()
        },
        BRepSelection::Edge(index) => {
            let segment = line(find(TopologyKind::Edge, index)?)?;
            Shape {
                points: segment.to_vec(),
                edges: vec![segment],
                ..Shape::default()
            }
        }
        BRepSelection::Face(_) | BRepSelection::Whole => {
            let edge_geometry: BTreeMap<_, _> = topology
                .elements
                .iter()
                .filter(|element| element.key.kind == TopologyKind::Edge)
                .map(|element| (element.key.ordinal, element.geometry))
                .collect();
            let mut shape = Shape::default();
            for face in solid.faces()?.into_iter().filter(|face| {
                selection == BRepSelection::Whole || selection == BRepSelection::Face(face.index)
            }) {
                let (point, normal) = face.plane.ok_or_else(curved)?;
                let boundary = crate::rust_display::planar_loops(solid, face.index)
                    .into_iter()
                    .flatten()
                    .map(|(index, _)| line(*edge_geometry.get(&index).ok_or_else(missing)?))
                    .collect::<Result<Vec<_>, GeometryError>>()?;
                shape.faces.push(Face {
                    point,
                    normal,
                    boundary,
                });
            }
            if shape.faces.is_empty() {
                return Err(missing());
            }
            if selection == BRepSelection::Whole {
                shape.solid = true;
                shape.points = topology
                    .elements
                    .iter()
                    .filter(|element| element.key.kind == TopologyKind::Vertex)
                    .map(|element| point(element.geometry))
                    .collect::<Result<Vec<_>, _>>()?;
                shape.edges = edge_geometry
                    .into_values()
                    .map(line)
                    .collect::<Result<Vec<_>, _>>()?;
            } else {
                shape.edges = shape.faces[0].boundary.clone();
                shape.points = shape.edges.iter().flatten().copied().collect();
            }
            shape
        }
    };
    shapes.push(shape);
    Ok(())
}
