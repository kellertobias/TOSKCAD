//! Use an uncut box in visible-line projection only when filling the through
//! apertures cannot hide any geometry. Emit the actual visible rims separately.
//! The same proof applies to blind cavities with fully shadowed floors.

use super::{pockets::Stock, *};
use multicad_rust_kernel::{
    curved_projection::Ellipse,
    orthogonal::Box3,
    planar_projection::{RimCandidate, SpatialRimCandidate},
};
use std::f64::consts::TAU;

const DIRECTION_TOLERANCE: f64 = 32. * f64::EPSILON;
const WORK_LIMIT: usize = 2_000_000;
type Point = [f64; 3];

pub(super) fn has_through_bores(solid: &Solid) -> bool {
    solid
        .orthogonal_body()
        .is_some_and(|(body, _)| !body.drawing_stock().through_bores.is_empty())
        || solid.polygon_prism().is_some_and(|stock| {
            stock.bore_treatments.is_none() && !stock.prism.circular_holes().is_empty()
        })
}

pub(super) struct Cavity {
    pub mouth: Point,
    pub normal: Point,
    pub view_normal: Point,
    pub radius: f64,
    pub depth: f64,
}

struct Bore {
    centers: [Point; 2],
    outward: Point,
    radius: f64,
}

struct Cuts {
    frame: PlaneFrame,
    bores: Vec<Bore>,
    blind_normals: Vec<Point>,
}

impl Cuts {
    fn for_box(solid: &Solid) -> Result<Self, GeometryError> {
        if let Some((body, frame)) = solid.orthogonal_body() {
            let stock = body.drawing_stock();
            let cell = stock.cells[0];
            let bores = stock.through_bores.iter().map(|bore| {
                let outward = stock.faces[bore.faces[0]].normal;
                let axis = outward.iter().position(|&value| value != 0.).expect("orthogonal support");
                let levels = bore.centers_mm.map(|point| point[axis]);
                if levels[0].min(levels[1]) != cell.min[axis]
                    || levels[0].max(levels[1]) != cell.max[axis]
                {
                    return Err(unsupported(
                        "Rust through-bore projection requires an aperture across the full stock depth",
                    ));
                }
                Ok(Bore { centers: bore.centers_mm, outward, radius: bore.radius_mm })
            }).collect::<Result<Vec<_>, _>>()?;
            let blind_normals = stock
                .pockets
                .iter()
                .map(|pocket| stock.faces[pocket.face].normal)
                .collect();
            return Ok(Self {
                frame,
                bores,
                blind_normals,
            });
        }

        let stock = solid.polygon_prism().expect("certified polygon stock");
        let sign = if stock.reverse_sweep { -1. } else { 1. };
        let height = sign * stock.prism.height().as_f64();
        let bores = stock
            .prism
            .circular_holes()
            .iter()
            .map(|bore| {
                let [x, y] = bore.center().map(|value| value.as_f64());
                Bore {
                    centers: [[x, y, 0.], [x, y, height]],
                    outward: [0., 0., -sign],
                    radius: bore.radius().as_f64(),
                }
            })
            .collect();
        let blind_normals = stock
            .pockets
            .iter()
            .map(|pocket| [0., 0., if pocket.end { sign } else { -sign }])
            .collect();
        Ok(Self {
            frame: stock.frame,
            bores,
            blind_normals,
        })
    }
}

pub(super) struct Scene<'a> {
    frame: PlaneFrame,
    axes: [Point; 3],
    extents: Vec<Box3>,
    world: Vec<Box3>,
    members: &'a [(usize, &'a Solid)],
    tolerance: f64,
    budget: usize,
}

