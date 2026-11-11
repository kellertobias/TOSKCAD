//! Analytic cap relief with one shared entity for every physical trim.
use super::*;
use crate::normal_relief::{Curve, Relief, Surface};
use crate::normal_sweep::Edge;
pub fn write_normal_relief(
    relief: &Relief,
    origin: Point,
    u: Point,
    v: Point,
) -> Result<String, StepError> {
    write_one(|writer| writer.append_normal_relief(relief, origin, u, v))
}

pub(super) fn encode(
    file: &mut StepFile,
    relief: &Relief,
    placement: Placement,
) -> Result<Entity, StepError> {
    precision(relief, &placement)?;
    let mut candidate = StepFile::new();
    let solid = encode_shell(&mut candidate, relief, &placement)?;
    candidate.finish(vec![solid]);
    let read = crate::step_read::read_analytic_solid(&candidate.data, relief.tolerance_mm())
        .map_err(|_| StepError::Unsupported)?;
    if read.solid_count != 1
        || read.face_count != relief.boundary().faces.len()
        || (read.volume_mm3 - relief.volume_mm3()).abs()
            > relief.tolerance_mm() * relief.surface_area_mm2()
    {
        return Err(StepError::Unsupported);
    }
    encode_shell(file, relief, &placement)
}
fn add_up(a: f64, b: f64) -> f64 {
    let value = a + b;
    let part = value - a;
    let error = (a - (value - part)) + (b - part);
    if error > 0. { value.next_up() } else { value }
}
fn mul_up(a: f64, b: f64) -> f64 {
    let value = a * b;
    if a != 0. && b != 0. && value.abs() < 1e-280 || a.mul_add(b, -value) > 0. {
        value.next_up()
    } else {
        value
    }
}
fn product_error(a: f64, b: f64, p: f64) -> f64 {
    let error = a.mul_add(b, -p).abs();
    if a != 0. && b != 0. && p.abs() < 1e-280 {
        error.next_up()
    } else {
        error
    }
}
fn dot_error(a: Point, b: Point, target: f64) -> f64 {
    let mut value = -target;
    let mut error = 0.;
    for i in 0..3 {
        let p = a[i] * b[i];
        error = add_up(error, product_error(a[i], b[i], p));
        let sum = value + p;
        let part = sum - value;
        error = add_up(error, ((value - (sum - part)) + (p - part)).abs());
        value = sum;
    }
    add_up(value.abs(), error)
}
fn evaluation_error(placement: &Placement, p: Point) -> f64 {
    let axes = [placement.u, placement.v, placement.normal];
    let mut total = 0.;
    for i in 0..3 {
        let mut value = placement.origin[i];
        let mut error = 0.;
        for j in 0..3 {
            let term = axes[j][i] * p[j];
            error = add_up(error, product_error(axes[j][i], p[j], term));
            let sum = value + term;
            let part = sum - value;
            error = add_up(error, ((value - (sum - part)) + (term - part)).abs());
            value = sum;
        }
        total = add_up(total, error);
    }
    total
}
fn precision(relief: &Relief, placement: &Placement) -> Result<(), StepError> {
    let axes = [placement.u, placement.v, placement.normal];
    let mut gram = 0.;
    for i in 0..3 {
        for j in 0..3 {
            let error = dot_error(axes[i], axes[j], if i == j { 1. } else { 0. });
            gram = add_up(gram, mul_up(error, error));
        }
    }
    let bounds = relief.bounds();
    let mut extent = 0.;
    for i in 0..3 {
        let e = bounds[0][i].abs().max(bounds[1][i].abs());
        extent = add_up(extent, mul_up(e, e));
    }
    let sqrt = |value: f64| {
        if value == 0. {
            0.
        } else {
            value.sqrt().next_up()
        }
    };
    if mul_up(sqrt(gram), sqrt(extent)) > relief.tolerance_mm() / 8. {
        return Err(StepError::Unsupported);
    }
    let graph = relief.boundary();
    let controls = graph
        .vertices
        .iter()
        .copied()
        .chain(graph.faces.iter().map(|face| match face.surface {
            Surface::Plane { origin_mm, .. } | Surface::Cylinder { origin_mm, .. } => origin_mm,
        }))
        .chain(graph.edges.iter().filter_map(|edge| match edge.curve {
            Curve::Profile {
                edge: Edge::Arc { center, .. } | Edge::Circle { center, .. },
                z_mm,
            } => Some([center[0], center[1], z_mm]),
            _ => None,
        }));
    if controls
        .into_iter()
        .any(|p| evaluation_error(placement, p) > relief.tolerance_mm() / 8.)
    {
        return Err(StepError::Unsupported);
    }
    Ok(())
}
fn encode_shell(
    file: &mut StepFile,
    relief: &Relief,
    placement: &Placement,
) -> Result<Entity, StepError> {
    let graph = relief.boundary();
    let placed = |p: Point| placement.point_f64([p[0], p[1]], p[2]);
    let vertices = graph
        .vertices
        .iter()
        .map(|&point| file.vertex(placed(point)))
        .collect::<Vec<_>>();
    let mut edges = Vec::with_capacity(graph.edges.len());
    for edge in &graph.edges {
        let ends = edge.vertices.map(|i| vertices[i]);
        let entity = match edge.curve {
            Curve::Line { .. }
            | Curve::Profile {
                edge: Edge::Line { .. },
                ..
            } => line_edge(file, ends[0], ends[1])?,
            Curve::Profile {
                edge:
                    Edge::Circle {
                        center,
                        radius_mm,
                        clockwise,
                    },
                z_mm,
            } => {
                let center = placement.point_f64(center, z_mm);
                let seam = std::array::from_fn(|i| center[i] + radius_mm * placement.u[i]);
                if center == seam
                    || (ends[0].point[0] - seam[0])
                        .hypot(ends[0].point[1] - seam[1])
                        .hypot(ends[0].point[2] - seam[2])
                        > relief.tolerance_mm() / 8.
                    || edge.vertices[0] != edge.vertices[1]
                {
                    return Err(StepError::InvalidGeometry);
                }
                let center = file.point(center);
                let axis = file.axis(
                    center,
                    placement.normal.map(|v| if clockwise { -v } else { v }),
                    placement.u,
                );
                let circle = file.entity(format_args!("CIRCLE('',{axis},{radius_mm:.16E})"));
                let vertex = ends[0].vertex;
                file.entity(format_args!(
                    "EDGE_CURVE('',{vertex},{vertex},{circle},.T.)"
                ))
            }
            Curve::Profile {
                edge:
                    edge @ Edge::Arc {
                        center,
                        a,
                        clockwise,
                        ..
                    },
                z_mm,
            } => {
                let radius = edge.radius_mm().expect("arc radius");
                let radial = [(a[0] - center[0]) / radius, (a[1] - center[1]) / radius, 0.];
                circular_arc_edge(
                    file,
                    ends,
                    placement.point_f64(center, z_mm),
                    placement.normal.map(|v| if clockwise { -v } else { v }),
                    placement.vector(radial),
                    radius,
                    edge.signed_sweep_rad().expect("arc sweep"),
                )?
                .0
            }
        };
        edges.push(entity);
    }
    let mut faces = Vec::with_capacity(graph.faces.len());
    for face in &graph.faces {
        let bounds = face
            .loops
            .iter()
            .enumerate()
            .map(|(i, loop_)| {
                let edges = loop_
                    .iter()
                    .map(|edge| file.oriented_edge(edges[edge.edge], edge.forward))
                    .collect::<Vec<_>>();
                file.bound(&edges, i == 0)
            })
            .collect::<Vec<_>>();
        let entity = match face.surface {
            Surface::Plane { origin_mm, u, v } => {
                let origin = file.point(placed(origin_mm));
                file.face(
                    &bounds,
                    origin,
                    placement.vector(cross(u, v)),
                    placement.vector(u),
                )
            }
            Surface::Cylinder {
                origin_mm,
                axis,
                u,
                radius_mm,
                outward,
            } => {
                let origin = file.point(placed(origin_mm));
                let axis = file.axis(origin, placement.vector(axis), placement.vector(u));
                let surface = file.entity(format_args!(
                    "CYLINDRICAL_SURFACE('',{axis},{radius_mm:.16E})"
                ));
                file.face_on(&bounds, surface, outward)
            }
        };
        faces.push(entity);
    }
    Ok(file.solid(&faces))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::normal_sweep::Sweep;
    #[test]
    fn relief_exchange_is_one_shared_shell_with_true_circular_supports() {
        let rectangle = |min: [f64; 2], max: [f64; 2]| {
            let p = [min, [max[0], min[1]], max, [min[0], max[1]]];
            (0..4)
                .map(|i| Edge::Line {
                    a: p[i],
                    b: p[(i + 1) % 4],
                })
                .collect::<Vec<_>>()
        };
        let base = Sweep::new(
            vec![
                rectangle([0.; 2], [20., 16.]),
                vec![Edge::Circle {
                    center: [10., 8.],
                    radius_mm: 1.,
                    clockwise: true,
                }],
            ],
            4.,
            1e-7,
            1e-5,
        )
        .unwrap();
        for kind in [
            crate::normal_relief::Kind::Raised,
            crate::normal_relief::Kind::Recessed,
        ] {
            let motif = Sweep::new(
                vec![vec![Edge::Circle {
                    center: [3., 3.],
                    radius_mm: 0.75,
                    clockwise: false,
                }]],
                1.5,
                1e-7,
                1e-5,
            )
            .unwrap();
            let arc = Sweep::new(
                vec![vec![
                    Edge::Line {
                        a: [12., 2.],
                        b: [14., 2.],
                    },
                    Edge::Arc {
                        center: [14., 3.],
                        a: [14., 2.],
                        b: [14., 4.],
                        clockwise: false,
                    },
                    Edge::Line {
                        a: [14., 4.],
                        b: [12., 4.],
                    },
                    Edge::Line {
                        a: [12., 4.],
                        b: [12., 2.],
                    },
                ]],
                0.6,
                1e-7,
                1e-5,
            )
            .unwrap();
            let relief = Relief::new(base.clone(), vec![motif, arc], kind, 1e-7, 1e-5).unwrap();
            for (origin, u, v) in [
                ([0.; 3], [1., 0., 0.], [0., 1., 0.]),
                ([80., 60., 20.], [0.8, 0.6, 0.], [-0.36, 0.48, 0.8]),
                (
                    [100000., -50000., 70000.],
                    [0.8, 0.6, 0.],
                    [-0.36, 0.48, 0.8],
                ),
            ] {
                let bytes = write_normal_relief(&relief, origin, u, v).unwrap();
                let read = crate::step_read::read_analytic_solid(&bytes, 1e-7).unwrap();
                assert_eq!(read.solid_count, 1);
                assert_eq!(read.face_count, relief.boundary().faces.len());
                assert!((read.volume_mm3 - relief.volume_mm3()).abs() < 1e-6);
            }
        }
    }
}
