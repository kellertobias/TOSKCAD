use super::*;
use crate::curved_projection::Ellipse;
use std::f64::consts::TAU;

/// Exact circular drawing edge at a constant view depth.
#[derive(Clone, Copy, Debug)]
pub struct RimCandidate {
    pub source: usize,
    pub center: Point2,
    pub depth: f64,
    pub radius: f64,
    pub trim: Span,
}

/// Exact circular drawing edge with an orthographic ellipse and varying depth.
/// The ellipse major axis has constant depth, and `depth_sine` is the depth
/// component along its minor axis. An edge-on circle has a zero minor radius.
#[derive(Clone, Copy, Debug)]
pub struct SpatialRimCandidate {
    pub source: usize,
    pub ellipse: Ellipse,
    pub depth: f64,
    pub depth_sine: f64,
}

/// Exact drawing-space line with depth at both endpoints.
#[derive(Clone, Copy, Debug)]
pub struct LineCandidate {
    pub source: usize,
    pub points: [Point; 2],
}

/// A certified blind circular cavity aligned with the viewing direction.
#[derive(Clone, Copy, Debug)]
pub struct FaceOpening {
    /// Index in the drawing bodies followed by the blocker-only bodies.
    pub body: usize,
    /// Index of the outward planar face at the cavity mouth.
    pub face: usize,
    pub center: Point2,
    pub radius: f64,
    pub floor_depth: f64,
}

/// Extra geometry for occlusion or explicit edge publication.
#[derive(Default)]
pub struct SceneExtras<'a> {
    pub rims: &'a [RimCandidate],
    pub spatial_rims: &'a [SpatialRimCandidate],
    pub lines: &'a [LineCandidate],
    pub blocker_bodies: &'a [Body],
    pub blocker_rounds: &'a [RoundOccluder],
    pub openings: &'a [FaceOpening],
}

fn bounded(values: impl IntoIterator<Item = f64>) -> bool {
    values.into_iter().all(|v| v.is_finite() && v.abs() <= 1e8)
}

fn radius(value: f64) -> bool {
    (1e-7..=1e7).contains(&value)
}

impl LineCandidate {
    pub(super) fn spatial(self) -> Result<SpatialCurve, ProjectionError> {
        if !bounded(self.points.into_iter().flatten()) {
            return Err(ProjectionError::Invalid);
        }
        Ok(SpatialCurve::Line(self.points))
    }
}

impl RimCandidate {
    pub(super) fn spatial(self) -> Result<Rim, ProjectionError> {
        SpatialRimCandidate {
            source: self.source,
            ellipse: Ellipse {
                center: self.center,
                u: [1., 0.],
                v: [0., 1.],
                major: self.radius,
                minor: self.radius,
                trim: self.trim,
            },
            depth: self.depth,
            depth_sine: 0.,
        }
        .spatial()
    }
}

impl SpatialRimCandidate {
    pub(super) fn spatial(self) -> Result<Rim, ProjectionError> {
        let ellipse = self.ellipse;
        let [lo, hi] = ellipse.trim;
        if !bounded(
            ellipse
                .center
                .into_iter()
                .chain([self.depth, self.depth_sine]),
        ) || !radius(ellipse.major)
            || !(0. ..=ellipse.major).contains(&ellipse.minor)
            || !(0. ..hi).contains(&lo)
            || hi > TAU
            || (ellipse.minor.hypot(self.depth_sine) - ellipse.major).abs() > 1e-9 * ellipse.major
            || !(1. - 1e-9..=1. + 1e-9).contains(&dot(ellipse.u, ellipse.u))
            || !(1. - 1e-9..=1. + 1e-9).contains(&dot(ellipse.v, ellipse.v))
            || dot(ellipse.u, ellipse.v).abs() > 1e-9
        {
            return Err(ProjectionError::Invalid);
        }
        Ok(Rim {
            ellipse,
            depth: self.depth,
            depth_sine: self.depth_sine,
        })
    }
}

pub(super) fn apply_openings(
    bodies: &[Body],
    extras: &SceneExtras<'_>,
    planes: &mut Vec<TrimmedPlane>,
    work: &mut Work,
) -> Result<(), ProjectionError> {
    if extras.openings.is_empty() {
        return Ok(());
    }
    work.charge(extras.openings.len())?;
    let mut offset = 1;
    let stocks: Vec<_> = bodies
        .iter()
        .chain(extras.blocker_bodies)
        .map(|body| {
            let entry = (body, offset);
            offset += body.faces.len();
            entry
        })
        .collect();
    let mut floors = Vec::with_capacity(extras.openings.len());
    for opening in extras.openings {
        let &(stock, offset) = stocks.get(opening.body).ok_or(ProjectionError::Invalid)?;
        let face = stock
            .faces
            .get(opening.face)
            .ok_or(ProjectionError::Invalid)?;
        if face.normal[2] <= 0.
            || face.normal[0].hypot(face.normal[1]) > 32. * f64::EPSILON
            || !bounded(opening.center.into_iter().chain([opening.floor_depth]))
            || !radius(opening.radius)
            || opening.floor_depth >= face.point[2] - work.tolerance
        {
            return Err(ProjectionError::Invalid);
        }
        let id = offset + opening.face;
        let index = planes
            .binary_search_by_key(&id, |plane| plane.id)
            .map_err(|_| ProjectionError::Invalid)?;
        let plane = &mut planes[index];
        work.charge(plane.boundary.len() + plane.circles.len() + stock.edges.len())?;
        let stock_back = stock
            .edges
            .iter()
            .flatten()
            .map(|point| point[2])
            .fold(f64::INFINITY, f64::min);
        if opening.floor_depth <= stock_back + work.tolerance
            || !plane.contains(opening.center, work.tolerance)
            || plane.boundary.iter().any(|&edge| {
                segment_distance(opening.center, edge) <= opening.radius + work.tolerance
            })
            || plane.circles.iter().any(|circle| {
                let [x, y] = sub(circle.center, opening.center);
                x.hypot(y) <= circle.radius + opening.radius + work.tolerance
            })
        {
            return Err(ProjectionError::Invalid);
        }
        let circle = CircleTrim {
            center: opening.center,
            radius: opening.radius,
        };
        plane.circles.push(circle);
        let [u, v] = opening.center;
        let depth = opening.floor_depth;
        let r = opening.radius;
        floors.push(TrimmedPlane {
            id: 0,
            point: [u, v, depth],
            normal: [0., 0., 1.],
            boundary: Vec::new(),
            circles: vec![circle],
            bounds: Bounds {
                min: [u - r, v - r, depth],
                max: [u + r, v + r, depth],
            },
        });
    }
    planes.extend(floors);
    Ok(())
}

fn segment_distance(point: Point2, [a, b]: [Point2; 2]) -> f64 {
    let axis = sub(b, a);
    let offset = sub(point, a);
    let length = dot(axis, axis);
    let t = if length == 0. {
        0.
    } else {
        (dot(offset, axis) / length).clamp(0., 1.)
    };
    (-t).mul_add(axis[0], offset[0])
        .hypot((-t).mul_add(axis[1], offset[1]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clearance_to_a_degenerate_boundary_uses_its_point() {
        assert_eq!(segment_distance([2., 3.], [[1., 3.]; 2]), 1.);
    }
}
