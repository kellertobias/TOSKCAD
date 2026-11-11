//! Receiving stock in the retained thread chart, with a finite reconstruction proof.
use super::*;

pub(super) struct Chart {
    origin: Point,
    inverse: [Point; 3],
}
impl Chart {
    pub(super) fn new(frame: PlaneFrame) -> Self {
        Self::from_axes(frame.origin, [frame.u, frame.v, frame.normal()])
    }
    fn from_axes(origin: Point, axes: [Point; 3]) -> Self {
        let cofactors = [
            cross(axes[1], axes[2]),
            cross(axes[2], axes[0]),
            cross(axes[0], axes[1]),
        ];
        let determinant = dot(axes[0], cofactors[0]);
        Self {
            origin,
            inverse: cofactors.map(|row| row.map(|value| value / determinant)),
        }
    }
    pub(super) fn vector(&self, world: Point) -> Point {
        self.inverse.map(|row| dot(row, world))
    }
    pub(super) fn point(&self, world: Point) -> Point {
        self.vector(std::array::from_fn(|i| world[i] - self.origin[i]))
    }
}

/// A change of chart is affine even when both public frames pass their
/// near-rigid admission. Its third column must not be reconstructed by cross.
pub(super) struct StoredFrame {
    origin: Point,
    axes: [Point; 3],
    inverse: Chart,
}
impl StoredFrame {
    pub(super) fn identity() -> Self {
        Self::new([0.; 3], [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]])
    }
    fn new(origin: Point, axes: [Point; 3]) -> Self {
        Self {
            origin,
            axes,
            inverse: Chart::from_axes(origin, axes),
        }
    }
    pub(super) fn point(&self, point: Point) -> Point {
        std::array::from_fn(|i| self.origin[i] + dot(point, self.axes.map(|axis| axis[i])))
    }
    pub(super) fn local(&self, point: Point) -> Point {
        self.inverse.point(point)
    }
    pub(super) fn placed(&self, thread: PlaneFrame) -> PlaneFrame {
        PlaneFrame {
            origin: position(thread, self.origin),
            u: direction(thread, self.axes[0]),
            v: direction(thread, self.axes[1]),
        }
    }
    pub(super) fn bounds(
        &self,
        thread: PlaneFrame,
        cells: &[multicad_rust_kernel::orthogonal::Box3],
    ) -> Bounds {
        let mut result = Bounds {
            min: [f64::INFINITY; 3],
            max: [f64::NEG_INFINITY; 3],
        };
        // Each cell is a certified axis-aligned box. A linear coordinate's
        // extrema occur at its eight corners; no sampled surface is used.
        for cell in cells {
            for corner in 0..8 {
                let point = std::array::from_fn(|i| {
                    if corner & (1 << i) == 0 {
                        cell.min[i]
                    } else {
                        cell.max[i]
                    }
                });
                let point = position(thread, self.point(point));
                for (i, value) in point.into_iter().enumerate() {
                    result.min[i] = result.min[i].min(value);
                    result.max[i] = result.max[i].max(value);
                }
            }
        }
        result
    }
}

fn direction(frame: PlaneFrame, local: Point) -> Point {
    let axes = [frame.u, frame.v, frame.normal()];
    std::array::from_fn(|i| dot(local, axes.map(|axis| axis[i])))
}
// Product FMA residuals and TwoSum retain cancellation error instead of
// certifying an accidentally rounded-zero forward coefficient difference.
fn evaluated(frame: PlaneFrame, point: Point, translate: bool) -> (Point, Point) {
    let axes = [frame.u, frame.v, frame.normal()];
    let mut error = [0.; 3];
    let result = std::array::from_fn(|i| {
        let mut sum = 0.;
        for (a, b) in point
            .into_iter()
            .zip(axes.map(|axis| axis[i]))
            .chain(translate.then_some((1., frame.origin[i])))
        {
            let product = a * b;
            let next = sum + product;
            let z = next - sum;
            let product_error = a.mul_add(b, -product).abs().next_up();
            let addition_error = ((sum - (next - z)) + (product - z)).abs().next_up();
            error[i] = (error[i] + product_error).next_up();
            error[i] = (error[i] + addition_error).next_up();
            sum = next;
        }
        sum
    });
    (result, error)
}
fn error_sum(error: Point) -> f64 {
    error
        .into_iter()
        .fold(0., |sum, value| (sum + value).next_up())
}

