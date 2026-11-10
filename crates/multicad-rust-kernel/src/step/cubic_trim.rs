//! Shared STEP encoding for clamped cubic edges in a normal sweep.
use super::*;
use crate::{
    spline_sweep::Cubic,
    spur_gear::{Error, Involute},
};
use std::borrow::Cow;

pub(super) struct CubicTrim<'a> {
    points: Cow<'a, [[f64; 2]]>,
    knots: Cow<'a, [f64]>,
    flank_error_mm: Option<f64>,
    tolerance_mm: f64,
    reversed: bool,
}

impl CubicTrim<'static> {
    pub(super) fn flank(flank: Involute, tolerance_mm: f64) -> Result<Self, StepError> {
        // Reserve half the budget for placement and endpoint sharing.
        let curve = flank
            .cubic_spline(tolerance_mm / 2.)
            .map_err(|error| match error {
                Error::Limit => StepError::Limit,
                Error::InvalidDimensions | Error::InvalidProfile => StepError::InvalidGeometry,
            })?;
        Ok(Self {
            points: Cow::Owned(curve.poles_mm),
            knots: Cow::Owned(curve.knots),
            flank_error_mm: Some(curve.error_bound_mm),
            tolerance_mm,
            reversed: false,
        })
    }
}

impl<'a> CubicTrim<'a> {
    pub(super) fn decoded(curve: &'a Cubic, tolerance_mm: f64) -> Self {
        Self {
            points: Cow::Borrowed(curve.poles()),
            knots: Cow::Borrowed(curve.knots()),
            flank_error_mm: None,
            tolerance_mm,
            reversed: curve.reversed(),
        }
    }

    pub(super) fn start(&self) -> [f64; 2] {
        self.points[if self.reversed {
            self.points.len() - 1
        } else {
            0
        }]
    }

    pub(super) fn reversed(&self) -> bool {
        self.reversed
    }

    pub(super) fn encode(
        &self,
        file: &mut StepFile,
        vertices: [Vertex; 2],
        placement: &Placement,
        z: f64,
    ) -> Result<(Entity, Entity), StepError> {
        let [from, to] = if self.reversed {
            [vertices[1], vertices[0]]
        } else {
            vertices
        };
        if from.point == to.point {
            return Err(StepError::InvalidGeometry);
        }
        let end = self.points.len() - 1;
        let first = placement.point_f64(self.points[0], z);
        let last = placement.point_f64(self.points[end], z);
        let discrepancy = |a: Point, b: Point| {
            let [x, y, z] = std::array::from_fn(|i| a[i] - b[i]);
            x.hypot(y).hypot(z)
        };
        let endpoint_error = discrepancy(from.point, first).max(discrepancy(to.point, last));
        let scale = self
            .points
            .iter()
            .map(|[x, y]| x.abs() + y.abs())
            .fold(0., f64::max)
            + placement
                .origin
                .into_iter()
                .map(f64::abs)
                .fold(0., f64::max)
            + z.abs();
        // Include placement and decimal rounding as well as any discrepancy
        // between the physical trim vertex and the endpoint of the spline.
        let bound = self.flank_error_mm.unwrap_or(0.) * (1. + 1e-9)
            + endpoint_error
            + 16. * f64::EPSILON * scale;
        if !bound.is_finite() || bound > self.tolerance_mm {
            return Err(StepError::Limit);
        }

        let mut poles = Vec::with_capacity(self.points.len());
        // Authored involutes already carry an approximation budget and share
        // the boundary's exact endpoints. Decoded cubics retain their poles.
        let pole = |file: &mut StepFile, point: Point, vertex: Vertex| {
            if self.flank_error_mm.is_some() || point == vertex.point {
                vertex.cartesian
            } else {
                file.point(point)
            }
        };
        poles.push(pole(file, first, from));
        poles.extend(
            self.points[1..end]
                .iter()
                .map(|&point| file.point(placement.point_f64(point, z))),
        );
        poles.push(pole(file, last, to));
        let multiplicities = self
            .knots
            .iter()
            .enumerate()
            .map(|(i, _)| {
                if i == 0 || i + 1 == self.knots.len() {
                    "4"
                } else {
                    "2"
                }
            })
            .collect::<Vec<_>>()
            .join(",");
        let knots = self
            .knots
            .iter()
            .map(|knot| format!("{knot:.16E}"))
            .collect::<Vec<_>>()
            .join(",");
        let curve = file.entity(format_args!(
            "B_SPLINE_CURVE_WITH_KNOTS('',3,{},.UNSPECIFIED.,.F.,.F.,({multiplicities}),({knots}),.UNSPECIFIED.)",
            Entities(&poles)
        ));
        let (start, end) = (from.vertex, to.vertex);
        let edge = file.entity(format_args!("EDGE_CURVE('',{start},{end},{curve},.T.)"));
        Ok((edge, curve))
    }
}
