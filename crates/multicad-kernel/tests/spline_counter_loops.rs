use multicad_core::{document::EntityId, references::*};
use multicad_kernel::*;
fn profile() -> CurvedProfile {
    let line = |a, b| CurvedEdge2d::Line { a, b };
    let points = [[2., 4.], [3., 5.], [4., 4.], [3., 3.]];
    let controls = [[2., 5.], [4., 5.], [4., 3.], [2., 3.]];
    let counter = (0..4)
        .map(|i| {
            CurvedEdge2d::BSpline(BSpline2d {
                degree: 2,
                poles: vec![points[i], controls[i], points[(i + 1) % 4]],
                knots: vec![0., 1.],
                multiplicities: vec![3, 3],
                weights: None,
            })
        })
        .collect();
    CurvedProfile {
        frame: PlaneFrame {
            origin: [80., 60., 20.],
            u: [0.8, 0.6, 0.],
            v: [-0.36, 0.48, 0.8],
        },
        outer: vec![
            line([0., 0.], [10., 0.]),
            line([10., 0.], [10., 8.]),
            line([10., 8.], [0., 8.]),
            line([0., 8.], [0., 0.]),
        ],
        holes: vec![
            counter,
            vec![CurvedEdge2d::Circle {
                center: [7., 4.],
                radius_mm: 0.5,
            }],
        ],
    }
}
fn origins(profile: &CurvedProfile) -> Vec<Origin> {
    profile
        .outer
        .iter()
        .chain(profile.holes.iter().flatten())
        .enumerate()
        .map(|(i, _)| Origin {
            source: EntityId::new(),
            role: format!("curve.{i}"),
        })
        .collect()
}
#[test]
fn real_quadratic_counters_keep_cap_material_source_frames_and_finite_face_selection() {
    let policy = GeometryPolicy::default();
    let profile = profile();
    let before = profile.clone();
    let origins = origins(&profile);
    let feature = EntityId::new();
    let area = 80. - 10. / 3. - std::f64::consts::PI / 4.;
    let perimeter = 36. + 4. * (1. + 1_f64.asinh() / 2_f64.sqrt()) + std::f64::consts::PI;
    for height in [3., -3.] {
        let solid =
            Solid::rust_extrude_spline_profile_named(&profile, height, feature, &origins, policy)
                .unwrap();
        let (source, frame) = solid.rust_spline_sweep_data().unwrap();
        assert_eq!(source.holes().len(), 2);
        assert!((source.area_mm2() - area).abs() < 1e-10);
        assert!((solid.volume_mm3().unwrap() - area * 3.).abs() < 1e-9);
        let faces = solid.faces().unwrap();
        assert_eq!(faces.len(), source.edges().count() + 2);
        let face_area: f64 = faces.iter().map(|f| f.area_mm2).sum();
        let error = policy.linear_tolerance_mm * (faces.len() - 2) as f64 * 3. + 1e-9;
        assert!((face_area - (2. * area + 3. * perimeter)).abs() <= error);
        let snapshot = solid.topology_snapshot(feature, 1).unwrap();
        let display = solid.display_edges(0.01).unwrap();
        let stride = source.edges().count();
        for edge in snapshot
            .elements
            .iter()
            .filter(|e| e.key.kind == TopologyKind::Edge && e.geometry.is_none())
        {
            let index = edge.key.ordinal as usize;
            let curve = source.edges().nth(index % stride).unwrap();
            let samples = display
                .iter()
                .find(|e| e.index == edge.key.ordinal)
                .unwrap();
            let z = if index >= stride {
                source.height_mm()
            } else {
                0.
            };
            for (sample, fraction) in [
                (samples.points_mm.first().unwrap(), 0.),
                (samples.points_mm.last().unwrap(), 1.),
            ] {
                let [x, y] = curve.point(fraction);
                let expected = frame.point([x, y]);
                for i in 0..3 {
                    assert!((sample[i] - (expected[i] + z * frame.normal()[i])).abs() < 1e-8);
                }
            }
        }

        for face in snapshot
            .elements
            .iter()
            .filter(|e| e.key.kind == TopologyKind::Face)
        {
            let intent = GeometryIntent::capture(&snapshot, face.key).unwrap();
            assert!(
                matches!(resolve(&intent,&snapshot,1),Resolution::Resolved{key,..} if key==face.key)
            );
        }
        for origin in &origins {
            assert!(
                snapshot
                    .elements
                    .iter()
                    .any(|e| e.key.kind == TopologyKind::Face && e.origins.contains(origin))
            );
        }
        for role in ["cap.start", "cap.end"] {
            let anchor = snapshot
                .elements
                .iter()
                .flat_map(|e| &e.frames)
                .find(|a| a.origin.source == feature && a.origin.role == role)
                .unwrap();
            for (a, b) in anchor
                .frame
                .u
                .into_iter()
                .chain(anchor.frame.v)
                .zip(profile.frame.u.into_iter().chain(profile.frame.v))
            {
                assert!((a - b).abs() < 1e-12);
            }
        }
        let options = MeshOptions {
            linear_deflection_mm: 0.001,
            angular_deflection_rad: 0.15,
            maximum_triangles: 100_000,
        };
        let mesh = solid.manufacturing_mesh(options).unwrap();
        let proof = rust_kernel::manufacturing::certify(&rust_kernel::display::DisplayMesh {
            vertices: mesh.vertices,
            indices: mesh.indices,
            face_indices: mesh.face_indices,
        })
        .unwrap();
        assert!(
            (proof.signed_volume_mm3 - area * 3.).abs()
                < 2. * (perimeter * options.linear_deflection_mm
                    + 3. * std::f64::consts::PI * options.linear_deflection_mm.powi(2))
                    * 3.
        );
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("counters.step");
        solid.write_step(&path).unwrap();
        let imported = Solid::read_step(&path, policy).unwrap();
        assert!((imported.volume_mm3().unwrap() - area * 3.).abs() < 1e-8);
        assert_eq!(
            imported.rust_spline_sweep_data().unwrap().0.holes().len(),
            2
        );
        // The source remains the original authored plane even for reverse sweeps.
        assert_eq!(frame.origin, profile.frame.origin);
    }
    assert_eq!(profile, before);
}

