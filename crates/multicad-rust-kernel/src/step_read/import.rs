//! Recognize swept solids by their complete, already validated boundaries.
use super::*;
use crate::{
    chamfered_bores::BoreChamfer,
    circular_pockets::CircularPocket,
    linear_sweep::Sweep,
    normal_sweep,
    orthogonal::OrthogonalSolid,
    orthogonal_pockets::{OrthogonalPocket, OrthogonalThroughBore},
    orthogonal_slots::OrthogonalSlot,
    stadium::Stadium,
    trimmed_sweep::TrimmedSweep,
};
use std::cmp::Ordering;

mod axial_profile_stack;
mod chamfered;
use chamfered::{Bore, canonical_bores};
mod cylindrical_revolution;
#[path = "helical_cut.rs"]
mod helical_cut;
pub(super) use helical_cut::read_member as read_helical_cut_member;
mod orthogonal;
mod pierced;
mod planar_boundary;
mod pocketed;
use pocketed::{PocketRim, canonical_pockets};

#[derive(Clone, Debug, PartialEq)]
pub enum ImportProfile {
    NormalRelief {
        relief: crate::normal_relief::Relief,
    },
    /// Independently inferred and completely verified analytic thread with every start retained.
    HelicalThread {
        spec: crate::helical_thread::Spec,
        stock_radius_mm: Option<f64>,
    },
    /// Independently verified complete thread and planar screw head.
    HelicalScrew {
        spec: crate::helical_thread::Spec,
        screw: crate::headed_screw::ScrewSpec,
    },
    /// Full hexagonal exterior and independently certified internal thread.
    HelicalReceiver {
        spec: crate::helical_thread::Spec,
        stock_radius_mm: f64,
        stock_length_mm: f64,
    },
    /// Exact externally threaded prefix followed by untouched cylindrical stock.
    ThreadedShaft {
        spec: crate::helical_thread::Spec,
        stock_radius_mm: f64,
        stock_length_mm: f64,
    },
    HelicalNut {
        spec: crate::helical_thread::Spec,
        across_flats_mm: f64,
    },
    Polygon {
        /// Exterior counterclockwise, followed by clockwise straight holes.
        rings: Vec<Vec<[f64; 2]>>,
        circular_holes: Vec<([f64; 2], f64)>,
    },
    PocketedPolygon {
        /// Exterior counterclockwise, followed by clockwise straight holes.
        rings: Vec<Vec<[f64; 2]>>,
        circular_holes: Vec<([f64; 2], f64)>,
        pockets: Vec<CircularPocket>,
    },
    ChamferedPolygon {
        /// Exterior counterclockwise, followed by clockwise straight holes.
        rings: Vec<Vec<[f64; 2]>>,
        circular_holes: Vec<([f64; 2], f64)>,
        chamfers: Vec<BoreChamfer>,
    },
    LinearSweep {
        /// Counterclockwise exterior and clockwise holes, paired with `top`.
        base: Vec<Vec<[f64; 2]>>,
        /// End sections in the base frame, including their in-plane offset.
        top: Vec<Vec<[f64; 2]>>,
    },
    TrimmedSweep {
        /// Exterior counterclockwise, followed by clockwise straight holes.
        rings: Vec<Vec<[f64; 2]>>,
        /// The common unit generator direction in the lower cap frame.
        direction: Point,
        /// Affine travel distances from the lower cap. The first row is zero.
        travels: [[f64; 3]; 2],
    },
    NormalSweep {
        sweep: normal_sweep::Sweep,
    },
    SplineSweep {
        sweep: crate::spline_sweep::Sweep,
    },
    NonplanarLoft {
        geometry: crate::planar_boolean::NonplanarLoft,
    },
    PiercedNormalSweep {
        sweep: normal_sweep::PiercedSweep,
    },
    Orthogonal {
        /// Certified material and directed boundary in the local frame.
        solid: OrthogonalSolid,
    },
    PocketedOrthogonal {
        /// Certified stock and circular recesses in the local frame.
        solid: OrthogonalSolid,
        pockets: Vec<OrthogonalPocket>,
    },
    DrilledOrthogonal {
        solid: OrthogonalSolid,
        pockets: Vec<OrthogonalPocket>,
        through: Vec<OrthogonalThroughBore>,
    },
    SlottedOrthogonal {
        solid: OrthogonalSolid,
        pockets: Vec<OrthogonalPocket>,
        slots: Vec<OrthogonalSlot>,
    },
    Stadium {
        profile: Stadium,
    },
    Sphere {
        radius_mm: f64,
    },
    HelicalTubeCut {
        stock_radius_mm: f64,
        radius_mm: f64,
        pitch_mm: f64,
        path_height_mm: f64,
        start_height_mm: f64,
        start_angle_rad: f64,
    },
    Torus {
        major_radius_mm: f64,
        minor_radius_mm: f64,
    },
    Cylinder {
        radius_mm: f64,
    },
    RoundedCylinder {
        radius_mm: f64,
        fillet_radii_mm: [Option<f64>; 2],
    },
    Annulus {
        inner_radius_mm: f64,
        outer_radius_mm: f64,
    },
    PlanarBoundary {
        /// Certified planar shell in the STEP world frame.
        shell: crate::planar_boolean::Union,
    },
    WarpedUnion {
        shell: crate::warped_union::Shell,
    },
    AxialProfileStack {
        stack: crate::axial_profile_stack::ProfileStack,
    },
    CylindricalRevolution {
        /// Counterclockwise rectilinear boundary in [radius, axial] coordinates.
        meridian: Vec<[f64; 2]>,
    },
    ConicalRevolution {
        /// Counterclockwise disk-to-disk meridian in [radius, axial] coordinates.
        meridian: Vec<[f64; 2]>,
    },
}

