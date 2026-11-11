use super::*;
use std::f64::consts::{PI, TAU};

use crate::spline_sweep::{ConstructionError, validation::*};
fn certificate_error(error: ConstructionError) -> ReadError {
    match error {
        ConstructionError::Invalid => ReadError::Invalid,
        ConstructionError::Limit => ReadError::Limit,
        ConstructionError::Unresolved | ConstructionError::Cancelled => ReadError::Unsupported,
    }
}

pub(super) fn verify(
    reader: &Shell<'_, '_>,
    edges: &Edges,
    plane: &Plane,
    face: &FaceWire,
) -> Result<f64> {
    if face.outer.len() < 3
        || face
            .outer
            .iter()
            .map(|edge| edge.start)
            .collect::<BTreeSet<_>>()
            .len()
            != face.outer.len()
        || !face
            .outer
            .iter()
            .any(|edge| matches!(edges[&edge.edge].geometry, Geometry::Cubic { .. }))
    {
        return Err(ReadError::Unsupported);
    }
    let tolerance = reader.tolerance;
    let section = face
        .outer
        .iter()
        .map(|edge| boundary(reader, &edges[&edge.edge].geometry, plane, edge))
        .collect::<Result<Vec<_>>>()?;
    let mut budget = Budget {
        remaining: MAX_PAIR_WORK,
        interrupt: None,
    };
    simple(&section, tolerance, &mut budget).map_err(certificate_error)?;
    let mut area = face
        .outer
        .iter()
        .map(|edge| section_area(reader, &edges[&edge.edge].geometry, plane, edge))
        .sum::<Result<f64>>()?;
    if let Some(hole) = &face.hole {
        let [edge] = hole.as_slice() else {
            return Err(ReadError::Unsupported);
        };
        let Geometry::Analytic(Curve::Circle(circle)) = edges[&edge.edge].geometry else {
            return Err(ReadError::Unsupported);
        };
        if !circle.within_plane(plane, tolerance)
            || (dot(circle.normal, plane.normal) > 0.) == edge.forward
        {
            return Err(ReadError::Invalid);
        }
        let center = project(plane, circle.center_mm);
        let winding: f64 = section
            .iter()
            .flat_map(|edge| &edge.pieces)
            .map(|&piece| clear_disk(piece, center, circle.radius_mm + tolerance, 0, &mut budget))
            .sum::<std::result::Result<f64, ConstructionError>>()
            .map_err(certificate_error)?;
        if (winding - TAU).abs() > 1e-7 {
            return Err(ReadError::Invalid);
        }
        area -= PI * circle.radius_mm.powi(2);
    }
    if !area.is_finite() || area <= tolerance.powi(2) {
        return Err(ReadError::Invalid);
    }
    Ok(area)
}

fn section_area(
    reader: &Shell<'_, '_>,
    geometry: &Geometry,
    plane: &Plane,
    edge: &Traversal,
) -> Result<f64> {
    let [a, b] = [edge.start, edge.end].map(|v| project(plane, reader.points[&v]));
    match geometry {
        Geometry::Analytic(Curve::Line) => Ok(cross2(a, b) / 2.),
        Geometry::Analytic(curve) => {
            let round = round_geometry(*curve).ok_or(ReadError::Invalid)?;
            let center = project(plane, round.center);
            let theta = directed_sweep(round, plane, edge.forward);
            Ok((cross2(center, sub2(b, a)) + round.radius.powi(2) * theta) / 2.)
        }
        Geometry::Cubic { curve, .. } => {
            let signed = curve.signed_area(plane.origin, plane.normal);
            Ok(if edge.forward { signed } else { -signed })
        }
    }
}

fn boundary(
    reader: &Shell<'_, '_>,
    geometry: &Geometry,
    plane: &Plane,
    edge: &Traversal,
) -> Result<Boundary> {
    let points = [edge.start, edge.end].map(|id| reader.points[&id]);
    let tolerance = reader.tolerance;
    let on_plane = |p: Point| dot(sub(p, plane.origin), plane.normal).abs() <= tolerance;
    if !points.into_iter().all(on_plane) {
        return Err(ReadError::Invalid);
    }
    let [a, b] = points.map(|p| project(plane, p));
    let pieces = match geometry {
        Geometry::Analytic(Curve::Line) => vec![Piece::Line([a, b])],
        Geometry::Analytic(curve) => {
            let round = round_geometry(*curve).ok_or(ReadError::Invalid)?;
            if round.full
                || !on_plane(round.center)
                || length(cross(round.normal, plane.normal)) * round.radius > tolerance
            {
                return Err(ReadError::Invalid);
            }
            let center = project(plane, round.center);
            let [x, y] = sub2(a, center);
            let arc = Arc {
                center,
                radius: round.radius,
                start: y.atan2(x),
                sweep: directed_sweep(round, plane, edge.forward),
            };
            vec![Piece::Arc(arc)]
        }
        Geometry::Cubic { curve, .. } => {
            if !curve.poles_mm().iter().copied().all(on_plane) {
                return Err(ReadError::Invalid);
            }
            let mut spans: Vec<_> = curve
                .bezier_spans()
                .map(|span| span.map(|p| project(plane, p)))
                .collect();
            if !edge.forward {
                spans.reverse();
                for span in &mut spans {
                    span.reverse();
                }
            }
            let chord = unit2(sub2(b, a)).ok_or(ReadError::Invalid)?;
            if spans
                .iter()
                .any(|span| !Piece::Bezier(*span).forward_along(chord))
            {
                return Err(ReadError::Unsupported);
            }
            spans.into_iter().map(Piece::Bezier).collect()
        }
    };
    let bounds = pieces
        .iter()
        .map(|p| p.bounds())
        .reduce(merge)
        .ok_or(ReadError::Invalid)?;
    Ok(Boundary { pieces, bounds })
}

fn directed_sweep(round: Round, plane: &Plane, forward: bool) -> f64 {
    if (dot(round.normal, plane.normal) > 0.) == forward {
        round.sweep
    } else {
        -round.sweep
    }
}

fn project(plane: &Plane, p: Point) -> Point2 {
    let d = sub(p, plane.origin);
    [dot(d, plane.u), dot(d, plane.v)]
}

#[cfg(test)]
mod error_tests {
    use super::*;
    #[test]
    fn section_certificate_keeps_typed_decoder_failure_contract() {
        for (source, expected) in [
            (ConstructionError::Invalid, ReadError::Invalid),
            (ConstructionError::Limit, ReadError::Limit),
            (ConstructionError::Unresolved, ReadError::Unsupported),
            (ConstructionError::Cancelled, ReadError::Unsupported),
        ] {
            assert_eq!(certificate_error(source), expected);
        }
    }
}
