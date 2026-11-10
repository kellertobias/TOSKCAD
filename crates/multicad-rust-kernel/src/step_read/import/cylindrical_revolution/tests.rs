use super::*;
use crate::step::{Writer, write_cylindrical_revolution, write_placed_annulus};

const TOLERANCE: f64 = 1e-7;
const BUTTON: [MeridianPoint; 10] = [
    [0., 0.],
    [7.5, 0.],
    [7.5, 4.],
    [6., 4.],
    [6., 5.5],
    [8., 5.5],
    [8., 7.5],
    [6., 7.5],
    [6., 25.5],
    [0., 25.5],
];

fn near(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() <= TOLERANCE,
        "{actual} != {expected}"
    );
}

fn revolved(meridian: &[MeridianPoint]) -> Revolution {
    Revolution::new(vec![meridian.to_vec()], TAU).unwrap()
}

#[test]
fn reconstructs_the_button_and_an_oblique_stepped_annulus() {
    let button = revolved(&BUTTON);
    let text =
        write_cylindrical_revolution(&button, [0., -5.5, 0.], [1., 0., 0.], [0., 0., -1.]).unwrap();
    let (verified, imported) = read_import_solid(&text, TOLERANCE).unwrap();
    assert_eq!(imported.origin_mm, [0., -5.5, 0.]);
    assert_eq!(imported.height_mm, 25.5);
    assert_eq!(
        imported.profile,
        ImportProfile::CylindricalRevolution {
            meridian: BUTTON.into()
        }
    );
    assert_eq!(verified.face_count, 9);
    assert_eq!(verified.bounds_mm, [[-8., -5.5, -8.], [8., 20., 8.]]);
    near(verified.volume_mm3, 1055. * PI);

    let meridian = [
        [2., 0.],
        [4., 0.],
        [4., 2.],
        [5., 2.],
        [5., 6.],
        [1., 6.],
        [1., 2.],
        [2., 2.],
    ];
    let diagonal = std::f64::consts::FRAC_1_SQRT_2;
    let mut writer = Writer::new();
    writer
        .append_cylindrical_revolution(
            &revolved(&meridian),
            [6., 9., 15.],
            [0., 0., 1.],
            [-diagonal, -diagonal, 0.],
        )
        .unwrap();
    writer
        .append_cylindrical_revolution(&button, [10., 20., 15.], [1., 0., 0.], [0., 0., 1.])
        .unwrap();
    let text = writer.finish().unwrap();
    let members = read_import_compound(&text, TOLERANCE).unwrap();
    assert_eq!(members.len(), 2);
    for ((verified, imported), volume) in members.into_iter().zip([120. * PI, 1055. * PI]) {
        let ImportProfile::CylindricalRevolution { meridian } = imported.profile else {
            panic!("Expected full stepped revolution");
        };
        assert!(preferred_axis(cross(imported.u, imported.v)));
        near(verified.volume_mm3, volume);
        near(revolved(&meridian).volume_mm3(), volume);
    }
}

#[test]
fn uniform_annuli_keep_the_existing_import_profile() {
    let annulus = crate::annulus::Annulus::new(1., 4., 6.).unwrap();
    let text = write_placed_annulus(&annulus, [0.; 3], [1., 0., 0.], [0., 1., 0.]).unwrap();
    let (_, imported) = read_import_solid(&text, TOLERANCE).unwrap();
    assert_eq!(
        imported.profile,
        ImportProfile::Annulus {
            inner_radius_mm: 1.,
            outer_radius_mm: 4.
        }
    );
}

#[test]
fn the_revolution_recognizer_refuses_noncoaxial_bores_and_conical_walls() {
    use crate::{
        Circle, Rational,
        polygon::PolygonPrism,
        step::{write_chamfered_polygon_prism, write_polygon_prism},
    };

    let n = |integer| Rational::new(integer, 1).unwrap();
    let holes = [4, 10].map(|x| Circle::new([n(x), n(4)], n(2)).unwrap());
    let stock = |holes| {
        PolygonPrism::with_circular_holes(
            vec![
                [[0, 0], [14, 0], [14, 8], [0, 8]]
                    .map(|p| p.map(n))
                    .to_vec(),
            ],
            holes,
            n(4),
        )
        .unwrap()
    };
    let [origin, u, v] = [[0.; 3], [1., 0., 0.], [0., 1., 0.]];
    let text = write_polygon_prism(&stock(holes.to_vec()), origin, u, v, false).unwrap();
    let (readback, solid) = read_import_solid(&text, TOLERANCE).unwrap();
    assert!(matches!(solid.profile, ImportProfile::Polygon { .. }));

    let entities = parse_entities(&text, TOLERANCE).unwrap();
    let mut shell = Shell::new(&entities, TOLERANCE, true);
    let faces = shell.validated_faces().unwrap();
    assert!(shell.cylindrical_revolution(&faces, &readback).is_none());

    let chamfer = BoreChamfer {
        hole: 0,
        end: false,
        size_mm: 1.,
    };
    let text =
        write_chamfered_polygon_prism(&stock(vec![holes[0]]), &[chamfer], origin, u, v, false)
            .unwrap();
    let entities = parse_entities(&text, TOLERANCE).unwrap();
    let mut shell = Shell::new(&entities, TOLERANCE, true);
    let mut faces = shell.validated_faces().unwrap();
    let readback = shell.readback(&faces).unwrap();
    let cone = faces
        .iter()
        .position(|face| matches!(face, Face::Conical(_)))
        .unwrap();
    faces.swap(0, cone);
    assert!(shell.cylindrical_revolution(&faces, &readback).is_none());
}

