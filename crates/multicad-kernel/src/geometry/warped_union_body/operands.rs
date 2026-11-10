//! Recognize quarter-cylinder fillet sweeps and preserve their exact volume in
//! the common height coordinate used by the Boolean.
use super::*;
use multicad_rust_kernel::normal_sweep::Edge;
use std::f64::consts::FRAC_PI_2;

#[derive(Clone, Copy)]
pub(super) struct QuarterFillet {
    pub(super) center: Point,
    pub(super) radius: f64,
    axis: usize,
    span: [f64; 2],
    direction: f64,
    back: f64,
    bottom: f64,
}

impl QuarterFillet {
    pub(super) fn recognize(solid: &Solid, tolerance: f64) -> Option<Self> {
        let (body, frame) = solid.normal_sweep_body()?;
        let (rings, height) = body.unpierced_profile()?;
        let [ring] = rings else { return None };
        if ring.len() != 5 {
            return None;
        }
        let normal = frame.normal();
        let axis = (0..2).find(|&i| normal[i].abs() == 1.)?;
        if normal[(axis + 1) % 2] != 0. || normal[2] != 0. {
            return None;
        }
        let transverse = 1 - axis;
        let mut arcs = ring.iter().filter(|edge| matches!(edge, Edge::Arc { .. }));
        let &arc = arcs.next()?;
        if arcs.next().is_some() {
            return None;
        }
        let Edge::Arc { center, a, b, .. } = arc else {
            return None;
        };
        let center = frame.point(center);
        let radius = arc.radius_mm()?;
        let [mut low, mut high] = [frame.point(a), frame.point(b)];
        if low[2] > high[2] {
            std::mem::swap(&mut low, &mut high);
        }
        let direction = (high[transverse] - center[transverse]).signum();
        let mut expected_low = center;
        expected_low[2] -= radius;
        let mut expected_high = center;
        expected_high[transverse] += radius * direction;
        let near = |a: Point, b: Point| norm(subtract(a, b)) <= tolerance;
        if radius <= tolerance
            || !near(low, expected_low)
            || !near(high, expected_high)
            || (arc.signed_sweep_rad()?.abs() - FRAC_PI_2).abs() * radius > tolerance
        {
            return None;
        }
        let lines: Vec<_> = ring
            .iter()
            .filter_map(|edge| match *edge {
                Edge::Line { a, b } => Some([frame.point(a), frame.point(b)]),
                _ => None,
            })
            .collect();
        if lines.len() != 4 {
            return None;
        }
        let bottom = lines.iter().flatten().map(|p| p[2]).reduce(f64::min)?;
        let back = if direction > 0. {
            lines
                .iter()
                .flatten()
                .map(|p| p[transverse])
                .reduce(f64::max)?
        } else {
            lines
                .iter()
                .flatten()
                .map(|p| p[transverse])
                .reduce(f64::min)?
        };
        if expected_low[2] - bottom <= tolerance
            || (back - expected_high[transverse]) * direction <= tolerance
        {
            return None;
        }
        let mut at_bottom = center;
        at_bottom[2] = bottom;
        let mut outer_bottom = at_bottom;
        outer_bottom[transverse] = back;
        let mut outer_top = center;
        outer_top[transverse] = back;
        let expected = [
            [expected_low, at_bottom],
            [at_bottom, outer_bottom],
            [outer_bottom, outer_top],
            [outer_top, expected_high],
        ];
        if !expected.into_iter().all(|[a, b]| {
            lines
                .iter()
                .any(|&[c, d]| near(a, c) && near(b, d) || near(a, d) && near(b, c))
        }) {
            return None;
        }
        let a = center[axis];
        let b = a + normal[axis] * height;
        Some(Self {
            center,
            radius,
            axis,
            span: [a.min(b), a.max(b)],
            direction,
            back,
            bottom,
        })
    }

    pub(super) fn pieces(
        self,
        center_z: f64,
        radius: f64,
        tolerance: f64,
    ) -> Result<[ConvexPiece; 2], planar_boolean::Error> {
        let axis = self.axis;
        let transverse = 1 - axis;
        let center = self.center[transverse];
        let floor = center_z - radius;
        let mut min = [0.; 3];
        let mut max = [0.; 3];
        [min[axis], max[axis]] = self.span;
        min[transverse] = center.min(self.back);
        max[transverse] = center.max(self.back);
        min[2] = self.bottom - floor;
        let base = ConvexPiece::from_box(min, max, tolerance)?;
        let mut section = [
            [center, 0.],
            [self.back, 0.],
            [self.back, radius],
            [center + self.direction * radius, radius],
        ];
        let unit = std::array::from_fn(|i| f64::from(i == axis));
        let at = |axis_value, [other, z]: [f64; 2]| {
            let mut point = [0., 0., z];
            point[axis] = axis_value;
            point[transverse] = other;
            point
        };
        if winding(&section.map(|p| at(self.span[0], p)), unit) < 0. {
            section.reverse();
        }
        let lower = section.map(|p| at(self.span[0], p));
        let upper = section.map(|p| at(self.span[1], p));
        let mut faces = vec![lower.into_iter().rev().collect(), upper.to_vec()];
        for i in 0..section.len() {
            let next = (i + 1) % section.len();
            faces.push(vec![lower[i], lower[next], upper[next], upper[i]]);
        }
        Ok([base, ConvexPiece::new(faces, tolerance)?])
    }
}
