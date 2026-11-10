use super::*;
use crate::warped_union::boss::{face_range, perpendicular_axes};
use std::f64::consts::TAU;

impl Tube {
    fn new(start: Point, end: Point, radius: f64, tolerance: f64) -> Result<Self, Error> {
        if !radius.is_finite()
            || radius <= tolerance
            || start.into_iter().chain(end).any(|p| !p.is_finite())
        {
            return Err(Error::Invalid);
        }
        if radius > 1e8 || start.into_iter().chain(end).any(|p| p.abs() > 1e8) {
            return Err(Error::Limit);
        }
        let delta = sub(end, start);
        let length = length3(delta);
        if length <= 2. * tolerance {
            return Err(Error::Invalid);
        }
        let axis = delta.map(|x| x / length);
        let [u, v] = perpendicular_axes(axis);
        Ok(Self {
            start,
            axes: [u, v, axis],
            length,
            radius,
        })
    }

    fn path(self, shell: &Shell, tolerance: f64) -> Result<Path, Error> {
        let mut mouths = Vec::with_capacity(2);
        for (index, face) in shell.faces.iter().enumerate() {
            if let Some((level, entering, normal)) = self.crossing(face, tolerance) {
                let center = self.point(level);
                let clearance = self.radius + tolerance;
                if disk_in_hole(shell, face, center, clearance) {
                    continue;
                }
                if disk_in_face(shell, face, normal, center, clearance) {
                    mouths.push(Mouth {
                        face: index,
                        level,
                        entering,
                    });
                    continue;
                }
            }
            if !self.clears_face(shell, face, tolerance) {
                return Err(Error::Unsupported);
            }
        }
        mouths.sort_by(|a, b| a.level.total_cmp(&b.level));
        let (entry, exit, reversed) = match *mouths.as_slice() {
            [entry] if entry.entering => (entry, None, false),
            [exit] => (
                Mouth {
                    face: exit.face,
                    level: self.length - exit.level,
                    entering: true,
                },
                None,
                true,
            ),
            [entry, exit]
                if entry.entering && !exit.entering && exit.level - entry.level > tolerance =>
            {
                (entry, Some(exit), false)
            }
            _ => return Err(Error::Unsupported),
        };
        Ok(Path {
            entry,
            exit,
            reversed,
        })
    }

    fn reversed(self) -> Self {
        let [u, v, axis] = self.axes;
        Self {
            start: self.point(self.length),
            axes: [u, v.map(|x| -x), axis.map(|x| -x)],
            ..self
        }
    }

    fn crossing(self, face: &Face, tolerance: f64) -> Option<(f64, bool, Point)> {
        let Support::Plane { normal } = face.support else {
            return None;
        };
        let axis = self.axes[2];
        let tilt = length3(cross(axis, normal));
        if tilt > 64. * f64::EPSILON || tilt * self.radius > tolerance {
            return None;
        }
        let cosine = dot(axis, normal);
        let level = dot(sub(face.point_mm, self.start), normal) / cosine;
        (level > tolerance && self.length - level > tolerance).then_some((
            level,
            cosine < 0.,
            normal,
        ))
    }

    fn clears_face(self, shell: &Shell, face: &Face, tolerance: f64) -> bool {
        let [u, v, axis] = self.axes;
        match face.support {
            Support::Plane { normal } => {
                let [lo, hi] = self.range(normal, face.point_mm);
                if lo > tolerance || hi < -tolerance {
                    return true;
                }
            }
            Support::Cylinder {
                axis_origin_mm,
                axis_direction,
                radius_mm,
                ..
            } => {
                let tilt = length3(cross(axis, axis_direction));
                if tilt <= 64. * f64::EPSILON {
                    let offset = length3(cross(sub(self.start, axis_origin_mm), axis_direction));
                    let reach = self.radius + self.length * tilt + tolerance;
                    if offset + reach < radius_mm || offset - reach > radius_mm {
                        return true;
                    }
                }
            }
        }
        let [lo, hi] = face_range(shell, face, axis, self.start);
        if lo > self.length + tolerance || hi < -tolerance {
            return true;
        }
        let [a, b] = [u, v].map(|direction| {
            let [lo, hi] = face_range(shell, face, direction, self.start);
            lo.max(-hi).max(0.)
        });
        a.hypot(b) > self.radius + tolerance
    }
}

