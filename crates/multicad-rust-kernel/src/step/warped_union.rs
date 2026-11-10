//! Write exact shared line and ellipse edges from a certified warped shell.

use super::*;
use crate::{
    trim_curve::EllipseArc,
    warped_union::{Curve, Shell, Support},
};
use std::f64::consts::TAU;

/// Whether the exact warped boundary fits STEP's single-shell profile.
pub fn supports_warped_union(shell: &Shell) -> bool {
    shell.shell_count() == 1
        && shell
            .faces()
            .iter()
            .all(|face| face.boundary.len() <= MAX_VERTICES)
}

/// Write a rigidly placed warped solid with analytic ellipse trims.
pub fn write_warped_union(
    shell: &Shell,
    origin: Point,
    u: Point,
    v: Point,
) -> Result<String, StepError> {
    write_one(|writer| writer.append_warped_union(shell, origin, u, v))
}

fn place(placement: &Placement, [x, y, z]: Point) -> Point {
    placement.point_f64([x, y], z)
}

fn distance(a: Point, b: Point) -> f64 {
    let [x, y, z] = std::array::from_fn(|i| a[i] - b[i]);
    x.hypot(y).hypot(z)
}

fn ellipse_edge(
    file: &mut StepFile,
    [from, to]: [Vertex; 2],
    arc: EllipseArc,
    placement: &Placement,
) -> Result<Entity, StepError> {
    let ellipse = arc.ellipse();
    let center = place(placement, ellipse.center_mm());
    let [start, end] = arc.endpoints().map(|point| place(placement, point));
    let complete = arc.sweep_rad().abs() == TAU;
    if (from.vertex.0 == to.vertex.0) != complete
        || (from.point == to.point) != complete
        || from.point == center
        || to.point == center
        || distance(from.point, start) > 1e-7
        || distance(to.point, end) > 1e-7
    {
        return Err(StepError::InvalidGeometry);
    }
    let normal = placement
        .vector(ellipse.normal())
        .map(|n| n * arc.sweep_rad().signum());
    let reference = placement.vector(ellipse.major_axis());
    let center = file.point(center);
    let axis = file.axis(center, normal, reference);
    let [major, minor] = ellipse.semiaxes_mm();
    let curve = if major == minor {
        file.entity(format_args!("CIRCLE('',{axis},{major:.16E})"))
    } else {
        file.entity(format_args!("ELLIPSE('',{axis},{major:.16E},{minor:.16E})"))
    };
    let (start, end) = (from.vertex, to.vertex);
    Ok(file.entity(format_args!("EDGE_CURVE('',{start},{end},{curve},.T.)")))
}

fn plane_reference(normal: Point) -> Result<Point, StepError> {
    let coordinate = (0..3)
        .min_by(|&a, &b| normal[a].abs().total_cmp(&normal[b].abs()))
        .expect("three coordinate axes");
    let axis = std::array::from_fn(|i| f64::from(i == coordinate));
    direction([0.; 3], cross(axis, normal))
}

