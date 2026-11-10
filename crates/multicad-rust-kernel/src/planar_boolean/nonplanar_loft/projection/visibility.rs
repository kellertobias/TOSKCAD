use super::*;

struct Wall {
    surface: [Point; 4],
    bounds: [Paper; 2],
}

impl Wall {
    fn new([a, b, c, d]: [Point; 4]) -> Self {
        let points = [a, b, c, d].map(paper);
        Self {
            surface: [a, sub(b, a), sub(c, a), sub(sub(d, c), sub(b, a))],
            bounds: [
                std::array::from_fn(|i| points.iter().map(|p| p[i]).fold(f64::INFINITY, f64::min)),
                std::array::from_fn(|i| {
                    points
                        .iter()
                        .map(|p| p[i])
                        .fold(f64::NEG_INFINITY, f64::max)
                }),
            ],
        }
    }

    fn at(&self, [u, t]: Paper) -> Point {
        let [a, b, c, d] = self.surface;
        add(add(a, scale(b, u)), scale(add(c, scale(d, u)), t))
    }

    fn silhouette(&self, epsilon: f64) -> Option<Curve> {
        let [_, b, c, d] = self.surface.map(paper);
        // A bilinear patch has an affine projected Jacobian determinant.
        let [constant, along_u, along_t] =
            [determinant(b, c), determinant(b, d), determinant(d, c)];
        let area_error = epsilon * length(b).max(length(c)).max(length(d));
        if along_u.abs().max(along_t.abs()) <= area_error {
            return None;
        }
        let mut hits: Vec<Paper> = Vec::new();
        for (axis, coefficients) in [[along_u, along_t], [along_t, along_u]]
            .into_iter()
            .enumerate()
        {
            if coefficients[1].abs() <= area_error {
                continue;
            }
            for side in [0., 1.] {
                let hit = -(coefficients[0].mul_add(side, constant)) / coefficients[1];
                if !(-1e-12..=1. + 1e-12).contains(&hit) {
                    continue;
                }
                let mut point = [side; 2];
                point[1 - axis] = hit.clamp(0., 1.);
                if hits.iter().all(|&p| length(minus(p, point)) > 1e-12) {
                    hits.push(point);
                }
            }
        }
        let [a, b] = hits.as_slice() else { return None };
        let mid = std::array::from_fn(|i| a[i].midpoint(b[i]));
        if mid.into_iter().any(|t| t <= 1e-12 || t >= 1. - 1e-12) {
            return None;
        }
        let [p, m, q] = [*a, mid, *b].map(|uv| self.at(uv));
        Some(Curve::through_midpoint(p, m, q))
    }

    fn blocks(&self, point: Point, near: f64, tolerance: f64) -> bool {
        let xy = paper(point);
        if !overlap(self.bounds, [xy; 2], tolerance) {
            return false;
        }
        let [a, b, c, d] = self.surface;
        let delta = minus(xy, paper(a));
        let [x, y, z] = [b, c, d].map(paper);
        // A ray through the paper point must meet a projected straight
        // ruling. Eliminating its edge parameter leaves a quadratic.
        let coefficients = [
            determinant(delta, x),
            determinant(delta, z) - determinant(y, x),
            -determinant(y, z),
        ];
        let hits = ray_roots(coefficients);
        for t in hits.into_iter().flatten() {
            if !(-1e-12..=1. + 1e-12).contains(&t) {
                continue;
            }
            let t = t.clamp(0., 1.);
            let origin = add(a, scale(c, t));
            let edge = add(b, scale(d, t));
            let distance = minus(xy, paper(origin));
            let along = paper(edge);
            let magnitude = length(along);
            let u = if magnitude <= tolerance {
                if length(distance) > tolerance {
                    continue;
                }
                f64::from(edge[2] > 0.)
            } else {
                let u = (distance[0].mul_add(along[0], distance[1] * along[1]))
                    / (magnitude * magnitude);
                if u * magnitude < -tolerance || (u - 1.) * magnitude > tolerance {
                    continue;
                }
                u.clamp(0., 1.)
            };
            let depth = edge[2].mul_add(u, origin[2]);
            if depth > point[2] + tolerance
                && depth <= near + tolerance
                && length(minus(distance, along.map(|value| value * u))) <= tolerance
            {
                return true;
            }
        }
        false
    }
}

fn ray_roots([c, b, a]: [f64; 3]) -> [Option<f64>; 2] {
    if a == 0. {
        return if b == 0. {
            // A wall projected edge-on has depth extrema on its trim.
            // Adjoining faces resolve visibility along that common edge.
            [None, None]
        } else {
            [Some(-c / b), None]
        };
    }
    let product = 4. * a * c;
    let disc = b.mul_add(b, -product);
    let roundoff = 16. * f64::EPSILON * (b * b + product.abs());
    if disc < -roundoff {
        return [None, None];
    }
    if disc <= roundoff {
        return [Some(-b / (2. * a)), None];
    }
    let q = -0.5 * (b + disc.sqrt().copysign(b));
    [Some(q / a), Some(c / q)]
}

struct Cap<'a> {
    ring: &'a [Point],
    normal: Point,
}

impl<'a> Cap<'a> {
    fn new(ring: &'a [Point]) -> Self {
        let a = sub(ring[1], ring[0]);
        let b = sub(ring[2], ring[1]);
        let normal = cross(a, b);
        let normal = scale(normal, norm(normal).recip());
        Self { ring, normal }
    }

