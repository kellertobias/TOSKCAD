//! Shared exact rim and wall records for openings in straight sweep sides.
use super::*;
use crate::normal_sweep::{CertifiedPiercing, Edge, PiercedSweep};

/// Write a normal sweep with paired circular or rectangular through-openings.
pub fn write_pierced_normal_sweep(
    pierced: &PiercedSweep,
    origin: Point,
    u: Point,
    v: Point,
    reverse_sweep: bool,
) -> Result<String, StepError> {
    write_one(|writer| writer.append_pierced_normal_sweep(pierced, origin, u, v, reverse_sweep))
}

pub(super) struct Openings {
    pub(super) holes: BTreeMap<usize, Vec<Entity>>,
    pub(super) walls: Vec<Entity>,
}

pub(super) fn encode_openings(
    file: &mut StepFile,
    piercings: &[CertifiedPiercing],
    placement: &Placement,
    reverse: bool,
) -> Result<Openings, StepError> {
    let sign = if reverse { -1. } else { 1. };
    let world_vector = |[x, y, z]: Point| placement.vector([x, y, sign * z]);
    let mut holes = BTreeMap::<usize, Vec<Entity>>::new();
    let mut walls = Vec::new();
    for opening in piercings {
        let spec = opening.spec();
        let axis = world_vector(spec.through.map(|x| x / opening.thickness_mm()));
        let reference = world_vector(spec.u);
        let world = |xy, end| {
            let [x, y, z] = opening.point(xy, end);
            placement.point_f64([x, y], sign * z)
        };
        let bounds = if let [
            Edge::Circle {
                center, radius_mm, ..
            },
        ] = spec.outline.as_slice()
        {
            let trim = CylindricalTrim::new(
                file,
                [false, true].map(|end| world(*center, end)),
                axis,
                reference,
                *radius_mm,
            )?;
            walls.push(trim.wall(file, false));
            [false, true].map(|end| trim.cap_bound(file, end, false))
        } else {
            let mut corners: Vec<_> = spec.outline.iter().map(|edge| edge.start()).collect();
            // The lower hole is clockwise viewed from outside stock. Store
            // its edge curves counterclockwise viewed along the cavity axis.
            if dot(cross(reference, world_vector(spec.v)), axis) < 0. {
                corners.reverse();
            }
            let (bounds, tunnel) = rectangle(file, &corners, axis, world)?;
            walls.extend(tunnel);
            bounds
        };
        for (side, bound) in spec.sides.into_iter().zip(bounds) {
            holes.entry(side).or_default().push(bound);
        }
    }
    Ok(Openings { holes, walls })
}

fn rectangle(
    file: &mut StepFile,
    corners: &[[f64; 2]],
    axis: Point,
    world: impl Fn([f64; 2], bool) -> Point,
) -> Result<([Entity; 2], Vec<Entity>), StepError> {
    let vertices = [false, true].map(|end| {
        corners
            .iter()
            .map(|&xy| file.vertex(world(xy, end)))
            .collect::<Vec<_>>()
    });
    let mut edges = [Vec::new(), Vec::new()];
    for (rim, points) in edges.iter_mut().zip(&vertices) {
        for (&a, &b) in points.iter().zip(points.iter().cycle().skip(1)) {
            rim.push(line_edge(file, a, b)?);
        }
    }
    let rails = vertices[0]
        .iter()
        .zip(&vertices[1])
        .map(|(&a, &b)| line_edge(file, a, b))
        .collect::<Result<Vec<_>, _>>()?;
    let mut faces = Vec::with_capacity(corners.len());
    for i in 0..corners.len() {
        let next = (i + 1) % corners.len();
        let boundary = [
            (rails[i], true),
            (edges[1][i], true),
            (rails[next], false),
            (edges[0][i], false),
        ]
        .map(|(edge, forward)| file.oriented_edge(edge, forward));
        let bound = file.bound(&boundary, true);
        let [a, b] = [vertices[0][i], vertices[0][next]];
        let tangent = direction(a.point, b.point)?;
        faces.push(file.face(&[bound], a.cartesian, cross(axis, tangent), tangent));
    }
    edges[1].reverse();
    let [front, back] = edges;
    let bounds = [(front, true), (back, false)].map(|(rim, forward)| {
        let oriented: Vec<_> = rim
            .into_iter()
            .map(|edge| file.oriented_edge(edge, forward))
            .collect();
        file.bound(&oriented, false)
    });
    Ok((bounds, faces))
}