pub(super) fn encode(
    file: &mut StepFile,
    placement: Placement,
    shell: &Shell,
) -> Result<Entity, StepError> {
    let vertices: Vec<_> = shell
        .vertices()
        .iter()
        .map(|&point| file.vertex(place(&placement, point)))
        .collect();
    let edges = shell
        .edges()
        .iter()
        .map(|edge| {
            let ends = edge.vertices.map(|id| vertices[id]);
            match edge.curve {
                Curve::Line => line_edge(file, ends[0], ends[1]),
                Curve::Ellipse(arc) => ellipse_edge(file, ends, arc, &placement),
            }
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut faces = Vec::with_capacity(shell.faces().len());
    for face in shell.faces() {
        let boundary: Vec<_> = face
            .boundary
            .iter()
            .map(|use_| file.oriented_edge(edges[use_.edge], use_.forward))
            .collect();
        let bound = file.bound(&boundary, true);
        let first = face.boundary[0].edge;
        let vertex = shell.edges()[first].vertices[0];
        let surface = match face.support {
            Support::Plane { normal } => {
                let normal = placement.vector(normal);
                file.face(
                    &[bound],
                    vertices[vertex].cartesian,
                    normal,
                    plane_reference(normal)?,
                )
            }
            Support::Cylinder {
                axis_origin_mm,
                axis_direction,
                radius_mm,
                outward,
            } => {
                // The certified patches span at most a quadrant. Place the
                // periodic seam opposite a boundary radial direction.
                let point = shell.vertices()[vertex];
                let delta = std::array::from_fn(|i| point[i] - axis_origin_mm[i]);
                let level = dot(delta, axis_direction);
                let center = std::array::from_fn(|i| axis_origin_mm[i] + level * axis_direction[i]);
                let reference = direction(point, center)?;
                let center = file.point(place(&placement, center));
                let axis = file.axis(
                    center,
                    placement.vector(axis_direction),
                    placement.vector(reference),
                );
                let cylinder = file.entity(format_args!(
                    "CYLINDRICAL_SURFACE('',{axis},{radius_mm:.16E})"
                ));
                file.face_on(&[bound], cylinder, outward)
            }
        };
        faces.push(surface);
    }
    Ok(file.solid(&faces))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        planar_boolean::{ConvexPiece, Union},
        trim_curve::Ellipse,
        warped_union::{Band, remap},
    };

    const X: Point = [1., 0., 0.];
    const Y: Point = [0., 1., 0.];
    const Z: Point = [0., 0., 1.];
    const ZERO: Point = [0.; 3];
    const TOL: f64 = 1e-7;
    const BAND: Band = Band {
        center_z_mm: 2.,
        radius_mm: 2.,
    };

    #[test]
    fn warped_step_shares_the_circular_and_elliptic_trims_of_one_fillet() {
        let [a, b, c, d, e, f] = [
            [0., 3., 0.],
            [2., 4., 0.],
            [2., 4., 2.],
            [0., 10., 0.],
            [2., 10., 0.],
            [2., 10., 2.],
        ];
        let wedge = ConvexPiece::new(
            vec![
                vec![a, b, c],
                vec![d, f, e],
                vec![a, d, e, b],
                vec![b, e, f, c],
                vec![c, f, d, a],
            ],
            TOL,
        )
        .unwrap();
        let block = ConvexPiece::from_box([2., 4., 0.], [3., 8., 2.], TOL).unwrap();
        let union = Union::new(&[wedge, block], TOL).unwrap();
        let shell = remap(&union, BAND, TOL).unwrap();
        let source = write_warped_union(&shell, ZERO, X, Y).unwrap();
        assert_eq!(source.matches("=EDGE_CURVE(").count(), shell.edges().len());
        assert_eq!(
            source.matches("=ADVANCED_FACE(").count(),
            shell.faces().len()
        );
        assert_eq!(source.matches("=MANIFOLD_SOLID_BREP(").count(), 1);
        assert!(source.contains("=CYLINDRICAL_SURFACE("));
        assert!(source.contains("=CIRCLE("));
        assert!(source.contains("=ELLIPSE("));
        assert_eq!(
            write_warped_union(&shell, ZERO, X, X),
            Err(StepError::InvalidPlacement)
        );
    }

    #[test]
    fn ellipse_encoding_rejects_an_inconsistent_or_collapsed_endpoint() {
        let ellipse = Ellipse::new(ZERO, Z, X, [2., 1.]).unwrap();
        let arc = EllipseArc::new(ellipse, 0., -TAU / 4.).unwrap();
        let placement = Placement::new(ZERO, X, Y).unwrap();
        let mut file = StepFile::new();
        let from = file.vertex([2., 0., 0.]);
        let to = file.vertex([0., -1., 0.]);
        ellipse_edge(&mut file, [from, to], arc, &placement).unwrap();
        assert!(file.data.contains("=ELLIPSE("));
        let normal = file
            .data
            .lines()
            .find(|line| line.contains("=DIRECTION("))
            .unwrap();
        assert!(normal.contains("-1.0000000000000000E0"));
        for bad in [from, file.vertex([0., 1., 0.])] {
            assert_eq!(
                ellipse_edge(&mut file, [from, bad], arc, &placement).err(),
                Some(StepError::InvalidGeometry)
            );
        }
        let full = EllipseArc::new(ellipse, 0., TAU).unwrap();
        ellipse_edge(&mut file, [from, from], full, &placement).unwrap();
    }

    #[test]
    fn rejects_a_separate_internal_shell() {
        let pieces = [
            ([0., 0., -3.], [3., 3., -2.]),
            ([0., 0., -1.], [3., 3., 0.]),
            ([0., 0., -2.], [1., 3., -1.]),
            ([2., 0., -2.], [3., 3., -1.]),
            ([1., 0., -2.], [2., 1., -1.]),
            ([1., 2., -2.], [2., 3., -1.]),
        ]
        .map(|(min, max)| ConvexPiece::from_box(min, max, TOL).unwrap());
        let union = Union::new(&pieces, TOL).unwrap();
        let shell = remap(&union, BAND, TOL).unwrap();
        assert!(!supports_warped_union(&shell));
        assert_eq!(
            write_warped_union(&shell, ZERO, X, Y),
            Err(StepError::Unsupported)
        );
    }
}