#[test]
fn invalid_counter_and_cancelled_candidates_publish_no_replacement_source() {
    let p = profile();
    let before = p.clone();
    let origins = origins(&p);
    let feature = EntityId::new();
    let policy = GeometryPolicy::default();
    let source =
        Solid::rust_extrude_spline_profile_named(&p, 3., feature, &origins, policy).unwrap();
    let topology = source.topology_snapshot(feature, 1).unwrap();
    let mut invalid = p.clone();
    invalid.holes[1] = vec![CurvedEdge2d::Circle {
        center: [30., 30.],
        radius_mm: 0.5,
    }];
    assert!(
        Solid::rust_extrude_spline_profile_named(&invalid, 3., feature, &origins, policy).is_err()
    );
    assert!(
        matches!(Solid::rust_extrude_spline_profile_with_interrupt(&p,3.,feature,&origins,policy,Some(&||true)),Err(error) if error.kind==GeometryErrorKind::Cancelled)
    );
    assert_eq!(source.topology_snapshot(feature, 1).unwrap(), topology);
    assert_eq!(p, before);
    Solid::rust_extrude_spline_profile_named(&p, 3., feature, &origins, policy).unwrap();
}

fn reordered_step(input: &str) -> String {
    let start = input.find("DATA;").unwrap() + 5;
    let end = start + input[start..].find("ENDSEC;").unwrap();
    let mut lines: Vec<_> = input[start..end]
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            if let Some(at) = line.find("=EDGE_LOOP('',(") {
                let begin = at + "=EDGE_LOOP('',(".len();
                let end = line.rfind("))").unwrap();
                let mut edges: Vec<_> = line[begin..end].split(',').collect();
                edges.rotate_left(1);
                format!("{}{}{}", &line[..begin], edges.join(","), &line[end..])
            } else {
                line.to_owned()
            }
        })
        .collect();
    lines.reverse();
    format!(
        "{}\n{}\n{}",
        &input[..start],
        lines.join("\n"),
        &input[end..]
    )
}