#[cfg(test)]
mod tests;

#[derive(Clone, Debug, PartialEq)]
pub struct ImportSolid {
    pub origin_mm: [f64; 3],
    pub u: [f64; 3],
    pub v: [f64; 3],
    /// Height at the lower cap origin for swept solids.
    pub height_mm: f64,
    pub profile: ImportProfile,
}

/// Decode a single analytic extrusion or rectilinear solid from a verified
/// STEP shell. Orthogonal solids can contain blind and through pockets.
pub fn read_import_solid(input: &str, tolerance_mm: f64) -> Result<(Readback, ImportSolid)> {
    compound::read_import_members(input, tolerance_mm, true)?
        .pop()
        .ok_or(ReadError::Invalid)
}

/// Decode the independent analytic members of one flat advanced BREP representation.
pub fn read_import_compound(
    input: &str,
    tolerance_mm: f64,
) -> Result<Vec<(Readback, ImportSolid)>> {
    compound::read_import_members(input, tolerance_mm, false)
}

pub(super) fn recognize_spline(
    readback: Readback,
    body: spline_sweep::DecodedSplineSweep,
) -> (Readback, ImportSolid) {
    let spline_sweep::DecodedSplineSweep {
        origin_mm,
        u,
        v,
        sweep,
    } = body;
    (
        readback,
        ImportSolid {
            origin_mm,
            u,
            v,
            height_mm: sweep.height_mm(),
            profile: ImportProfile::SplineSweep { sweep },
        },
    )
}

pub(super) fn recognize_warped(
    readback: Readback,
    imported: ImportedWarped,
) -> (Readback, ImportSolid) {
    let ImportedWarped {
        shell,
        origin_mm,
        u,
        v,
    } = imported;
    let [lo, hi] = shell.bounds_mm();
    (
        readback,
        ImportSolid {
            origin_mm,
            u,
            v,
            height_mm: hi[2] - lo[2],
            profile: ImportProfile::WarpedUnion { shell },
        },
    )
}

pub(super) fn recognize_nonplanar_loft(
    readback: Readback,
    imported: ImportedNonplanarLoft,
) -> (Readback, ImportSolid) {
    let ImportedNonplanarLoft {
        geometry,
        origin_mm,
        u,
        v,
    } = imported;
    (
        readback,
        ImportSolid {
            origin_mm,
            u,
            v,
            height_mm: geometry.rings()[1][0][2],
            profile: ImportProfile::NonplanarLoft { geometry },
        },
    )
}

