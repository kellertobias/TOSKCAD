//! Verify a complete periodic spherical face with a collapsed pole loop.
use super::*;
use crate::sphere::Sphere;

pub(super) fn read_member(
    shell: &mut Shell<'_, '_>,
    faces: &[u32],
) -> Result<Option<(Readback, ImportSolid)>> {
    let [id] = faces else {
        return Ok(None);
    };
    let [Value::String(_), bounds, support, orientation] = shell.args(*id, "ADVANCED_FACE")? else {
        return Err(ReadError::Invalid);
    };
    let support = shell.simple(reference(support)?)?;
    if support.name != "SPHERICAL_SURFACE" {
        return Ok(None);
    }
    let [Value::String(_), placement, Value::Number(radius)] = support.args.as_slice() else {
        return Err(ReadError::Invalid);
    };
    let radius = *radius;
    let sphere = Sphere::new(radius).ok_or(ReadError::Invalid)?;
    let frame = shell.placement(reference(placement)?)?;
    let bounds = refs(bounds)?;
    let [bound] = bounds.as_slice() else {
        return Err(ReadError::Unsupported);
    };
    if !sense(orientation)? {
        return Err(ReadError::Invalid);
    }
    let [Value::String(_), loop_id, orientation] = shell.args(*bound, "FACE_BOUND")? else {
        return Err(ReadError::Invalid);
    };
    if !sense(orientation)? {
        return Err(ReadError::Invalid);
    }
    let [Value::String(_), pole] = shell.args(reference(loop_id)?, "VERTEX_LOOP")? else {
        return Err(ReadError::Invalid);
    };
    let pole = shell.vertex(reference(pole)?)?;
    let pole_error = |sign: f64| {
        let expected = std::array::from_fn(|i| frame.origin[i] + sign * radius * frame.normal[i]);
        length(sub(pole, expected))
    };
    // Either collapsed pole can anchor the same complete surface. A large
    // caller tolerance cannot turn an arbitrary trim into a pole.
    if pole_error(-1.).min(pole_error(1.)) > shell.tolerance.min(radius * 0.01) {
        return Err(ReadError::Invalid);
    }
    let bounds_mm = [
        frame.origin.map(|x| x - radius),
        frame.origin.map(|x| x + radius),
    ];
    if bounds_mm[0]
        .into_iter()
        .zip(bounds_mm[1])
        .any(|(lo, hi)| lo >= hi)
    {
        return Err(ReadError::Invalid);
    }
    let readback = Readback {
        bounds_mm,
        volume_mm3: sphere.volume_mm3(),
        face_count: 1,
        edges_mm: vec![],
        circles: vec![],
        semicircles: vec![SemicircleEdge {
            center_mm: frame.origin,
            normal: cross(frame.u, frame.normal),
            radius_mm: radius,
            radial_mid: frame.u,
        }],
        arcs: vec![],
        ellipses: vec![],
        splines: vec![],
        solid_count: 1,
    };
    let imported = ImportSolid {
        origin_mm: frame.origin,
        u: frame.u,
        v: frame.v,
        height_mm: 0.,
        profile: ImportProfile::Sphere { radius_mm: radius },
    };
    Ok(Some((readback, imported)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        Rational,
        step::{StepError, Writer},
    };
    use std::f64::consts::PI;

    const CENTER: Point = [17., 4., -12.];
    const U: Point = [0.8, 0., -0.6];
    const V: Point = [0., 1., 0.];
    const TOLERANCE: f64 = 1e-7;

    fn specimen() -> String {
        let mut writer = Writer::new();
        let sphere = Sphere::new(4.5).unwrap();
        assert_eq!(
            writer.append_sphere(sphere, CENTER, U, U),
            Err(StepError::InvalidPlacement)
        );
        writer.append_sphere(sphere, CENTER, U, V).unwrap();
        writer.finish().unwrap()
    }

    #[test]
    fn periodic_sphere_preserves_exact_metrics_and_seam_in_single_and_flat_compound() {
        let text = specimen();
        assert!(text.contains("SPHERICAL_SURFACE("));
        assert!(text.contains("VERTEX_LOOP("));
        assert!(!text.contains("EDGE_CURVE("));
        let (proof, body) = read_import_solid(&text, TOLERANCE).unwrap();
        assert_eq!(proof, read_analytic_solid(&text, TOLERANCE).unwrap());
        assert_eq!(body.origin_mm, CENTER);
        assert_eq!(body.u, U);
        assert_eq!(body.v, V);
        assert_eq!(body.profile, ImportProfile::Sphere { radius_mm: 4.5 });
        assert_eq!(proof.bounds_mm, [[12.5, -0.5, -16.5], [21.5, 8.5, -7.5]]);
        assert!((proof.volume_mm3 - 4. * PI * 4.5_f64.powi(3) / 3.).abs() < 1e-12);
        assert_eq!([proof.solid_count as usize, proof.face_count], [1, 1]);
        assert_eq!(proof.semicircles[0].radial_mid, U);
        assert_eq!(proof.semicircles[0].normal, [0., -1., 0.]);
        assert!(read_planar_solid(&text, TOLERANCE).is_err());
        let north = text
            .replace("1.4300000000000001E1", "1.9700000000000000E1")
            .replace("-1.5600000000000000E1", "-8.4000000000000004E0");
        assert_ne!(north, text);
        assert_eq!(read_analytic_solid(&north, TOLERANCE).unwrap(), proof);

        let mut writer = Writer::new();
        writer
            .append_box([Rational::new(4, 1).unwrap(); 3], [0.; 3], [1., 0., 0.], V)
            .unwrap();
        writer
            .append_sphere(Sphere::new(4.5).unwrap(), CENTER, U, V)
            .unwrap();
        let text = writer.finish().unwrap();
        let decoded = read_import_compound(&text, TOLERANCE).unwrap();
        assert_eq!(decoded.len(), 2);
        assert_eq!(decoded[1], (proof, body));
        assert_eq!(
            read_analytic_compound(&text, TOLERANCE).unwrap()[1],
            decoded[1].0
        );
    }

    fn edit(text: &str, kind: &str, change: impl FnOnce(&str) -> String) -> String {
        let line = text
            .lines()
            .find(|line| {
                line.split_once('=')
                    .is_some_and(|(_, entity)| entity.starts_with(kind))
            })
            .unwrap();
        text.replacen(line, &change(line), 1)
    }

    #[test]
    fn malformed_sphere_support_or_trim_cannot_be_reconstructed_as_full_sphere() {
        let text = specimen();
        let mutate_sense = |line: &str| line.replace(".T.", ".F.");
        let invalid = [
            edit(&text, "ADVANCED_FACE", mutate_sense),
            edit(&text, "FACE_BOUND", mutate_sense),
            edit(&text, "SPHERICAL_SURFACE", |line| {
                line.replace("4.5000000000000000E0", "-1.")
            }),
            edit(&text, "SPHERICAL_SURFACE", |line| {
                format!("{}=SPHERICAL_SURFACE('');", line.split_once('=').unwrap().0)
            }),
            edit(&text, "FACE_BOUND", |line| {
                format!("{}=FACE_BOUND('');", line.split_once('=').unwrap().0)
            }),
            edit(&text, "VERTEX_LOOP", |line| {
                format!("{}=VERTEX_LOOP('');", line.split_once('=').unwrap().0)
            }),
            edit(&text, "ADVANCED_FACE", |line| {
                format!("{}=ADVANCED_FACE('');", line.split_once('=').unwrap().0)
            }),
            edit(&text, "ADVANCED_FACE", |line| {
                let end = line.find("),").unwrap();
                let start = line.find("',(").unwrap() + 3;
                format!("{}{}", &line[..start], &line[end..])
            }),
            text.replace("-1.5600000000000000E1", "-1.5500000000000000E1"),
            edit(&text, "SPHERICAL_SURFACE", |line| {
                line.replace("SPHERICAL_SURFACE", "PLANE")
            }),
        ];
        for malformed in invalid {
            assert_ne!(text, malformed);
            assert!(
                read_analytic_solid(&malformed, TOLERANCE).is_err(),
                "{malformed}"
            );
            assert!(
                read_import_solid(&malformed, TOLERANCE).is_err(),
                "{malformed}"
            );
        }

        let mut writer = Writer::new();
        writer
            .append_sphere(Sphere::new(1e-6).unwrap(), [1e11, 0., 0.], [1., 0., 0.], V)
            .unwrap();
        assert_eq!(
            read_analytic_solid(&writer.finish().unwrap(), TOLERANCE),
            Err(ReadError::Invalid)
        );
    }
}
