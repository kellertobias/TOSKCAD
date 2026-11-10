//! Certify that every ray through an opening hits an intact nearer face.
use super::*;
use multicad_rust_kernel::orthogonal::Box3;
use std::collections::BTreeSet;

type Point = [f64; 3];
const DIRECTION_TOLERANCE: f64 = 32. * f64::EPSILON;
const WORK_LIMIT: usize = 2_000_000;

pub(super) struct Mouth {
    pub centers: [Point; 2],
    pub normal: Point,
    pub radius: f64,
}

pub(super) struct Shadows {
    faces: Vec<Cover>,
    tolerance: f64,
    budget: usize,
}

impl Shadows {
    pub(super) fn get<'a>(
        shadows: &'a mut Option<Self>,
        members: &[(usize, &Solid)],
        frame: PlaneFrame,
    ) -> Result<&'a mut Self, GeometryError> {
        if shadows.is_none() {
            *shadows = Some(Self::new(members, frame)?);
        }
        Ok(shadows.as_mut().expect("created cover scene"))
    }

    fn new(members: &[(usize, &Solid)], frame: PlaneFrame) -> Result<Self, GeometryError> {
        let view = planar::view_axes(frame)[2];
        let mut shadows = Self {
            faces: Vec::new(),
            tolerance: GeometryPolicy::default().linear_tolerance_mm,
            budget: WORK_LIMIT,
        };
        for (owner, &(_, solid)) in members.iter().enumerate() {
            shadows.tolerance = shadows
                .tolerance
                .max(solid.geometry_policy().linear_tolerance_mm);
            if let Some((body, placement)) = solid.orthogonal_body() {
                let stock = body.drawing_stock();
                charge(&mut shadows.budget, stock.faces.len())?;
                let machined: BTreeSet<_> = stock
                    .pockets
                    .iter()
                    .map(|pocket| pocket.face)
                    .chain(stock.slots.iter().map(|slot| slot.face))
                    .chain(stock.through_bores.iter().flat_map(|bore| bore.faces))
                    .collect();
                for (face, surface) in stock.faces.iter().enumerate() {
                    let [ring] = surface.rings.as_slice() else {
                        continue;
                    };
                    let Ok(corners) = <[Point; 4]>::try_from(ring.as_slice()) else {
                        continue;
                    };
                    if !machined.contains(&face) {
                        shadows.add(owner, corners, placement, view);
                    }
                }
            } else if let Some((cells, placement)) = solid.drawing_rectilinear_cells()? {
                charge(&mut shadows.budget, 6 * cells.len())?;
                for cell in cells {
                    for corners in box_faces(cell) {
                        shadows.add(owner, corners, placement, view);
                    }
                }
            }
        }
        Ok(shadows)
    }

    fn add(&mut self, owner: usize, corners: [Point; 4], placement: PlaneFrame, view: Point) {
        let position = |p| crate::rust_prism_topology::position(placement, p);
        if let Some(cover) = Cover::new(owner, corners.map(position), view, self.tolerance) {
            self.faces.push(cover);
        }
    }

    pub(super) fn covers(&mut self, owner: usize, mouth: Mouth) -> Result<bool, GeometryError> {
        for cover in &self.faces {
            charge(&mut self.budget, 1)?;
            if cover.owner != owner && cover.contains(&mouth, self.tolerance) {
                return Ok(true);
            }
        }
        Ok(false)
    }
}

struct Cover {
    owner: usize,
    constraints: [(Point, Point); 5],
}

impl Cover {
    fn new(owner: usize, corners: [Point; 4], view: Point, tolerance: f64) -> Option<Self> {
        let edges: [Point; 4] = std::array::from_fn(|i| sub(corners[(i + 1) % 4], corners[i]));
        let normal = unit(cross(edges[0], edges[1]));
        let toward = dot(normal, view);
        if !normal.into_iter().all(f64::is_finite) || toward <= DIRECTION_TOLERANCE {
            return None;
        }
        let mut constraints = [(normal.map(|v| -v), corners[0]); 5];
        for (i, &edge) in edges.iter().enumerate() {
            let inward = unit(cross(normal, edge));
            if !inward.into_iter().all(f64::is_finite)
                || corners
                    .iter()
                    .any(|&p| dot(inward, sub(p, corners[i])) < -tolerance)
            {
                return None;
            }
            // Pull the face edge half-space back along the view rays.
            let factor = dot(inward, view) / toward;
            let axis = std::array::from_fn(|i| inward[i] - factor * normal[i]);
            constraints[i + 1] = (axis, corners[i]);
        }
        Some(Self { owner, constraints })
    }

    fn contains(&self, mouth: &Mouth, tolerance: f64) -> bool {
        let normal = unit(mouth.normal);
        self.constraints.iter().all(|&(axis, anchor)| {
            let radius = mouth.radius * length(cross(axis, normal));
            let minimum = mouth.centers.map(|center| dot(axis, sub(center, anchor)));
            minimum[0].min(minimum[1]) - radius > tolerance * length(axis).max(1.)
        })
    }
}

fn box_faces(bounds: Box3) -> impl Iterator<Item = [Point; 4]> {
    (0..3).flat_map(move |axis| {
        [false, true].map(move |high| {
            let u = (axis + 1) % 3;
            let v = (axis + 2) % 3;
            let mut corners = [bounds.min; 4];
            for p in &mut corners {
                if high {
                    p[axis] = bounds.max[axis];
                }
            }
            corners[1][u] = bounds.max[u];
            corners[2][u] = bounds.max[u];
            corners[2][v] = bounds.max[v];
            corners[3][v] = bounds.max[v];
            if !high {
                corners.reverse();
            }
            corners
        })
    })
}

fn charge(budget: &mut usize, count: usize) -> Result<(), GeometryError> {
    *budget = budget.checked_sub(count).ok_or_else(|| {
        err(
            GeometryErrorKind::Limit,
            "Rust aperture shadow proof exceeded its work budget",
        )
    })?;
    Ok(())
}

fn sub(a: Point, b: Point) -> Point {
    std::array::from_fn(|i| a[i] - b[i])
}

fn cross(a: Point, b: Point) -> Point {
    std::array::from_fn(|i| a[(i + 1) % 3] * b[(i + 2) % 3] - a[(i + 2) % 3] * b[(i + 1) % 3])
}

fn length(a: Point) -> f64 {
    dot(a, a).sqrt()
}
