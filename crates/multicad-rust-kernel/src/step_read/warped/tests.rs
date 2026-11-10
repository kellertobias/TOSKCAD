use super::*;
use crate::{
    planar_boolean::ConvexPiece,
    step::{self, Writer},
};

const TOL: f64 = 1e-7;
const ORIGIN: Point = [13., -7., 42.];
const U: Point = [0.8, 0.6, 0.];
const V: Point = [-0.36, 0.48, 0.8];

fn fixture() -> warped_union::Shell {
    fixture_with_block_start(4.)
}

fn fixture_with_block_start(start_y: f64) -> warped_union::Shell {
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
    let block = ConvexPiece::from_box([2., start_y, 0.], [3., 8., 2.], TOL).unwrap();
    let below = ConvexPiece::from_box([2., 4., -1.], [3., 8., 0.], TOL).unwrap();
    let above = ConvexPiece::from_box([2., 4., 2.], [3., 8., 3.], TOL).unwrap();
    let union = Union::new(&[wedge, block, below, above], TOL).unwrap();
    warped_union::remap(
        &union,
        Band {
            center_z_mm: 2.,
            radius_mm: 2.,
        },
        TOL,
    )
    .unwrap()
}

#[test]
fn independently_reconstructs_shared_ellipse_in_a_mixed_representation() {
    let expected = fixture();
    let mut writer = Writer::new();
    writer.append_warped_union(&expected, ORIGIN, U, V).unwrap();
    writer
        .append_box([crate::Rational::new(4, 1).unwrap(); 3], [2., 3., 4.], U, V)
        .unwrap();
    let source = writer.finish().unwrap();
    let members = read_analytic_compound(&source, TOL).unwrap();
    assert_eq!(members.len(), 2);
    let [warped, box_] = &members[..] else {
        unreachable!()
    };
    assert_eq!(warped.face_count, expected.faces().len());
    assert_eq!(warped.ellipses.len(), 1);
    assert_eq!(warped.arcs.len(), 1);
    assert!((warped.volume_mm3 - expected.volume_mm3()).abs() < TOL);
    assert!(box_.ellipses.is_empty());
    assert!((box_.volume_mm3 - 64.).abs() < TOL);
    assert_eq!(
        read_warped_solid(&source, TOL).unwrap_err(),
        ReadError::Unsupported
    );

    let single = step::write_warped_union(&expected, ORIGIN, U, V).unwrap();
    let reconstructed = read_warped_solid(&single, TOL).unwrap();
    let area = |shell: &warped_union::Shell| shell.faces().iter().map(|f| f.area_mm2).sum::<f64>();
    assert!((area(&reconstructed.shell) - area(&expected)).abs() < TOL);
    assert_eq!(read_analytic_solid(&single, TOL).unwrap(), *warped);
    let (readback, decoded) = read_import_solid(&single, TOL).unwrap();
    assert_eq!(readback, *warped);
    assert!(matches!(decoded.profile, ImportProfile::WarpedUnion { .. }));
    let decoded = read_import_compound(&source, TOL).unwrap();
    assert!(matches!(
        decoded[0].1.profile,
        ImportProfile::WarpedUnion { .. }
    ));
    assert!(matches!(
        decoded[1].1.profile,
        ImportProfile::Polygon { .. }
    ));

    assert!(read_planar_solid(&single, TOL).is_err());
}

fn rewrite(source: &str, kind: &str, transform: impl FnOnce(&str) -> String) -> String {
    let line = source.lines().find(|line| line.contains(kind)).unwrap();
    source.replacen(line, &transform(line), 1)
}

#[test]
fn refuses_bad_curve_support_and_unsupported_surface_wrappers() {
    let source = step::write_warped_union(&fixture(), ORIGIN, U, V).unwrap();
    let mutated = rewrite(&source, "=CYLINDRICAL_SURFACE(", |line| {
        format!("{},3.0);", line.rsplit_once(',').unwrap().0)
    });
    assert!(read_analytic_solid(&mutated, TOL).is_err());
    let mutated = rewrite(&source, "=ELLIPSE(", |line| {
        format!("{},1.9);", line.rsplit_once(',').unwrap().0)
    });
    assert!(read_analytic_solid(&mutated, TOL).is_err());
    let mutated = rewrite(&source, "=ORIENTED_EDGE(", |line| {
        format!("{},.F.);", line.rsplit_once(',').unwrap().0)
    });
    assert!(read_analytic_solid(&mutated, TOL).is_err());
    let mutated = source.replacen("=ELLIPSE(", "=B_SPLINE_CURVE_WITH_KNOTS(", 1);
    assert!(read_analytic_solid(&mutated, TOL).is_err());
    assert!(read_warped_solid(&mutated, TOL).is_err());
}

