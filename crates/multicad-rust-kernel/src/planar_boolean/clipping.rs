use super::*;

pub(super) fn exterior(
    pieces: &[ConvexPiece],
    tolerance: f64,
    mut budget: usize,
) -> Result<shell::Boundary, Error> {
    let mut boundary = shell::Boundary::new(tolerance);
    for (source, piece) in pieces.iter().enumerate() {
        for (source_face, (face, &support)) in piece.faces.iter().zip(&piece.planes).enumerate() {
            let face_bounds = bounds(face.iter().copied());
            let mut fragments = vec![face.clone()];
            for (blocker, other) in pieces.iter().enumerate() {
                if source == blocker || !bounds_overlap(face_bounds, other.bounds, tolerance) {
                    continue;
                }
                // A same-facing coincident patch belongs to the earlier piece.
                // Opposite-facing patches are internal and disappear on both sides.
                if blocker > source
                    && other.planes.iter().any(|&plane| {
                        support.same_direction(plane, tolerance)
                            && face.iter().all(|&p| plane.distance(p).abs() <= tolerance)
                    })
                {
                    continue;
                }
                let mut remaining = Vec::new();
                let mut fragment_budget = MAX_INCIDENCES;
                for polygon in fragments {
                    partition(
                        polygon,
                        &other.planes,
                        tolerance,
                        &mut remaining,
                        &mut budget,
                        &mut fragment_budget,
                    )?;
                }
                fragments = remaining;
                if fragments.is_empty() {
                    break;
                }
            }
            for polygon in fragments {
                boundary.add(&polygon, support.normal, source, source_face)?;
            }
        }
    }
    boundary.split_edges(&mut budget)?;
    Ok(boundary)
}

pub(super) fn bounds_overlap(a: [Point; 2], b: [Point; 2], tolerance: f64) -> bool {
    (0..3).all(|i| a[0][i] <= b[1][i] + tolerance && b[0][i] <= a[1][i] + tolerance)
}

pub(super) fn partition(
    polygon: Vec<Point>,
    planes: &[Plane],
    tolerance: f64,
    outside: &mut Vec<Vec<Point>>,
    budget: &mut usize,
    fragment_budget: &mut usize,
) -> Result<Option<Vec<Point>>, Error> {
    // Reject a nonblocking piece before splitting. A contact along only a
    // point or line does not remove any area of the source face.
    charge(budget, planes.len() * polygon.len())?;
    if planes.iter().any(|&plane| {
        let mut positive = false;
        for &point in &polygon {
            let d = plane.distance(point);
            if d < -tolerance {
                return false;
            }
            positive |= d > tolerance;
        }
        positive
    }) {
        charge(fragment_budget, polygon.len())?;
        outside.push(polygon);
        return Ok(None);
    }

    let mut remaining = polygon;
    for &plane in planes {
        charge(budget, remaining.len())?;
        let (inside, fragment) = split(remaining, plane, tolerance);
        if fragment.len() >= 3 {
            charge(fragment_budget, fragment.len())?;
            outside.push(fragment);
        }
        if inside.len() < 3 {
            return Ok(None);
        }
        remaining = inside;
    }
    Ok(Some(remaining))
}

fn split(polygon: Vec<Point>, plane: Plane, tolerance: f64) -> (Vec<Point>, Vec<Point>) {
    let distance = |point| {
        let d = plane.distance(point);
        if d.abs() <= tolerance { 0. } else { d }
    };
    let (negative, positive) = polygon
        .iter()
        .copied()
        .fold((false, false), |(neg, pos), point| {
            let d = distance(point);
            (neg || d < 0., pos || d > 0.)
        });
    if !positive {
        return (polygon, Vec::new());
    }
    if !negative {
        return (Vec::new(), polygon);
    }
    let mut inside = Vec::new();
    let mut outside = Vec::new();
    for (&start, &end) in polygon.iter().zip(polygon.iter().cycle().skip(1)) {
        let a = distance(start);
        let b = distance(end);
        if a <= 0. {
            inside.push(start);
        }
        if a >= 0. {
            outside.push(start);
        }
        if (a < 0. && b > 0.) || (a > 0. && b < 0.) {
            let crossing = add(start, scale(sub(end, start), a / (a - b)));
            inside.push(crossing);
            outside.push(crossing);
        }
    }
    (inside, outside)
}
