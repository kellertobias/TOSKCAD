use super::*;
use crate::rounded_thread::{Spec, Thread};
fn thread(scale: f64) -> Thread {
    let p = 1.27 * scale;
    let a = 27.5_f64.to_radians();
    let r = p * a.cos() / (12. * (1. - a.sin()));
    Thread::new(Spec {
        major_radius_mm: 3.175 * scale,
        pitch_mm: p,
        length_mm: 3.7 * p,
        depth_mm: p / (3. * a.tan()),
        crest_radius_mm: r,
        root_radius_mm: r,
        flank_angle_deg: 27.5,
        starts: 1,
        left_hand: false,
    })
    .unwrap()
}
fn tilted_top_cap(file: &str, tilt: f64) -> String {
    let entities = parse_entities(file, 1e-7).unwrap();
    let shell = Shell::new(&entities, 1e-7, true);
    let solid = shell.represented_solids(true).unwrap()[0];
    let faces = shell.member_faces(solid).unwrap();
    let normal = faces
        .into_iter()
        .find_map(|id| {
            let (surface, _, _) = face(&shell, id).unwrap();
            let e = shell.simple(surface).unwrap();
            if e.name != "PLANE" {
                return None;
            }
            let axis = reference(&e.args[1]).unwrap();
            if shell.placement(axis).unwrap().origin[2] <= 0. {
                return None;
            }
            Some(reference(&shell.args(axis, "AXIS2_PLACEMENT_3D").unwrap()[2]).unwrap())
        })
        .unwrap();
    let prefix = format!("#{normal}=");
    let start = file.find(&prefix).unwrap();
    let end = start + file[start..].find(';').unwrap() + 1;
    format!(
        "{}#{normal}=DIRECTION('',(0.,{tilt:.16E},1.));{}",
        &file[..start],
        &file[end..]
    )
}
#[test]
fn cap_support_normal_error_scales_with_actual_radius_and_preserves_small_controls() {
    let t = thread(1000.);
    let f = [[0.; 3], [1., 0., 0.], [0., 1., 0.]];
    let file = crate::step::write_rounded_thread(&t, f[0], f[1], f[2], 1e-7).unwrap();
    let damaged = tilted_top_cap(&file, 8e-11);
    let result = crate::step_read::verify_rounded_thread(&damaged, &t, f[0], f[1], f[2], 1e-7);
    let entities = parse_entities(&damaged, 1e-7).unwrap();
    let shell = Shell::new(&entities, 1e-7, true);
    let mut witness = 0_f64;
    for e in entities
        .values()
        .flatten()
        .filter(|e| e.name == "VERTEX_POINT")
    {
        let p = shell.point(reference(&e.args[1]).unwrap()).unwrap();
        if p[2] == t.spec().length_mm {
            witness = witness.max((8e-11 * p[1]).abs() / (1_f64 + 8e-11 * 8e-11).sqrt());
        }
    }
    assert!(witness > 1e-7, "actual finite cap witness {witness}");
    assert!(crate::step_read::verify_rounded_thread(&file, &t, f[0], f[1], f[2], 1e-7).is_ok());
    assert!(
        result.is_err(),
        "large finite cap does not lie on its declared plane"
    );
}