impl<'a> Plan<'a> {
    pub(super) fn prove(
        shell: &Shell,
        spec: &'a AxialCut,
        radial: Option<Point>,
        tolerance: f64,
    ) -> Result<Self, Error> {
        let mut tube = Tube::new(spec.start_mm, spec.end_mm, spec.radius_mm, tolerance)?;
        if let Some(radial) = radial {
            let axis = tube.axes[2];
            let length = length3(radial);
            if !length.is_finite() || (length - 1.).abs() > 1e-10 || dot(radial, axis).abs() > 1e-10
            {
                return Err(Error::Invalid);
            }
            let side = cross(axis, radial);
            let v = side.map(|x| x / length3(side));
            tube.axes = [cross(v, axis), v, axis];
        }
        let path = tube.path(shell, tolerance)?;
        let tube = if path.reversed { tube.reversed() } else { tube };
        let recess = spec
            .recess
            .as_ref()
            .map(|profile| prove_recess(shell, tube, &path, profile, tolerance))
            .transpose()?;
        let mut envelope = tube;
        if let Some((_, extra)) = &recess {
            envelope.radius = envelope.radius.max(extra.radius);
            let axis = tube.axes[2];
            let low = dot(sub(extra.start, tube.start), axis);
            let high = dot(sub(extra.point(extra.length), tube.start), axis);
            let start = low.min(high).min(0.);
            envelope.start = tube.point(start);
            envelope.length = low.max(high).max(tube.length) - start;
        }
        Ok(Self {
            spec,
            tube,
            path,
            recess: recess.map(|(plan, _)| plan),
            envelope,
        })
    }

    pub(super) fn disjoint(&self, other: &Self, tolerance: f64) -> bool {
        let a = self.envelope;
        let b = other.envelope;
        a.axes.into_iter().chain(b.axes).any(|axis| {
            let [lo, hi] = a.range(axis, a.start);
            let [other_lo, other_hi] = b.range(axis, a.start);
            hi + tolerance < other_lo || other_hi + tolerance < lo
        })
    }
}

fn prove_recess(
    shell: &Shell,
    tube: Tube,
    path: &Path,
    recess: &ConvexRecess,
    tolerance: f64,
) -> Result<(RecessPlan, Tube), Error> {
    let exit = path.exit.ok_or(Error::Unsupported)?;
    let count = recess.rim_mm.len();
    if count > MAX_RECESS_SIDES {
        return Err(Error::Limit);
    }
    if count < 3
        || recess.wall_source_faces.len() != count
        || recess
            .rim_mm
            .iter()
            .flatten()
            .any(|v| !v.is_finite() || v.abs() > 1e8)
        || recess.travel_mm.iter().any(|v| !v.is_finite())
    {
        return Err(Error::Invalid);
    }
    let [u, v, axis] = tube.axes;
    let start = dot(sub(recess.rim_mm[0], tube.start), axis);
    let distance = dot(recess.travel_mm, axis);
    if length3(cross(recess.travel_mm, axis)) > tolerance {
        return Err(Error::Unsupported);
    }
    let end = start + distance;
    let lo = start.min(end);
    let hi = start.max(end);
    let from_entry = lo < path.entry.level - tolerance && hi < exit.level - tolerance;
    let from_exit = hi > exit.level + tolerance && lo > path.entry.level + tolerance;
    let [outer, shoulder, host] = if from_entry && hi > path.entry.level + tolerance {
        [lo, hi, path.entry.level]
    } else if from_exit && lo < exit.level - tolerance {
        [hi, lo, exit.level]
    } else {
        return Err(Error::Unsupported);
    };

    let center = tube.point(start);
    let mut polygon = recess
        .rim_mm
        .iter()
        .map(|&point| {
            let delta = sub(point, center);
            (dot(delta, axis).abs() <= tolerance).then(|| [dot(delta, u), dot(delta, v)])
        })
        .collect::<Option<Vec<_>>>()
        .ok_or(Error::Unsupported)?;
    let mut sides = recess.wall_source_faces.clone();
    let mut area = polygon
        .iter()
        .zip(polygon.iter().cycle().skip(1))
        .map(|(&a, &b)| det(a, b))
        .sum::<f64>()
        * 0.5;
    if area < 0. {
        polygon.reverse();
        sides.reverse();
        sides.rotate_left(1);
        area = -area;
    }
    if polygon.iter().enumerate().any(|(i, &p)| {
        polygon[..i].iter().any(|&q| {
            let [x, y] = difference(p, q);
            x.hypot(y) <= tolerance
        })
    }) {
        return Err(Error::Unsupported);
    }
    for (&a, &b) in polygon.iter().zip(polygon.iter().cycle().skip(1)) {
        let edge = difference(b, a);
        let length = edge[0].hypot(edge[1]);
        if length <= tolerance
            || det(edge, a.map(|x| -x)) <= length * (tube.radius + tolerance)
            || polygon
                .iter()
                .any(|&p| det(edge, difference(p, a)) < -tolerance * length)
        {
            return Err(Error::Unsupported);
        }
    }

    let radius = polygon.iter().map(|p| p[0].hypot(p[1])).fold(0., f64::max);
    let wider = Tube::new(tube.point(outer), tube.point(shoulder), radius, tolerance)?;
    let wider_path = wider.path(shell, tolerance)?;
    let face = if from_entry {
        path.entry.face
    } else {
        exit.face
    };
    if wider_path.reversed
        || wider_path.exit.is_some()
        || wider_path.entry.face != face
        || (wider_path.entry.level - (outer - host).abs()).abs() > tolerance
    {
        return Err(Error::Unsupported);
    }
    Ok((
        RecessPlan {
            polygon,
            sides,
            area,
            shoulder,
            from_entry,
            source: recess.source,
            shoulder_source: recess.cap_source_faces[usize::from(shoulder == end)],
        },
        wider,
    ))
}

