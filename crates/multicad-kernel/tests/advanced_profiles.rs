use multicad_core::{document::EntityId, references::*};
use multicad_kernel::*;
fn p() -> GeometryPolicy {
    GeometryPolicy::default()
}

#[cfg(feature = "occt")]
#[test]
fn exact_bilinear_loft_step_reads_in_opencascade() {
    use multicad_rust_kernel::{planar_boolean::NonplanarLoft, step::write_nonplanar_loft};

    let low = [[-2., -1.], [2., -1.], [2., 1.], [-2., 1.]];
    let (sin, cos) = std::f64::consts::FRAC_PI_6.sin_cos();
    let high = low.map(|[x, y]| [cos * x - sin * y + 1., sin * x + cos * y + 0.5]);
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("twisted.step");
    for clockwise in [false, true] {
        let mut rings = [low, high];
        if clockwise {
            rings.iter_mut().for_each(|ring| ring.reverse());
        }
        let loft = NonplanarLoft::new([&rings[0], &rings[1]], 6., p().linear_tolerance_mm).unwrap();
        let origin = [13., -7., 4.];
        let u = [0.8, 0.6, 0.];
        let v = [-0.36, 0.48, 0.8];
        std::fs::write(&path, write_nonplanar_loft(&loft, origin, u, v).unwrap()).unwrap();
        let solid = Solid::read_step(&path, p()).unwrap();
        assert_eq!(solid.solid_count().unwrap(), 1);
        let faces = solid.faces().unwrap();
        assert_eq!(faces.len(), 6);
        near(solid.volume_mm3().unwrap(), loft.volume_mm3());
        let analytic_area = loft.cap_areas_mm2().into_iter().sum::<f64>()
            + loft
                .walls()
                .map(|wall| wall.area_mm2().unwrap())
                .sum::<f64>();
        near(faces.iter().map(|face| face.area_mm2).sum(), analytic_area);
        let placed = |[x, y, z]: [f64; 3]| {
            PlaneFrame { origin, u, v }
                .point([x, y])
                .into_iter()
                .zip([0.48, -0.64, 0.6])
                .map(|(base, n)| base + n * z)
                .collect::<Vec<_>>()
        };
        let points: Vec<_> = loft.rings().iter().flatten().copied().map(placed).collect();
        let bounds = solid.bounds().unwrap();
        for i in 0..3 {
            near(
                bounds.min[i],
                points.iter().map(|p| p[i]).fold(f64::INFINITY, f64::min),
            );
            near(
                bounds.max[i],
                points
                    .iter()
                    .map(|p| p[i])
                    .fold(f64::NEG_INFINITY, f64::max),
            );
        }
    }
}
fn near(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-5, "{a} != {b}");
}
fn rect(w: f64, h: f64, origin: [f64; 3]) -> CurvedProfile {
    let points = [[0., 0.], [w, 0.], [w, h], [0., h]];
    CurvedProfile {
        frame: PlaneFrame {
            origin,
            ..Default::default()
        },
        outer: (0..4)
            .map(|i| CurvedEdge2d::Line {
                a: points[i],
                b: points[(i + 1) % 4],
            })
            .collect(),
        holes: vec![],
    }
}
fn origins(count: usize) -> Vec<Origin> {
    (0..count)
        .map(|_| Origin {
            source: EntityId::new(),
            role: "curve".into(),
        })
        .collect()
}
fn verify(solid: &Solid, feature: EntityId) {
    assert_eq!(solid.solid_count().unwrap(), 1);
    let snapshot = solid.topology_snapshot(feature, 0).unwrap();
    for role in ["cap.start", "cap.end"] {
        let origin = Origin {
            source: feature,
            role: role.into(),
        };
        let cap = snapshot
            .elements
            .iter()
            .find(|e| e.key.kind == TopologyKind::Face && e.origins.contains(&origin))
            .unwrap();
        assert!(matches!(cap.geometry, Some(AnalyticGeometry::Plane { .. })));
        assert!(cap.frames.iter().any(|frame| frame.origin == origin));
    }
    for element in snapshot
        .elements
        .iter()
        .filter(|e| e.key.kind == TopologyKind::Face)
    {
        GeometryIntent::capture(&snapshot, element.key).unwrap();
    }
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("profile.step");
    solid.write_step(&path).unwrap();
    let loaded = Solid::read_step(&path, p()).unwrap();
    near(loaded.volume_mm3().unwrap(), solid.volume_mm3().unwrap());
}
#[test]
fn straight_sweep_and_right_corner_polyline_are_real_named_solids() {
    let profile = rect(10., 6., [0.; 3]);
    let before = profile.clone();
    let feature = EntityId::new();
    let curves = origins(4);
    let result = Solid::sweep_polyline_named(
        &profile,
        &[[0.; 3], [0., 0., 30.]],
        feature,
        &curves,
        &origins(1),
        p(),
    )
    .unwrap();
    near(result.volume_mm3().unwrap(), 1800.);
    verify(&result, feature);
    assert_eq!(profile, before);
    let circle = CurvedProfile {
        frame: PlaneFrame::default(),
        outer: vec![CurvedEdge2d::Circle {
            center: [0.; 2],
            radius_mm: 2.,
        }],
        holes: vec![],
    };
    let elbow = Solid::sweep_polyline_named(
        &circle,
        &[[0.; 3], [0., 0., 20.], [20., 0., 20.]],
        feature,
        &origins(1),
        &origins(2),
        p(),
    )
    .unwrap();
    near(elbow.volume_mm3().unwrap(), std::f64::consts::PI * 4. * 40.);
    verify(&elbow, feature);
}
#[test]
fn loft_authored_sections_preserve_correspondence_named_references_and_step() {
    let feature = EntityId::new();
    let sections = vec![rect(20., 10., [0.; 3]), rect(10., 5., [5., 2.5, 30.])];
    let before = sections.clone();
    let names = vec![origins(4), origins(4)];
    for ruled in [true, false] {
        let result = Solid::loft_profiles_named(&sections, feature, &names, ruled, p()).unwrap();
        near(result.volume_mm3().unwrap(), 3500.);
        verify(&result, feature);
        let snapshot = result.topology_snapshot(feature, 0).unwrap();
        let selected = snapshot
            .elements
            .iter()
            .find(|e| e.key.kind == TopologyKind::Face && e.origins.contains(&names[0][0]))
            .unwrap();
        let intent = GeometryIntent::capture(&snapshot, selected.key).unwrap();
        let edited = vec![sections[0].clone(), rect(12., 6., [4., 2., 35.])];
        let next = Solid::loft_profiles_named(&edited, feature, &names, ruled, p()).unwrap();
        assert!(matches!(
            resolve(&intent, &next.topology_snapshot(feature, 1).unwrap(), 1),
            Resolution::Resolved { .. }
        ));
    }
    assert_eq!(sections, before);
    let three = vec![
        rect(10., 6., [0.; 3]),
        rect(14., 8., [-2., -1., 15.]),
        rect(10., 6., [0., 0., 30.]),
    ];
    let names = vec![origins(4), origins(4), origins(4)];
    let ruled = Solid::loft_profiles_named(&three, feature, &names, true, p()).unwrap();
    near(ruled.volume_mm3().unwrap(), 2540.);
    verify(&ruled, feature);
    let smooth = Solid::loft_profiles_named(&three, feature, &names, false, p()).unwrap();
    assert!(smooth.volume_mm3().unwrap() > 1800.);
    verify(&smooth, feature);
}
#[test]
fn unsupported_profiles_and_bad_paths_fail_explicitly_without_mutating_sources() {
    let feature = EntityId::new();
    let profile = rect(10., 6., [0.; 3]);
    let before = profile.clone();
    let curves = origins(4);
    for path in [
        vec![[0.; 3]],
        vec![[0., 0., 1.], [0., 0., 20.]],
        vec![[0.; 3], [20., 0., 0.]],
        vec![[0.; 3], [0.; 3]],
        vec![
            [0.; 3],
            [0., 0., 10.],
            [10., 0., 10.],
            [10., 0., 0.],
            [0., 0., 10.],
        ],
    ] {
        assert!(
            Solid::sweep_polyline_named(
                &profile,
                &path,
                feature,
                &curves,
                &origins(path.len().saturating_sub(1)),
                p()
            )
            .is_err()
        );
    }
    let mut holes = profile.clone();
    holes.holes.push(rect(2., 2., [0.; 3]).outer);
    assert!(
        Solid::sweep_polyline_named(
            &holes,
            &[[0.; 3], [0., 0., 20.]],
            feature,
            &curves,
            &origins(1),
            p()
        )
        .err()
        .unwrap()
        .message
        .contains("without holes")
    );
    let mut missing = rect(5., 3., [0., 0., 20.]);
    missing.outer.pop();
    assert!(
        Solid::loft_profiles_named(
            &[profile.clone(), missing],
            feature,
            &[curves.clone(), origins(3)],
            true,
            p()
        )
        .err()
        .unwrap()
        .message
        .contains("correspondence")
    );
    assert!(
        Solid::loft_profiles_named(
            &[profile.clone(), profile.clone()],
            feature,
            &[curves.clone(), origins(4)],
            true,
            p()
        )
        .is_err()
    );
    assert_eq!(profile, before);
}