#[test]
fn each_wall_and_shoulder_must_preserve_the_verified_rims() {
    let text =
        write_cylindrical_revolution(&revolved(&BUTTON), [0.; 3], [1., 0., 0.], [0., 1., 0.])
            .unwrap();
    let entities = parse_entities(&text, TOLERANCE).unwrap();
    let reject = |change: fn(&mut Shell<'_, '_>, &mut Vec<Face>, &mut Readback)| {
        let mut shell = Shell::new(&entities, TOLERANCE, true);
        let mut faces = shell.validated_faces().unwrap();
        let mut readback = shell.readback(&faces).unwrap();
        change(&mut shell, &mut faces, &mut readback);
        assert!(shell.cylindrical_revolution(&faces, &readback).is_none());
    };

    reject(|_, faces, _| {
        faces.resize_with(MAX_VERTICES + 1, || Face::Cylindrical { seam: 0, flux: 1. });
    });
    reject(|_, faces, _| {
        let Face::Cylindrical { seam, .. } = &mut faces[1] else {
            panic!("expected first cylindrical wall");
        };
        *seam = 0;
    });
    reject(|shell, faces, _| {
        let cap = shell.indexed_edges(faces.len())[0][0];
        let Curve::Circle(circle) = &mut shell.edges.get_mut(&cap).unwrap().curve else {
            panic!("expected lower circular rim");
        };
        circle.radius_mm += 0.5;
    });
    reject(|shell, faces, _| {
        let trims = shell.indexed_edges(faces.len());
        let inner = *trims[2].iter().find(|id| !trims[1].contains(id)).unwrap();
        let Curve::Circle(circle) = &mut shell.edges.get_mut(&inner).unwrap().curve else {
            panic!("expected inner shoulder rim");
        };
        circle.center_mm[2] += 0.5;
    });
    reject(|shell, faces, _| {
        let cap = shell.indexed_edges(faces.len())[0][0];
        shell
            .edges
            .get_mut(&cap)
            .unwrap()
            .uses
            .retain(|use_| use_.face != 0);
    });
    for face in [0, 1, 2] {
        let mut shell = Shell::new(&entities, TOLERANCE, true);
        let faces = shell.validated_faces().unwrap();
        let readback = shell.readback(&faces).unwrap();
        let trims = shell.indexed_edges(faces.len());
        let rim = if face == 0 {
            trims[0][0]
        } else if face == 1 {
            *trims[1].iter().find(|id| trims[2].contains(id)).unwrap()
        } else {
            *trims[2].iter().find(|id| !trims[1].contains(id)).unwrap()
        };
        shell.edges.get_mut(&rim).unwrap().curve = Curve::Line;
        assert!(shell.cylindrical_revolution(&faces, &readback).is_none());
    }
    reject(|_, faces, _| {
        let Face::Planar { normal, .. } = &mut faces[0] else {
            panic!("expected first disk");
        };
        *normal = [1., 0., 0.];
    });
    reject(|_, faces, _| {
        let Face::Cylindrical { flux, .. } = &mut faces[1] else {
            panic!("expected first cylindrical wall");
        };
        *flux = -*flux;
    });
    reject(|shell, faces, _| {
        let trims = shell.indexed_edges(faces.len());
        let inner = *trims[2].iter().find(|id| !trims[1].contains(id)).unwrap();
        shell
            .edges
            .get_mut(&inner)
            .unwrap()
            .uses
            .retain(|use_| use_.face != 2);
    });
    reject(|shell, _, _| {
        for edge in shell.edges.values_mut() {
            if let Curve::Circle(circle) = &mut edge.curve {
                circle.radius_mm *= 1e7;
            }
        }
    });
    reject(|_, _, readback| readback.volume_mm3 += 1.);
}

