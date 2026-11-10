//! Full rectilinear meridians become one shell of cylinders and plane faces.

use super::*;
use crate::revolution::Revolution;

/// Whether a full revolution has one connected shell of cylinders and
/// annular shoulders, with no enclosed cavity requiring another shell.
pub fn supports_cylindrical_revolution(revolution: &Revolution) -> bool {
    meridian(revolution).is_some()
}

/// Write a full cylindrical revolution about the positive normal of `u, v`.
pub fn write_cylindrical_revolution(
    revolution: &Revolution,
    origin: Point,
    u: Point,
    v: Point,
) -> Result<String, StepError> {
    write_one(|writer| writer.append_cylindrical_revolution(revolution, origin, u, v))
}

pub(super) fn meridian(revolution: &Revolution) -> Option<&[[f64; 2]]> {
    let [ring] = revolution.rings() else {
        return None;
    };
    let edges = || ring.iter().zip(ring.iter().cycle().skip(1));
    // Multiple disjoint intervals on the axis enclose a void on revolution.
    let axis_runs = edges()
        .filter(|(a, b)| a[0] > 0. && b[0] == 0.)
        .take(2)
        .count();
    (revolution.is_full() && axis_runs <= 1 && edges().all(|(a, b)| a[0] == b[0] || a[1] == b[1]))
        .then_some(ring)
}

pub(super) fn encode(
    file: &mut StepFile,
    placement: Placement,
    ring: &[[f64; 2]],
) -> Result<Entity, StepError> {
    let reference = placement.u;
    let axis = placement.normal;
    let rims = ring
        .iter()
        .map(|&[radius, level]| {
            (radius > 0.)
                .then(|| {
                    CircularRim::new(
                        file,
                        placement.point_f64([0.; 2], level),
                        axis,
                        reference,
                        radius,
                    )
                })
                .transpose()
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut faces = Vec::with_capacity(ring.len());
    for (i, &a) in ring.iter().enumerate() {
        let j = (i + 1) % ring.len();
        let b = ring[j];
        let [first, last] = [i, j].map(|index| rims[index].as_ref());
        if let (Some(first), Some(last)) = (first, last)
            && a[0] == b[0]
        {
            let outward = a[1] < b[1];
            let [bottom, top] = if outward {
                [first, last]
            } else {
                [last, first]
            };
            let trim = CylindricalTrim::from_rims(file, bottom, top)?;
            faces.push(trim.wall(file, outward));
        } else if first.is_some() || last.is_some() {
            let up = a[0] > b[0];
            let (outer, inner) = if up { (first, last) } else { (last, first) };
            let outer = outer.expect("a radial face has a positive outer radius");
            let mut bounds = vec![circular_bound(file, outer.edge, up, true)];
            if let Some(inner) = inner {
                bounds.push(circular_bound(file, inner.edge, !up, false));
            }
            let normal = axis.map(|component| if up { component } else { -component });
            faces.push(file.face(&bounds, outer.center, normal, reference));
        }
    }
    Ok(file.solid(&faces))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::step_read::read_analytic_solid;
    use std::f64::consts::TAU;

    const ORIGIN: Point = [40., -8., 12.];
    const U: Point = [0.8, 0.6, 0.];
    const V: Point = [-0.36, 0.48, 0.8];

    #[test]
    fn stepped_outer_and_inner_walls_share_exact_rims_in_both_directions() {
        let annular = [
            [2., 0.],
            [6., 0.],
            [6., 3.],
            [8., 3.],
            [8., 6.],
            [6., 6.],
            [6., 9.],
            [3., 9.],
            [3., 4.],
            [2., 4.],
        ];
        let solid = [
            [0., -2.],
            [6., -2.],
            [6., 0.],
            [5., 0.],
            [5., 3.],
            [3., 3.],
            [3., 6.],
            [0., 6.],
        ];
        for (ring, faces, circles) in [(annular.as_slice(), 10, 10), (solid.as_slice(), 7, 6)] {
            for angle in [TAU, -TAU] {
                let revolution = Revolution::new(vec![ring.to_vec()], angle).unwrap();
                assert!(supports_cylindrical_revolution(&revolution));
                let encoded = write_cylindrical_revolution(&revolution, ORIGIN, U, V).unwrap();
                let read = read_analytic_solid(&encoded, 1e-7).unwrap();
                assert_eq!(read.solid_count, 1);
                assert_eq!(read.face_count, faces);
                assert_eq!(read.circles.len(), circles);
                assert!(read.edges_mm.is_empty());
                assert!((read.volume_mm3 - revolution.volume_mm3()).abs() < 1e-7);
                let placement = Placement::new(ORIGIN, U, V).unwrap();
                let support = |i, sign: f64| {
                    let direction = [placement.u[i], placement.v[i], placement.normal[i]];
                    revolution.support(direction.map(|value| sign * value))
                };
                for (i, coordinate) in ORIGIN.into_iter().enumerate() {
                    let expected = [coordinate - support(i, -1.), coordinate + support(i, 1.)];
                    assert!((read.bounds_mm[0][i] - expected[0]).abs() < 1e-7);
                    assert!((read.bounds_mm[1][i] - expected[1]).abs() < 1e-7);
                }
            }
        }
    }

    #[test]
    fn writer_refuses_open_sweeps_slopes_and_extra_shells() {
        let simple = vec![[0., 0.], [8., 0.], [8., 8.], [0., 8.]];
        let slope = vec![[0., 0.], [4., 0.], [8., 8.], [0., 8.]];
        let axial_void = vec![
            [0., 0.],
            [4., 0.],
            [4., 8.],
            [0., 8.],
            [0., 6.],
            [2., 6.],
            [2., 2.],
            [0., 2.],
        ];
        let enclosed = vec![[2., 2.], [2., 6.], [4., 6.], [4., 2.]];
        for (wires, angle) in [
            (vec![simple.clone()], TAU / 2.),
            (vec![slope], TAU),
            (vec![axial_void], TAU),
            (vec![simple.clone(), enclosed], TAU),
        ] {
            let unsupported = Revolution::new(wires, angle).unwrap();
            assert!(!supports_cylindrical_revolution(&unsupported));
            assert_eq!(
                write_cylindrical_revolution(&unsupported, ORIGIN, U, V),
                Err(StepError::Unsupported)
            );
        }
        let solid = Revolution::new(vec![simple], TAU).unwrap();
        assert_eq!(
            write_cylindrical_revolution(&solid, ORIGIN, U, U),
            Err(StepError::InvalidPlacement)
        );
        let mut writer = Writer::new();
        assert_eq!(
            writer.append_cylindrical_revolution(&solid, [1e30; 3], U, V),
            Err(StepError::InvalidGeometry),
        );
        writer
            .append_cylindrical_revolution(&solid, ORIGIN, U, V)
            .unwrap();
        assert_eq!(
            writer.finish().unwrap(),
            write_cylindrical_revolution(&solid, ORIGIN, U, V).unwrap()
        );
    }
}