fn difference_bound(a: Point, b: Point) -> f64 {
    a.into_iter()
        .zip(b)
        .fold(0., |sum, (a, b)| (sum + (a - b).abs().next_up()).next_up())
}

pub(super) fn store(
    thread: PlaneFrame,
    stock: PlaneFrame,
    local_bounds: Bounds,
    policy: GeometryPolicy,
) -> Result<StoredFrame, GeometryError> {
    thread.checked(policy)?;
    let chart = Chart::new(thread);
    let stored = StoredFrame::new(
        chart.point(stock.origin),
        [stock.u, stock.v, stock.normal()].map(|axis| chart.vector(axis)),
    );
    let recovered = stored.placed(thread);
    let (_, origin_error) = evaluated(thread, stored.origin, true);
    let evaluated_axes = stored.axes.map(|axis| evaluated(thread, axis, false));
    let extents = std::array::from_fn::<_, 3, _>(|i| {
        local_bounds.min[i].abs().max(local_bounds.max[i].abs())
    });
    let mut physical =
        (difference_bound(stock.origin, recovered.origin) + error_sum(origin_error)).next_up();
    for ((original, (actual, roundoff)), extent) in [stock.u, stock.v, stock.normal()]
        .into_iter()
        .zip(evaluated_axes)
        .zip(extents)
    {
        let coefficient = (difference_bound(original, actual) + error_sum(roundoff)).next_up();
        physical = (physical + (coefficient * extent).next_up()).next_up();
    }
    let mut metadata = difference_bound(stock.origin, recovered.origin);
    for ((original, actual), extent) in [stock.u, stock.v, stock.normal()]
        .into_iter()
        .zip([recovered.u, recovered.v, recovered.normal()])
        .zip(extents)
    {
        metadata = (metadata + (difference_bound(original, actual) * extent).next_up()).next_up();
    }
    let residual = physical.max(metadata);
    if !residual.is_finite() || residual > policy.linear_tolerance_mm / 16. {
        return Err(error(
            OPERATION,
            GeometryErrorKind::Unsupported,
            "Receiving stock cannot preserve its authored chart within model tolerance",
        ));
    }
    Ok(stored)
}

/// Preserve authored attachment coordinates independently of stock promotion's
/// choice of outward geometric plane charts.
pub(super) fn store_anchors(
    thread: PlaneFrame,
    snapshot: TopologySnapshot,
    policy: GeometryPolicy,
) -> Result<std::collections::BTreeMap<Origin, ReferenceFrame>, GeometryError> {
    let chart = Chart::new(thread);
    let mut result = std::collections::BTreeMap::new();
    for anchor in snapshot.elements.into_iter().flat_map(|e| e.frames) {
        let original = anchor.frame;
        let stored = ReferenceFrame {
            origin_mm: chart.point(original.origin_mm),
            u: chart.vector(original.u),
            v: chart.vector(original.v),
        };
        let recovered = boundary::placed_frame(thread, stored.origin_mm, stored.u, stored.v);
        let residual = [
            (original.origin_mm, recovered.origin_mm),
            (original.u, recovered.u),
            (original.v, recovered.v),
        ]
        .into_iter()
        .map(|(a, b)| difference_bound(a, b))
        .fold(0_f64, f64::max);
        if !residual.is_finite()
            || residual > policy.linear_tolerance_mm / 16.
            || result
                .insert(anchor.origin, stored)
                .is_some_and(|previous| previous != stored)
        {
            return Err(error(
                OPERATION,
                GeometryErrorKind::Unsupported,
                "Receiving stock cannot preserve an unambiguous authored attachment chart",
            ));
        }
    }
    Ok(result)
}
