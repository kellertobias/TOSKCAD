use super::*;
use crate::{normal_sweep, spline_sweep};

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct DecodedSplineSweep {
    pub origin_mm: Point,
    pub u: Point,
    pub v: Point,
    pub sweep: spline_sweep::Sweep,
}

pub(in crate::step_read) fn import_member(
    reader: &mut Shell<'_, '_>,
    faces: Vec<u32>,
) -> Result<(Readback, DecodedSplineSweep)> {
    let (readback, solid) = read_member_with(reader, faces, reconstruct)?;
    let solid = solid.ok_or(ReadError::Unsupported)?;
    compare_volume(
        solid.sweep.area_mm2(),
        solid.sweep.height_mm(),
        readback.volume_mm3,
        reader.tolerance,
    )
    .map(|()| (readback, solid))
}

fn compare_volume(area: f64, height: f64, volume: f64, tolerance: f64) -> Result<()> {
    if (area * height - volume).abs() > tolerance * area.sqrt().max(1.) * height * 32. {
        return Err(ReadError::Invalid);
    }
    Ok(())
}

fn reconstruct(
    reader: &Shell<'_, '_>,
    edges: &Edges,
    base: &Plane,
    low: &FaceWire,
    axis: Point,
    height: f64,
) -> Result<DecodedSplineSweep> {
    if height * length(cross(axis, base.normal)) > reader.tolerance {
        return Err(ReadError::Invalid);
    }
    let u = base.u;
    let v = base.v.map(|value| -value);
    let project = |point| {
        let relative = sub(point, base.origin);
        [dot(relative, u), dot(relative, v)]
    };
    // The lower cap is outward facing. Reverse its traversals for the
    // positive sweep frame and retain the source pole and knot order.
    let outline = low
        .outer
        .iter()
        .rev()
        .map(|edge| {
            let [a, b] = [edge.end, edge.start].map(|id| project(reader.points[&id]));
            let direction = !edge.forward;
            Ok(match &edges[&edge.edge].geometry {
                Geometry::Analytic(Curve::Line) => {
                    spline_sweep::Edge::Analytic(normal_sweep::Edge::Line { a, b })
                }
                Geometry::Analytic(curve) => {
                    let round = round_geometry(*curve).ok_or(ReadError::Invalid)?;
                    spline_sweep::Edge::Analytic(normal_sweep::Edge::Arc {
                        center: project(round.center),
                        a,
                        b,
                        clockwise: (dot(round.normal, axis) > 0.) != direction,
                    })
                }
                Geometry::Cubic { curve, .. } => spline_sweep::Edge::Cubic(
                    spline_sweep::Cubic::new(
                        curve.poles_mm().iter().copied().map(project).collect(),
                        curve.knots().to_vec(),
                        !direction,
                    )
                    .map_err(body_error)?,
                ),
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let hole = low
        .hole
        .as_ref()
        .map(|hole| {
            // Section verification allows only a singleton full circular hole.
            let Geometry::Analytic(Curve::Circle(circle)) = edges[&hole[0].edge].geometry else {
                return Err(ReadError::Invalid);
            };
            Ok(normal_sweep::Edge::Circle {
                center: project(circle.center_mm),
                radius_mm: circle.radius_mm,
                clockwise: true,
            })
        })
        .transpose()?;
    let sweep = spline_sweep::Sweep::from_verified(outline, hole, height, reader.tolerance)
        .map_err(body_error)?;
    Ok(DecodedSplineSweep {
        origin_mm: base.origin,
        u,
        v,
        sweep,
    })
}

fn body_error(error: spline_sweep::Error) -> ReadError {
    match error {
        spline_sweep::Error::Invalid => ReadError::Invalid,
        spline_sweep::Error::Limit => ReadError::Limit,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reconstitution_refuses_changed_volume_and_inconsistent_verified_frame() {
        assert_eq!(compare_volume(3., 2., 7., 1e-7), Err(ReadError::Invalid));
        let entities = BTreeMap::new();
        let reader = Shell::new(&entities, 1e-7, true);
        let plane = Plane {
            origin: [0.; 3],
            normal: [0., 0., -1.],
            u: [1., 0., 0.],
            v: [0., -1., 0.],
        };
        let trim = Traversal {
            edge: 1,
            start: 2,
            end: 2,
            forward: true,
        };
        let face = FaceWire {
            surface: 0,
            forward: true,
            outer: vec![],
            hole: Some(vec![trim]),
        };
        let edges = BTreeMap::from([(
            1,
            EdgeData {
                vertices: [2, 2],
                geometry: Geometry::Analytic(Curve::Line),
                uses: [1, 1],
            },
        )]);
        assert_eq!(
            reconstruct(&reader, &edges, &plane, &face, [0., 1e-10, 1.], 1e4),
            Err(ReadError::Invalid)
        );
        assert_eq!(
            reconstruct(&reader, &edges, &plane, &face, [0., 0., 1.], 3.),
            Err(ReadError::Invalid)
        );
        assert_eq!(body_error(spline_sweep::Error::Invalid), ReadError::Invalid);
        assert_eq!(body_error(spline_sweep::Error::Limit), ReadError::Limit);
    }
}