impl<'a> Scene<'a> {
    pub(super) fn get<'scene>(
        scene: &'scene mut Option<Self>,
        members: &'a [(usize, &'a Solid)],
        frame: PlaneFrame,
    ) -> Result<&'scene mut Self, GeometryError> {
        if scene.is_none() {
            *scene = Some(Self::new(members, frame)?);
        }
        Ok(scene.as_mut().expect("created aperture scene"))
    }

    fn new(members: &'a [(usize, &'a Solid)], frame: PlaneFrame) -> Result<Self, GeometryError> {
        let axes = planar::view_axes(frame);
        let world_axes = std::array::from_fn(|i| axes.map(|axis| axis[i]));
        let offset = axes.map(|axis| -dot(axis, frame.origin));
        let tolerance = members
            .iter()
            .map(|(_, solid)| solid.geometry_policy().linear_tolerance_mm)
            .fold(0., f64::max);
        let mut budget = WORK_LIMIT;
        charge(&mut budget, members.len())?;
        let world = members
            .iter()
            .map(|(_, solid)| {
                let bounds = solid.bounds()?;
                Ok(Box3 {
                    min: bounds.min,
                    max: bounds.max,
                })
            })
            .collect::<Result<Vec<_>, GeometryError>>()?;
        let extents = world
            .iter()
            .map(|&bounds| opaque::projected_bounds(bounds, world_axes, offset))
            .collect();
        Ok(Self {
            frame,
            axes,
            extents,
            world,
            members,
            tolerance,
            budget,
        })
    }

    pub(super) fn stock(
        &mut self,
        solid: &Solid,
        owner: usize,
        source: usize,
    ) -> Result<Stock, GeometryError> {
        let proxy = solid.drawing_through_box().ok_or_else(|| {
            unsupported("Rust through-bore projection requires a single rectangular stock body")
        })?;
        let Cuts {
            frame,
            bores,
            blind_normals,
        } = Cuts::for_box(solid)?;
        let position = |p: Point| crate::rust_prism_topology::position(frame, p);
        let local = planar::view_axes(frame);
        let local_to_view = self.axes.map(|axis| local.map(|basis| dot(axis, basis)));
        let aim = |normal| local_to_view.map(|axis| dot(axis, normal));

        for normal in blind_normals {
            let [_, _, z] = aim(normal);
            if z > DIRECTION_TOLERANCE {
                return Err(unsupported(
                    "Rust through-bore projection requires blind pockets to face away or lie edge-on",
                ));
            }
        }

        let mut rims = Vec::with_capacity(bores.len());
        let mut spatial_rims = Vec::new();
        for bore in bores {
            let [x, y, z] = aim(bore.outward);
            if z.abs() <= DIRECTION_TOLERANCE {
                // Complete disks are strictly inset from the rectangular
                // stock's side planes. Edge-on rays still hit the first side.
                continue;
            }
            let near = usize::from(z < 0.);
            let mouth = position(bore.centers[near]);
            if x.hypot(y) <= DIRECTION_TOLERANCE {
                let offset = std::array::from_fn(|i| mouth[i] - self.frame.origin[i]);
                let [u, v, depth] = self.axes.map(|axis| dot(axis, offset));
                let center = [u, v];
                self.clear(owner, center, bore.radius, depth)?;
                rims.push(RimCandidate {
                    source,
                    center,
                    depth,
                    radius: bore.radius,
                    trim: [0., TAU],
                });
                continue;
            }

            let sign = if near == 0 { 1. } else { -1. };
            let normal =
                std::array::from_fn(|i| sign * dot(bore.outward, local.map(|axis| axis[i])));
            let far = position(bore.centers[1 - near]);
            let bore_depth = (0..3).map(|i| (mouth[i] - far[i]) * normal[i]).sum();
            spatial_rims.push(self.oblique(
                owner,
                source,
                Cavity {
                    mouth,
                    normal,
                    view_normal: [sign * x, sign * y, z.abs()],
                    radius: bore.radius,
                    depth: bore_depth,
                },
            )?);
        }
        Ok(Stock {
            solid: proxy,
            rims,
            spatial_rims,
            openings: Vec::new(),
            intrinsic_hidden: Vec::new(),
        })
    }

    pub(super) fn oblique(
        &mut self,
        owner: usize,
        source: usize,
        cavity: Cavity,
    ) -> Result<SpatialRimCandidate, GeometryError> {
        let Cavity {
            mouth,
            view_normal: [x, y, cosine],
            radius,
            depth: bore_depth,
            ..
        } = cavity;
        let sine = x.hypot(y);
        // No ray inside the mouth crosses more than one disk diameter
        // transverse to the cylinder axis before it hits solid stock.
        let penetration = 2. * radius * cosine / sine;
        if penetration + self.tolerance >= bore_depth {
            return Err(unsupported(
                "Rust oblique aperture projection requires its far rim to be fully shadowed",
            ));
        }
        self.clear_volume(owner, cavity, penetration)?;

        let offset = std::array::from_fn(|i| mouth[i] - self.frame.origin[i]);
        let [u, v, depth] = self.axes.map(|axis| dot(axis, offset));
        Ok(SpatialRimCandidate {
            source,
            ellipse: Ellipse {
                center: [u, v],
                u: [-y / sine, x / sine],
                v: [-x / sine, -y / sine],
                major: radius,
                minor: radius * cosine,
                trim: [0., TAU],
            },
            depth,
            depth_sine: radius * sine,
        })
    }

    fn clear_volume(
        &mut self,
        owner: usize,
        opening: Cavity,
        penetration: f64,
    ) -> Result<(), GeometryError> {
        let Cavity {
            mouth,
            normal,
            radius,
            ..
        } = opening;
        let cavity = cavity_bounds(mouth, normal, radius, penetration);
        let mut unresolved = false;
        for (index, &bounds) in self.world.iter().enumerate() {
            charge(&mut self.budget, 1)?;
            if index == owner
                || outside_mouth(bounds, mouth, normal)
                || !overlaps(bounds, cavity, self.tolerance)
            {
                continue;
            }
            if let Some((body, frame)) = self.members[index].1.orthogonal_body() {
                let cells = body.drawing_stock().cells;
                charge(&mut self.budget, cells.len())?;
                let axes = planar::view_axes(frame);
                if cells.iter().all(|&cell| {
                    let bounds = opaque::projected_bounds(cell, axes, frame.origin);
                    outside_mouth(bounds, mouth, normal)
                        || !overlaps(bounds, cavity, self.tolerance)
                }) {
                    continue;
                }
            }
            if seals_mouth(self.members[index].1, mouth, normal, radius, self.tolerance) {
                // Every affected view ray hits the pin on or before the
                // proxy plane, even if another member lies inside the bore.
                return Ok(());
            }
            unresolved = true;
        }
        if unresolved {
            return Err(unsupported(
                "Rust oblique aperture visibility is unresolved because another body may enter the cavity",
            ));
        }
        Ok(())
    }

    fn clear(
        &mut self,
        owner: usize,
        center: [f64; 2],
        radius: f64,
        depth: f64,
    ) -> Result<(), GeometryError> {
        for (index, bounds) in self.extents.iter().enumerate() {
            charge(&mut self.budget, 1)?;
            // A body wholly in front of the aperture can hide its rim;
            // the regular scene handles it without filling any visible ray.
            if index == owner || bounds.min[2] > depth + self.tolerance {
                continue;
            }
            let [x, y] =
                std::array::from_fn(|i| center[i] - center[i].clamp(bounds.min[i], bounds.max[i]));
            if x.hypot(y) <= radius + self.tolerance {
                return Err(unsupported(
                    "Rust through-bore visibility is unresolved because another body overlaps the aperture",
                ));
            }
        }
        Ok(())
    }
}

