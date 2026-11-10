//! Symmetric 45° chamfers on the cap rims of circular through-bores.
use crate::{
    Rational,
    circular_pockets::{PocketError, ring_contains_clear_disk},
    polygon::PolygonPrism,
};
use std::{f64::consts::PI, sync::Arc};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BoreChamfer {
    /// Index in the stock prism's circular holes.
    pub hole: usize,
    /// False for the starting cap, true for the ending cap.
    pub end: bool,
    /// Radial and axial size of the symmetric bevel.
    pub size_mm: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BoreLayer {
    /// Positive distance from the starting cap in the stock frame.
    pub z_mm: f64,
    pub radius_mm: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct BoreStack {
    pub center_mm: [f64; 2],
    /// Circular rims ordered from the starting cap to the ending cap.
    pub layers: Vec<BoreLayer>,
    /// One face per adjacent pair of rims. Cylinders keep their original face
    /// ordinal and conical faces follow all original faces.
    pub face_indices: Vec<u32>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ChamferedBores {
    stock: Arc<PolygonPrism>,
    chamfers: Vec<BoreChamfer>,
    stacks: Vec<BoreStack>,
    caps: [(f64, [f64; 2]); 2],
    volume_mm3: f64,
}

impl ChamferedBores {
    pub fn new(
        prism: &PolygonPrism,
        chamfers: &[BoreChamfer],
        clearance_mm: f64,
        minimum_mm: f64,
    ) -> Result<Self, PocketError> {
        use PocketError::{BudgetExceeded, InvalidInput, UnresolvedBoundary};
        if chamfers.is_empty()
            || !clearance_mm.is_finite()
            || clearance_mm < 0.
            || !minimum_mm.is_finite()
            || minimum_mm <= 0.
        {
            return Err(InvalidInput);
        }
        let holes = prism.circular_holes();
        if chamfers.len() > 2 * holes.len() {
            return Err(BudgetExceeded);
        }
        let mut chamfers = chamfers.to_vec();
        chamfers.sort_unstable_by_key(|chamfer| (chamfer.hole, chamfer.end));
        let mut sizes = vec![[0.; 2]; holes.len()];
        for chamfer in &chamfers {
            let size =
                &mut sizes.get_mut(chamfer.hole).ok_or(InvalidInput)?[usize::from(chamfer.end)];
            if *size != 0. || !chamfer.size_mm.is_finite() || chamfer.size_mm < minimum_mm {
                return Err(InvalidInput);
            }
            *size = chamfer.size_mm;
        }
        let height = prism.height().as_f64();
        let guard = clearance_mm.max(minimum_mm);
        let axial_guard = guard + 32. * f64::EPSILON * height;
        if sizes
            .iter()
            .any(|[start, end]| height - start - end <= axial_guard)
        {
            return Err(UnresolvedBoundary);
        }
        let first_bore = 2 + prism.rings().iter().map(Vec::len).sum::<usize>();
        let mut next_face = (first_bore + holes.len()) as u32;
        let mut lost_volume = 0.;
        let mut cap_loss = [0.; 2];
        let mut cap_moment = [[0.; 2]; 2];
        let centroid = prism.centroid();
        let mut stacks = Vec::with_capacity(holes.len());
        for (hole_index, (hole, [start, end])) in holes.iter().zip(sizes).enumerate() {
            let center_mm = hole.center().map(Rational::as_f64);
            let radius = hole.radius().as_f64();
            let mut layers = vec![BoreLayer {
                z_mm: 0.,
                radius_mm: radius + start,
            }];
            let mut face_indices = vec![];
            for (cap, size) in [start, end].into_iter().enumerate() {
                let area = PI * size * (2. * radius + size);
                cap_loss[cap] += area;
                for (axis, moment) in cap_moment[cap].iter_mut().enumerate() {
                    *moment = area.mul_add(center_mm[axis] - centroid[axis], *moment);
                }
                lost_volume += PI * size.powi(2) * (radius + size / 3.);
            }
            if start > 0. {
                layers.push(BoreLayer {
                    z_mm: start,
                    radius_mm: radius,
                });
                face_indices.push(next_face);
                next_face += 1;
            }
            if end > 0. {
                layers.push(BoreLayer {
                    z_mm: height - end,
                    radius_mm: radius,
                });
            }
            layers.push(BoreLayer {
                z_mm: height,
                radius_mm: radius + end,
            });
            face_indices.push((first_bore + hole_index) as u32);
            if end > 0. {
                face_indices.push(next_face);
                next_face += 1;
            }
            stacks.push(BoreStack {
                center_mm,
                layers,
                face_indices,
            });
        }
        check_clearance(prism, &stacks, guard)?;
        let volume_mm3 = prism.volume().as_f64() - lost_volume;
        let caps = std::array::from_fn(|cap| {
            let area = prism.area().as_f64() - cap_loss[cap];
            (
                area,
                std::array::from_fn(|axis| centroid[axis] - cap_moment[cap][axis] / area),
            )
        });
        (volume_mm3.is_finite()
            && volume_mm3 > 0.
            && caps.iter().all(|(area, center)| {
                area.is_finite() && *area > 0. && center.iter().all(|value| value.is_finite())
            }))
        .then(|| Self {
            stock: Arc::new(prism.clone()),
            chamfers,
            stacks,
            caps,
            volume_mm3,
        })
        .ok_or(UnresolvedBoundary)
    }

    pub fn chamfers(&self) -> &[BoreChamfer] {
        &self.chamfers
    }

    pub fn stacks(&self) -> &[BoreStack] {
        &self.stacks
    }

    pub fn volume_mm3(&self) -> f64 {
        self.volume_mm3
    }

    pub fn cap_properties(&self, end: bool) -> (f64, [f64; 2]) {
        self.caps[usize::from(end)]
    }

    pub(crate) fn belongs_to(&self, prism: &PolygonPrism) -> bool {
        *self.stock == *prism
    }
}

fn check_clearance(
    prism: &PolygonPrism,
    stacks: &[BoreStack],
    guard: f64,
) -> Result<(), PocketError> {
    use PocketError::UnresolvedBoundary;
    let rings: Vec<Vec<_>> = prism
        .rings()
        .iter()
        .map(|ring| {
            ring.iter()
                .map(|point| point.map(Rational::as_f64))
                .collect()
        })
        .collect();
    let scale = rings
        .iter()
        .flatten()
        .flatten()
        .fold(1_f64, |scale, x| scale.max(x.abs()));
    let guard = guard + 32. * f64::EPSILON * scale;
    let cap_radii = |stack: &BoreStack| {
        [
            stack.layers[0].radius_mm,
            stack.layers.last().expect("end rim").radius_mm,
        ]
    };
    for (index, stack) in stacks.iter().enumerate() {
        let [start, end] = cap_radii(stack);
        let exclusion = start.max(end) + guard;
        if stack.layers.len() > 2 {
            // The stock already proved which side of every straight ring
            // contains each bore. The expanded disk must clear all edges.
            for ring in &rings {
                ring_contains_clear_disk(ring, stack.center_mm, exclusion)?;
            }
        }
        for other in &stacks[..index] {
            if stack.layers.len() == 2 && other.layers.len() == 2 {
                continue;
            }
            // Each radius is convex along the axis, so the sum of the radii
            // reaches its maximum on one of the two caps.
            let [other_start, other_end] = cap_radii(other);
            let [dx, dy] = std::array::from_fn(|i| stack.center_mm[i] - other.center_mm[i]);
            if dx.hypot(dy) <= (start + other_start).max(end + other_end) + guard {
                return Err(UnresolvedBoundary);
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