fn disk_in_face(shell: &Shell, face: &Face, normal: Point, center: Point, clearance: f64) -> bool {
    let in_outer = if let Some(ellipse) = circle(shell, &face.boundary) {
        length3(sub(center, ellipse.center_mm())) + clearance < ellipse.semiaxes_mm()[0]
    } else {
        face.boundary.iter().all(|&use_| {
            let [a, b] = shell.oriented_vertices(use_).map(|i| shell.vertices[i]);
            let inward = cross(normal, sub(b, a));
            shell.edges[use_.edge].curve == Curve::Line
                && dot(sub(center, a), inward) > length3(inward) * clearance
        })
    };
    in_outer
        && face.holes.iter().all(|hole| {
            circle(shell, hole).is_some_and(|ellipse| {
                length3(sub(center, ellipse.center_mm())) > ellipse.semiaxes_mm()[0] + clearance
            })
        })
}

fn disk_in_hole(shell: &Shell, face: &Face, center: Point, clearance: f64) -> bool {
    face.holes.iter().any(|hole| {
        circle(shell, hole).is_some_and(|ellipse| {
            length3(sub(center, ellipse.center_mm())) + clearance < ellipse.semiaxes_mm()[0]
        })
    })
}

fn circle(shell: &Shell, boundary: &[OrientedEdge]) -> Option<Ellipse> {
    let arcs = || {
        boundary
            .iter()
            .filter_map(|edge| match shell.edges[edge.edge].curve {
                Curve::Ellipse(arc) => Some((arc, edge.forward)),
                Curve::Line => None,
            })
    };
    let (first, _) = arcs().next()?;
    let ellipse = first.ellipse();
    let [major, minor] = ellipse.semiaxes_mm();
    if major != minor || arcs().count() != boundary.len() {
        return None;
    }
    let mut sweep = 0.;
    for (arc, forward) in arcs() {
        let e = arc.ellipse();
        if e.center_mm() != ellipse.center_mm() || e.semiaxes_mm() != [major, minor] {
            return None;
        }
        let same_axis = dot(e.normal(), ellipse.normal());
        if length3(cross(e.normal(), ellipse.normal())) > 64. * f64::EPSILON {
            return None;
        }
        sweep += same_axis.signum()
            * if forward {
                arc.sweep_rad()
            } else {
                -arc.sweep_rad()
            };
    }
    ((sweep.abs() - TAU).abs() <= 64. * f64::EPSILON).then_some(ellipse)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::warped_union::axial_cut::tests::{attached_stock, rail_cut};

    const TOL: f64 = 1e-7;

    #[test]
    fn parallel_cylinders_require_radial_or_axial_separation() {
        let shell = attached_stock();
        let face = shell
            .faces()
            .iter()
            .find(|face| matches!(face.support, Support::Cylinder { .. }))
            .unwrap();
        let clearance = |lo, hi| {
            Tube::new([lo, 20., 20.], [hi, 20., 20.], 6., TOL)
                .unwrap()
                .clears_face(&shell, face, TOL)
        };
        assert!(clearance(20., 30.));
        assert!(!clearance(11., 14.));
        let across = Tube::new([20., 0., 20.], [20., 30., 20.], 1., TOL).unwrap();
        assert!(across.clears_face(&shell, face, TOL));
    }

    #[test]
    fn recess_proof_checks_the_corresponding_round_mouth() {
        let shell = attached_stock();
        let cut = rail_cut(20., 2);
        let tube = Tube::new(cut.start_mm, cut.end_mm, cut.radius_mm, TOL).unwrap();
        let mut path = tube.path(&shell, TOL).unwrap();
        path.exit.as_mut().unwrap().face = usize::MAX;
        assert!(matches!(
            prove_recess(&shell, tube, &path, cut.recess.as_ref().unwrap(), TOL),
            Err(Error::Unsupported)
        ));
    }

    #[test]
    fn an_existing_clearance_wire_must_be_one_complete_circle() {
        let mut shell = attached_stock();
        let face = shell.faces().iter().find(|f| f.holes.len() == 2).unwrap();
        let first = face.holes[0].clone();
        let second = face.holes[1][0].edge;
        let altered = first[1].edge;
        let original = shell.edges[altered].curve;
        let ellipse = circle(&shell, &first).unwrap();
        assert!(circle(&shell, &first[..1]).is_none());

        shell.edges[altered].curve = Curve::Line;
        assert!(circle(&shell, &first).is_none());
        shell.edges[altered].curve = shell.edges[second].curve;
        assert!(circle(&shell, &first).is_none());
        let tilted = Ellipse::new(
            ellipse.center_mm(),
            [0., 1., 0.],
            [0., 0., 1.],
            ellipse.semiaxes_mm(),
        )
        .unwrap();
        shell.edges[altered].curve =
            Curve::Ellipse(EllipseArc::new(tilted, 0., FRAC_PI_2).unwrap());
        assert!(circle(&shell, &first).is_none());
        shell.edges[altered].curve = original;
        assert_eq!(circle(&shell, &first), Some(ellipse));
    }
}
