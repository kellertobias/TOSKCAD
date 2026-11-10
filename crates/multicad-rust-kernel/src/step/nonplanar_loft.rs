//! Exact degree-one surfaces with shared straight cap edges and wall rails.
use super::*;
use crate::planar_boolean::NonplanarLoft;

pub fn write_nonplanar_loft(
    loft: &NonplanarLoft,
    origin: Point,
    u: Point,
    v: Point,
) -> Result<String, StepError> {
    write_one(|writer| writer.append_nonplanar_loft(loft, origin, u, v))
}

pub(super) fn encode(
    file: &mut StepFile,
    placement: &Placement,
    loft: &NonplanarLoft,
) -> Result<Entity, StepError> {
    let count = loft.rings()[0].len();
    let points = loft
        .rings()
        .iter()
        .flatten()
        .map(|&[x, y, z]| placement.point_f64([x, y], z))
        .collect();
    let mut brep = BRep::new(file, points);
    let mut faces = Vec::with_capacity(count + 2);
    for (cap, ring) in loft.rings().iter().enumerate() {
        let offset = cap * count;
        let mut indices: Vec<_> = (offset..offset + count).collect();
        if (cap == 0) != loft.clockwise() {
            indices.reverse();
        }
        let bound = brep.bound(&indices, true)?;
        let reference = placement.vector(direction(ring[0], ring[1])?);
        let normal = placement.vector([0., 0., if cap == 0 { -1. } else { 1. }]);
        faces.push(
            brep.file
                .face(&[bound], brep.vertices[offset].cartesian, normal, reference),
        );
    }
    for i in 0..count {
        let j = (i + 1) % count;
        let mut indices = [i, j, j + count, i + count];
        if loft.clockwise() {
            indices.reverse();
        }
        let bound = brep.bound(&indices, true)?;
        let [a, b, c, d] = [i, j, j + count, i + count].map(|i| brep.vertices[i].cartesian);
        // Rows increase along the section edge. Columns advance along the
        // straight rails from the first section to the second.
        let surface = brep.file.entity(format_args!(
            "B_SPLINE_SURFACE_WITH_KNOTS('',1,1,(({a},{d}),({b},{c})),.RULED_SURF.,.F.,.F.,.F.,(2,2),(2,2),(0.,1.),(0.,1.),.UNSPECIFIED.)"
        ));
        faces.push(brep.file.face_on(&[bound], surface, !loft.clockwise()));
    }
    Ok(brep.file.solid(&faces))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_bilinear_walls_with_shared_edges_in_both_windings() {
        let low = [[0., 0.], [2., 0.], [2., 2.], [0., 2.]];
        let high = [[0.5, 0.], [2.5, 0.5], [2., 2.5], [0., 2.]];
        for clockwise in [false, true] {
            let mut rings = [low, high];
            if clockwise {
                rings.iter_mut().for_each(|ring| ring.reverse());
            }
            let loft = NonplanarLoft::new([&rings[0], &rings[1]], 4., 1e-7).unwrap();
            let u = [1., 0., 0.];
            let v = [0., 0.6, 0.8];
            let step = write_nonplanar_loft(&loft, [1., -2., 5.], u, v).unwrap();
            assert_eq!(
                step.matches("=B_SPLINE_SURFACE_WITH_KNOTS('',1,1,").count(),
                4
            );
            assert_eq!(step.matches("=EDGE_CURVE(").count(), 12);
            assert_eq!(step.matches("=ORIENTED_EDGE(").count(), 24);
            assert_eq!(step.matches("=VERTEX_POINT(").count(), 8);
            assert_eq!(step.matches("=PLANE(").count(), 2);
            assert_eq!(step.matches("=MANIFOLD_SOLID_BREP(").count(), 1);
            assert_eq!(
                write_nonplanar_loft(&loft, [0.; 3], u, u),
                Err(StepError::InvalidPlacement)
            );
        }
    }
}