#[test]
fn rust_straight_sweep_translates_the_authored_section_in_both_directions() {
    let feature = EntityId::new();
    let frame = PlaneFrame {
        origin: [11., -7., 23.],
        u: [1., 0., 0.],
        v: [0., 0.6, 0.8],
    };
    let mut rectangle = rect(10., 6., frame.origin);
    rectangle.frame = frame;
    let circle = CurvedProfile {
        frame,
        outer: vec![CurvedEdge2d::Circle {
            center: [4., 3.],
            radius_mm: 2.,
        }],
        holes: vec![],
    };
    let mixed = CurvedProfile {
        frame,
        outer: vec![
            CurvedEdge2d::Line {
                a: [0., 0.],
                b: [8., 0.],
            },
            CurvedEdge2d::Arc {
                center: [8., 3.],
                a: [8., 0.],
                b: [8., 6.],
                clockwise: false,
            },
            CurvedEdge2d::Line {
                a: [8., 6.],
                b: [0., 6.],
            },
            CurvedEdge2d::Line {
                a: [0., 6.],
                b: [0., 0.],
            },
        ],
        holes: vec![],
    };
    for (profile, area) in [
        (rectangle, 60.),
        (circle, 4. * std::f64::consts::PI),
        (mixed, 48. + 4.5 * std::f64::consts::PI),
    ] {
        let curve_origins = origins(profile.outer.len());
        let path_origins = origins(1);
        for height in [-30., 30.] {
            let start = frame.point([2.5, 2.]);
            let end = std::array::from_fn(|i| start[i] + frame.normal()[i] * height);
            let path = [start, end];
            let rust = Solid::rust_sweep_straight_named(
                &profile,
                &path,
                feature,
                &curve_origins,
                &path_origins,
                p(),
            )
            .unwrap();
            assert!(rust.uses_rust_geometry());
            assert_eq!(rust.solid_count().unwrap(), 1);
            near(rust.volume_mm3().unwrap(), area * height.abs());
            let topology = rust.topology_snapshot(feature, 0).unwrap();
            let path_roles = [
                path_origins[0].clone(),
                Origin {
                    source: feature,
                    role: format!("side/{}/{}", path_origins[0].source, path_origins[0].role),
                },
            ];
            let check_path_ancestry = |snapshot: &TopologySnapshot| {
                for element in &snapshot.elements {
                    let side = element.key.kind != TopologyKind::Face
                        || curve_origins
                            .iter()
                            .any(|origin| element.origins.contains(origin));
                    for origin in &path_roles {
                        assert_eq!(element.origins.contains(origin), side, "{:?}", element.key);
                        if side && matches!(element.geometry, Some(AnalyticGeometry::Plane { .. }))
                        {
                            assert!(element.frames.iter().any(|anchor| anchor.origin == *origin));
                        }
                    }
                }
            };
            check_path_ancestry(&topology);
            for origin in curve_origins
                .iter()
                .cloned()
                .chain(["cap.start", "cap.end"].map(|role| Origin {
                    source: feature,
                    role: role.into(),
                }))
            {
                assert_eq!(
                    topology
                        .elements
                        .iter()
                        .filter(|element| element.key.kind == TopologyKind::Face
                            && element.origins.contains(&origin))
                        .count(),
                    1,
                    "{origin:?}"
                );
            }
            let receipt = rust
                .manufacturing_mesh_with_receipt(Default::default())
                .unwrap();
            assert!(!receipt.mesh.indices.is_empty());

            #[cfg(feature = "occt")]
            {
                let native = Solid::sweep_polyline_named(
                    &profile,
                    &path,
                    feature,
                    &curve_origins,
                    &path_origins,
                    p(),
                )
                .unwrap();
                check_path_ancestry(&native.topology_snapshot(feature, 0).unwrap());
                near(rust.volume_mm3().unwrap(), native.volume_mm3().unwrap());
                assert_eq!(rust.faces().unwrap().len(), native.faces().unwrap().len());
                let surface_area =
                    |solid: &Solid| solid.faces().unwrap().iter().map(|f| f.area_mm2).sum();
                near(surface_area(&rust), surface_area(&native));
                let bounds = |solid: &Solid| {
                    let b = solid.bounds().unwrap();
                    b.min.into_iter().chain(b.max)
                };
                for (a, b) in bounds(&rust).zip(bounds(&native)) {
                    near(a, b);
                }
            }
        }
    }
}

