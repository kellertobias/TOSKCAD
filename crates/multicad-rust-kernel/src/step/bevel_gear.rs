//! Closed spherical bevel solids with exact cones and ruled spline flanks.
//!
//! Root, tip and back caps are conical surfaces and meridians are planes.
//! Fillet and involute flanks are degree (3, 1) ruled B-splines whose toe
//! rails are the exact homothetic image of their heel rails.
use super::*;
use crate::{
    bevel_gear::{Bevel, Curve, End, FaceKind, Piece, Point as LocalPoint, Shape},
    step_read::MAX_STEP_ENTITIES,
};
use std::collections::HashMap;

const MAX_SPANS: usize = 128;

/// One C¹ cubic Hermite B-spline in the parameter of its piece.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Rail {
    pub(crate) knots: Vec<f64>,
    pub(crate) poles: Vec<LocalPoint>,
}

impl Rail {
    fn fit(gear: &Bevel, piece: Piece, tolerance: f64) -> Result<Self, StepError> {
        let delta = gear.spec().pitch_cone_angle_rad;
        let heel = gear.end_distance_mm(End::Heel);
        let point = |s: f64| {
            let sample = gear.sample(piece, s);
            let r = gear.end_distance_along(End::Heel, sample.gamma);
            let dr = heel * (sample.gamma - delta).sin() / (sample.gamma - delta).cos().powi(2);
            let dgamma = -sample.derivative[2] / sample.gamma.sin();
            (
                gear.point(End::Heel, sample.direction, sample.gamma),
                std::array::from_fn(|i| {
                    r * sample.derivative[i] + dr * dgamma * sample.direction[i]
                }),
            )
        };
        let mut spans = 4;
        while spans <= MAX_SPANS {
            let knots: Vec<f64> = (0..=spans).map(|i| i as f64 / spans as f64).collect();
            let ends: Vec<(LocalPoint, LocalPoint)> = knots.iter().map(|&s| point(s)).collect();
            // Uniform C¹ Hermite spans: each joint is the midpoint of its two
            // neighboring poles, so double interior knots omit it exactly.
            let mut poles = vec![ends[0].0];
            let mut error = 0_f64;
            for i in 0..spans {
                let h = (knots[i + 1] - knots[i]) / 3.;
                let (a, da) = ends[i];
                let (b, db) = ends[i + 1];
                let p1 = std::array::from_fn(|k| a[k] + da[k] * h);
                let p2 = std::array::from_fn(|k| b[k] - db[k] * h);
                for t in [0.25_f64, 0.5, 0.75] {
                    let w = [
                        (1. - t).powi(3),
                        3. * t * (1. - t).powi(2),
                        3. * t * t * (1. - t),
                        t.powi(3),
                    ];
                    let bezier: LocalPoint = std::array::from_fn(|k| {
                        w[0] * a[k] + w[1] * p1[k] + w[2] * p2[k] + w[3] * b[k]
                    });
                    error = error.max(norm(sub(bezier, point(knots[i] + 3. * h * t).0)));
                }
                poles.extend([p1, p2]);
            }
            poles.push(ends[spans].0);
            if error <= tolerance / 16. {
                return Ok(Self { knots, poles });
            }
            spans *= 2;
        }
        Err(StepError::Limit)
    }

    fn multiplicities(&self) -> String {
        let last = self.knots.len() - 1;
        (0..=last)
            .map(|i| if i == 0 || i == last { "4" } else { "2" })
            .collect::<Vec<_>>()
            .join(",")
    }

    fn knot_values(&self) -> String {
        self.knots
            .iter()
            .map(|k| format!("{k:.16E}"))
            .collect::<Vec<_>>()
            .join(",")
    }
}

fn norm(a: LocalPoint) -> f64 {
    crate::bevel_gear::norm(a)
}

fn sub(a: LocalPoint, b: LocalPoint) -> LocalPoint {
    crate::bevel_gear::sub(a, b)
}

pub(crate) struct Prepared {
    pub(crate) rails: HashMap<Piece, Rail>,
}

pub(crate) fn prepare(gear: &Bevel, tolerance: f64) -> Result<Prepared, StepError> {
    if !tolerance.is_finite() || tolerance <= 0. {
        return Err(StepError::InvalidGeometry);
    }
    let mut rails = HashMap::new();
    let mut poles = 0;
    for &piece in gear.pieces() {
        if matches!(
            piece,
            Piece::FilletLeft | Piece::FilletRight | Piece::InvoluteLeft | Piece::InvoluteRight
        ) {
            let rail = Rail::fit(gear, piece, tolerance)?;
            poles += rail.poles.len();
            rails.insert(piece, rail);
        }
    }
    let faces = gear.pieces().len() + 4;
    let cost = (poles * 6 + faces * 40).checked_mul(gear.teeth() as usize);
    let within = cost.is_some_and(|cost| cost + 256 <= MAX_STEP_ENTITIES);
    within.then_some(Prepared { rails }).ok_or(StepError::Limit)
}