pub(super) fn recognize_member(
    shell: &Shell<'_, '_>,
    faces: &[Face],
) -> Result<(Readback, ImportSolid)> {
    let readback = shell.readback(faces)?;
    if faces.iter().any(|face| matches!(face, Face::Conical(_))) {
        let solid = shell
            .chamfered_polygon(faces)
            .or_else(|| shell.conical_revolution(faces, &readback))
            .ok_or(ReadError::Unsupported)?;
        return Ok((readback, solid));
    }
    let solid = match shell
        .recognize(faces)
        .or_else(|| shell.pocketed_polygon(faces))
        .or_else(|| shell.pierced_normal_sweep(faces))
        .or_else(|| shell.cylindrical_revolution(faces, &readback))
        .or_else(|| shell.axial_profile_stack(faces, &readback))
    {
        Some(solid) => solid,
        None => match shell.orthogonal(faces) {
            Err(ReadError::Unsupported) => shell.planar_boundary(faces)?,
            other => other?,
        },
    };
    Ok((readback, solid))
}

fn compare<const N: usize>(a: [f64; N], b: [f64; N]) -> Ordering {
    a.into_iter()
        .zip(b)
        .map(|(a, b)| a.total_cmp(&b))
        .find(|order| !order.is_eq())
        .unwrap_or(Ordering::Equal)
}

fn zero(value: f64) -> f64 {
    if value == 0. { 0. } else { value }
}

fn preferred_axis(normal: Point) -> bool {
    let major = (0..3)
        .max_by(|&a, &b| normal[a].abs().total_cmp(&normal[b].abs()))
        .expect("3D direction");
    normal[major] > 0.
}

pub(super) fn reference_axis(normal: Point) -> Point {
    let reference = (0..3)
        .min_by(|&a, &b| normal[a].abs().total_cmp(&normal[b].abs()))
        .expect("3D direction");
    std::array::from_fn(|i| f64::from(i == reference))
}

pub(super) fn frame_axes(normal: Point, reference: Point) -> [Point; 2] {
    let side = cross(normal, reference);
    let width = length(side);
    let v = side.map(|x| x / width);
    let u = cross(v, normal);
    [u.map(zero), v.map(zero)]
}

struct Wall {
    face: usize,
    base: u32,
    top: u32,
}