#[test]
fn rust_sweep_refuses_unsupported_transitions_and_invalid_placement() {
    let feature = EntityId::new();
    let profile = rect(10., 6., [0.; 3]);
    let curves = origins(4);
    for (path, kind, message) in [
        (
            vec![[0.; 3], [0., 0., 10.], [1., 0., 11.]],
            GeometryErrorKind::Unsupported,
            "corner transitions",
        ),
        (
            vec![[0., 0., 1.], [0., 0., 10.]],
            GeometryErrorKind::Input,
            "path start",
        ),
        (
            vec![[0.; 3], [1e-5, 0., 10.]],
            GeometryErrorKind::Input,
            "perpendicular",
        ),
        (vec![[0.; 3], [0.; 3]], GeometryErrorKind::Input, "Length"),
    ] {
        let path_origins = origins(path.len() - 1);
        let error =
            Solid::rust_sweep_straight_named(&profile, &path, feature, &curves, &path_origins, p())
                .err()
                .unwrap();
        assert_eq!(error.kind, kind, "{error}");
        assert!(error.message.contains(message), "{error}");
    }
    let error = Solid::rust_sweep_straight_named(
        &profile,
        &[[0.; 3], [0., 0., 10.]],
        feature,
        &curves,
        &curves[..1],
        p(),
    )
    .err()
    .unwrap();
    assert!(
        error.message.contains("distinct authored curve origins"),
        "{error}"
    );
}