    fn blocks(&self, point: Point, near: f64, tolerance: f64) -> bool {
        let n = self.normal;
        if n[2].abs() <= 1e-12 {
            return false;
        }
        let height = -dot(n, sub(point, self.ring[0])) / n[2];
        if height <= tolerance || height + point[2] > near + tolerance {
            return false;
        }
        self.ring
            .iter()
            .zip(self.ring.iter().cycle().skip(1))
            .all(|(&a, &b)| {
                let edge = minus(paper(b), paper(a));
                determinant(edge, minus(paper(point), paper(a))) * n[2].signum()
                    >= -tolerance * length(edge)
            })
    }
}

pub(super) fn project(
    rings: [Vec<Point>; 2],
    options: LoftProjection,
    epsilon: f64,
    clip: Option<clip::Clip<'_>>,
    mut curves: Vec<Curve>,
) -> Result<Vec<LoftDrawingEdge>, Error> {
    let [first, last] = &rings;
    let n = first.len();
    curves.reserve(4 * n);
    let mut walls = Vec::with_capacity(n);
    for i in 0..n {
        let j = (i + 1) % n;
        let corners = [first[i], first[j], last[i], last[j]];
        curves.extend([
            Curve::line(corners[0], corners[1]),
            Curve::line(corners[2], corners[3]),
            Curve::line(corners[0], corners[2]),
        ]);
        let wall = Wall::new(corners);
        if let Some(silhouette) = wall.silhouette(epsilon) {
            curves.push(silhouette);
        }
        walls.push(wall);
    }
    curves.retain(|curve| length(minus(paper(curve.at(0.)), paper(curve.at(1.)))) > epsilon);
    let bounds: Vec<_> = curves.iter().map(|curve| curve.bounds()).collect();
    let caps = [Cap::new(first), Cap::new(last)];
    let near = clip.as_ref().map_or(f64::INFINITY, clip::Clip::near);
    let blocked = |point| {
        clip.as_ref()
            .is_some_and(|clip| clip.blocks(point, options.tolerance))
            || caps
                .iter()
                .any(|cap| cap.blocks(point, near, options.tolerance))
            || walls
                .iter()
                .any(|wall| wall.blocks(point, near, options.tolerance))
    };
    let mut output = Vec::new();
    let mut cuts = Vec::new();
    for (index, &curve) in curves.iter().enumerate() {
        cuts.clear();
        cuts.extend([0., 1.]);
        for (other_index, &other) in curves.iter().enumerate() {
            if index != other_index && overlap(bounds[index], bounds[other_index], epsilon) {
                curve.intersections(other, epsilon, &mut cuts);
            }
        }
        if let Some((center, radius)) = options.detail {
            curve.detail_crossings(center, radius, &mut cuts);
        }
        if let Some(clip) = &clip {
            clip.crossings(curve, &mut cuts);
        }
        cuts.sort_unstable_by(f64::total_cmp);
        cuts.dedup_by(|a, b| (*a - *b).abs() <= 1e-12);
        let mut runs: Vec<([f64; 2], bool)> = Vec::new();
        for pair in cuts.windows(2) {
            let [a, b] = [pair[0], pair[1]];
            let point = curve.at(a.midpoint(b));
            if options
                .detail
                .is_some_and(|(center, radius)| length(minus(paper(point), center)) > radius)
            {
                continue;
            }
            if clip
                .as_ref()
                .is_some_and(|clip| !clip.includes(point, epsilon))
            {
                continue;
            }
            let hidden = blocked(point);
            if let Some((span, _)) = runs
                .last_mut()
                .filter(|(span, old)| *old == hidden && span[1] == a)
            {
                span[1] = b;
            } else if !hidden || options.hidden_lines {
                runs.push(([a, b], hidden));
            }
        }
        for (span, hidden) in runs {
            if let Some(edge) = curve
                .slice(span)
                .publish(hidden, epsilon, options.tolerance)
            {
                if output.len() >= options.edge_limit {
                    return Err(Error::Limit);
                }
                output.push(edge);
            }
        }
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ray_through_a_collapsed_ruling_uses_its_front_end_and_checks_the_hit() {
        let wall = Wall::new([[0., 0., 0.], [1., 0., 1.], [0., 1., 0.], [-1., 1., 1.]]);
        let tolerance = 1e-7;
        assert!(wall.blocks([0., 0.5, 0.2], f64::INFINITY, tolerance));
        assert!(!wall.blocks([0.25, 0.5, 0.2], f64::INFINITY, tolerance));
        let fold = wall.silhouette(tolerance).unwrap();
        assert_eq!(paper(fold.at(0.)), [0., 0.5]);
        assert_eq!(paper(fold.at(1.)), [0., 0.5]);
        assert!(fold.publish(false, tolerance, tolerance).is_none());
    }

    #[test]
    fn projected_boundaries_and_edge_on_walls_add_no_interior_silhouette() {
        let boundary = Wall::new([[0., 0., 0.], [1., 0., 1.], [0., 1., 0.], [0., 1., 1.]]);
        assert!(boundary.silhouette(1e-7).is_none());
        let planar = Wall::new([[0., 0., 0.], [1., 0., 0.], [0., 0., 1.], [1., 0., 1.]]);
        assert!(planar.silhouette(1e-7).is_none());
        assert!(!planar.blocks([0.5, 0., 0.], f64::INFINITY, 1e-7));
        assert_eq!(ray_roots([-0.5, 1., 0.]), [Some(0.5), None]);
        assert_eq!(ray_roots([0.25, -1., 1.]), [Some(0.5), None]);
    }
}