enum PlanarMatch<'a> {
    Straight(f64, &'a [PocketRim]),
    Chamfered(f64, &'a [Bore]),
    Ruled(f64, &'a BTreeMap<u32, u32>),
    Trimmed {
        direction: Point,
        top_origin: Point,
        top_normal: Point,
    },
}

impl Shell<'_, '_> {
    fn indexed_edges(&self, face_count: usize) -> Vec<Vec<u32>> {
        let mut face_edges = vec![Vec::new(); face_count];
        for (&id, edge) in &self.edges {
            for use_ in &edge.uses {
                let edges = &mut face_edges[use_.face];
                if edges.last() != Some(&id) {
                    edges.push(id);
                }
            }
        }
        face_edges
    }

    fn recognize(&self, faces: &[Face]) -> Option<ImportSolid> {
        let face_edges = self.indexed_edges(faces.len());
        let mut caps: Vec<_> = faces
            .iter()
            .enumerate()
            .filter_map(|(face, support)| {
                let Face::Planar { origin, normal, .. } = support else {
                    return None;
                };
                let direction = normal.map(|v| zero(-v));
                (face_edges[face].len() == faces.len() - 2).then_some((face, direction, *origin))
            })
            .collect();
        // A box may have several valid extrusion axes. Prefer Z, then Y,
        // then X so independent record order gives the same reconstructed solid.
        caps.sort_unstable_by(|a, b| {
            let preference =
                |direction: Point| [direction[2].abs(), direction[1].abs(), direction[0].abs()];
            compare(preference(b.1), preference(a.1)).then(compare(b.1, a.1))
        });
        let preferred = || {
            caps.iter()
                .filter(|(_, direction, _)| preferred_axis(*direction))
        };
        preferred()
            .find_map(|&(lower, direction, origin)| {
                self.prism(faces, &face_edges, lower, direction, origin)
            })
            .or_else(|| {
                preferred().find_map(|&(lower, direction, origin)| {
                    self.linear_sweep(faces, &face_edges, lower, direction, origin)
                })
            })
            .or_else(|| {
                caps.iter().find_map(|&(lower, direction, _)| {
                    self.trimmed_sweep(faces, &face_edges, lower, direction)
                })
            })
    }

    fn paired_walls(&self, face_edges: &[Vec<u32>], lower: usize) -> Option<(usize, Vec<Wall>)> {
        let mut walls = BTreeMap::new();
        for &edge in &face_edges[lower] {
            let wall = self.edges[&edge]
                .uses
                .iter()
                .find(|use_| use_.face != lower)?
                .face;
            if walls.insert(wall, edge).is_some() {
                return None;
            }
        }
        let upper =
            (0..face_edges.len()).find(|face| *face != lower && !walls.contains_key(face))?;
        if face_edges[upper].len() != walls.len() {
            return None;
        }
        let mut paired = Vec::with_capacity(walls.len());
        for (wall, bottom_edge) in walls {
            let mut upper_edges = face_edges[wall]
                .iter()
                .filter(|id| face_edges[upper].binary_search(id).is_ok());
            let &top_edge = upper_edges.next()?;
            if upper_edges.next().is_some() {
                return None;
            }
            paired.push(Wall {
                face: wall,
                base: bottom_edge,
                top: top_edge,
            });
        }
        Some((upper, paired))
    }

    fn cap_walls(
        &self,
        faces: &[Face],
        face_edges: &[Vec<u32>],
        lower: usize,
        direction: Point,
        bottom: Point,
    ) -> Option<(f64, Vec<Wall>)> {
        let (upper, walls) = self.paired_walls(face_edges, lower)?;
        let Face::Planar {
            origin: top,
            normal,
            ..
        } = faces[upper]
        else {
            return None;
        };
        let height_mm = dot(sub(top, bottom), direction);
        (length(sub(direction, normal)) <= 1e-10 && height_mm > self.tolerance)
            .then_some((height_mm, walls))
    }

    fn prism(
        &self,
        faces: &[Face],
        face_edges: &[Vec<u32>],
        lower: usize,
        direction: Point,
        bottom: Point,
    ) -> Option<ImportSolid> {
        let (height_mm, walls) = self.cap_walls(faces, face_edges, lower, direction, bottom)?;
        let sweep = direction.map(|v| v * height_mm);
        for Wall { face, base, top } in walls {
            self.wall_matches(&faces[face], &face_edges[face], base, top, sweep)?;
        }
        self.profile(&face_edges[lower], lower, direction, height_mm, None)
    }

    fn linear_sweep(
        &self,
        faces: &[Face],
        face_edges: &[Vec<u32>],
        lower: usize,
        direction: Point,
        bottom: Point,
    ) -> Option<ImportSolid> {
        let (height_mm, walls) = self.cap_walls(faces, face_edges, lower, direction, bottom)?;
        let mut correspondence = BTreeMap::new();
        for Wall { face, base, top } in walls {
            for [a, b] in self.wall_generators(&faces[face], &face_edges[face], base, top)? {
                // Each generator belongs to both walls incident to its base
                // vertex, so both walls prove the same correspondence.
                correspondence.insert(a, b);
            }
        }
        self.profile(
            &face_edges[lower],
            lower,
            direction,
            height_mm,
            Some(&correspondence),
        )
    }

    fn trimmed_sweep(
        &self,
        faces: &[Face],
        face_edges: &[Vec<u32>],
        lower: usize,
        inward: Point,
    ) -> Option<ImportSolid> {
        let (upper, walls) = self.paired_walls(face_edges, lower)?;
        let Face::Planar {
            origin: top_origin,
            normal: top_normal,
            ..
        } = faces[upper]
        else {
            return None;
        };
        let mut rails = Vec::with_capacity(walls.len() * 2);
        for Wall { face, base, top } in walls {
            rails.extend(
                self.wall_generators(&faces[face], &face_edges[face], base, top)?
                    .map(|[a, b]| [self.points[&a], self.points[&b]]),
            );
        }
        let longest = rails
            .iter()
            .map(|[a, b]| sub(*b, *a))
            .max_by(|&a, &b| length(a).total_cmp(&length(b)).then(compare(a, b)))?;
        let direction = unit(longest).ok()?;
        let denominator = dot(direction, top_normal);
        if dot(direction, inward) < 1e-10 || denominator < 1e-10 {
            return None;
        }
        // The shared shell is already validated. Compare each rail to the
        // candidate ray's actual intersection with the opposing cap plane.
        // This bounds any mismatch even when the cap is nearly parallel.
        for [a, b] in rails {
            let travel = dot(sub(top_origin, a), top_normal) / denominator;
            if travel <= self.tolerance
                || length(sub(sub(b, a), direction.map(|value| value * travel))) > self.tolerance
            {
                return None;
            }
        }
        self.planar_profile(
            &face_edges[lower],
            lower,
            inward,
            PlanarMatch::Trimmed {
                direction,
                top_origin,
                top_normal,
            },
        )
    }

    fn wall_generators(
        &self,
        face: &Face,
        edges: &[u32],
        bottom: u32,
        top: u32,
    ) -> Option<[[u32; 2]; 2]> {
        let base = &self.edges[&bottom];
        let end = &self.edges[&top];
        if !matches!(
            (face, base.curve, end.curve),
            (Face::Planar { .. }, Curve::Line, Curve::Line)
                | (
                    Face::HalfCylinder(_),
                    Curve::Semicircle(_),
                    Curve::Semicircle(_)
                )
                | (Face::PartialCylinder(_), Curve::Arc(_), Curve::Arc(_))
        ) || edges.len() != 4
        {
            return None;
        }
        let mut bridges = edges.iter().filter(|&&id| id != bottom && id != top);
        let pairs = [bridges.next()?, bridges.next()?].map(|bridge| {
            let [a, b] = self.edges[bridge].vertices;
            if base.vertices.contains(&a) {
                [a, b]
            } else {
                [b, a]
            }
        });
        let [[a, b], [c, d]] = pairs;
        (base.vertices.contains(&a)
            && end.vertices.contains(&b)
            && base.vertices.contains(&c)
            && end.vertices.contains(&d)
            && a != c
            && b != d)
            .then_some(pairs)
    }

    fn wall_matches(
        &self,
        face: &Face,
        edges: &[u32],
        bottom: u32,
        top: u32,
        sweep: Point,
    ) -> Option<()> {
        let base = &self.edges[&bottom];
        let end = &self.edges[&top];
        let displaced = |a: Point, b: Point| length(sub(sub(b, a), sweep)) <= self.tolerance;
        match (face, base.curve, end.curve) {
            (Face::Planar { .. }, Curve::Line, Curve::Line)
            | (Face::HalfCylinder(_), Curve::Semicircle(_), Curve::Semicircle(_))
            | (Face::PartialCylinder(_), Curve::Arc(_), Curve::Arc(_)) => {
                // Each of these walls has two distinct straight generators.
                // The shell has already validated a half-cylinder's matching rims.
                for [a, b] in self.wall_generators(face, edges, bottom, top)? {
                    if !displaced(self.points[&a], self.points[&b]) {
                        return None;
                    }
                }
            }
            (Face::Cylindrical { .. }, Curve::Circle(a), Curve::Circle(b)) => {
                // The shell reader already checks the shared cylindrical seam,
                // radius, coaxiality and orientations on the whole periodic wall.
                if !displaced(a.center_mm, b.center_mm)
                    || (a.radius_mm - b.radius_mm).abs() > self.tolerance
                {
                    return None;
                }
            }
            _ => return None,
        }
        Some(())
    }

    fn profile(
        &self,
        edges: &[u32],
        lower: usize,
        direction: Point,
        height_mm: f64,
        correspondence: Option<&BTreeMap<u32, u32>>,
    ) -> Option<ImportSolid> {
        if correspondence.is_none()
            && let Some([rim]) = self.stadium_loops.get(&lower).map(Vec::as_slice)
            && rim.ccw
            && edges.len() == 4
        {
            let mut arcs = rim
                .edges
                .iter()
                .filter_map(|id| match self.edges[id].curve {
                    Curve::Semicircle(arc) => Some(arc),
                    _ => None,
                });
            let [a, b] = [arcs.next()?, arcs.next()?];
            let (a, b) = if compare(a.center_mm, b.center_mm).is_gt() {
                (b, a)
            } else {
                (a, b)
            };
            let delta = sub(b.center_mm, a.center_mm);
            let [u, v] = frame_axes(direction, delta);
            let profile = Stadium::new(
                [[0.; 2], [dot(delta, u), 0.]],
                a.radius_mm.midpoint(b.radius_mm),
            )
            .ok()?;
            return Some(ImportSolid {
                origin_mm: a.center_mm.map(zero),
                u,
                v,
                height_mm,
                profile: ImportProfile::Stadium { profile },
            });
        }
        if let [id] = edges
            && let Curve::Circle(circle) = self.edges[id].curve
        {
            let [u, v] = frame_axes(direction, reference_axis(direction));
            return Some(ImportSolid {
                origin_mm: circle.center_mm.map(zero),
                u,
                v,
                height_mm,
                profile: ImportProfile::Cylinder {
                    radius_mm: circle.radius_mm,
                },
            });
        }
        if let [a, b] = edges
            && let [Curve::Circle(a), Curve::Circle(b)] = [a, b].map(|id| self.edges[id].curve)
        {
            // The shell validator has proved cap nesting and opposite
            // winding, and prism recognition has matched both full walls.
            // An eccentric hole would require a different canonical solid.
            if length(sub(a.center_mm, b.center_mm)) > self.tolerance {
                return None;
            }
            let [inner, outer] = if a.radius_mm < b.radius_mm {
                [a, b]
            } else {
                [b, a]
            };
            let [u, v] = frame_axes(direction, reference_axis(direction));
            return Some(ImportSolid {
                origin_mm: outer.center_mm.map(zero),
                u,
                v,
                height_mm,
                profile: ImportProfile::Annulus {
                    inner_radius_mm: inner.radius_mm,
                    outer_radius_mm: outer.radius_mm,
                },
            });
        }
        if correspondence.is_none()
            && edges
                .iter()
                .any(|id| matches!(self.edges[id].curve, Curve::Arc(_) | Curve::Semicircle(_)))
        {
            return self.normal_sweep_profile(edges, lower, direction, height_mm);
        }
        let kind = correspondence.map_or(PlanarMatch::Straight(height_mm, &[]), |correspondence| {
            PlanarMatch::Ruled(height_mm, correspondence)
        });
        self.planar_profile(edges, lower, direction, kind)
    }

    fn planar_profile(
        &self,
        edges: &[u32],
        lower: usize,
        direction: Point,
        kind: PlanarMatch<'_>,
    ) -> Option<ImportSolid> {
        let correspondence = match kind {
            PlanarMatch::Ruled(_, correspondence) => Some(correspondence),
            _ => None,
        };
        let mut successors = BTreeMap::new();
        let mut circles = Vec::new();
        for &id in edges {
            let edge = &self.edges[&id];
            let forward = edge.uses.iter().find(|use_| use_.face == lower)?.forward;
            match edge.curve {
                Curve::Line => {
                    let [a, b] = edge.vertices;
                    // Reverse the lower cap's outward winding for the
                    // positive normal of the reconstructed extrusion.
                    let (start, end) = if forward { (b, a) } else { (a, b) };
                    if successors.insert(start, end).is_some() {
                        return None;
                    }
                }
                Curve::Circle(circle) => {
                    if (dot(circle.normal, direction) > 0.) != forward {
                        return None;
                    }
                    circles.push(circle);
                }
                Curve::Semicircle(_) | Curve::Arc(_) => return None,
            }
        }
        // Following a long rim edge preserves the profile's native axes on
        // rotated rectangular parts and avoids introducing needless rounding.
        let reference = successors
            .iter()
            .map(|(a, b)| {
                let delta = sub(self.points[b], self.points[a]);
                let delta = if preferred_axis(delta) {
                    delta
                } else {
                    delta.map(|x| -x)
                };
                delta.map(zero)
            })
            .max_by(|&a, &b| length(a).total_cmp(&length(b)).then(compare(a, b)))?;
        let [u, v] = frame_axes(direction, reference);
        let origin_mm = successors
            .keys()
            .map(|vertex| self.points[vertex].map(zero))
            .min_by(|&a, &b| compare(a, b))?;
        let project = |point| {
            let delta = sub(point, origin_mm);
            [zero(dot(delta, u)), zero(dot(delta, v))]
        };
        let mut outer = None;
        let mut holes = Vec::new();
        while let Some((&start, _)) = successors.first_key_value() {
            let mut ring = Vec::new();
            let mut end_ring = Vec::new();
            let mut current = start;
            loop {
                ring.push(project(self.points[&current]));
                if let Some(correspondence) = correspondence {
                    end_ring.push(project(self.points[correspondence.get(&current)?]));
                }
                current = successors.remove(&current)?;
                if current == start {
                    break;
                }
            }
            let least = ring
                .iter()
                .enumerate()
                .min_by(|(_, a), (_, b)| compare(**a, **b))?
                .0;
            ring.rotate_left(least);
            if correspondence.is_some() {
                end_ring.rotate_left(least);
            }
            if signed_area(&ring) > 0. {
                if outer.replace((ring, end_ring)).is_some() {
                    return None;
                }
            } else {
                holes.push((ring, end_ring));
            }
        }
        holes.sort_unstable_by(|a, b| compare(a.0[0], b.0[0]));
        let rings = std::iter::once(outer?).chain(holes);
        let (height_mm, profile) = match kind {
            PlanarMatch::Straight(height_mm, pocket_rims) => {
                let mut circular_holes: Vec<_> = circles
                    .into_iter()
                    .map(|circle| (project(circle.center_mm), circle.radius_mm))
                    .collect();
                circular_holes.sort_unstable_by(|a, b| compare(a.0, b.0).then(a.1.total_cmp(&b.1)));
                let rings = rings.map(|(ring, _)| ring).collect();
                let profile = if pocket_rims.is_empty() {
                    ImportProfile::Polygon {
                        rings,
                        circular_holes,
                    }
                } else {
                    ImportProfile::PocketedPolygon {
                        rings,
                        circular_holes,
                        pockets: canonical_pockets(
                            pocket_rims,
                            origin_mm,
                            [u, v],
                            height_mm,
                            self.tolerance,
                        )?,
                    }
                };
                (height_mm, profile)
            }
            PlanarMatch::Ruled(height_mm, _) => {
                let (base, top): (Vec<_>, Vec<_>) = rings.unzip();
                Sweep::with_profile_budget(
                    base.clone(),
                    top.clone(),
                    height_mm,
                    crate::linear_sweep::MAX_PROFILE_VERTICES,
                )
                .ok()?;
                (height_mm, ImportProfile::LinearSweep { base, top })
            }
            PlanarMatch::Chamfered(height_mm, bores) => {
                let (circular_holes, chamfers) = canonical_bores(bores, project);
                (
                    height_mm,
                    ImportProfile::ChamferedPolygon {
                        rings: rings.map(|(ring, _)| ring).collect(),
                        circular_holes,
                        chamfers,
                    },
                )
            }
            PlanarMatch::Trimmed {
                direction: generator,
                top_origin,
                top_normal,
            } => {
                let denominator = dot(generator, top_normal);
                let direction = [u, v, direction].map(|axis| dot(generator, axis));
                let travels = [
                    [0.; 3],
                    [
                        dot(sub(top_origin, origin_mm), top_normal) / denominator,
                        -dot(u, top_normal) / denominator,
                        -dot(v, top_normal) / denominator,
                    ],
                ];
                let rings: Vec<_> = rings.map(|(ring, _)| ring).collect();
                TrimmedSweep::new(rings.clone(), direction, travels, self.tolerance).ok()?;
                (
                    travels[1][0] * direction[2],
                    ImportProfile::TrimmedSweep {
                        rings,
                        direction,
                        travels,
                    },
                )
            }
        };
        Some(ImportSolid {
            origin_mm,
            u,
            v,
            height_mm,
            profile,
        })
    }
}