// Material in the outward half-space is in front of the proxy's mouth
// plane on every ray through the aperture. An exact cardinal plane can
// prove this even when an adjacent face touches the mouth.
fn outside_mouth(bounds: Box3, mouth: Point, normal: Point) -> bool {
    (0..3).any(|i| {
        normal[(i + 1) % 3] == 0.
            && normal[(i + 2) % 3] == 0.
            && ((normal[i] > 0. && bounds.min[i] >= mouth[i])
                || (normal[i] < 0. && bounds.max[i] <= mouth[i]))
    })
}

// A solid cylindrical member that spans the bore mouth can serve as the
// first material hit. Circular inserts with a gap or a bore cannot seal it.
fn seals_mouth(solid: &Solid, mouth: Point, normal: Point, radius: f64, tolerance: f64) -> bool {
    let Some((frame, outer, height, None)) = circular_dimensions(solid) else {
        return false;
    };
    let [u, v, axis] = planar::view_axes(frame);
    let relative = std::array::from_fn(|i| mouth[i] - frame.origin[i]);
    let level = dot(relative, axis);
    let alignment = dot(u, normal).hypot(dot(v, normal));
    let displacement = dot(u, relative).hypot(dot(v, relative));
    let roundoff = (0..3)
        .map(|i| mouth[i].abs() + frame.origin[i].abs())
        .sum::<f64>()
        * (16. * f64::EPSILON);
    let axial_margin = tolerance + radius * alignment;
    alignment <= DIRECTION_TOLERANCE
        && level > axial_margin
        && level < height - axial_margin
        && outer >= radius
        && displacement <= outer - radius + roundoff
}

fn cavity_bounds(mouth: Point, normal: Point, radius: f64, penetration: f64) -> Box3 {
    let end: Point = std::array::from_fn(|i| mouth[i] - normal[i] * penetration);
    let radial: Point =
        std::array::from_fn(|i| radius * normal[(i + 1) % 3].hypot(normal[(i + 2) % 3]));
    Box3 {
        min: std::array::from_fn(|i| mouth[i].min(end[i]) - radial[i]),
        max: std::array::from_fn(|i| mouth[i].max(end[i]) + radial[i]),
    }
}

fn overlaps(a: Box3, b: Box3, tolerance: f64) -> bool {
    (0..3).all(|i| a.min[i] <= b.max[i] + tolerance && b.min[i] <= a.max[i] + tolerance)
}

fn charge(budget: &mut usize, count: usize) -> Result<(), GeometryError> {
    *budget = budget.checked_sub(count).ok_or_else(|| {
        err(
            GeometryErrorKind::Limit,
            "Rust aperture visibility proof exceeded its work budget",
        )
    })?;
    Ok(())
}

fn unsupported(message: &str) -> GeometryError {
    err(GeometryErrorKind::Unsupported, message)
}
