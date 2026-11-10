//! Decompose a closed bent section by canceling internal radial chords.
use multicad_rust_kernel::normal_sweep::Edge;
use std::f64::consts::FRAC_PI_2;

type Point = [f64; 2];
type Segment = [Point; 2];

pub(super) struct Bend {
    pub center: Point,
    pub spokes: [Point; 2],
    pub radii: [f64; 2],
}

pub(super) struct Profile {
    pub bends: Vec<Bend>,
    pub rectangles: Vec<Segment>,
}

#[derive(Clone, Copy)]
struct Arc {
    center: Point,
    ends: Segment,
    spokes: Segment,
    radius: f64,
}

impl Arc {
    fn new(edge: Edge) -> Option<Self> {
        let Edge::Arc { center, a, b, .. } = edge else {
            return None;
        };
        if (edge.signed_sweep_rad()?.abs() - FRAC_PI_2).abs() > 32. * f64::EPSILON {
            return None;
        }
        let radius = edge.radius_mm()?;
        let spokes = [a, b].map(|p| std::array::from_fn(|i| (p[i] - center[i]) / radius));
        Some(Self {
            center,
            ends: [a, b],
            spokes,
            radius,
        })
    }
}

impl Profile {
    pub(super) fn new(rings: &[Vec<Edge>]) -> Option<Self> {
        let [ring] = rings else { return None };
        if ring.len() > 256 {
            return None;
        }
        let mut lines = Vec::new();
        let mut inner = Vec::new();
        let mut outer = Vec::new();
        let scale = ring
            .iter()
            .flat_map(|edge| edge.start())
            .map(f64::abs)
            .fold(1., f64::max);
        let tolerance = 64. * f64::EPSILON * scale;
        for &edge in ring {
            match edge {
                Edge::Line { a, b } => lines.push([a, b]),
                Edge::Arc {
                    clockwise: true, ..
                } => inner.push(Arc::new(edge)?),
                Edge::Arc {
                    clockwise: false, ..
                } => outer.push(Arc::new(edge)?),
                Edge::Circle { .. } => return None,
            }
        }
        if outer.is_empty() || inner.len() != outer.len() {
            return None;
        }
        let mut bends = Vec::new();
        for arc in outer {
            let i = inner.iter().position(|candidate| {
                same(candidate.center, arc.center, tolerance)
                    && arc.radius - candidate.radius > tolerance
                    && same(candidate.spokes[0], arc.spokes[1], 32. * f64::EPSILON)
                    && same(candidate.spokes[1], arc.spokes[0], 32. * f64::EPSILON)
            })?;
            let nested = inner.swap_remove(i);
            // Opposite orientation to the radial walls of the annular sector.
            lines.extend([[nested.ends[0], arc.ends[1]], [arc.ends[0], nested.ends[1]]]);
            bends.push(Bend {
                center: arc.center,
                spokes: arc.spokes,
                radii: [nested.radius, arc.radius],
            });
        }

        let mut rectangles = Vec::new();
        while let Some(first) = lines.pop() {
            let mut cycle = vec![first[0]];
            let mut endpoint = first[1];
            while !same(endpoint, first[0], tolerance) {
                cycle.push(endpoint);
                let i = lines
                    .iter()
                    .position(|edge| same(edge[0], endpoint, tolerance))?;
                endpoint = lines.swap_remove(i)[1];
            }
            rectangles.push(rectangle(&cycle, tolerance)?);
        }
        // The internal chords cancel, leaving the original simple CCW
        // boundary. Every piece has positive winding, so their total winding
        // proves exact coverage with no overlapping interiors.
        Some(Self { bends, rectangles })
    }
}

fn same(a: Point, b: Point, tolerance: f64) -> bool {
    (0..2).all(|i| (a[i] - b[i]).abs() <= tolerance)
}

fn rectangle(ring: &[Point], tolerance: f64) -> Option<Segment> {
    let [a, b, c, d] = <[Point; 4]>::try_from(ring).ok()?;
    let bounds = [false, true].map(|high| {
        std::array::from_fn(|i| {
            ring.iter()
                .map(|p| p[i])
                .reduce(|a, b| if high { a.max(b) } else { a.min(b) })
                .expect("four corners")
        })
    });
    let [low, high] = bounds;
    let corners = [low, [high[0], low[1]], high, [low[0], high[1]]];
    let ordered = [a, b, c, d];
    if (0..2).any(|i| high[i] - low[i] <= tolerance)
        || !(0..4)
            .any(|shift| (0..4).all(|i| same(ordered[i], corners[(i + shift) % 4], tolerance)))
    {
        return None;
    }
    Some(bounds)
}
