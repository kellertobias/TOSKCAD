//! Recover physical plane and cylinder patches from a certified warped union.
use crate::{
    planar_boolean::Union,
    trim_curve::{Ellipse, EllipseArc},
};
use std::{collections::BTreeMap, f64::consts::FRAC_PI_2};

mod axial_cut;
mod boss;
mod mesh;
mod metrics;
pub use axial_cut::{AxialCut, ConvexRecess, OpenBoreSide};
pub use boss::Boss;

type Point = [f64; 3];

const FRAME_EPS: f64 = 1e-10;
const MAX_METRIC_WORK: usize = 24_000_000;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Band {
    pub center_z_mm: f64,
    pub radius_mm: f64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    Invalid,
    Unsupported,
    Limit,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Curve {
    Line,
    Ellipse(EllipseArc),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Edge {
    pub vertices: [usize; 2],
    pub curve: Curve,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OrientedEdge {
    pub edge: usize,
    pub forward: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Support {
    Plane {
        normal: Point,
    },
    Cylinder {
        axis_origin_mm: Point,
        axis_direction: Point,
        radius_mm: f64,
        outward: bool,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Face {
    pub boundary: Vec<OrientedEdge>,
    /// Clockwise inner loops of a planar face.
    pub holes: Vec<Vec<OrientedEdge>>,
    pub support: Support,
    pub point_mm: Point,
    pub area_mm2: f64,
    pub source: usize,
    pub source_face: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Shell {
    vertices: Vec<Point>,
    edges: Vec<Edge>,
    faces: Vec<Face>,
    bounds_mm: [Point; 2],
    volume_mm3: f64,
    solid_count: usize,
    shell_count: usize,
    band: Option<Band>,
    chart_w: Vec<f64>,
    chart_edges: usize,
}

impl Shell {
    pub fn vertices(&self) -> &[Point] {
        &self.vertices
    }

    pub fn edges(&self) -> &[Edge] {
        &self.edges
    }

    pub fn faces(&self) -> &[Face] {
        &self.faces
    }

    pub fn volume_mm3(&self) -> f64 {
        self.volume_mm3
    }

    pub fn bounds_mm(&self) -> [Point; 2] {
        self.bounds_mm
    }

    pub fn solid_count(&self) -> usize {
        self.solid_count
    }

    pub fn shell_count(&self) -> usize {
        self.shell_count
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Region {
    Below,
    Round,
    Above,
}

impl Band {
    fn region(self, lo: f64, hi: f64) -> Result<Region, Error> {
        if hi <= 0. {
            Ok(Region::Below)
        } else if lo >= self.radius_mm {
            Ok(Region::Above)
        } else if lo >= 0. && hi <= self.radius_mm {
            Ok(Region::Round)
        } else {
            Err(Error::Unsupported)
        }
    }

    fn radial_drop(self, w: f64) -> f64 {
        let r = self.radius_mm;
        ((r - w) * (r + w)).sqrt()
    }

    fn theta(self, w: f64) -> f64 {
        (-self.radial_drop(w)).atan2(w)
    }

    fn point(self, [x, y, w]: Point) -> Point {
        let c = self.center_z_mm;
        let r = self.radius_mm;
        let z = if w <= 0. {
            (c - r) + w
        } else if w >= r {
            c + (w - r)
        } else {
            c - self.radial_drop(w)
        };
        [x, y, z]
    }
}

/// Map the middle band using w = sqrt(r² − (z − c)²). Below and above it
/// the mapping is a translation. The input must be split at the two seams.
pub fn remap(union: &Union, band: Band, tolerance_mm: f64) -> Result<Shell, Error> {
    boundary(union, Some(band), tolerance_mm)
}

/// Reuse a certified planar Boolean boundary without altering its coordinates.
pub fn from_planar_union(union: &Union, tolerance_mm: f64) -> Result<Shell, Error> {
    boundary(union, None, tolerance_mm)
}

fn boundary(union: &Union, band: Option<Band>, tolerance_mm: f64) -> Result<Shell, Error> {
    if !(1e-10..=1e-3).contains(&tolerance_mm)
        || band.is_some_and(|band| {
            !band.center_z_mm.is_finite()
                || !band.radius_mm.is_finite()
                || band.radius_mm <= tolerance_mm
        })
    {
        return Err(Error::Invalid);
    }
    if band.is_some_and(|band| band.center_z_mm.abs().max(band.radius_mm) > 1e8) {
        return Err(Error::Limit);
    }
    let model_point = |p| band.map_or(p, |band| band.point(p));
    let vertices: Vec<_> = union.vertices().iter().copied().map(model_point).collect();
    let mut faces = Vec::with_capacity(union.faces().len());
    let mut edges = Vec::new();
    let mut edge_indices = BTreeMap::new();
    let mut volume = metrics::Sum::default();
    let mut work = 0;
    let center = union.bounds();
    let anchor = [
        center[0][0].midpoint(center[1][0]),
        center[0][1].midpoint(center[1][1]),
    ];
    for source in union.faces() {
        let ring: Vec<_> = source
            .vertices
            .iter()
            .map(|&i| union.vertices()[i])
            .collect();
        work = metric_work(work, ring.len())?;
        let normal = source.normal;
        let (support, area_mm2) = if let Some(band) = band {
            let [lo, hi] = axial_extent(&ring);
            let region = band.region(lo, hi)?;
            let support = support(band, region, &ring, normal, tolerance_mm)?;
            let metrics = metrics::face_metrics(band, region, support, normal, &ring, anchor)?;
            volume.add(metrics.volume_mm3);
            (support, metrics.area_mm2)
        } else {
            (
                Support::Plane { normal },
                metrics::polygon_area(&ring, normal),
            )
        };
        let point_mm = model_point(std::array::from_fn(|i| {
            ring[0][i] + ring.iter().map(|p| p[i] - ring[0][i]).sum::<f64>() / ring.len() as f64
        }));
        let mut boundary = Vec::with_capacity(ring.len());
        for (&a, &b) in source
            .vertices
            .iter()
            .zip(source.vertices.iter().cycle().skip(1))
        {
            let [lo, hi] = [a.min(b), a.max(b)];
            let index = if let Some(&index) = edge_indices.get(&[lo, hi]) {
                index
            } else {
                let model = [lo, hi].map(|i| union.vertices()[i]);
                let curve = match band {
                    Some(band) => edge_curve(band, model, tolerance_mm)?,
                    None => Curve::Line,
                };
                let index = edges.len();
                edges.push(Edge {
                    vertices: [lo, hi],
                    curve,
                });
                edge_indices.insert([lo, hi], index);
                index
            };
            boundary.push(OrientedEdge {
                edge: index,
                forward: a == lo,
            });
        }
        faces.push(Face {
            boundary,
            holes: Vec::new(),
            support,
            point_mm,
            area_mm2,
            source: source.source,
            source_face: source.source_face,
        });
    }
    let volume_mm3 = if band.is_some() {
        positive_volume(volume.value())?
    } else {
        union.volume_mm3()
    };
    // X and Y are unchanged and the axial mapping is monotone, so no
    // interior point can extend these bounds beyond the chart vertices.
    let bounds_mm = center.map(model_point);
    Ok(Shell {
        vertices,
        chart_edges: if band.is_some() { edges.len() } else { 0 },
        edges,
        faces,
        bounds_mm,
        volume_mm3,
        solid_count: union.solid_count(),
        shell_count: union.shell_count(),
        band,
        chart_w: band.map_or_else(Vec::new, |_| {
            union.vertices().iter().map(|p| p[2]).collect()
        }),
    })
}

fn metric_work(work: usize, vertices: usize) -> Result<usize, Error> {
    let next = work + vertices * vertices;
    if next > MAX_METRIC_WORK {
        Err(Error::Limit)
    } else {
        Ok(next)
    }
}

fn positive_volume(volume: f64) -> Result<f64, Error> {
    if !volume.is_finite() || volume <= 0. {
        Err(Error::Invalid)
    } else {
        Ok(volume)
    }
}

fn axial_extent(ring: &[Point]) -> [f64; 2] {
    ring.iter()
        .fold([f64::INFINITY, f64::NEG_INFINITY], |[lo, hi], p| {
            [lo.min(p[2]), hi.max(p[2])]
        })
}

fn support(
    band: Band,
    region: Region,
    ring: &[Point],
    normal: Point,
    tolerance: f64,
) -> Result<Support, Error> {
    if region != Region::Round {
        return Ok(Support::Plane { normal });
    }
    let [nx, ny, nz] = normal;
    let planar_length = nx.hypot(ny);
    if planar_length <= FRAME_EPS {
        let [lo, hi] = axial_extent(ring);
        if (band.point([0., 0., hi])[2] - band.point([0., 0., lo])[2]).abs() > tolerance {
            return Err(Error::Unsupported);
        }
        return Ok(Support::Plane {
            normal: [0., 0., nz.signum()],
        });
    }
    if nz.abs() <= FRAME_EPS {
        let normal = [nx / planar_length, ny / planar_length, 0.];
        if ring
            .iter()
            .any(|&p| dot(sub(p, ring[0]), normal).abs() > tolerance)
        {
            return Err(Error::Unsupported);
        }
        return Ok(Support::Plane { normal });
    }
    let xy = usize::from(ny.abs() > nx.abs());
    if normal[1 - xy].abs() > FRAME_EPS || (normal[xy].abs() - nz.abs()).abs() > FRAME_EPS {
        return Err(Error::Unsupported);
    }
    let drift = (-nz * normal[xy]).signum();
    let center = ring[0][xy] - drift * ring[0][2];
    if ring
        .iter()
        .any(|p| (p[xy] - drift * p[2] - center).abs() > tolerance)
    {
        return Err(Error::Unsupported);
    }
    let mut axis_origin_mm = [0., 0., band.center_z_mm];
    axis_origin_mm[xy] = center;
    let mut axis_direction = [0.; 3];
    axis_direction[1 - xy] = 1.;
    Ok(Support::Cylinder {
        axis_origin_mm,
        axis_direction,
        radius_mm: band.radius_mm,
        outward: normal[xy] * drift > 0.,
    })
}

fn edge_curve(band: Band, points: [Point; 2], tolerance: f64) -> Result<Curve, Error> {
    let [a, b] = points;
    let [dx, dy, dw] = sub(b, a);
    if dw == 0. || band.region(a[2].min(b[2]), a[2].max(b[2]))? != Region::Round {
        return Ok(Curve::Line);
    }
    let length = dx.hypot(dy);
    let slack = 64. * f64::EPSILON * a[0].abs().max(a[1].abs()).max(1.);
    if length <= slack {
        return Ok(Curve::Line);
    }
    let radial = [dx / dw, dy / dw, 0.];
    let horizontal = band.radius_mm * radial[0].hypot(radial[1]);
    let center = [
        a[0] - radial[0] * a[2],
        a[1] - radial[1] * a[2],
        band.center_z_mm,
    ];
    let up = [0., 0., 1.];
    let normal = cross(radial, up);
    let radius = band.radius_mm;
    let (reference, semiaxes, shift) = if horizontal >= radius {
        (radial, [horizontal, radius], 0.)
    } else {
        (up, [radius, horizontal], -FRAC_PI_2)
    };
    let ellipse = Ellipse::new(center, normal, reference, semiaxes).map_err(|_| Error::Limit)?;
    let [start, end] = points.map(|p| band.theta(p[2]));
    let arc = EllipseArc::new(ellipse, start + shift, end - start).map_err(|_| Error::Invalid)?;
    if arc
        .endpoints()
        .into_iter()
        .zip(points)
        .any(|(a, b)| length3(sub(a, band.point(b))) > tolerance)
    {
        return Err(Error::Unsupported);
    }
    Ok(Curve::Ellipse(arc))
}

fn sub(a: Point, b: Point) -> Point {
    std::array::from_fn(|i| a[i] - b[i])
}

fn dot([a, b, c]: Point, [x, y, z]: Point) -> f64 {
    a * x + b * y + c * z
}

fn cross([a, b, c]: Point, [x, y, z]: Point) -> Point {
    [b * z - c * y, c * x - a * z, a * y - b * x]
}

fn length3([x, y, z]: Point) -> f64 {
    x.hypot(y).hypot(z)
}

#[cfg(test)]
mod tests;
