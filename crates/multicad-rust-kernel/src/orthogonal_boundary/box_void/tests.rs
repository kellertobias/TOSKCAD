use super::*;
use crate::orthogonal::{Box3, Operation, boolean};
pub(crate) fn fixture() -> Boundary {
    let outer = Box3 {
        min: [0.; 3],
        max: [15., 10., 8.],
    };
    let inner = Box3 {
        min: [2., 3., 1.],
        max: [7., 7., 4.],
    };
    let solid = boolean(&[outer], &[inner], Operation::Subtract, 1e-7)
        .unwrap()
        .unwrap();
    super::super::build(solid.quads()).unwrap()
}
#[test]
fn exact_box_cavity_has_one_material_component_and_two_support_groups() {
    let source = fixture();
    let proof = boundary_certificate(&source).unwrap();
    assert_eq!(proof.groups[0].len(), 6);
    assert_eq!(proof.groups[1].len(), 6);
    assert_eq!(proof.solid.solid_count(), 1);
    assert_eq!(proof.solid.volume_mm3(), 1140.);
}

fn faces() -> Vec<OrientedFace> {
    fixture()
        .faces
        .into_iter()
        .map(|face| OrientedFace {
            normal: face.normal,
            rings: face.rings,
        })
        .collect()
}

#[test]
fn proof_refuses_missing_invalid_touched_and_nonrectangular_boundary_data() {
    assert!(recognize(&[], 0.).is_none());
    let mut source = fixture();
    source.shell_count = 1;
    assert!(boundary_certificate(&source).is_err());
    source.shell_count = 2;
    source.faces[0].rings.push(vec![]);
    assert!(boundary_certificate(&source).is_err());
    let mut source = faces();
    source[0].normal = source[0].normal.map(|n| 2. * n);
    assert!(recognize(&source, 0.).is_none());
    let mut source = faces();
    source[3].rings.push(vec![]);
    assert!(recognize(&source, 0.).is_none());
    let mut source = faces();
    source[3].rings[0].truncate(3);
    assert!(recognize(&source, 0.).is_none());
    let mut source = faces();
    source[3].normal = [0.6, 0.8, 0.];
    assert!(recognize(&source, 0.).is_none());
    let mut source = faces();
    source[3].rings[0][0][0] = f64::NAN;
    assert!(recognize(&source, 0.).is_none());
    let source = faces();
    let proof = recognize(&source, 0.).unwrap();
    let exterior = proof.groups[0]
        .iter()
        .map(|&i| OrientedFace {
            normal: source[i].normal,
            rings: source[i].rings.clone(),
        })
        .collect::<Vec<_>>();
    assert!(recognize(&exterior, 0.).is_none());
    let mut source = faces();
    for &i in &proof.groups[1] {
        for point in &mut source[i].rings[0] {
            if point[0] == 2. {
                point[0] = 0.;
            }
        }
    }
    assert!(recognize(&source, 0.).is_none());
    let mut source = faces();
    let i = proof.groups[1][0];
    source[i].rings[0][0][0] += 0.125;
    assert!(recognize(&source, 0.).is_none());
    let mut source = faces();
    source.remove(proof.groups[1][0]);
    assert!(recognize(&source, 0.).is_none());
}

#[test]
fn proof_restores_roundoff_in_a_rigid_chart_without_merging_distinct_supports() {
    let u = [0.8, 0.6, 0.];
    let v = [-0.36, 0.48, 0.8];
    let w = cross(u, v);
    let origin = [13., -5., 41.];
    let faces = faces()
        .into_iter()
        .map(|face| OrientedFace {
            normal: std::array::from_fn(|i| {
                u[i] * face.normal[0] + v[i] * face.normal[1] + w[i] * face.normal[2]
            }),
            rings: face
                .rings
                .into_iter()
                .map(|ring| {
                    ring.into_iter()
                        .map(|point| {
                            std::array::from_fn(|i| {
                                origin[i] + u[i] * point[0] + v[i] * point[1] + w[i] * point[2]
                            })
                        })
                        .collect()
                })
                .collect(),
        })
        .collect::<Vec<_>>();
    let proof = recognize(&faces, 1e-7).unwrap();
    assert!((proof.solid.volume_mm3() - 1140.).abs() < 1e-7);
}

#[test]
fn cardinal_proof_does_not_weld_distinct_planes_inside_the_callers_tolerance() {
    let mut source = faces();
    source[3].rings[0][0][0] += 1e-9;
    assert!(recognize(&source, 1e-7).is_none());
}

#[test]
fn material_equivalence_does_not_admit_unpaired_duplicate_or_degenerate_edges() {
    let mut source = faces();
    let a = source[0].rings[0][0];
    let b = source[0].rings[0][1];
    let midpoint = std::array::from_fn(|i| a[i].midpoint(b[i]));
    source[0].rings[0].insert(1, midpoint);
    assert!(recognize(&source, 1e-7).is_none());
    let mut source = faces();
    source.push(OrientedFace {
        normal: source[0].normal,
        rings: source[0].rings.clone(),
    });
    assert!(recognize(&source, 1e-7).is_none());
    let mut source = faces();
    let point = source[0].rings[0][0];
    source[0].rings[0].insert(1, point);
    assert!(recognize(&source, 1e-7).is_none());
    let mut source = faces();
    source[0].rings[0].reverse();
    assert!(recognize(&source, 1e-7).is_none());
}

