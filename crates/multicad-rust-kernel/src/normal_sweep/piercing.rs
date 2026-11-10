//! Through openings between paired planar walls of a normal sweep.
use super::{validation::clear_pair, *};

type Point3 = [f64; 3];

#[derive(Clone, Debug, PartialEq)]
pub struct Piercing {
    /// Flattened profile-edge ordinals, without the two sweep caps.
    pub sides: [usize; 2],
    /// Origin and axes of the opening on the first side in sweep coordinates.
    pub origin: Point3,
    pub u: Point3,
    pub v: Point3,
    /// Full displacement from the first side to the second.
    pub through: Point3,
    /// Counterclockwise outline in the opening plane.
    pub outline: Vec<Edge>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CertifiedPiercing {
    spec: Piercing,
    area_mm2: f64,
    centroid: Point,
    thickness_mm: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PiercedSweep {
    sweep: Sweep,
    openings: Vec<CertifiedPiercing>,
    volume_mm3: f64,
}

impl CertifiedPiercing {
    pub fn spec(&self) -> &Piercing {
        &self.spec
    }

    pub fn area_mm2(&self) -> f64 {
        self.area_mm2
    }

    pub fn centroid(&self) -> Point {
        self.centroid
    }

    pub fn thickness_mm(&self) -> f64 {
        self.thickness_mm
    }

    pub fn point(&self, [u, v]: Point, end: bool) -> Point3 {
        let spec = &self.spec;
        let t = f64::from(end);
        std::array::from_fn(|i| {
            spec.origin[i] + u * spec.u[i] + v * spec.v[i] + t * spec.through[i]
        })
    }

    fn projection(&self, direction: Point3) -> [f64; 2] {
        let offset = dot3(self.spec.origin, direction);
        let basis = [dot3(self.spec.u, direction), dot3(self.spec.v, direction)];
        let support = |basis| {
            self.spec
                .outline
                .iter()
                .map(|edge| edge.support(basis))
                .fold(f64::NEG_INFINITY, f64::max)
        };
        [offset - support(scale(basis, -1.)), offset + support(basis)]
    }

    fn bounds(&self) -> [[f64; 2]; 3] {
        std::array::from_fn(|i| {
            let mut axis = [0.; 3];
            axis[i] = 1.;
            let [lo, hi] = self.projection(axis);
            let movement = self.spec.through[i];
            [lo + movement.min(0.), hi + movement.max(0.)]
        })
    }

    fn projected_outline(&self, tangent: Point) -> Vec<Edge> {
        let project = |point| {
            let [x, y, z] = self.point(point, false);
            [dot([x, y], tangent), z]
        };
        // The certificate stores either a single circle or four straight
        // edges. Only containment and boundary distance use this projection.
        if let [
            Edge::Circle {
                center,
                radius_mm,
                clockwise,
            },
        ] = self.spec.outline.as_slice()
        {
            return vec![Edge::Circle {
                center: project(*center),
                radius_mm: *radius_mm,
                clockwise: *clockwise,
            }];
        }
        self.spec
            .outline
            .iter()
            .map(|edge| Edge::Line {
                a: project(edge.start()),
                b: project(edge.end()),
            })
            .collect()
    }
}

fn dot3(a: Point3, b: Point3) -> f64 {
    a.into_iter().zip(b).map(|(a, b)| a * b).sum()
}

fn in_section([x, y, _]: Point3) -> Point {
    [x, y]
}

fn lift([x, y]: Point) -> Point3 {
    [x, y, 0.]
}

#[derive(Clone, Copy)]
struct Side {
    ends: [Point; 2],
    tangent: Point,
    inward: Point,
}

impl Side {
    fn new(edge: Edge) -> Result<Self, Error> {
        let Edge::Line { a, b } = edge else {
            return Err(Error::InvalidProfile);
        };
        let delta = sub(b, a);
        let [x, y] = scale(delta, 1. / length(delta));
        Ok(Self {
            ends: [a, b],
            tangent: [x, y],
            inward: [-y, x],
        })
    }
}

impl PiercedSweep {
    pub fn new(
        sweep: &Sweep,
        openings: Vec<Piercing>,
        tolerance_mm: f64,
        minimum_mm: f64,
    ) -> Result<Self, Error> {
        if openings.is_empty()
            || !tolerance_mm.is_finite()
            || tolerance_mm < 0.
            || !minimum_mm.is_finite()
            || minimum_mm <= tolerance_mm
        {
            return Err(Error::InvalidProfile);
        }
        if openings.len() > MAX_VERTICES
            || openings
                .iter()
                .map(|opening| opening.outline.len())
                .sum::<usize>()
                > MAX_VERTICES
        {
            return Err(Error::Limit);
        }
        let mut openings_checked = Vec::<CertifiedPiercing>::with_capacity(openings.len());
        let mut removed = Sum::default();
        let edges: Vec<_> = sweep.rings().iter().flatten().copied().collect();
        let extent = edges
            .iter()
            .flat_map(|edge| [edge.start(), edge.end()])
            .flatten()
            .map(f64::abs)
            .fold(sweep.height_mm(), f64::max);
        let roundoff = 32. * f64::EPSILON * extent.max(1.);
        let tolerance = tolerance_mm.max(roundoff);
        let clearance = tolerance_mm.max(minimum_mm) + roundoff;
        for opening in openings {
            let checked = certify(sweep, &edges, opening, tolerance, minimum_mm, clearance)?;
            for earlier in &openings_checked {
                if !openings_clear(&checked, earlier, &edges, clearance, roundoff) {
                    return Err(Error::InvalidProfile);
                }
            }
            removed.add(checked.area_mm2() * checked.thickness_mm());
            openings_checked.push(checked);
        }
        let volume_mm3 = sweep.volume_mm3() - removed.value();
        (volume_mm3 > 0.)
            .then(|| Self {
                sweep: sweep.clone(),
                openings: openings_checked,
                volume_mm3,
            })
            .ok_or(Error::InvalidProfile)
    }

    pub fn sweep(&self) -> &Sweep {
        &self.sweep
    }

    pub fn openings(&self) -> &[CertifiedPiercing] {
        &self.openings
    }

    pub fn volume_mm3(&self) -> f64 {
        self.volume_mm3
    }
}

fn certify(
    sweep: &Sweep,
    edges: &[Edge],
    mut opening: Piercing,
    tolerance: f64,
    minimum: f64,
    clearance: f64,
) -> Result<CertifiedPiercing, Error> {
    let invalid = Error::InvalidProfile;
    let [a, b] = opening
        .sides
        .map(|index| edges.get(index).copied().ok_or(invalid).and_then(Side::new));
    let [front, back] = [a?, b?];
    let thickness = dot(front.inward, sub(back.ends[0], front.ends[0]));
    let expected = lift(scale(front.inward, thickness));
    const ANGULAR: f64 = 1e-10;
    if opening
        .origin
        .into_iter()
        .chain(opening.u)
        .chain(opening.v)
        .chain(opening.through)
        .any(|v| !v.is_finite())
        || dot(front.tangent, back.tangent) > -1. + ANGULAR
        || cross(front.tangent, back.tangent).abs() > ANGULAR
        || thickness < minimum
        || (dot(front.inward, sub(back.ends[1], front.ends[0])) - thickness).abs() > tolerance
        || expected
            .into_iter()
            .zip(opening.through)
            .any(|(a, b)| (a - b).abs() > tolerance)
    {
        return Err(invalid);
    }
    let plane_offset = dot(front.inward, sub(in_section(opening.origin), front.ends[0]));
    if plane_offset.abs() > tolerance {
        return Err(invalid);
    }
    opening.origin[0] -= plane_offset * front.inward[0];
    opening.origin[1] -= plane_offset * front.inward[1];
    opening.through = expected;
    let axes = [opening.u, opening.v];
    let [u, v] = axes.map(|axis| [dot(in_section(axis), front.tangent), axis[2]]);
    if axes
        .iter()
        .any(|&axis| dot(front.inward, in_section(axis)).abs() > ANGULAR)
        || (length(u) - 1.).abs() > ANGULAR
        || (length(v) - 1.).abs() > ANGULAR
        || dot(u, v).abs() > ANGULAR
    {
        return Err(invalid);
    }
    let u = scale(u, 1. / length(u));
    let v = scale([-u[1], u[0]], cross(u, v).signum());
    [opening.u, opening.v] =
        [u, v].map(|[run, axial]| [run * front.tangent[0], run * front.tangent[1], axial]);

    let outline = Sweep::new(vec![opening.outline], thickness, tolerance, minimum)?;
    let ring = &outline.rings()[0];
    match ring.as_slice() {
        [Edge::Circle { .. }] => (),
        [
            Edge::Line { .. },
            Edge::Line { .. },
            Edge::Line { .. },
            Edge::Line { .. },
        ] if ring
            .iter()
            .zip(ring.iter().cycle().skip(1))
            .all(|(&a, &b)| {
                let [a, b] = [a, b].map(|edge| sub(edge.end(), edge.start()));
                dot(a, b).abs() <= ANGULAR * length(a) * length(b)
            }) => (),
        _ => return Err(invalid),
    }
    opening.outline = ring.clone();
    let checked = CertifiedPiercing {
        spec: opening,
        area_mm2: outline.area_mm2(),
        centroid: outline.centroid(),
        thickness_mm: thickness,
    };
    let [z_min, z_max] = checked.projection([0., 0., 1.]);
    let [low, high] = checked.projection(lift(front.tangent));
    let inside_side = |side: Side| {
        let [a, b] = side.ends.map(|p| dot(front.tangent, p));
        low - a.min(b) > clearance && a.max(b) - high > clearance
    };
    if z_min <= clearance
        || sweep.height_mm() - z_max <= clearance
        || !inside_side(front)
        || !inside_side(back)
        || !continuous_stock(
            sweep,
            edges,
            checked.spec.sides,
            front,
            [low, high],
            thickness,
            tolerance,
        )
    {
        return Err(invalid);
    }
    Ok(checked)
}

fn continuous_stock(
    sweep: &Sweep,
    edges: &[Edge],
    sides: [usize; 2],
    front: Side,
    [low, high]: [f64; 2],
    thickness: f64,
    tolerance: f64,
) -> bool {
    let offset = dot(front.inward, front.ends[0]);
    let point = |along, depth| {
        add(
            scale(front.tangent, along),
            scale(front.inward, offset + depth),
        )
    };
    let corners = [
        point(low, 0.),
        point(high, 0.),
        point(high, thickness),
        point(low, thickness),
    ];
    let transverse = std::array::from_fn::<_, 4, _>(|i| Edge::Line {
        a: corners[i],
        b: corners[(i + 1) % 4],
    });
    // Crossing either selected boundary is intended. Every other profile
    // edge must remain outside the whole transverse rectangle. Checking an
    // interior point distinguishes a material bridge from empty space.
    let mid = point(low.midpoint(high), thickness / 2.);
    sweep.contains([mid[0], mid[1], sweep.height_mm() / 2.], 0.)
        && edges
            .iter()
            .copied()
            .enumerate()
            .filter(|(i, _)| !sides.contains(i))
            .all(|(_, edge)| {
                !inside(&transverse, edge.start())
                    && transverse
                        .iter()
                        .all(|&segment| clear_pair(segment, edge, &[], tolerance, tolerance))
            })
}

fn openings_clear(
    opening: &CertifiedPiercing,
    earlier: &CertifiedPiercing,
    edges: &[Edge],
    clearance: f64,
    roundoff: f64,
) -> bool {
    let [a, b] = opening.spec.sides;
    if earlier.spec.sides.contains(&a) && earlier.spec.sides.contains(&b) {
        let tangent = Side::new(edges[a]).expect("certified side").tangent;
        let [a, b] = [opening, earlier].map(|opening| opening.projected_outline(tangent));
        return !inside(&a, b[0].start())
            && !inside(&b, a[0].start())
            && a.iter().all(|&a| {
                b.iter()
                    .all(|&b| clear_pair(a, b, &[], clearance, roundoff))
            });
    }
    opening
        .bounds()
        .into_iter()
        .zip(earlier.bounds())
        .any(|([a, b], [c, d])| c - b > clearance || a - d > clearance)
}

#[cfg(test)]
mod tests;
