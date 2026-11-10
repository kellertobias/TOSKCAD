use super::*;

impl Union {
    pub(crate) fn verify_boundary(rings: &[Vec<Point>], tolerance: f64) -> Result<Self, Error> {
        validate_tolerance(tolerance)?;
        let mut boundary = shell::Boundary::new(tolerance);
        let mut budget = MAX_WORK;
        let mut bounds_mm = Vec::new();
        for (ordinal, ring) in rings.iter().enumerate() {
            if ring.len() > crate::polygon::MAX_VERTICES
                || ring.iter().flatten().any(|v| v.abs() > MAX_COORDINATE)
            {
                return Err(Error::Limit);
            }
            if ring.len() < 3 || ring.iter().flatten().any(|v| !v.is_finite()) {
                return Err(Error::Invalid);
            }
            charge(&mut budget, ring.len() * ring.len())?;
            let plane = supporting_plane(ring, tolerance)?;
            boundary.add(ring, plane.normal, 0, ordinal)?;
            bounds_mm.push((bounds(ring.iter().copied()), plane));
        }
        let proof = boundary.prove()?;
        if proof.solid_count != 1 || proof.shell_count != 1 {
            return Err(Error::Invalid);
        }
        // Two convex faces meet only at the vertices/edge already shared by
        // the certified topology, or they are disjoint. This tests the actual
        // planar chart before any curved patch is admitted.
        for (index, face) in boundary.faces.iter().enumerate() {
            charge(&mut budget, index)?;
            for (other_index, other) in boundary.faces[..index].iter().enumerate() {
                let (aabb, plane) = bounds_mm[index];
                let (other_aabb, other_plane) = bounds_mm[other_index];
                if (0..3).any(|i| {
                    aabb[0][i] > other_aabb[1][i] + tolerance
                        || other_aabb[0][i] > aabb[1][i] + tolerance
                }) {
                    continue;
                }
                charge(&mut budget, face.vertices.len() * other.vertices.len() * 2)?;
                let a = Polygon {
                    plane,
                    ring: &face.vertices,
                    points: &boundary.vertices,
                };
                let b = Polygon {
                    plane: other_plane,
                    ring: &other.vertices,
                    points: &boundary.vertices,
                };
                verify_pair(a, b, tolerance)?;
            }
        }
        let bounds_mm = bounds(boundary.vertices.iter().copied());
        Ok(Self {
            vertices: boundary.vertices,
            faces: boundary.faces,
            bounds: bounds_mm,
            volume_mm3: proof.volume_mm3,
            solid_count: proof.solid_count,
            shell_count: proof.shell_count,
        })
    }
}

#[derive(Clone, Copy)]
struct Polygon<'a> {
    plane: Plane,
    ring: &'a [usize],
    points: &'a [Point],
}

impl Polygon<'_> {
    fn vertices(self) -> impl Iterator<Item = Point> + Clone {
        self.ring.iter().map(|&id| self.points[id])
    }

    fn edges(self) -> impl Iterator<Item = [Point; 2]> {
        self.vertices()
            .zip(self.vertices().cycle().skip(1))
            .map(|(a, b)| [a, b])
    }

    fn section(
        self,
        cut: Plane,
        origin: Point,
        direction: Point,
        tolerance: f64,
    ) -> Option<[f64; 2]> {
        let project = |point| dot(sub(point, origin), direction);
        let mut extent = [f64::INFINITY, f64::NEG_INFINITY];
        let mut include = |point| {
            let x = project(point);
            extent = [extent[0].min(x), extent[1].max(x)];
        };
        for [a, b] in self.edges() {
            let d = cut.distance(a);
            let e = cut.distance(b);
            if d.abs() <= tolerance {
                include(a);
            }
            if (d > tolerance && e < -tolerance) || (d < -tolerance && e > tolerance) {
                include(add(a, scale(sub(b, a), d / (d - e))));
            }
        }
        (extent[0] <= extent[1]).then_some(extent)
    }
}

fn verify_pair(a: Polygon<'_>, b: Polygon<'_>, tolerance: f64) -> Result<(), Error> {
    let axis = cross(a.plane.normal, b.plane.normal);
    let sine = norm(axis);
    if sine > 1e-10 {
        let direction = scale(axis, 1. / sine);
        // Project both cut intervals using the same point even when the
        // supporting planes were constructed at different source vertices.
        return verify_overlap([a, b], [b.plane, a.plane], direction, tolerance);
    }
    let separated = |outer: Polygon<'_>, other: Polygon<'_>| {
        let mut distances = other.vertices().map(|p| outer.plane.distance(p));
        distances.clone().all(|d| d > tolerance) || distances.all(|d| d < -tolerance)
    };
    if separated(a, b) || separated(b, a) {
        return Ok(());
    }
    // A tiny angle can still cross over a large face. Treat as coplanar
    // only when both entire convex rings fit the other's plane.
    if a.vertices().any(|p| b.plane.distance(p).abs() > tolerance)
        || b.vertices().any(|p| a.plane.distance(p).abs() > tolerance)
    {
        return Err(Error::Unresolved);
    }
    for (outside, inside) in [(a, b), (b, a)] {
        for [start, end] in inside.edges() {
            let tangent = sub(end, start);
            let direction = scale(tangent, 1. / norm(tangent));
            let cut = Plane {
                point: start,
                normal: cross(inside.plane.normal, direction),
            };
            if outside
                .vertices()
                .all(|point| cut.distance(point) <= tolerance)
            {
                return verify_overlap([outside, inside], [cut, cut], direction, tolerance);
            }
        }
    }
    Err(Error::Invalid)
}

fn verify_overlap(
    [a, b]: [Polygon<'_>; 2],
    [cut_a, cut_b]: [Plane; 2],
    direction: Point,
    tolerance: f64,
) -> Result<(), Error> {
    let origin = cut_a.point;
    let (Some([a0, a1]), Some([b0, b1])) = (
        a.section(cut_a, origin, direction, tolerance),
        b.section(cut_b, origin, direction, tolerance),
    ) else {
        return Ok(());
    };
    let [lo, hi] = [a0.max(b0), a1.min(b1)];
    if lo > hi + tolerance {
        return Ok(());
    }
    let cuts_interior = |face: Polygon<'_>, plane: Plane| {
        face.vertices().any(|p| plane.distance(p) > tolerance)
            && face.vertices().any(|p| plane.distance(p) < -tolerance)
    };
    if hi - lo > tolerance && (cuts_interior(a, cut_a) || cuts_interior(b, cut_b)) {
        return Err(Error::Invalid);
    }
    let position = |point| dot(sub(point, origin), direction);
    let on_line = |point| {
        cut_a.distance(point).abs() <= tolerance && cut_b.distance(point).abs() <= tolerance
    };
    // Every end of the intersection must be a shared vertex. A nonzero
    // overlap must also follow the same subdivision in both face loops.
    for end in [lo, hi] {
        if !a.ring.iter().filter(|id| b.ring.contains(id)).any(|&id| {
            let point = a.points[id];
            on_line(point) && (position(point) - end).abs() <= tolerance
        }) {
            return Err(Error::Invalid);
        }
    }
    for (one, other) in [(a, b), (b, a)] {
        for &id in one.ring {
            let point = one.points[id];
            let t = position(point);
            if t >= lo - tolerance
                && t <= hi + tolerance
                && on_line(point)
                && !other.ring.contains(&id)
            {
                return Err(Error::Invalid);
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