fn box_faces(bounds: Box3, inward: bool) -> Vec<OrientedFace> {
    let solid = boolean(&[bounds], &[], Operation::Union, 0.)
        .unwrap()
        .unwrap();
    super::super::build(solid.quads())
        .unwrap()
        .faces
        .into_iter()
        .map(|mut face| {
            if inward {
                face.normal = face.normal.map(|n| -n);
                for ring in &mut face.rings {
                    ring.reverse();
                }
            }
            OrientedFace {
                normal: face.normal,
                rings: face.rings,
            }
        })
        .collect()
}
fn enclosed(outer: Box3, holes: &[Box3]) -> Vec<OrientedFace> {
    box_faces(outer, false)
        .into_iter()
        .chain(holes.iter().flat_map(|&bounds| box_faces(bounds, true)))
        .collect()
}

#[test]
fn any_admitted_count_of_separated_cavities_keeps_one_material_and_ordered_components() {
    for count in [2, 5, 17] {
        let outer = Box3 {
            min: [0.; 3],
            max: [(2 * count + 2) as f64, 4., 4.],
        };
        let holes = (0..count)
            .map(|i| Box3 {
                min: [(2 * i + 1) as f64, 1., 1.],
                max: [(2 * i + 2) as f64, 2., 2.],
            })
            .collect::<Vec<_>>();
        let mut faces = enclosed(outer, &holes);
        // The first supplied component need not be the exterior.
        faces.rotate_left(6);
        let certificate = certify(&faces, 1e-7).unwrap();
        assert_eq!(certificate.groups.len(), count + 1);
        assert_eq!(
            certificate.groups[0],
            (count * 6..(count + 1) * 6).collect::<Vec<_>>()
        );
        assert_eq!(certificate.solid.solid_count(), 1);
        assert!(
            (certificate.solid.volume_mm3() - ((2 * count + 2) * 16 - count) as f64).abs() < 1e-7
        );
        assert_eq!(
            super::super::build(certificate.solid.quads())
                .unwrap()
                .shell_count,
            count + 1
        );
    }
}

#[test]
fn touching_intersecting_nested_outside_and_inverted_cavities_are_not_material() {
    let outer = Box3 {
        min: [0.; 3],
        max: [20.; 3],
    };
    let first = Box3 {
        min: [2.; 3],
        max: [6.; 3],
    };
    for second in [
        Box3 {
            min: [6., 2., 2.],
            max: [8., 6., 6.],
        },
        Box3 {
            min: [4.; 3],
            max: [8.; 3],
        },
        Box3 {
            min: [3.; 3],
            max: [4.; 3],
        },
        Box3 {
            min: [19.; 3],
            max: [21.; 3],
        },
        Box3 {
            min: [6. + 1e-8, 2., 2.],
            max: [8., 6., 6.],
        },
    ] {
        assert!(matches!(
            certify(&enclosed(outer, &[first, second]), 1e-7),
            Err(BooleanError::Invalid)
        ));
    }
    let mut faces = enclosed(
        outer,
        &[
            first,
            Box3 {
                min: [10.; 3],
                max: [12.; 3],
            },
        ],
    );
    for face in &mut faces[12..] {
        face.normal = face.normal.map(|n| -n);
        face.rings[0].reverse();
    }
    assert!(certify(&faces, 1e-7).is_err());
    assert!(certify(&enclosed(outer, &[first, first]), 1e-7).is_err());
}

#[test]
fn resource_exhaustion_reports_limits_before_unbounded_copy_pair_or_grid_work() {
    let mut faces = (0..MAX_QUADS + 1)
        .map(|_| OrientedFace {
            normal: [1., 0., 0.],
            rings: Vec::new(),
        })
        .collect::<Vec<_>>();
    assert!(matches!(certify(&faces, 0.), Err(BooleanError::Limit)));
    faces.clear();
    faces.push(OrientedFace {
        normal: [1., 0., 0.],
        rings: vec![vec![[0.; 3]; MAX_WORK + 1]],
    });
    assert!(matches!(certify(&faces, 0.), Err(BooleanError::Limit)));
    let holes = (0..1250)
        .map(|i| Box3 {
            min: [(2 * i + 1) as f64, 1., 1.],
            max: [(2 * i + 2) as f64, 2., 2.],
        })
        .collect::<Vec<_>>();
    assert!(matches!(
        certify(
            &enclosed(
                Box3 {
                    min: [0.; 3],
                    max: [2502., 4., 4.]
                },
                &holes
            ),
            0.
        ),
        Err(BooleanError::Limit)
    ));
    let holes = (0..25)
        .map(|i| Box3 {
            min: [(2 * i + 1) as f64; 3],
            max: [(2 * i + 2) as f64; 3],
        })
        .collect::<Vec<_>>();
    assert!(matches!(
        certify(
            &enclosed(
                Box3 {
                    min: [0.; 3],
                    max: [52.; 3]
                },
                &holes
            ),
            0.
        ),
        Err(BooleanError::Limit)
    ));
}

#[test]
fn declared_shell_count_must_match_the_exact_discovered_components() {
    let mut source = fixture();
    source.shell_count = 3;
    assert!(matches!(
        boundary_certificate(&source),
        Err(BooleanError::Invalid)
    ));
}
