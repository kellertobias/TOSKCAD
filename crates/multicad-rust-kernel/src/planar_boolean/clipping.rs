use super::*;
use std::cell::Cell;

#[derive(Clone, Copy)]
pub(super) struct Control<'a> {
    pub interrupt: Option<&'a dyn Fn() -> bool>,
    pub uncertain: Option<&'a Cell<bool>>,
}

pub(super) fn observe_snap(distance: f64, tolerance: f64, uncertain: Option<&Cell<bool>>) {
    if distance != 0.
        && distance.abs() <= tolerance
        && let Some(uncertain) = uncertain
    {
        uncertain.set(true);
    }
}

pub(super) fn exterior(
    pieces: &[ConvexPiece],
    tolerance: f64,
    mut budget: usize,
) -> Result<shell::Boundary, Error> {
    exterior_with_budget(pieces, tolerance, &mut budget)
}

pub(super) fn exterior_with_budget(
    pieces: &[ConvexPiece],
    tolerance: f64,
    budget: &mut usize,
) -> Result<shell::Boundary, Error> {
    exterior_with_interrupt(pieces, tolerance, budget, None)
}

pub(super) fn exterior_with_interrupt(
    pieces: &[ConvexPiece],
    tolerance: f64,
    budget: &mut usize,
    interrupt: Option<&dyn Fn() -> bool>,
) -> Result<shell::Boundary, Error> {
    let mut boundary = shell::Boundary::new(tolerance);
    for (source, piece) in pieces.iter().enumerate() {
        for (source_face, (face, &support)) in piece.faces.iter().zip(&piece.planes).enumerate() {
            checkpoint(interrupt)?;
            let face_bounds = bounds(face.iter().copied());
            let mut fragments = vec![face.clone()];
            for (blocker, other) in pieces.iter().enumerate() {
                checkpoint(interrupt)?;
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
                    checkpoint(interrupt)?;
                    partition(
                        polygon,
                        &other.planes,
                        tolerance,
                        &mut remaining,
                        budget,
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
    boundary.split_edges(budget)?;
    Ok(boundary)
}

pub(super) fn bounds_overlap(a: [Point; 2], b: [Point; 2], tolerance: f64) -> bool {
    (0..3).all(|i| a[0][i] <= b[1][i] + tolerance && b[0][i] <= a[1][i] + tolerance)
}

/// Find the coverage by a union of selected cells. Partition each fragment
/// only until one cell owns it, so overlapping cells cannot duplicate faces.
pub(super) fn covered(
    fragments: Vec<Vec<Point>>,
    pieces: &[ConvexPiece],
    face_bounds: [Point; 2],
    tolerance: f64,
    budget: &mut usize,
    eligible: impl Fn(&ConvexPiece) -> bool,
) -> Result<Vec<Vec<Point>>, Error> {
    covered_with_interrupt(
        fragments,
        pieces,
        face_bounds,
        tolerance,
        budget,
        eligible,
        None,
    )
}

pub(super) fn covered_with_interrupt(
    fragments: Vec<Vec<Point>>,
    pieces: &[ConvexPiece],
    face_bounds: [Point; 2],
    tolerance: f64,
    budget: &mut usize,
    eligible: impl Fn(&ConvexPiece) -> bool,
    interrupt: Option<&dyn Fn() -> bool>,
) -> Result<Vec<Vec<Point>>, Error> {
    covered_with_control(
        fragments,
        pieces,
        face_bounds,
        tolerance,
        budget,
        eligible,
        Control {
            interrupt,
            uncertain: None,
        },
    )
}

pub(super) fn covered_with_control(
    mut fragments: Vec<Vec<Point>>,
    pieces: &[ConvexPiece],
    face_bounds: [Point; 2],
    tolerance: f64,
    budget: &mut usize,
    eligible: impl Fn(&ConvexPiece) -> bool,
    control: Control<'_>,
) -> Result<Vec<Vec<Point>>, Error> {
    let mut covered = Vec::new();
    let mut covered_budget = MAX_INCIDENCES;
    for piece in pieces {
        checkpoint(control.interrupt)?;
        if fragments.is_empty() {
            break;
        }
        if !bounds_overlap(face_bounds, piece.bounds, tolerance) || !eligible(piece) {
            continue;
        }
        let mut remaining = Vec::new();
        let mut fragment_budget = MAX_INCIDENCES;
        for ring in fragments {
            checkpoint(control.interrupt)?;
            if let Some(inside) = partition_tracked(
                ring,
                &piece.planes,
                tolerance,
                &mut remaining,
                budget,
                &mut fragment_budget,
                control.uncertain,
            )? {
                charge(&mut covered_budget, inside.len())?;
                covered.push(inside);
            }
        }
        fragments = remaining;
    }
    Ok(covered)
}

pub(super) fn partition(
    polygon: Vec<Point>,
    planes: &[Plane],
    tolerance: f64,
    outside: &mut Vec<Vec<Point>>,
    budget: &mut usize,
    fragment_budget: &mut usize,
) -> Result<Option<Vec<Point>>, Error> {
    partition_tracked(
        polygon,
        planes,
        tolerance,
        outside,
        budget,
        fragment_budget,
        None,
    )
}

pub(super) fn partition_tracked(
    polygon: Vec<Point>,
    planes: &[Plane],
    tolerance: f64,
    outside: &mut Vec<Vec<Point>>,
    budget: &mut usize,
    fragment_budget: &mut usize,
    uncertain: Option<&Cell<bool>>,
) -> Result<Option<Vec<Point>>, Error> {
    // Reject a nonblocking piece before splitting. A contact along only a
    // point or line does not remove any area of the source face.
    charge(budget, planes.len() * polygon.len())?;
    if planes.iter().any(|&plane| {
        let mut positive = false;
        for &point in &polygon {
            let d = plane.distance(point);
            observe_snap(d, tolerance, uncertain);
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
        let (inside, fragment) = split(remaining, plane, tolerance, uncertain);
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

fn split(
    polygon: Vec<Point>,
    plane: Plane,
    tolerance: f64,
    uncertain: Option<&Cell<bool>>,
) -> (Vec<Point>, Vec<Point>) {
    let distance = |point| {
        let d = plane.distance(point);
        observe_snap(d, tolerance, uncertain);
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
