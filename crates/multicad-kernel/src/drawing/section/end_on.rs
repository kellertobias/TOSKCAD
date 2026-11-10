//! Retain planar stock and project exact blind cap profiles in one visibility
//! scene with the other sectioned parts and any circular hardware.
use super::*;
use multicad_rust_kernel::{
    planar_projection::{
        self, Body, FaceOpening, LineCandidate, RimCandidate, SceneExtras, StadiumOpening,
    },
    round_occlusion::RoundOccluder,
    stadium::Stadium,
};
use std::f64::consts::{PI, TAU};

type Point = [f64; 3];
type Span = [f64; 2];
const AXIS_TOLERANCE: f64 = 32. * f64::EPSILON;

fn unsupported() -> GeometryError {
    err(
        GeometryErrorKind::Unsupported,
        "Rust retained projection cannot yet clip this curved boundary",
    )
}

fn dot(a: Point, b: Point) -> f64 {
    a.into_iter().zip(b).map(|(a, b)| a * b).sum()
}

fn limit() -> GeometryError {
    planar::projection_error(planar_projection::ProjectionError::Limit)
}

pub(super) fn project(
    solids: &[DrawingSolid<'_>],
    mut options: DrawingProjection,
    depth: f64,
) -> Result<Vec<ProjectedVector>, GeometryError> {
    let tolerance = solids
        .iter()
        .map(|input| input.solid.geometry_policy().linear_tolerance_mm)
        .fold(GeometryPolicy::default().linear_tolerance_mm, f64::max);
    let far = options.section_depth_mm.map(|range| depth - range);
    let volume = Volume {
        axes: planar::view_axes(options.frame),
        origin: options.frame.origin,
        bounds: [
            options
                .section_u_mm
                .unwrap_or([f64::NEG_INFINITY, f64::INFINITY]),
            options
                .section_v_mm
                .unwrap_or([f64::NEG_INFINITY, f64::INFINITY]),
            [far.unwrap_or(f64::NEG_INFINITY), depth],
        ],
        tolerance,
    };
    let segment_limit = (options.maximum_curves as usize).min(options.maximum_points as usize / 2);
    let slice = planar_section::Options {
        depth,
        windows: [options.section_u_mm, options.section_v_mm],
        tolerance,
        segment_limit,
    };
    let mut scene = Scene::default();
    let mut cuts = Vec::new();
    for (source, solid) in projection_members(solids) {
        let bounds = volume.project_bounds(solid.bounds()?);
        if !volume.overlaps(bounds) {
            continue;
        }
        if bounds[2][0] < depth - tolerance && bounds[2][1] > depth + tolerance {
            cuts.push((
                source,
                DrawingSolid {
                    solid,
                    hatch: solids[source].hatch,
                },
            ));
        }
        if let Some((frame, radius, height, bore)) = curved::circular_dimensions(solid) {
            scene.rounds.push(if volume.contains(bounds) {
                curved::round_occluder(solid, options.frame)?.expect("classified round solid")
            } else {
                volume.round(frame, radius, height, bore)?
            });
            scene.round_sources.push(source);
        } else if solid.rounded_slot_body().is_some() {
            if !volume.contains(bounds) {
                return Err(unsupported());
            }
            scene.rounded.add(solid, source, options)?;
        } else if let Some((stock, circles, slots)) = solid.drawing_machined_stock() {
            let body = scene.sources.len();
            scene.push_stock(source, Some(stock), solid);
            for opening in circles {
                scene.opening(
                    &volume,
                    body,
                    Mouth::Circle(volume.point(opening.center), opening.radius),
                    volume.vector(opening.normal),
                    opening.depth,
                )?;
            }
            for slot in slots {
                scene.opening(
                    &volume,
                    body,
                    Mouth::Slot(slot.centers.map(|point| volume.point(point)), slot.radius),
                    volume.vector(slot.normal),
                    slot.depth,
                )?;
            }
        } else {
            scene.push_stock(source, None, solid);
        }
    }
    let inputs: Vec<_> = scene
        .stock
        .iter()
        .map(|(proxy, solid)| DrawingSolid {
            solid: proxy.as_ref().unwrap_or(solid),
            hatch: None,
        })
        .collect();
    let (bodies, _) = planar::prepare(&inputs, options.frame)?;
    let bodies = planar_section::retain(bodies, slice, far)
        .map_err(planar::projection_error)?
        .bodies;
    let mut output = scene.visible(&bodies, options, tolerance, segment_limit)?;
    options.maximum_curves -= output.len() as u32;
    options.maximum_points -= output
        .iter()
        .map(|vector| vector.preview_mm.len() as u32)
        .sum::<u32>();
    options.section_only = true;
    options.section_depth_mm = None;
    output.extend(machined::project_members(cuts, options, depth)?);
    Ok(output)
}

struct Volume {
    axes: [Point; 3],
    origin: Point,
    bounds: [Span; 3],
    tolerance: f64,
}

impl Volume {
    fn point(&self, world: Point) -> Point {
        self.vector(std::array::from_fn(|i| world[i] - self.origin[i]))
    }

    fn vector(&self, world: Point) -> Point {
        self.axes.map(|axis| dot(axis, world))
    }

    fn project_bounds(&self, bounds: crate::Bounds) -> [Span; 3] {
        let [lo, hi]: [Point; 2] = [false, true].map(|high| {
            std::array::from_fn(|i| {
                let axis = self.axes[i];
                let corner: Point = std::array::from_fn(|j| {
                    if high == (axis[j] >= 0.) {
                        bounds.max[j]
                    } else {
                        bounds.min[j]
                    }
                });
                dot(axis, std::array::from_fn(|j| corner[j] - self.origin[j]))
            })
        });
        std::array::from_fn(|i| [lo[i], hi[i]])
    }

    fn overlaps(&self, bounds: [Span; 3]) -> bool {
        self.bounds
            .into_iter()
            .zip(bounds)
            .all(|([lo, hi], [a, b])| b > lo + self.tolerance && a < hi - self.tolerance)
    }

    fn contains(&self, bounds: [Span; 3]) -> bool {
        self.bounds
            .into_iter()
            .zip(bounds)
            .all(|([lo, hi], [a, b])| a >= lo - self.tolerance && b <= hi + self.tolerance)
    }

    fn round(
        &self,
        frame: PlaneFrame,
        radius: f64,
        height: f64,
        bore: Option<f64>,
    ) -> Result<RoundOccluder, GeometryError> {
        let [u, v, base] = self.point(frame.origin);
        let [x, y, z] = self.vector(planar::view_axes(frame)[2]);
        let [[left, right], [bottom, top], [far, near]] = self.bounds;
        if x.hypot(y) > AXIS_TOLERANCE
            || u - radius < left - self.tolerance
            || u + radius > right + self.tolerance
            || v - radius < bottom - self.tolerance
            || v + radius > top + self.tolerance
        {
            return Err(unsupported());
        }
        let end = z.mul_add(height, base);
        let start = base.min(end).max(far);
        let end = base.max(end).min(near);
        RoundOccluder::new([u, v, start], [0., 0., 1.], radius, end - start, bore)
            .map_err(planar::projection_error)
    }
}

#[derive(Clone, Copy)]
enum Mouth {
    Circle(Point, f64),
    Slot([Point; 2], f64),
}

impl Mouth {
    fn span(self, normal: Point, depth: f64) -> [Span; 3] {
        let (points, radius) = match self {
            Self::Circle(center, radius) => ([center; 2], radius),
            Self::Slot(centers, radius) => (centers, radius),
        };
        std::array::from_fn(|i| {
            let [a, b] = points.map(|point| point[i]);
            let extent = radius * (1. - normal[i].powi(2)).max(0.).sqrt();
            let sweep = -normal[i] * depth;
            [
                a.min(b) - extent + sweep.min(0.),
                a.max(b) + extent + sweep.max(0.),
            ]
        })
    }

    fn depth(self) -> f64 {
        match self {
            Self::Circle(point, _) | Self::Slot([point, _], _) => point[2],
        }
    }

    fn center(self) -> [f64; 2] {
        match self {
            Self::Circle([u, v, _], _) => [u, v],
            Self::Slot([[u, v, _], [x, y, _]], _) => [u.midpoint(x), v.midpoint(y)],
        }
    }
}

struct Opening {
    body: usize,
    shape: Mouth,
    mouth_depth: f64,
    floor_depth: f64,
}

#[derive(Default)]
struct Scene<'a> {
    stock: Vec<(Option<Solid>, &'a Solid)>,
    sources: Vec<usize>,
    round_sources: Vec<usize>,
    rounds: Vec<RoundOccluder>,
    rounded: curved::stadium::Shapes,
    openings: Vec<Opening>,
}

impl<'a> Scene<'a> {
    fn push_stock(&mut self, source: usize, proxy: Option<Solid>, solid: &'a Solid) {
        self.sources.push(source);
        self.stock.push((proxy, solid));
    }

    fn opening(
        &mut self,
        volume: &Volume,
        body: usize,
        shape: Mouth,
        normal: Point,
        depth: f64,
    ) -> Result<(), GeometryError> {
        let bounds = shape.span(normal, depth);
        if !volume.overlaps(bounds) {
            return Ok(());
        }
        if normal[2].abs() <= AXIS_TOLERANCE
            || (normal[2] < 0. && normal[0].hypot(normal[1]) <= AXIS_TOLERANCE)
        {
            // A side-on or back-facing opening preserves every first hit
            // only while the retained volume leaves the opening intact.
            return if volume.contains(bounds) {
                Ok(())
            } else {
                Err(unsupported())
            };
        }
        if normal[2] <= 0. || normal[0].hypot(normal[1]) > AXIS_TOLERANCE {
            return Err(unsupported());
        }
        let mouth_depth = shape.depth().min(volume.bounds[2][1]);
        let floor_depth = shape.depth() - depth;
        let mut retained = bounds;
        retained[2][1] = mouth_depth;
        if !volume.contains(retained) || floor_depth <= volume.bounds[2][0] + volume.tolerance {
            return Err(unsupported());
        }
        self.openings.push(Opening {
            body,
            shape,
            mouth_depth,
            floor_depth,
        });
        Ok(())
    }

    fn visible(
        mut self,
        bodies: &[Body],
        options: DrawingProjection,
        tolerance: f64,
        segment_limit: usize,
    ) -> Result<Vec<ProjectedVector>, GeometryError> {
        let mut circles = Vec::new();
        let mut slots = Vec::new();
        let mut rims = Vec::new();
        let mut work = 2_000_000usize;
        for Opening {
            body,
            shape,
            mouth_depth,
            floor_depth,
        } in self.openings
        {
            let face = mouth_face(
                &bodies[body],
                shape.center(),
                mouth_depth,
                tolerance,
                &mut work,
            )?;
            let source = self.sources[body];
            match shape {
                Mouth::Circle([u, v, _], radius) => {
                    let center = [u, v];
                    circles.push(FaceOpening {
                        body,
                        face,
                        center,
                        radius,
                        end: planar_projection::CircularOpeningEnd::Floor(floor_depth),
                    });
                    rims.push(RimCandidate {
                        source,
                        center,
                        depth: mouth_depth,
                        radius,
                        trim: [0., TAU],
                    });
                }
                Mouth::Slot(points, radius) => {
                    let centers = points.map(|[u, v, _]| [u, v]);
                    let stadium = Stadium::new(centers, radius).map_err(|_| unsupported())?;
                    slots.push(StadiumOpening {
                        body,
                        face,
                        centers,
                        radius,
                        floor_depth,
                    });
                    let corners = stadium.corners();
                    for i in [0, 2] {
                        self.rounded.lines.push(LineCandidate {
                            source,
                            points: [corners[i], corners[i + 1]].map(|[u, v]| [u, v, mouth_depth]),
                        });
                    }
                    for (i, corner) in [(0, 3), (1, 1)] {
                        let center = centers[i];
                        let start = (corners[corner][1] - center[1])
                            .atan2(corners[corner][0] - center[0])
                            .rem_euclid(TAU);
                        let end = start + PI;
                        for trim in [[start, end.min(TAU)], [0., end - TAU]] {
                            if trim[1] - trim[0] > AXIS_TOLERANCE {
                                rims.push(RimCandidate {
                                    source,
                                    center,
                                    depth: mouth_depth,
                                    radius,
                                    trim,
                                });
                            }
                        }
                    }
                }
            }
        }
        if bodies.iter().all(|body| body.faces.is_empty())
            && self.rounds.is_empty()
            && self.rounded.lines.is_empty()
            && self.rounded.rims.is_empty()
        {
            return Ok(Vec::new());
        }
        let projection = planar_projection::project_scene_with_candidates(
            bodies,
            &self.rounds,
            SceneExtras {
                rims: &rims,
                spatial_rims: &self.rounded.rims,
                lines: &self.rounded.lines,
                blocker_bodies: &self.rounded.bodies,
                blocker_rounds: &self.rounded.rounds,
                openings: &circles,
                stadium_openings: &slots,
                ..Default::default()
            },
            planar_projection::Options {
                tolerance,
                hidden_lines: false,
                tangent_edges: false,
                detail: None,
                segment_limit,
            },
        )
        .map_err(planar::projection_error)?;
        curved::publish_scene(projection, options, &self.sources, &self.round_sources)
    }
}

fn mouth_face(
    body: &Body,
    [u, v]: [f64; 2],
    depth: f64,
    tolerance: f64,
    work: &mut usize,
) -> Result<usize, GeometryError> {
    for (i, face) in body.faces.iter().enumerate() {
        if face.normal[2] <= 0.
            || face.normal[0].hypot(face.normal[1]) > AXIS_TOLERANCE
            || (depth - face.point[2]).abs() > tolerance
        {
            continue;
        }
        *work = work.checked_sub(face.edges.len()).ok_or_else(limit)?;
        let inside = face.edges.iter().fold(false, |inside, &edge| {
            let [[x, y, _], [a, b, _]] = body.edges[edge];
            inside ^ ((y > v) != (b > v) && u < ((v - y) / (b - y)).mul_add(a - x, x))
        });
        if inside {
            return Ok(i);
        }
    }
    Err(unsupported())
}
