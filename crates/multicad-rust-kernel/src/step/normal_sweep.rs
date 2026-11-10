use super::*;
use crate::normal_sweep::{CertifiedPiercing, Edge, Sweep};

/// Write a mixed profile sweep as planar and partial cylindrical faces.
pub fn write_normal_sweep(
    sweep: &Sweep,
    origin: Point,
    u: Point,
    v: Point,
    reverse_sweep: bool,
) -> Result<String, StepError> {
    write_one(|writer| writer.append_normal_sweep(sweep, origin, u, v, reverse_sweep))
}

pub(super) enum ProfileEdge<'a> {
    Analytic(Edge),
    Cubic(cubic_trim::CubicTrim<'a>),
}

impl ProfileEdge<'_> {
    fn start(&self) -> [f64; 2] {
        match self {
            Self::Analytic(edge) => edge.start(),
            Self::Cubic(spline) => spline.start(),
        }
    }
}

enum WallSupport {
    Plane,
    Cylinder(Entity),
    LinearExtrusion(Entity),
}

struct Rim {
    vertices: Vec<Vertex>,
    edges: Vec<(Entity, bool)>,
    supports: Vec<WallSupport>,
}

impl Rim {
    fn new(
        file: &mut StepFile,
        ring: &[ProfileEdge<'_>],
        placement: &Placement,
        z: f64,
    ) -> Result<Self, StepError> {
        let vertices: Vec<_> = ring
            .iter()
            .map(|edge| file.vertex(placement.point_f64(edge.start(), z)))
            .collect();
        let (edges, supports) = ring
            .iter()
            .zip(vertices.iter().zip(vertices.iter().cycle().skip(1)))
            .map(|(edge, (&from, &to))| match edge {
                ProfileEdge::Analytic(Edge::Line { .. }) => {
                    Ok(((line_edge(file, from, to)?, true), WallSupport::Plane))
                }
                ProfileEdge::Analytic(
                    edge @ Edge::Arc {
                        center,
                        a,
                        clockwise,
                        ..
                    },
                ) => {
                    let [x, y] = std::array::from_fn(|i| a[i] - center[i]);
                    let radius = x.hypot(y);
                    let reference = placement.vector([x / radius, y / radius, 0.]);
                    let normal = placement.normal.map(|n| if *clockwise { -n } else { n });
                    let center = placement.point_f64(*center, z);
                    let sweep = edge.signed_sweep_rad().ok_or(StepError::InvalidGeometry)?;
                    let (edge, axis) = circular_arc_edge(
                        file,
                        [from, to],
                        center,
                        normal,
                        reference,
                        radius,
                        sweep,
                    )?;
                    Ok(((edge, true), WallSupport::Cylinder(axis)))
                }
                ProfileEdge::Analytic(Edge::Circle { .. }) => Err(StepError::InvalidGeometry),
                ProfileEdge::Cubic(spline) => {
                    let (edge, curve) = spline.encode(file, [from, to], placement, z)?;
                    Ok((
                        (edge, !spline.reversed()),
                        WallSupport::LinearExtrusion(curve),
                    ))
                }
            })
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .unzip();
        Ok(Self {
            vertices,
            edges,
            supports,
        })
    }

    fn bound(&self, file: &mut StepFile, end: bool, outer: bool) -> Entity {
        let mut edges = self.edges.clone();
        if !end {
            edges.reverse();
        }
        let edges: Vec<_> = edges
            .into_iter()
            .map(|(edge, forward)| file.oriented_edge(edge, end == forward))
            .collect();
        file.bound(&edges, outer)
    }
}

pub(super) fn encode(
    file: &mut StepFile,
    sweep: &Sweep,
    piercings: &[CertifiedPiercing],
    placement: Placement,
    reverse: bool,
) -> Result<Entity, StepError> {
    let mut rings = sweep
        .rings()
        .iter()
        .map(|ring| ring.iter().copied().map(ProfileEdge::Analytic).collect());
    encode_rings(
        file,
        &mut rings,
        sweep.height_mm(),
        piercings,
        placement,
        reverse,
    )
}

pub(super) fn encode_rings<'a>(
    file: &mut StepFile,
    rings: &mut dyn Iterator<Item = Vec<ProfileEdge<'a>>>,
    height: f64,
    piercings: &[CertifiedPiercing],
    placement: Placement,
    reverse: bool,
) -> Result<Entity, StepError> {
    encode_rings_with_trims(
        file,
        rings,
        height,
        piercings,
        placement,
        reverse,
        EndTrims::default(),
    )
}

#[derive(Default)]
pub(super) struct EndTrims {
    pub(super) holes: [Option<Entity>; 2],
    pub(super) faces: Vec<Entity>,
}