#[test]
fn same_small_cap_perturbation_remains_within_existing_linear_precision() {
    let t = thread(1.);
    let f = [[0.; 3], [1., 0., 0.], [0., 1., 0.]];
    let file = crate::step::write_rounded_thread(&t, f[0], f[1], f[2], 1e-7).unwrap();
    let damaged = tilted_top_cap(&file, 8e-11);
    assert!(crate::step_read::verify_rounded_thread(&damaged, &t, f[0], f[1], f[2], 1e-7).is_ok());
}
fn bore() -> ThroughBore {
    ThroughBore::new(thread(1.), 5.).unwrap()
}
fn rewrite(file: &str, id: u32, body: &str) -> String {
    let start = file.find(&format!("#{id}=")).unwrap();
    let end = start + file[start..].find(';').unwrap() + 1;
    format!("{}#{id}={body}{}", &file[..start], &file[end..])
}
fn entity(file: &str, name: &str) -> u32 {
    file.lines()
        .find_map(|line| {
            let (id, body) = line.strip_prefix('#')?.split_once('=')?;
            body.starts_with(&format!("{name}("))
                .then(|| id.parse().unwrap())
        })
        .unwrap()
}
#[test]
fn untrusted_receiver_wrong_supports_domains_and_expected_material_require_repair() {
    let b = bore();
    let f = [[0.; 3], [1., 0., 0.], [0., 1., 0.]];
    let file = crate::step::write_rounded_thread_bore(&b, f[0], f[1], f[2], 1e-7).unwrap();
    for tolerance in [0., -1., f64::NAN] {
        assert!(read_rounded_thread_bore(&file, tolerance).is_err());
    }
    assert!(read_rounded_thread_bore("", 1e-7).is_err());
    let shaft = crate::step::write_rounded_thread(b.thread(), f[0], f[1], f[2], 1e-7).unwrap();
    assert_eq!(
        read_rounded_thread_bore(&shaft, 1e-7).unwrap_err(),
        ReadError::Unsupported
    );
    let bigger = ThroughBore::new(*b.thread(), 6.).unwrap();
    assert!(verify_rounded_thread_bore(&file, &bigger, f[0], f[1], f[2], 1e-7).is_err());
    let entities = parse_entities(&file, 1e-7).unwrap();
    let shell = Shell::new(&entities, 1e-7, true);
    let cylinder = entity(&file, "CYLINDRICAL_SURFACE");
    let args = shell.args(cylinder, "CYLINDRICAL_SURFACE").unwrap();
    let axis = reference(&args[1]).unwrap();
    for radius in [3., 3.175 + 1e-8, 6.] {
        let bad = rewrite(
            &file,
            cylinder,
            &format!("CYLINDRICAL_SURFACE('',#{axis},{radius:.16E});"),
        );
        assert!(read_rounded_thread_bore(&bad, 1e-7).is_err());
    }
    let mut lines = file.lines().map(str::to_owned).collect::<Vec<_>>();
    let data = lines.iter().position(|l| l == "DATA;").unwrap() + 1;
    let end = data + lines[data..].iter().position(|l| l == "ENDSEC;").unwrap();
    lines[data..end].reverse();
    let changed_order = lines.join("\n");
    let (_, imported) = read_rounded_thread_bore(&changed_order, 1e-7).unwrap();
    assert!(rounded_thread_bore_import_matches(&imported, &b, f, 1e-7));
}
#[test]
fn outer_cap_support_is_checked_across_the_whole_large_circumference() {
    let b = ThroughBore::new(thread(1.), 1e5).unwrap();
    let f = [[0.; 3], [1., 0., 0.], [0., 1., 0.]];
    let file = crate::step::write_rounded_thread_bore(&b, f[0], f[1], f[2], 1e-7).unwrap();
    assert!(8e-11 * b.radius_mm() > 1e-7);
    let damaged = tilted_top_cap(&file, 8e-11);
    assert!(verify_rounded_thread_bore(&damaged, &b, f[0], f[1], f[2], 1e-7).is_err());
    assert!(read_rounded_thread_bore(&damaged, 1e-7).is_err());
    assert!(read_rounded_thread_bore(&file, 1e-7).is_ok());
}
fn ids(file: &str, name: &str) -> Vec<u32> {
    file.lines()
        .filter_map(|line| {
            let (id, body) = line.strip_prefix('#')?.split_once('=')?;
            body.starts_with(&format!("{name}("))
                .then(|| id.parse().unwrap())
        })
        .collect()
}
#[test]
fn annular_bound_order_is_free_but_missing_wrong_and_non_circular_wires_refuse() {
    let b = bore();
    let f = [[0.; 3], [1., 0., 0.], [0., 1., 0.]];
    let file = crate::step::write_rounded_thread_bore(&b, f[0], f[1], f[2], 1e-7).unwrap();
    let entities = parse_entities(&file, 1e-7).unwrap();
    let shell = Shell::new(&entities, 1e-7, true);
    let (cap, bounds, surface) = ids(&file, "ADVANCED_FACE")
        .into_iter()
        .find_map(|id| {
            let (surface, _, bounds) = face(&shell, id).unwrap();
            (shell.simple(surface).unwrap().name == "PLANE").then_some((id, bounds, surface))
        })
        .unwrap();
    let [outer, inner] = bounds[..] else { panic!() };
    let swapped = rewrite(
        &file,
        cap,
        &format!("ADVANCED_FACE('',(#{inner},#{outer}),#{surface},.T.);"),
    );
    assert!(verify_rounded_thread_bore(&swapped, &b, f[0], f[1], f[2], 1e-7).is_ok());
    assert!(read_rounded_thread_bore(&swapped, 1e-7).is_ok());
    for bounds in [format!("#{outer}"), format!("#{outer},#{outer}")] {
        let damaged = rewrite(
            &file,
            cap,
            &format!("ADVANCED_FACE('',({bounds}),#{surface},.T.);"),
        );
        assert!(verify_rounded_thread_bore(&damaged, &b, f[0], f[1], f[2], 1e-7).is_err());
    }
    let wire = reference(&shell.simple(outer).unwrap().args[1]).unwrap();
    let oriented = refs(&shell.args(wire, "EDGE_LOOP").unwrap()[1]).unwrap()[0];
    let doubled = rewrite(
        &file,
        wire,
        &format!("EDGE_LOOP('',(#{oriented},#{oriented}));"),
    );
    assert!(verify_rounded_thread_bore(&doubled, &b, f[0], f[1], f[2], 1e-7).is_err());
    let line = entity(&file, "LINE");
    let line_edge = ids(&file, "EDGE_CURVE")
        .into_iter()
        .find(|&id| reference(&shell.simple(id).unwrap().args[3]).unwrap() == line)
        .unwrap();
    let mut parsed = Shell::new(&entities, 1e-7, true);
    assert!(super::super::rounded_thread::outer_circle(&mut parsed, line_edge).is_err());
    let outer_edge = reference(&shell.simple(oriented).unwrap().args[3]).unwrap();
    assert!(super::super::rounded_thread::outer_circle(&mut parsed, outer_edge).is_ok());
    // Both cap traversal senses must refuse an unrelated seam edge.
    for forward in [true, false] {
        let wrong_curve = rewrite(
            &file,
            oriented,
            &format!(
                "ORIENTED_EDGE('',*,*,#{line_edge},.{});",
                if forward { "T" } else { "F" }
            ),
        );
        assert!(verify_rounded_thread_bore(&wrong_curve, &b, f[0], f[1], f[2], 1e-7).is_err());
    }
    let cylinder = entity(&file, "CYLINDRICAL_SURFACE");
    let wall = ids(&file, "ADVANCED_FACE")
        .into_iter()
        .find(|&id| face(&shell, id).unwrap().0 == cylinder)
        .unwrap();
    let (_, _, bound) = face(&shell, wall).unwrap();
    let wrong_side = rewrite(
        &file,
        wall,
        &format!(
            "ADVANCED_FACE('',(#{bound}),#{cylinder},.F.);",
            bound = bound[0]
        ),
    );
    assert!(verify_rounded_thread_bore(&wrong_side, &b, f[0], f[1], f[2], 1e-7).is_err());
}
#[test]
fn malformed_outer_support_schemas_and_independent_closure_ownership_refuse() {
    let b = bore();
    let f = [[0.; 3], [1., 0., 0.], [0., 1., 0.]];
    let file = crate::step::write_rounded_thread_bore(&b, f[0], f[1], f[2], 1e-7).unwrap();
    let entities = parse_entities(&file, 1e-7).unwrap();
    let shell = Shell::new(&entities, 1e-7, true);
    let cylinder = entity(&file, "CYLINDRICAL_SURFACE");
    let axis = reference(&shell.simple(cylinder).unwrap().args[1]).unwrap();
    let wall = ids(&file, "ADVANCED_FACE")
        .into_iter()
        .find(|&id| face(&shell, id).unwrap().0 == cylinder)
        .unwrap();
    for body in [
        "CYLINDRICAL_SURFACE('');".to_string(),
        format!("SPHERICAL_SURFACE('',#{axis},5.);"),
        format!("PLANE('',#{axis});"),
    ] {
        let damaged = rewrite(&file, cylinder, &body);
        assert!(read_rounded_thread_bore(&damaged, 1e-7).is_err());
        let data = parse_entities(&damaged, 1e-7).unwrap();
        let malformed = Shell::new(&data, 1e-7, true);
        assert!(
            verify_outer_frame(
                &malformed,
                wall,
                [[0.; 3], [0., 0., b.length_mm()]],
                f,
                5.,
                b.length_mm()
            )
            .is_err()
        );
    }
    for mut remaining in [BTreeSet::new(), BTreeSet::from([1, 2])] {
        let before = remaining.clone();
        assert_eq!(
            super::super::rounded_thread::remaining_cylinder(&mut remaining),
            Err(ReadError::Invalid)
        );
        assert_eq!(remaining, before);
    }
    let mut remaining = BTreeSet::from([7]);
    assert_eq!(
        super::super::rounded_thread::remaining_cylinder(&mut remaining),
        Ok(7)
    );
    assert!(remaining.is_empty());
    let (_, mut imported) = read_rounded_thread_bore(&file, 1e-7).unwrap();
    assert!(!rounded_thread_bore_import_matches(&imported, &b, f, 0.));
    imported.u = [f64::NAN, 0., 0.];
    assert!(!rounded_thread_bore_import_matches(&imported, &b, f, 1e-7));
}
#[test]
fn large_outer_circle_normal_is_proved_over_its_entire_curve() {
    let b = ThroughBore::new(thread(1.), 1e5).unwrap();
    let f = [[0.; 3], [1., 0., 0.], [0., 1., 0.]];
    let file = crate::step::write_rounded_thread_bore(&b, f[0], f[1], f[2], 1e-7).unwrap();
    let entities = parse_entities(&file, 1e-7).unwrap();
    let shell = Shell::new(&entities, 1e-7, true);
    let circle = entity(&file, "CIRCLE");
    let axis = reference(&shell.simple(circle).unwrap().args[1]).unwrap();
    let normal = reference(&shell.args(axis, "AXIS2_PLACEMENT_3D").unwrap()[2]).unwrap();
    let damaged = rewrite(&file, normal, "DIRECTION('',(0.,8.E-11,1.));");
    assert!(verify_rounded_thread_bore(&damaged, &b, f[0], f[1], f[2], 1e-7).is_err());
    assert!(read_rounded_thread_bore(&damaged, 1e-7).is_err());
    assert!(read_rounded_thread_bore(&file, 1e-7).is_ok());
}