#[test]
fn the_axis_closure_requires_a_single_positive_length_interval() {
    let [lower, upper, third] = [0, 1, 2].map(Node::Axis);
    let mut points = BTreeMap::from([(lower, [0., 0.]), (upper, [0., 2.])]);
    let mut links = BTreeMap::new();
    assert_eq!(close_axis(&[lower], &points, &mut links, TOLERANCE), None);
    assert_eq!(
        close_axis(&[lower, upper, third], &points, &mut links, TOLERANCE),
        None
    );
    assert_eq!(
        close_axis(&[upper, lower], &points, &mut links, TOLERANCE),
        Some(())
    );
    assert_eq!(links[&upper], lower);
    assert_eq!(
        close_axis(&[upper, lower], &points, &mut links, TOLERANCE),
        None
    );
    points.insert(upper, [0., TOLERANCE]);
    links.clear();
    assert_eq!(
        close_axis(&[lower, upper], &points, &mut links, TOLERANCE),
        None
    );
    assert!(links.is_empty());
}

#[test]
fn a_verified_meridian_must_agree_with_the_independent_readback() {
    let revolution = revolved(&BUTTON);
    let text =
        write_cylindrical_revolution(&revolution, [0.; 3], [1., 0., 0.], [0., 1., 0.]).unwrap();
    let (mut verified, _) = read_import_solid(&text, TOLERANCE).unwrap();
    let basis = [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]];
    verified.bounds_mm[1][2] += 0.1;
    assert!(!matches_readback(
        &revolution,
        &BUTTON,
        [0.; 3],
        basis,
        &verified,
        TOLERANCE
    ));
    verified.volume_mm3 += 1.;
    assert!(!matches_readback(
        &revolution,
        &BUTTON,
        [0.; 3],
        basis,
        &verified,
        TOLERANCE
    ));
}

#[test]
fn the_rim_graph_must_be_one_complete_cycle() {
    let [a, b, c, d, missing] = [1, 2, 3, 4, 5].map(Node::Rim);
    let points = BTreeMap::from([(a, [1., 0.]), (b, [2., 0.]), (c, [2., 1.]), (d, [1., 1.])]);
    assert!(cycle(&points, &BTreeMap::from([(a, b), (b, c), (c, d)])).is_none());
    assert!(cycle(&points, &BTreeMap::from([(a, b), (b, a), (c, d), (d, c)])).is_none());
    assert!(cycle(&points, &BTreeMap::from([(a, b), (b, c), (c, d), (d, c)])).is_none());
    assert!(
        cycle(
            &points,
            &BTreeMap::from([(a, b), (b, c), (c, missing), (d, a)])
        )
        .is_none()
    );
    assert!(
        cycle(
            &points,
            &BTreeMap::from([(a, b), (b, c), (c, d), (missing, a)])
        )
        .is_none()
    );
    assert!(cycle(&BTreeMap::new(), &BTreeMap::new()).is_none());
    assert!(cycle(&points, &BTreeMap::from([(a, b), (b, c), (c, d), (d, a)])).is_some());
}

#[test]
fn conical_frustum_requires_one_outward_wall_and_its_two_shared_rims() {
    let cone = revolved(&[[0., 0.], [2., 0.], [4., 20.], [0., 20.]]);
    let mut writer = Writer::new();
    writer
        .append_conical_frustum(&cone, [0.; 3], [1., 0., 0.], [0., 1., 0.], TOLERANCE)
        .unwrap();
    let text = writer.finish().unwrap();
    let entities = parse_entities(&text, TOLERANCE).unwrap();
    let mut shell = Shell::new(&entities, TOLERANCE, true);
    let mut faces = shell.validated_faces().unwrap();
    let readback = shell.readback(&faces).unwrap();
    assert!(shell.conical_revolution(&faces[..2], &readback).is_none());
    let wall = faces
        .iter_mut()
        .find_map(|face| {
            if let Face::Conical(wall) = face {
                Some(wall)
            } else {
                None
            }
        })
        .unwrap();
    wall.seam = 0;
    assert!(shell.conical_revolution(&faces, &readback).is_none());
}

#[test]
fn reconstruction_refuses_a_taper_below_the_requested_resolution() {
    let cone = revolved(&[[0., 0.], [2., 0.], [2.0002, 20.], [0., 20.]]);
    let mut writer = Writer::new();
    writer
        .append_conical_frustum(&cone, [0.; 3], [1., 0., 0.], [0., 1., 0.], TOLERANCE)
        .unwrap();
    let text = writer.finish().unwrap();
    let entities = parse_entities(&text, TOLERANCE).unwrap();
    let mut shell = Shell::new(&entities, TOLERANCE, true);
    let faces = shell.validated_faces().unwrap();
    let readback = shell.readback(&faces).unwrap();
    assert!(shell.conical_revolution(&faces, &readback).is_some());

    // Keep the tighter face proofs to isolate the loft reconstruction check.
    shell.tolerance = 1e-3;
    assert!(shell.conical_revolution(&faces, &readback).is_none());
}
