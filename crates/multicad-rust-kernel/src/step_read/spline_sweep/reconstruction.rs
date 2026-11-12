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
    let section_plane = Plane {
        origin: base.origin,
        u,
        v,
        normal: axis,
    };
    let outline = section_edges(reader, edges, &section_plane, &low.outer, true)?;
    let holes = low
        .holes
        .iter()
        .map(|ring| section_edges(reader, edges, &section_plane, ring, true))
        .collect::<Result<Vec<_>>>()?;
    let sweep = spline_sweep::Sweep::from_verified_rings(outline, holes, height, reader.tolerance)
        .map_err(body_error)?;
    Ok(DecodedSplineSweep {
        origin_mm: base.origin,
        u,
        v,
        sweep,
    })
}

pub(super) fn section_edges(
    reader: &Shell<'_, '_>,
    edges: &Edges,
    plane: &Plane,
    ring: &[Traversal],
    reverse: bool,
) -> Result<Vec<spline_sweep::Edge>> {
    let project = |point| {
        let relative = sub(point, plane.origin);
        [dot(relative, plane.u), dot(relative, plane.v)]
    };
    (0..ring.len())
        .map(|i| {
            let edge = &ring[if reverse { ring.len() - 1 - i } else { i }];
            let [start, end] = if reverse {
                [edge.end, edge.start]
            } else {
                [edge.start, edge.end]
            };
            let a = project(*reader.points.get(&start).ok_or(ReadError::Invalid)?);
            let b = project(*reader.points.get(&end).ok_or(ReadError::Invalid)?);
            let direction = edge.forward != reverse;
            Ok(match &edges[&edge.edge].geometry {
                Geometry::Analytic(Curve::Line) => {
                    spline_sweep::Edge::Analytic(normal_sweep::Edge::Line { a, b })
                }
                Geometry::Analytic(curve) => {
                    let round = round_geometry(*curve).ok_or(ReadError::Invalid)?;
                    let center = project(round.center);
                    let clockwise = (dot(round.normal, plane.normal) > 0.) != direction;
                    spline_sweep::Edge::Analytic(if round.full {
                        normal_sweep::Edge::Circle {
                            center,
                            radius_mm: round.radius,
                            clockwise,
                        }
                    } else {
                        normal_sweep::Edge::Arc {
                            center,
                            a,
                            b,
                            clockwise,
                        }
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
        .collect()
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
            holes: vec![vec![trim]],
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