#[test]
fn rust_same_phase_circle_loft_has_exact_geometry_and_both_section_origins() {
    let feature = EntityId::new();
    let frame = PlaneFrame {
        origin: [11., -7., 23.],
        u: [1., 0., 0.],
        v: [0., 0.6, 0.8],
    };
    let section_origins = [origins(1), origins(1)];
    for height in [-20_f64, 20.] {
        for radii in [[2., 4.], [4., 2.], [2., 2.]] {
            let [start, end] = [0., height].map(|level| PlaneFrame {
                origin: std::array::from_fn(|i| frame.origin[i] + frame.normal()[i] * level),
                ..frame
            });
            let profiles = [start, end]
                .into_iter()
                .zip(radii)
                .map(|(frame, radius_mm)| CurvedProfile {
                    frame,
                    outer: vec![CurvedEdge2d::Circle {
                        center: [3., 2.],
                        radius_mm,
                    }],
                    holes: vec![],
                })
                .collect::<Vec<_>>();
            let rust =
                Solid::rust_loft_coaxial_circles_named(&profiles, feature, &section_origins, p())
                    .unwrap();
            assert!(rust.uses_rust_geometry());
            assert_eq!(rust.solid_count().unwrap(), 1);
            let [a, b] = radii;
            let volume = std::f64::consts::PI * height.abs() / 3. * (a * a + a * b + b * b);
            near(rust.volume_mm3().unwrap(), volume);
            let check_ancestry = |topology: &TopologySnapshot| {
                for face in topology
                    .elements
                    .iter()
                    .filter(|element| element.key.kind == TopologyKind::Face)
                {
                    let is_cap = ["cap.start", "cap.end"].into_iter().any(|role| {
                        face.origins.contains(&Origin {
                            source: feature,
                            role: role.into(),
                        })
                    });
                    for origin in section_origins.iter().flatten() {
                        assert_eq!(face.origins.contains(origin), !is_cap);
                        assert_eq!(
                            face.origins.contains(&Origin {
                                source: feature,
                                role: format!("side/{}/{}", origin.source, origin.role)
                            }),
                            !is_cap
                        );
                    }
                }
            };
            check_ancestry(&rust.topology_snapshot(feature, 0).unwrap());
            let receipt = rust
                .manufacturing_mesh_with_receipt(Default::default())
                .unwrap();
            assert!(!receipt.mesh.indices.is_empty());

            #[cfg(feature = "occt")]
            for ruled in [false, true] {
                let native =
                    Solid::loft_profiles_named(&profiles, feature, &section_origins, ruled, p())
                        .unwrap();
                near(native.volume_mm3().unwrap(), volume);
                assert_eq!(rust.faces().unwrap().len(), native.faces().unwrap().len());
                let area = |solid: &Solid| {
                    solid
                        .faces()
                        .unwrap()
                        .iter()
                        .map(|face| face.area_mm2)
                        .sum()
                };
                near(area(&rust), area(&native));
                let bounds = |solid: &Solid| {
                    let bounds = solid.bounds().unwrap();
                    bounds.min.into_iter().chain(bounds.max)
                };
                for (actual, expected) in bounds(&rust).zip(bounds(&native)) {
                    near(actual, expected);
                }
                check_ancestry(&native.topology_snapshot(feature, 0).unwrap());
            }

            let folder = tempfile::tempdir().unwrap();
            let step = folder.path().join("circle-loft.step");
            rust.write_step(&step).unwrap();
            let imported =
                Solid::import_rust_step(&std::fs::read_to_string(&step).unwrap(), feature, p())
                    .unwrap();
            near(imported.volume_mm3().unwrap(), volume);
            #[cfg(feature = "occt")]
            near(
                Solid::read_step(&step, p()).unwrap().volume_mm3().unwrap(),
                volume,
            );
        }
    }
}

#[test]
fn rust_circle_loft_refuses_phase_change_and_offset_axis() {
    let feature = EntityId::new();
    let sections = [origins(1), origins(1)];
    let circle = |z| CurvedProfile {
        frame: PlaneFrame {
            origin: [0., 0., z],
            ..Default::default()
        },
        outer: vec![CurvedEdge2d::Circle {
            center: [0.; 2],
            radius_mm: 2.,
        }],
        holes: vec![],
    };
    let a = circle(0.);
    let mut b = circle(20.);
    b.frame.u = [0., 1., 0.];
    b.frame.v = [-1., 0., 0.];
    let error = Solid::rust_loft_coaxial_circles_named(&[a.clone(), b], feature, &sections, p())
        .err()
        .unwrap();
    assert_eq!(error.kind, GeometryErrorKind::Unsupported);
    assert!(error.message.contains("seam directions"), "{error}");

    let mut b = circle(20.);
    b.frame.origin[0] = 1.;
    let error = Solid::rust_loft_coaxial_circles_named(&[a, b], feature, &sections, p())
        .err()
        .unwrap();
    assert_eq!(error.kind, GeometryErrorKind::Unsupported);
    assert!(error.message.contains("coaxial"), "{error}");
}