/// Whether this authored bevel fits the placement and spline budget.
pub fn supports_bevel(gear: &Bevel, origin: Point, u: Point, v: Point, tolerance: f64) -> bool {
    Placement::new(origin, u, v).is_ok() && prepare(gear, tolerance).is_ok()
}

pub fn write_bevel(
    gear: &Bevel,
    origin: Point,
    u: Point,
    v: Point,
    tolerance: f64,
) -> Result<String, StepError> {
    write_one(|writer| writer.append_bevel(gear, origin, u, v, tolerance))
}

fn rotate(p: LocalPoint, angle: f64) -> LocalPoint {
    let (s, c) = angle.sin_cos();
    [c * p[0] - s * p[1], s * p[0] + c * p[1], p[2]]
}

pub(super) fn encode(
    file: &mut StepFile,
    gear: &Bevel,
    placement: Placement,
    tolerance: f64,
) -> Result<Entity, StepError> {
    let Prepared { rails } = prepare(gear, tolerance)?;
    let world = |p: LocalPoint| placement.point_f64([p[0], p[1]], p[2]);
    let topology = gear.topology();
    let vertices: Vec<Vertex> = topology
        .vertices_mm
        .iter()
        .map(|p| file.vertex(world(*p)))
        .collect();
    let spacing = std::f64::consts::TAU / f64::from(gear.teeth());
    let turn = |sector: u32| gear.sector_start(sector) + spacing / 2.;
    let ratio = gear.end_distance_mm(End::Toe) / gear.end_distance_mm(End::Heel);
    let axis_z = placement.normal;
    let mut edges = HashMap::new();
    for edge in &topology.edges {
        let [a, b] = edge.vertices.map(|v| vertices[v as usize]);
        let entity = match gear.curve_shape(&topology, edge) {
            Shape::Line(_) => line_edge(file, a, b)?,
            Shape::Arc {
                z_mm,
                radius_mm,
                start_rad,
                sweep_rad,
            } => {
                let reference = placement.vector([start_rad.cos(), start_rad.sin(), 0.]);
                let arc = circular_arc_edge(
                    file,
                    [a, b],
                    world([0., 0., z_mm]),
                    axis_z,
                    reference,
                    radius_mm,
                    sweep_rad,
                );
                arc?.0
            }
            Shape::Sampled { sector, piece, end } => {
                let rail = &rails[&piece];
                let scale = if end == End::Heel { 1. } else { ratio };
                let last = rail.poles.len() - 1;
                let poles = rail
                    .poles
                    .iter()
                    .enumerate()
                    .map(|(i, p)| {
                        if i == 0 {
                            a.cartesian
                        } else if i == last {
                            b.cartesian
                        } else {
                            file.point(world(rotate(p.map(|x| x * scale), turn(sector))))
                        }
                    })
                    .collect::<Vec<_>>();
                let curve = file.entity(format_args!(
                    "B_SPLINE_CURVE_WITH_KNOTS('',3,{},.UNSPECIFIED.,.F.,.F.,({}),({}),.UNSPECIFIED.)",
                    Entities(&poles),
                    rail.multiplicities(),
                    rail.knot_values(),
                ));
                file.entity(format_args!(
                    "EDGE_CURVE('',{},{},{curve},.T.)",
                    a.vertex, b.vertex
                ))
            }
        };
        edges.insert(edge.curve, entity);
    }
    let edge = |curve: Curve| edges[&curve];
    let teeth = gear.teeth();
    let count = gear.pieces().len();
    let origin = file.point(placement.origin);
    let mut faces = vec![];
    for sector in 0..teeth {
        let next = (sector + 1) % teeth;
        let ray = |joint: usize| {
            if joint == count {
                edge(Curve::Ray {
                    sector: next,
                    joint: 0,
                })
            } else {
                edge(Curve::Ray { sector, joint })
            }
        };
        for (j, &piece) in gear.pieces().iter().enumerate() {
            let heel = edge(Curve::Profile {
                sector,
                piece,
                end: End::Heel,
            });
            let toe = edge(Curve::Profile {
                sector,
                piece,
                end: End::Toe,
            });
            let loop_edges = [
                file.oriented_edge(heel, false),
                file.oriented_edge(ray(j), false),
                file.oriented_edge(toe, true),
                file.oriented_edge(ray(j + 1), true),
            ];
            let bound = file.bound(&loop_edges, true);
            let surface = match piece {
                Piece::RootLeft | Piece::RootRight | Piece::Tip => {
                    let gamma = if piece == Piece::Tip {
                        gear.tip_gamma()
                    } else {
                        gear.root_gamma()
                    };
                    let axis = file.axis(origin, axis_z, placement.u);
                    file.entity(format_args!("CONICAL_SURFACE('',{axis},0.,{gamma:.16E})"))
                }
                Piece::MeridianLeft | Piece::MeridianRight => {
                    let (_, _, plane) = gear.face_metrics(crate::bevel_gear::Face {
                        kind: FaceKind::Side(piece),
                        sector,
                    });
                    let (_, normal) = plane.expect("meridian plane");
                    let direction = gear.rotate(sector, gear.sample(piece, 0.).direction);
                    let axis = file.axis(
                        origin,
                        placement.vector(normal),
                        placement.vector(direction),
                    );
                    file.entity(format_args!("PLANE('',{axis})"))
                }
                _ => {
                    let rail = &rails[&piece];
                    let rows = rail
                        .poles
                        .iter()
                        .map(|p| {
                            let heel = rotate(*p, turn(sector));
                            let toe = heel.map(|x| x * ratio);
                            let row = [file.point(world(toe)), file.point(world(heel))];
                            Entities(&row).to_string()
                        })
                        .collect::<Vec<_>>()
                        .join(",");
                    file.entity(format_args!(
                        "B_SPLINE_SURFACE_WITH_KNOTS('',3,1,({rows}),.UNSPECIFIED.,.F.,.F.,.F.,({}),(2,2),({}),(0.,1.),.UNSPECIFIED.)",
                        rail.multiplicities(),
                        rail.knot_values(),
                    ))
                }
            };
            faces.push(file.face_on(&[bound], surface, true));
        }
        for end in End::BOTH {
            let apex = file.point(world([0., 0., gear.back_cone_apex_mm(end)]));
            let axis = file.axis(apex, axis_z.map(|x| -x), placement.u);
            let semi = (std::f64::consts::FRAC_PI_2 - gear.spec().pitch_cone_angle_rad).max(0.);
            let cone = file.entity(format_args!("CONICAL_SURFACE('',{axis},0.,{semi:.16E})"));
            let heel = end == End::Heel;
            let mut outer: Vec<(Entity, bool)> = gear
                .pieces()
                .iter()
                .map(|&piece| (edge(Curve::Profile { sector, piece, end }), true))
                .collect();
            outer.push((
                edge(Curve::SectorLine {
                    sector: next,
                    end,
                    core: false,
                }),
                true,
            ));
            let core = gear.has_core(end);
            let floor = if core {
                Curve::InnerArc { sector, end }
            } else {
                Curve::BoreArc { sector, end }
            };
            outer.push((edge(floor), false));
            outer.push((
                edge(Curve::SectorLine {
                    sector,
                    end,
                    core: false,
                }),
                false,
            ));
            let mut loops = vec![outer];
            if core {
                let mut inner = vec![
                    (edge(Curve::InnerArc { sector, end }), true),
                    (
                        edge(Curve::SectorLine {
                            sector: next,
                            end,
                            core: true,
                        }),
                        true,
                    ),
                ];
                if gear.bore_radius_mm().is_some() {
                    inner.push((edge(Curve::BoreArc { sector, end }), false));
                }
                inner.push((
                    edge(Curve::SectorLine {
                        sector,
                        end,
                        core: true,
                    }),
                    false,
                ));
                loops.push(inner);
            }
            for mut ring in loops {
                if !heel {
                    ring.reverse();
                    for item in &mut ring {
                        item.1 = !item.1;
                    }
                }
                let oriented: Vec<_> = ring
                    .into_iter()
                    .map(|(e, forward)| file.oriented_edge(e, forward))
                    .collect();
                let bound = file.bound(&oriented, true);
                faces.push(file.face_on(&[bound], cone, heel));
            }
        }
    }
    if let Some(bore) = gear.bore_radius_mm() {
        let axis = file.axis(origin, axis_z, placement.u);
        let surface = file.entity(format_args!("CYLINDRICAL_SURFACE('',{axis},{bore:.16E})"));
        let seam = edge(Curve::BoreSeam);
        let mut ring = vec![file.oriented_edge(seam, true)];
        for sector in 0..teeth {
            ring.push(file.oriented_edge(
                edge(Curve::BoreArc {
                    sector,
                    end: End::Heel,
                }),
                true,
            ));
        }
        ring.push(file.oriented_edge(seam, false));
        for sector in (0..teeth).rev() {
            ring.push(file.oriented_edge(
                edge(Curve::BoreArc {
                    sector,
                    end: End::Toe,
                }),
                false,
            ));
        }
        let bound = file.bound(&ring, true);
        faces.push(file.face_on(&[bound], surface, false));
    }
    let solid = file.solid(&faces);
    let within = file.next + 128 <= MAX_STEP_ENTITIES
        && file.data.len() + 4096 <= crate::step_read::MAX_PLANAR_STEP_BYTES;
    within.then_some(solid).ok_or(StepError::Limit)
}

#[cfg(test)]
mod tests;