#[test]
fn refuses_malformed_or_unbounded_analytic_topology() {
    let source = step::write_warped_union(&fixture(), ORIGIN, U, V).unwrap();
    for (kind, args) in [
        ("ADVANCED_FACE", "'',()"),
        ("ADVANCED_FACE", "'',(),#1,.T."),
        ("EDGE_LOOP", "''"),
        ("ORIENTED_EDGE", "'',*,*"),
        ("EDGE_CURVE", "'',#1,#1"),
        ("PLANE", "'',#1,#1"),
    ] {
        let mutated = rewrite(&source, &format!("={kind}("), |line| {
            format!("{}={kind}({args});", line.split_once('=').unwrap().0)
        });
        assert!(read_warped_solid(&mutated, TOL).is_err(), "{kind}: {args}");
    }
    let empty_bounds = rewrite(&source, "=ADVANCED_FACE(", |line| {
        let (prefix, remainder) = line.split_once(",(").unwrap();
        let (_, suffix) = remainder.split_once("),").unwrap();
        format!("{prefix},(),{suffix}")
    });
    assert_eq!(
        read_warped_solid(&empty_bounds, TOL).unwrap_err(),
        ReadError::Unsupported
    );
    let reverse = |line: &str| {
        let (prefix, end) = line.rsplit_once(',').unwrap();
        let orientation = if end.starts_with(".T.") { ".F." } else { ".T." };
        format!("{prefix},{orientation});")
    };
    for kind in ["=FACE_OUTER_BOUND(", "=ORIENTED_EDGE("] {
        assert!(read_warped_solid(&rewrite(&source, kind, reverse), TOL).is_err());
    }
    let ellipse_id = source
        .lines()
        .find(|line| line.contains("=ELLIPSE("))
        .unwrap()
        .split_once('=')
        .unwrap()
        .0;
    let curve = source
        .lines()
        .find(|line| line.contains(&format!(",{ellipse_id},")))
        .unwrap();
    assert!(read_warped_solid(&source.replacen(curve, &reverse(curve), 1), TOL).is_err());

    let mutated = rewrite(&source, "=CLOSED_SHELL(", |line| {
        let (begin, last) = line.rsplit_once(',').unwrap();
        assert!(last.ends_with("));"));
        format!("{begin}));")
    });
    assert_eq!(
        read_warped_solid(&mutated, TOL).unwrap_err(),
        ReadError::Invalid
    );
    let oversized = rewrite(&source, "=EDGE_LOOP(", |line| {
        let (prefix, args) = line.split_once(",(").unwrap();
        let id = args.split_once(',').unwrap().0;
        let references = vec![id; MAX_VERTICES + 1].join(",");
        format!("{prefix},({references}));")
    });
    assert_eq!(
        read_warped_solid(&oversized, TOL).unwrap_err(),
        ReadError::Limit
    );
    let late_loop = source
        .lines()
        .rfind(|line| line.contains("=EDGE_LOOP("))
        .unwrap();
    let (prefix, args) = late_loop.split_once(",(").unwrap();
    let id = args.split_once(',').unwrap().0;
    let references = vec![id; MAX_VERTICES + 1].join(",");
    let oversized = source.replacen(late_loop, &format!("{prefix},({references}));"), 1);
    assert_eq!(
        read_analytic_solid(&oversized, TOL).unwrap_err(),
        ReadError::Limit
    );
    let bad_radius = rewrite(&source, "=CYLINDRICAL_SURFACE(", |line| {
        format!("{},-2.0);", line.rsplit_once(',').unwrap().0)
    });
    assert_eq!(
        read_warped_solid(&bad_radius, TOL).unwrap_err(),
        ReadError::Invalid
    );
}

#[test]
fn refuses_a_separate_curve_or_surface_even_if_the_chart_is_valid() {
    let source = step::write_warped_union(&fixture(), [0.; 3], [1., 0., 0.], [0., 1., 0.]).unwrap();
    let entities = parse_entities(&source, TOL).unwrap();
    let mut reader = Shell::new(&entities, TOL, true);
    let solid = reader.represented_solids(true).unwrap()[0];
    let ImportedWarped {
        shell,
        origin_mm: origin,
        u,
        v,
    } = read_member(&mut reader, solid).unwrap().1;
    let up = cross(u, v);
    let band = Band {
        center_z_mm: 2. - origin[2],
        radius_mm: 2.,
    };
    let chart = Chart {
        origin,
        u,
        v,
        up,
        band,
    };
    let mut boundary = Boundary::parse(&mut reader, solid).unwrap();
    verify_geometry(&reader, &boundary, chart, &shell).unwrap();
    let (first, _) = boundary.curves.first_key_value().unwrap();
    let point = boundary.curves[first].0[0];
    let original = reader.points[&point];
    reader.points.get_mut(&point).unwrap()[0] += 1e-2;
    assert_eq!(
        reconstruct(&reader, &boundary, chart),
        Err(ReadError::Invalid)
    );
    assert_eq!(
        verify_geometry(&reader, &boundary, chart, &shell),
        Err(ReadError::Invalid)
    );
    reader.points.insert(point, original);
    reader.points.insert(u32::MAX, original);
    assert_eq!(
        verify_geometry(&reader, &boundary, chart, &shell),
        Err(ReadError::Invalid)
    );
    reader.points.remove(&u32::MAX);

    let (&trim, &(_, TrimCurve::Arc(original))) = boundary
        .curves
        .iter()
        .find(|(_, (_, trim))| matches!(trim, TrimCurve::Arc(_)))
        .unwrap()
    else {
        unreachable!()
    };
    boundary.curves.get_mut(&trim).unwrap().1 = TrimCurve::Line;
    assert_eq!(
        verify_geometry(&reader, &boundary, chart, &shell),
        Err(ReadError::Invalid)
    );
    let shape = original.ellipse();
    let center = shape.center_mm().map(|x| x + 1.);
    let shifted = Ellipse::new(
        center,
        shape.normal(),
        shape.major_axis(),
        shape.semiaxes_mm(),
    )
    .unwrap();
    boundary.curves.get_mut(&trim).unwrap().1 = TrimCurve::Arc(
        EllipseArc::new(shifted, original.start_rad(), original.sweep_rad()).unwrap(),
    );
    assert_eq!(
        verify_geometry(&reader, &boundary, chart, &shell),
        Err(ReadError::Invalid)
    );

    boundary.faces[0].surface = Surface::Cylinder {
        origin: [0.; 3],
        axis: [1., 0., 0.],
        radius: 2.,
        outward: true,
    };
    assert_eq!(
        verify_geometry(&reader, &boundary, chart, &shell),
        Err(ReadError::Invalid)
    );
}

