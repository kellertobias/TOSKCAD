//! Exact rounded extrusions for mixed drawing scenes. The occluder is the
//! union of an interior rectangular sweep and two cylindrical end sweeps.
use super::*;
use multicad_core::references::{AnalyticGeometry, ReferenceFrame, TopologyKind};
use multicad_rust_kernel::{
    curved_projection::Ellipse,
    planar_projection::{Body, Face, LineCandidate, SpatialRimCandidate},
};
use std::f64::consts::TAU;

type Point = [f64; 3];
const ANGULAR_TOLERANCE: f64 = 32. * f64::EPSILON;

#[derive(Default)]
pub(in crate::drawing) struct Shapes {
    pub bodies: Vec<Body>,
    pub rounds: Vec<RoundOccluder>,
    pub lines: Vec<LineCandidate>,
    pub rims: Vec<SpatialRimCandidate>,
}

impl Shapes {
    pub(in crate::drawing) fn add(
        &mut self,
        solid: &Solid,
        source: usize,
        options: DrawingProjection,
    ) -> Result<(), GeometryError> {
        let (body, frame) = solid
            .rounded_slot_body()
            .expect("classified rounded extrusion");
        let (profile, [start, end]) = body.section_profile();
        let screen = planar::view_axes(options.frame);
        let vector = |direction| screen.map(|axis| dot(axis, direction));
        let point = |world: Point| vector(subtract(world, options.frame.origin));
        let local = |position| point(crate::rust_prism_topology::position(frame, position));
        let axial = vector(unit(frame.normal()));
        let caps = [start, end].map(|z| profile.corners().map(|[u, v]| local([u, v, z])));
        self.bodies.push(corridor(caps));
        for [u, v] in profile.centers() {
            self.rounds.push(
                RoundOccluder::new(
                    local([u, v, start]),
                    axial,
                    profile.radius_mm(),
                    end - start,
                    None,
                )
                .map_err(planar::projection_error)?,
            );
        }

        let topology = solid.rust_topology().expect("Rust rounded extrusion")?;
        for element in topology.elements {
            if element.key.kind != TopologyKind::Edge
                || (element.key.ordinal >= 8 && !options.tangent_edges)
            {
                continue;
            }
            match element.geometry {
                Some(AnalyticGeometry::Line { a_mm, b_mm }) => {
                    self.lines.push(LineCandidate {
                        source,
                        points: [point(a_mm), point(b_mm)],
                    });
                }
                Some(AnalyticGeometry::CircularArc {
                    frame,
                    radius_mm,
                    start_rad,
                    sweep_rad,
                }) => {
                    self.rims.extend(spatial_rims(
                        source, frame, radius_mm, start_rad, sweep_rad, point, vector,
                    ));
                }
                _ => {
                    return Err(err(
                        GeometryErrorKind::InvalidResult,
                        "Rounded extrusion has an invalid analytic drawing edge",
                    ));
                }
            }
        }

        // The smooth side silhouette follows the two extremal generators
        // perpendicular to the projected extrusion axis.
        let projected = axial[0].hypot(axial[1]);
        if projected > ANGULAR_TOLERANCE {
            for sign in [-1., 1.] {
                let direction = [-sign * axial[1] / projected, sign * axial[0] / projected];
                let normal = [vector(frame.u), vector(frame.v)]
                    .map(|basis| direction[0] * basis[0] + direction[1] * basis[1]);
                let center = profile
                    .centers()
                    .into_iter()
                    .max_by(|a, b| {
                        let support = |p: [f64; 2]| p[0] * normal[0] + p[1] * normal[1];
                        let depth = |p: [f64; 2]| local([p[0], p[1], start])[2];
                        support(*a)
                            .total_cmp(&support(*b))
                            .then_with(|| depth(*a).total_cmp(&depth(*b)))
                    })
                    .expect("two stadium centers");
                let [u, v] =
                    std::array::from_fn(|i| normal[i].mul_add(profile.radius_mm(), center[i]));
                self.lines.push(LineCandidate {
                    source,
                    points: [start, end].map(|z| local([u, v, z])),
                });
            }
        }
        Ok(())
    }
}

fn subtract(a: Point, b: Point) -> Point {
    std::array::from_fn(|i| a[i] - b[i])
}

fn cross(a: Point, b: Point) -> Point {
    std::array::from_fn(|i| a[(i + 1) % 3] * b[(i + 2) % 3] - a[(i + 2) % 3] * b[(i + 1) % 3])
}

fn corridor([bottom, top]: [[Point; 4]; 2]) -> Body {
    let edges = [bottom, top]
        .into_iter()
        .flat_map(|cap| (0..4).map(move |i| [cap[i], cap[(i + 1) % 4]]))
        .chain((0..4).map(|i| [bottom[i], top[i]]))
        .collect();
    let axial = unit(subtract(top[0], bottom[0]));
    let mut faces = vec![
        Face {
            point: bottom[0],
            normal: axial.map(|v| -v),
            edges: (0..4).collect(),
        },
        Face {
            point: top[0],
            normal: axial,
            edges: (4..8).collect(),
        },
    ];
    faces.extend((0..4).map(|i| {
        let next = (i + 1) % 4;
        Face {
            point: bottom[i],
            normal: unit(cross(subtract(bottom[next], bottom[i]), axial)),
            edges: vec![i, i + 4, i + 8, next + 8],
        }
    }));
    Body { edges, faces }
}

fn spatial_rims(
    source: usize,
    frame: ReferenceFrame,
    radius: f64,
    start: f64,
    sweep: f64,
    point: impl Fn(Point) -> Point,
    vector: impl Fn(Point) -> Point,
) -> impl Iterator<Item = SpatialRimCandidate> {
    let [original_u, original_v] = [vector(frame.u), vector(frame.v)];
    let [a, b] = [original_v[2], -original_u[2]];
    let inclination = a.hypot(b);
    let [cos, sin] = if inclination <= ANGULAR_TOLERANCE {
        [1., 0.]
    } else {
        [a / inclination, b / inclination]
    };
    let major: Point = std::array::from_fn(|i| cos.mul_add(original_u[i], sin * original_v[i]));
    let minor: Point = std::array::from_fn(|i| (-sin).mul_add(original_u[i], cos * original_v[i]));
    let screen_u = [major[0], major[1]];
    let minor_scale = minor[0].hypot(minor[1]);
    let screen_v = if minor_scale <= ANGULAR_TOLERANCE {
        [-screen_u[1], screen_u[0]]
    } else {
        [minor[0] / minor_scale, minor[1] / minor_scale]
    };
    let [u, v, depth] = point(frame.origin_mm);
    let start = (start + sweep.min(0.) - sin.atan2(cos)).rem_euclid(TAU);
    let end = start + sweep.abs();
    [[start, end.min(TAU)], [0., end - TAU]]
        .into_iter()
        .filter(|[a, b]| b - a > ANGULAR_TOLERANCE)
        .map(move |trim| SpatialRimCandidate {
            source,
            ellipse: Ellipse {
                center: [u, v],
                u: screen_u,
                v: screen_v,
                major: radius,
                minor: radius * minor_scale,
                trim,
            },
            depth,
            depth_sine: radius * minor[2],
        })
}
