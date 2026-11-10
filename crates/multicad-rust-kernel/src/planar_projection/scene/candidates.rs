use super::*;
use crate::curved_projection::Ellipse;
use crate::stadium::{Stadium, segments_distance};
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

/// A certified physical edge already hidden by the owning solid.
#[derive(Clone, Copy, Debug)]
pub enum HiddenCandidate {
    Line(LineCandidate),
    Rim(RimCandidate),
}

/// End condition of a certified circular cut.
#[derive(Clone, Copy, Debug)]
pub enum CircularOpeningEnd {
    /// Depth of the blind floor.
    Floor(f64),
    /// Opposing face ordinal of a through bore on the same stock.
    OppositeFace(usize),
}

/// A certified circular aperture aligned with the viewing direction.
#[derive(Clone, Copy, Debug)]
pub struct FaceOpening {
    /// Index in the drawing bodies followed by the blocker-only bodies.
    pub body: usize,
    /// Index of the outward planar face at the cavity mouth.
    pub face: usize,
    pub center: Point2,
    pub radius: f64,
    pub end: CircularOpeningEnd,
}

/// A certified blind rounded-slot cavity aligned with the viewing direction.
#[derive(Clone, Copy, Debug)]
pub struct StadiumOpening {
    /// Index in the drawing bodies followed by the blocker-only bodies.
    pub body: usize,
    /// Index of the outward planar face at the cavity mouth.
    pub face: usize,
    pub centers: [Point2; 2],
    pub radius: f64,
    pub floor_depth: f64,
}

/// Extra geometry for occlusion or explicit edge publication.
#[derive(Default)]
pub struct SceneExtras<'a> {
    pub rims: &'a [RimCandidate],
    pub spatial_rims: &'a [SpatialRimCandidate],
    pub lines: &'a [LineCandidate],
    pub intrinsic_hidden: &'a [HiddenCandidate],
    pub blocker_bodies: &'a [Body],
    pub blocker_rounds: &'a [RoundOccluder],
    pub openings: &'a [FaceOpening],
    pub stadium_openings: &'a [StadiumOpening],
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
    let count = extras.openings.len() + extras.stadium_openings.len();
    if count == 0 {
        return Ok(());
    }
    work.charge(count)?;
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
    let circles = extras.openings.iter().map(|opening| {
        Ok((
            opening.body,
            opening.face,
            opening.end,
            ProfileTrim::Circle {
                center: opening.center,
                radius: opening.radius,
            },
        ))
    });
    let stadiums = extras.stadium_openings.iter().map(|opening| {
        let profile =
            Stadium::new(opening.centers, opening.radius).map_err(|_| ProjectionError::Invalid)?;
        Ok((
            opening.body,
            opening.face,
            CircularOpeningEnd::Floor(opening.floor_depth),
            ProfileTrim::Stadium(profile),
        ))
    });
    let mut floors = Vec::with_capacity(count);
    for opening in circles.chain(stadiums) {
        let (body, face_index, end, profile) = opening?;
        let centers = profile.centers();
        let r = profile.radius();
        let &(stock, offset) = stocks.get(body).ok_or(ProjectionError::Invalid)?;
        let face = stock
            .faces
            .get(face_index)
            .ok_or(ProjectionError::Invalid)?;
        if face.normal[2] <= 0.
            || face.normal[0].hypot(face.normal[1]) > 32. * f64::EPSILON
            || !bounded(centers.into_iter().flatten())
            || !radius(r)
        {
            return Err(ProjectionError::Invalid);
        }
        trim_plane(planes, offset + face_index, profile, work)?;
        let near = face.point[2] - work.tolerance;
        match end {
            CircularOpeningEnd::Floor(depth) => {
                work.charge(stock.edges.len())?;
                let stock_back = stock
                    .edges
                    .iter()
                    .flatten()
                    .map(|point| point[2])
                    .fold(f64::INFINITY, f64::min);
                if !bounded([depth]) || depth >= near || depth <= stock_back + work.tolerance {
                    return Err(ProjectionError::Invalid);
                }
                let [u, v] = centers[0];
                floors.push(TrimmedPlane {
                    id: 0,
                    point: [u, v, depth],
                    normal: [0., 0., 1.],
                    boundary: Vec::new(),
                    profiles: vec![profile],
                    bounds: profile.bounds(depth),
                });
            }
            CircularOpeningEnd::OppositeFace(rear) => {
                let face = stock.faces.get(rear).ok_or(ProjectionError::Invalid)?;
                if face.normal[2] >= 0.
                    || face.normal[0].hypot(face.normal[1]) > 32. * f64::EPSILON
                    || face.point[2] >= near
                {
                    return Err(ProjectionError::Invalid);
                }
                trim_plane(planes, offset + rear, profile, work)?;
            }
        }
    }
    planes.extend(floors);
    Ok(())
}

fn trim_plane(
    planes: &mut [TrimmedPlane],
    id: usize,
    profile: ProfileTrim,
    work: &mut Work,
) -> Result<(), ProjectionError> {
    let index = planes
        .binary_search_by_key(&id, |plane| plane.id)
        .map_err(|_| ProjectionError::Invalid)?;
    let plane = &mut planes[index];
    work.charge(plane.boundary.len() + plane.profiles.len())?;
    let centers = profile.centers();
    let radius = profile.radius();
    if !plane.contains(centers[0], work.tolerance)
        || plane
            .boundary
            .iter()
            .any(|&edge| segments_distance(centers, edge) <= radius + work.tolerance)
        || plane.profiles.iter().any(|&other| {
            segments_distance(centers, other.centers()) <= other.radius() + radius + work.tolerance
        })
    {
        return Err(ProjectionError::Invalid);
    }
    plane.profiles.push(profile);
    Ok(())
}