#[test]
fn inverse_height_keeps_the_interior_of_the_round_band() {
    let chart = Chart {
        origin: [0.; 3],
        u: [1., 0., 0.],
        v: [0., 1., 0.],
        up: [0., 0., 1.],
        band: Band {
            center_z_mm: 2.,
            radius_mm: 2.,
        },
    };
    assert!((chart.height(2. - 3_f64.sqrt(), TOL) - 1.).abs() < 1e-12);
}

#[test]
fn reimport_refuses_an_out_of_range_chart() {
    let geometry = fixture_with_block_start(-1e8);
    let source = step::write_warped_union(&geometry, [0.; 3], [1., 0., 0.], [0., 1., 0.]).unwrap();
    assert_eq!(
        read_warped_solid(&source, TOL).unwrap_err(),
        ReadError::Limit
    );
}

pub(super) fn boss_step_fixture() -> (warped_union::Shell, String) {
    use crate::warped_union::Boss;
    let piece = ConvexPiece::from_box([0., 0., 3.], [10., 80., 30.], TOL).unwrap();
    let base = warped_union::remap(
        &Union::new(&[piece], TOL).unwrap(),
        Band {
            center_z_mm: 10.,
            radius_mm: 2.,
        },
        TOL,
    )
    .unwrap();
    let bosses = [(20., 2.), (60., 3.)].map(|(y, radius_mm)| Boss {
        start_mm: [9.95, y, 20.],
        end_mm: [13.5, y, 20.],
        radius_mm,
        source: 1,
        wall_source_face: 2,
        cap_source_face: 1,
    });
    let shell = base.attach_bosses(&bosses, TOL).unwrap();
    let source = step::write_warped_union(&shell, ORIGIN, U, V).unwrap();
    (shell, source)
}

#[test]
fn round_trips_two_bosses_with_exact_rim_phase_in_an_oblique_frame() {
    let (shell, source) = boss_step_fixture();
    let (metrics, imported) = read_import_solid(&source, TOL).unwrap();
    assert_eq!(metrics.arcs.len(), 16);
    assert_eq!(metrics.solid_count, 1);
    assert!((metrics.volume_mm3 - shell.volume_mm3()).abs() < TOL);
    let ImportProfile::WarpedUnion { shell: output } = imported.profile else {
        unreachable!()
    };
    assert!(output.faces().iter().any(|f| f.holes.len() == 2));
    let rewritten =
        step::write_warped_union(&output, imported.origin_mm, imported.u, imported.v).unwrap();
    let second = read_analytic_solid(&rewritten, TOL).unwrap();
    assert!((second.volume_mm3 - metrics.volume_mm3).abs() < TOL);
    assert_eq!(second.arcs.len(), metrics.arcs.len());
    for i in 0..3 {
        assert!((second.bounds_mm[0][i] - metrics.bounds_mm[0][i]).abs() < TOL);
        assert!((second.bounds_mm[1][i] - metrics.bounds_mm[1][i]).abs() < TOL);
    }
}

#[test]
fn rejects_duplicate_outer_bounds_and_nonwarped_standalone_shell() {
    let (_, source) = boss_step_fixture();
    let malformed = source.replacen("=FACE_BOUND(", "=FACE_OUTER_BOUND(", 1);
    assert_eq!(
        read_warped_solid(&malformed, TOL).unwrap_err(),
        ReadError::Invalid
    );

    let mut writer = Writer::new();
    writer
        .append_box([crate::Rational::new(4, 1).unwrap(); 3], ORIGIN, U, V)
        .unwrap();
    assert_eq!(
        read_warped_solid(&writer.finish().unwrap(), TOL).unwrap_err(),
        ReadError::Unsupported
    );
}