pub(super) fn encode_rings_with_trims<'a>(
    file: &mut StepFile,
    rings: &mut dyn Iterator<Item = Vec<ProfileEdge<'a>>>,
    height: f64,
    piercings: &[CertifiedPiercing],
    placement: Placement,
    reverse: bool,
    trims: EndTrims,
) -> Result<Entity, StepError> {
    let levels = if reverse { [-height, 0.] } else { [0., height] };
    let normal = placement.normal;
    let openings = pierced_sweep::encode_openings(file, piercings, &placement, reverse)?;
    let supports = levels.map(|z| file.point(placement.point_f64([0.; 2], z)));
    let mut caps = [Vec::new(), Vec::new()];
    let mut walls = Vec::new();
    let mut side_ordinal = 0;
    for (index, ring) in rings.enumerate() {
        let ring_ordinal = side_ordinal;
        side_ordinal += ring.len();
        let outer = index == 0;
        if let [
            ProfileEdge::Analytic(Edge::Circle {
                center, radius_mm, ..
            }),
        ] = ring.as_slice()
        {
            let trim = CylindricalTrim::new(
                file,
                levels.map(|z| placement.point_f64(*center, z)),
                normal,
                placement.u,
                *radius_mm,
            )?;
            for (cap, bounds) in caps.iter_mut().enumerate() {
                bounds.push(trim.cap_bound(file, cap == 1, outer));
            }
            walls.push(trim.wall(file, outer));
            continue;
        }
        let low = Rim::new(file, &ring, &placement, levels[0])?;
        let high = Rim::new(file, &ring, &placement, levels[1])?;
        caps[0].push(low.bound(file, false, outer));
        caps[1].push(high.bound(file, true, outer));
        let rails = low
            .vertices
            .iter()
            .zip(&high.vertices)
            .map(|(&a, &b)| line_edge(file, a, b))
            .collect::<Result<Vec<_>, _>>()?;
        for (i, edge) in ring.iter().enumerate() {
            let next = (i + 1) % ring.len();
            let boundary = [
                low.edges[i],
                (rails[next], true),
                (high.edges[i].0, !high.edges[i].1),
                (rails[i], false),
            ]
            .map(|(edge, forward)| file.oriented_edge(edge, forward));
            let bound = file.bound(&boundary, true);
            let face = if let (
                ProfileEdge::Analytic(Edge::Arc {
                    center,
                    a,
                    clockwise,
                    ..
                }),
                WallSupport::Cylinder(axis),
            ) = (edge, &low.supports[i])
            {
                let [x, y] = std::array::from_fn(|j| a[j] - center[j]);
                let radius = x.hypot(y);
                // Cylinder support always follows the sweep axis. The
                // circular edge itself may run clockwise around this axis.
                let axis = if *clockwise {
                    let center = file.point(placement.point_f64(*center, levels[0]));
                    file.axis(
                        center,
                        normal,
                        placement.vector([x / radius, y / radius, 0.]),
                    )
                } else {
                    *axis
                };
                let cylinder =
                    file.entity(format_args!("CYLINDRICAL_SURFACE('',{axis},{radius:.16E})"));
                file.face_on(&[bound], cylinder, !*clockwise)
            } else if let WallSupport::LinearExtrusion(curve) = low.supports[i] {
                let axis = file.direction(normal);
                let vector = file.entity(format_args!("VECTOR('',{axis},1.)"));
                let surface = file.entity(format_args!(
                    "SURFACE_OF_LINEAR_EXTRUSION('',{curve},{vector})"
                ));
                file.face_on(&[bound], surface, low.edges[i].1)
            } else {
                let tangent = direction(low.vertices[i].point, low.vertices[next].point)?;
                let mut boundaries = vec![bound];
                if let Some(holes) = openings.holes.get(&(ring_ordinal + i)) {
                    boundaries.extend(holes);
                }
                file.face(
                    &boundaries,
                    low.vertices[i].cartesian,
                    cross(tangent, normal),
                    tangent,
                )
            };
            walls.push(face);
        }
    }
    for (cap, hole) in caps.iter_mut().zip(trims.holes) {
        cap.extend(hole);
    }
    let mut faces = vec![
        file.face(&caps[0], supports[0], normal.map(|n| -n), placement.u),
        file.face(&caps[1], supports[1], normal, placement.u),
    ];
    faces.extend(walls);
    faces.extend(openings.walls);
    faces.extend(trims.faces);
    Ok(file.solid(&faces))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exchange_refuses_curves_collapsed_by_world_placement() {
        let x = [1., 0., 0.];
        let y = [0., 1., 0.];
        let placement = Placement::new([1e16, 0., 0.], x, y).unwrap();
        let arc = Edge::Arc {
            center: [0., 0.],
            a: [1., 0.],
            b: [0., 1.],
            clockwise: false,
        };
        let arc_wire = [
            arc,
            Edge::Line {
                a: arc.end(),
                b: arc.start(),
            },
        ]
        .map(ProfileEdge::Analytic);
        assert_eq!(
            Rim::new(&mut StepFile::new(), &arc_wire, &placement, 0.).err(),
            Some(StepError::InvalidGeometry)
        );
        let circle = Edge::Circle {
            center: [0., 0.],
            radius_mm: 1.,
            clockwise: false,
        };
        assert_eq!(
            Rim::new(
                &mut StepFile::new(),
                &[ProfileEdge::Analytic(circle)],
                &placement,
                0.
            )
            .err(),
            Some(StepError::InvalidGeometry)
        );
        let sweep = Sweep::new(vec![vec![circle]], 1., 1e-7, 1e-5).unwrap();
        assert_eq!(
            write_normal_sweep(&sweep, [0., 0., 1e16], x, y, false),
            Err(StepError::InvalidGeometry)
        );
        let corners = [[0., 0.], [1., 0.], [1., 1.], [0., 1.]];
        let square = (0..4)
            .map(|i| Edge::Line {
                a: corners[i],
                b: corners[(i + 1) % 4],
            })
            .collect();
        let sweep = Sweep::new(vec![square], 1., 1e-7, 1e-5).unwrap();
        for origin in [[1e16, 0., 0.], [0., 0., 1e16]] {
            assert_eq!(
                write_normal_sweep(&sweep, origin, x, y, false),
                Err(StepError::InvalidGeometry)
            );
        }
    }
}