#[test]
fn opaque_spline_faces_preserve_only_proved_finite_regions_and_placement() {
    let policy = GeometryPolicy::default();
    for kind in 0..3 {
        let source_feature = EntityId::new();
        let imported_feature = EntityId::new();
        let mut profile = profile();
        if kind < 2 {
            profile.outer[2] = CurvedEdge2d::BSpline(BSpline2d {
                degree: 2,
                poles: vec![[10., 8.], [5., 10.], [0., 8.]],
                knots: vec![0., 1.],
                multiplicities: vec![3, 3],
                weights: None,
            });
            profile.holes = if kind == 0 {
                vec![]
            } else {
                vec![vec![CurvedEdge2d::Circle {
                    center: [7., 4.],
                    radius_mm: 0.5,
                }]]
            };
        }
        let origins = origins(&profile);
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("opaque.step");
        let load = |profile: &CurvedProfile, height: f64| {
            Solid::rust_extrude_spline_profile_named(
                profile,
                height,
                source_feature,
                &origins,
                policy,
            )
            .unwrap()
            .write_step(&path)
            .unwrap();
            Solid::import_rust_step(
                &std::fs::read_to_string(&path).unwrap(),
                imported_feature,
                policy,
            )
            .unwrap()
        };
        let original = load(&profile, 3.);
        let text = std::fs::read_to_string(&path).unwrap();
        let before = original.topology_snapshot(imported_feature, 0).unwrap();
        let original_faces = original.faces().unwrap();
        let intents: Vec<_> = before
            .elements
            .iter()
            .filter(|e| e.key.kind == TopologyKind::Face)
            .map(|e| (e.key, GeometryIntent::capture(&before, e.key).unwrap()))
            .collect();
        let check_same = |intent: &GeometryIntent, key: TopologyKey, solid: &Solid| {
            let current = solid.topology_snapshot(imported_feature, 1).unwrap();
            let Resolution::Resolved { key: found, .. } = resolve(intent, &current, 1) else {
                panic!("unchanged finite face lost")
            };
            let old = original_faces
                .iter()
                .find(|f| f.index == key.ordinal)
                .unwrap();
            let faces = solid.faces().unwrap();
            let new = faces.iter().find(|f| f.index == found.ordinal).unwrap();
            assert!((old.area_mm2 - new.area_mm2).abs() < 1e-6);
            for (a, b) in old.center_mm.into_iter().zip(new.center_mm) {
                assert!((a - b).abs() < 1e-6);
            }
        };
        let identical = Solid::import_rust_step(&text, imported_feature, policy).unwrap();
        let reordered =
            Solid::import_rust_step(&reordered_step(&text), imported_feature, policy).unwrap();
        for (key, intent) in &intents {
            check_same(intent, *key, &identical);
            check_same(intent, *key, &reordered);
        }
        let moved = original
            .transformed(RigidTransform {
                rotation: [[1., 0., 0.], [0., 0.8, -0.6], [0., 0.6, 0.8]],
                translation_mm: [12., -4., 7.],
            })
            .unwrap();
        let placed = moved.topology_snapshot(imported_feature, 1).unwrap();
        for (_, intent) in &intents {
            assert!(matches!(
                resolve(intent, &placed, 1),
                Resolution::Resolved { .. }
            ));
        }
        let taller = load(&profile, 4.);
        let current = taller.topology_snapshot(imported_feature, 1).unwrap();
        for (key, intent) in &intents {
            if key.ordinal == 0 {
                check_same(intent, *key, &taller);
            } else {
                assert!(
                    !matches!(resolve(intent, &current, 1), Resolution::Resolved { .. }),
                    "changed opaque finite wall/far cap silently rebound"
                );
            }
        }
        if kind == 0 {
            let CurvedEdge2d::BSpline(spline) = &mut profile.outer[2] else {
                unreachable!()
            };
            spline.poles[1][1] = 11.;
        } else {
            let index = profile.holes.len() - 1;
            profile.holes[index] = vec![CurvedEdge2d::Circle {
                center: [7., 4.],
                radius_mm: 0.75,
            }];
        }
        let changed = load(&profile, 3.);
        let current = changed.topology_snapshot(imported_feature, 1).unwrap();
        for (key, intent) in &intents {
            if key.ordinal < 2 {
                assert!(
                    !matches!(resolve(intent, &current, 1), Resolution::Resolved { .. }),
                    "changed opaque finite cap silently rebound"
                );
            }
            // The untouched bottom outer side has identical finite material.
            if key.ordinal == 2 {
                check_same(intent, *key, &changed);
            }
        }
    }
}
