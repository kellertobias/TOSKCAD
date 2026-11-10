//! Use an uncut box in visible-line projection only when filling the through
//! apertures cannot hide any geometry. Emit the actual visible rims separately.

use super::{pockets::Stock, *};
use multicad_rust_kernel::{orthogonal::Box3, planar_projection::RimCandidate};
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

pub(super) struct Scene {
    frame: PlaneFrame,
    axes: [Point; 3],
    extents: Vec<Box3>,
    tolerance: f64,
    budget: usize,
}

impl Scene {
    pub(super) fn new(
        members: &[(usize, &Solid)],
        frame: PlaneFrame,
    ) -> Result<Self, GeometryError> {
        let axes = planar::view_axes(frame);
        let world_axes = std::array::from_fn(|i| axes.map(|axis| axis[i]));
        let offset = axes.map(|axis| -dot(axis, frame.origin));
        let tolerance = members
            .iter()
            .map(|(_, solid)| solid.geometry_policy().linear_tolerance_mm)
            .fold(0., f64::max);
        let extents = members
            .iter()
            .map(|(_, solid)| {
                let bounds = solid.bounds()?;
                Ok(opaque::projected_bounds(
                    Box3 {
                        min: bounds.min,
                        max: bounds.max,
                    },
                    world_axes,
                    offset,
                ))
            })
            .collect::<Result<Vec<_>, GeometryError>>()?;
        Ok(Self {
            frame,
            axes,
            extents,
            tolerance,
            budget: WORK_LIMIT,
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
            let [x, y, z] = aim(normal);
            if z.abs() > DIRECTION_TOLERANCE && !(z < 0. && x.hypot(y) <= DIRECTION_TOLERANCE) {
                return Err(unsupported(
                    "Rust through-bore projection requires blind pockets to face away or lie edge-on",
                ));
            }
        }

        let mut rims = Vec::with_capacity(bores.len());
        for bore in bores {
            let [x, y, z] = aim(bore.outward);
            if z.abs() <= DIRECTION_TOLERANCE {
                // Complete disks are strictly inset from the rectangular
                // stock's side planes. Edge-on rays still hit the first side.
                continue;
            }
            if x.hypot(y) > DIRECTION_TOLERANCE {
                return Err(unsupported(
                    "Rust through-bore projection requires axial or edge-on circular apertures",
                ));
            }
            let near = position(bore.centers[usize::from(z < 0.)]);
            let offset = std::array::from_fn(|i| near[i] - self.frame.origin[i]);
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
        }
        Ok(Stock {
            solid: proxy,
            rims,
            openings: Vec::new(),
        })
    }

    fn clear(
        &mut self,
        owner: usize,
        center: [f64; 2],
        radius: f64,
        depth: f64,
    ) -> Result<(), GeometryError> {
        for (index, bounds) in self.extents.iter().enumerate() {
            self.budget = self.budget.checked_sub(1).ok_or_else(|| {
                err(
                    GeometryErrorKind::Limit,
                    "Rust through-bore visibility proof exceeded its work budget",
                )
            })?;
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

fn unsupported(message: &str) -> GeometryError {
    err(GeometryErrorKind::Unsupported, message)
}
