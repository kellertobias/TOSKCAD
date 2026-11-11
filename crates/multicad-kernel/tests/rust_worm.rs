use multicad_core::{document::EntityId, references::TopologyKind};
use multicad_kernel::{
    GeneratedZaWormWheel, GeometryPolicy, MeshOptions, PlaneFrame, RigidTransform, Solid, ZaWorm,
    rust_kernel::Rational,
};
use std::collections::{BTreeMap, BTreeSet};

fn roles(solid: &Solid, feature: EntityId) -> BTreeSet<String> {
    solid
        .topology_snapshot(feature, 0)
        .unwrap()
        .elements
        .iter()
        .filter(|e| e.key.kind == TopologyKind::Face)
        .flat_map(|e| e.origins.iter().map(|o| o.role.clone()))
        .collect()
}

/// Total area and area-weighted center per role.
fn by_role(solid: &Solid, feature: EntityId) -> BTreeMap<String, (f64, [f64; 3])> {
    let faces = solid.faces().unwrap();
    let topology = solid.topology_snapshot(feature, 0).unwrap();
    let mut out: BTreeMap<String, (f64, [f64; 3])> = BTreeMap::new();
    for element in topology
        .elements
        .iter()
        .filter(|e| e.key.kind == TopologyKind::Face)
    {
        let face = &faces[element.key.ordinal as usize];
        for origin in &element.origins {
            let entry = out.entry(origin.role.clone()).or_insert((0., [0.; 3]));
            entry.0 += face.area_mm2;
            for i in 0..3 {
                entry.1[i] += face.center_mm[i] * face.area_mm2;
            }
        }
    }
    for (area, center) in out.values_mut() {
        *center = center.map(|c| c / *area);
    }
    out
}

fn compare(rust: &Solid, native: &Solid, feature: EntityId, volume_tolerance: f64, label: &str) {
    let volume = rust.volume_mm3().unwrap();
    let reference = native.volume_mm3().unwrap();
    assert!(
        (volume - reference).abs() / reference < volume_tolerance,
        "{label} volume {volume} {reference}"
    );
    let [rb, nb] = [rust.bounds().unwrap(), native.bounds().unwrap()];
    for i in 0..3 {
        assert!(
            (rb.min[i] - nb.min[i]).abs() < 5e-3 && (rb.max[i] - nb.max[i]).abs() < 5e-3,
            "{label} bounds {rb:?} {nb:?}"
        );
    }
    assert_eq!(roles(rust, feature), roles(native, feature), "{label}");
    let [ra, na] = [by_role(rust, feature), by_role(native, feature)];
    for (role, (area, center)) in &ra {
        let (native_area, native_center) = na[role];
        assert!(
            (area - native_area).abs() / native_area < 2e-3,
            "{label} {role} area {area} {native_area}"
        );
        let scale = native_area.sqrt();
        for i in 0..3 {
            assert!(
                (center[i] - native_center[i]).abs() < 0.02 * scale,
                "{label} {role} center {center:?} {native_center:?}"
            );
        }
    }
    let total = |m: &BTreeMap<String, (f64, [f64; 3])>| -> f64 { m.values().map(|(a, _)| a).sum() };
    println!(
        "{{\"case\":\"{label}\",\"rust_volume\":{volume},\"native_volume\":{reference},\"rust_area\":{},\"native_area\":{}}}",
        total(&ra),
        total(&na)
    );
}

#[test]
fn rust_worms_and_wheels_match_native_solids() {
    let policy = GeometryPolicy::default();
    let options = MeshOptions {
        linear_deflection_mm: 0.01,
        angular_deflection_rad: 0.3,
        maximum_triangles: 3_000_000,
    };
    let transform = RigidTransform {
        rotation: [[0.8, 0., 0.6], [0., 1., 0.], [-0.6, 0., 0.8]],
        translation_mm: [10., -20., 30.],
    };
    for (m, length, bore, thinning) in
        [(1., 20., 0., 0.02), (2., 16., 3., 0.1), (0.5, 32., 0.8, 0.)]
    {
        let spec = ZaWorm {
            axial_module_mm: m,
            threaded_length_mm: length * m,
            bore_radius_mm: bore,
            axial_thinning_mm: thinning,
        };
        let feature = EntityId::new();
        let rust = Solid::rust_za_worm_named(spec, feature, policy).unwrap();
        assert!(rust.uses_rust_geometry());
        rust.topology_snapshot(feature, 1)
            .unwrap()
            .validate()
            .unwrap();
        assert!(!rust.manufacturing_mesh(options).unwrap().indices.is_empty());
        assert!(rust.display_edges(0.01).unwrap().len() >= 12);
        let placed = rust.transformed(transform).unwrap();
        assert_eq!(placed.volume_mm3().unwrap(), rust.volume_mm3().unwrap());
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("worm.step");
        placed.write_step(&path).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        let decoded = placed.verify_worm_step(&text).unwrap().unwrap();
        let volume = rust.volume_mm3().unwrap();
        assert!(
            (decoded - volume).abs() / volume < 1e-8,
            "{decoded} {volume}"
        );
        if !multicad_kernel::OCCT_LINKED {
            continue;
        }
        let native = Solid::za_worm_named(spec, feature, policy).unwrap();
        compare(
            &rust,
            &native,
            feature,
            1e-6,
            &format!("worm m{m} L{length} b{bore}"),
        );
        let imported = multicad_kernel::Shape::read_step(&path).unwrap();
        assert!(imported.valid().unwrap());
        assert_eq!(Solid::step_solids(&path).unwrap().len(), 1);
        let read = imported.volume().unwrap();
        assert!((read - volume).abs() / volume < 1e-6, "{read} {volume}");
        println!(
            "{{\"case\":\"worm m{m} b{bore} STEP\",\"rust_decoded_volume\":{decoded},\"occt_step_volume\":{read}}}"
        );
        let [pb, qb] = [
            placed.bounds().unwrap(),
            native.transformed(transform).unwrap().bounds().unwrap(),
        ];
        for i in 0..3 {
            assert!((pb.min[i] - qb.min[i]).abs() < 5e-3 && (pb.max[i] - qb.max[i]).abs() < 5e-3);
        }
    }
    for (m, teeth, width, bore, thinning) in [
        (1., 32, 1., 0., 0.),
        (2., 40, 2., 6., 0.1),
        (0.5, 64, 0.5, 2., 0.02),
    ] {
        let spec = GeneratedZaWormWheel {
            teeth,
            axial_module_mm: m,
            face_width_mm: width * m,
            bore_radius_mm: bore,
            pitch_thinning_mm: thinning,
        };
        let feature = EntityId::new();
        let rust = Solid::rust_generated_za_worm_wheel_named(spec, feature, policy).unwrap();
        rust.topology_snapshot(feature, 1)
            .unwrap()
            .validate()
            .unwrap();
        assert!(!rust.manufacturing_mesh(options).unwrap().indices.is_empty());
        assert!(rust.display_edges(0.01).unwrap().len() >= 23 * teeth as usize);
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("wheel.step");
        let placed = rust.transformed(transform).unwrap();
        let started = std::time::Instant::now();
        placed.write_step(&path).unwrap();
        let written = started.elapsed();
        let text = std::fs::read_to_string(&path).unwrap();
        let decoded = placed.verify_worm_step(&text).unwrap().unwrap();
        let volume = rust.volume_mm3().unwrap();
        assert!(
            (decoded - volume).abs() / volume < 1e-8,
            "{decoded} {volume}"
        );
        // The proof belongs to this placement and wheel.
        assert!(rust.verify_worm_step(&text).unwrap().is_err());
        assert!(rust.verify_worm_step("").unwrap().is_err());
        if !multicad_kernel::OCCT_LINKED {
            continue;
        }
        let imported = multicad_kernel::Shape::read_step(&path).unwrap();
        assert!(imported.valid().unwrap());
        assert_eq!(Solid::step_solids(&path).unwrap().len(), 1);
        let read = imported.volume().unwrap();
        assert!((read - volume).abs() / volume < 1e-6, "{read} {volume}");
        println!(
            "{{\"case\":\"wheel m{m} z{teeth} b{bore} STEP\",\"bytes\":{},\"write_ms\":{},\"rust_decoded_volume\":{decoded},\"occt_step_volume\":{read},\"rust_volume\":{volume}}}",
            text.len(),
            written.as_secs_f64() * 1e3
        );
        let native = Solid::generated_za_worm_wheel_named(spec, feature, policy).unwrap();
        // The native wheel interpolates each generating patch within its
        // certified polynomial bound; the Rust wheel is exact.
        compare(
            &rust,
            &native,
            feature,
            1e-4,
            &format!("wheel m{m} z{teeth} b{bore}"),
        );
    }
}

#[test]
fn assembled_worm_pairs_export_verified_multi_solid_step() {
    let policy = GeometryPolicy::default();
    let worm = Solid::rust_za_worm_named(
        ZaWorm {
            axial_module_mm: 1.,
            threaded_length_mm: 20.,
            bore_radius_mm: 2.,
            axial_thinning_mm: 0.04,
        },
        EntityId::new(),
        policy,
    )
    .unwrap();
    let wheel = Solid::rust_generated_za_worm_wheel_named(
        GeneratedZaWormWheel {
            teeth: 40,
            axial_module_mm: 1.,
            face_width_mm: 1.5,
            bore_radius_mm: 4.,
            pitch_thinning_mm: 0.04,
        },
        EntityId::new(),
        policy,
    )
    .unwrap();
    // Mating pose: worm axis along wheel +Y at center distance 25 mm, then
    // the whole pair tilted and moved.
    let mate = RigidTransform {
        rotation: [[1., 0., 0.], [0., 0., 1.], [0., -1., 0.]],
        translation_mm: [25., 0., 0.],
    };
    let tilt = RigidTransform {
        rotation: [[0.8, 0., 0.6], [0., 1., 0.], [-0.6, 0., 0.8]],
        translation_mm: [10., -20., 30.],
    };
    let worm = worm.transformed(mate).unwrap().transformed(tilt).unwrap();
    let wheel = wheel.transformed(tilt).unwrap();
    let (clearance, _) = worm
        .certify_rust_worm_pair(&wheel, policy, 0.2, None)
        .unwrap();
    assert_ne!(
        clearance,
        multicad_kernel::rust_kernel::worm_gear::clearance::ResultKind::CommonInterior
    );
    let block = Solid::rust_box(
        EntityId::new().uuid_bits(),
        [4; 3].map(|n| Rational::new(n, 1).unwrap()),
        PlaneFrame {
            origin: [60., 0., 0.],
            ..Default::default()
        },
        policy,
    )
    .unwrap();
    let members = [&worm, &block, &wheel];
    let compound = Solid::compound_named(&members, EntityId::new(), policy).unwrap();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("pair.step");
    compound.write_step(&path).unwrap();
    let source = std::fs::read_to_string(&path).unwrap();
    assert_eq!(
        source.matches("MANIFOLD_SOLID_BREP(").count(),
        members.len()
    );
    let (count, volume, _) = compound
        .verify_twisted_step_compound(&source, policy.linear_tolerance_mm)
        .unwrap()
        .unwrap();
    assert_eq!(count as usize, members.len());
    let expected: f64 = members.iter().map(|m| m.volume_mm3().unwrap()).sum();
    assert!(
        (volume - expected).abs() / expected < 1e-8,
        "{volume} {expected}"
    );
    // Exchanging the members' placements breaks the proof.
    let swapped = Solid::compound_named(&[&wheel, &worm], EntityId::new(), policy).unwrap();
    assert!(
        swapped
            .verify_twisted_step_compound(&source, policy.linear_tolerance_mm)
            .unwrap()
            .is_err()
    );
    if multicad_kernel::OCCT_LINKED {
        let native = multicad_kernel::Shape::read_step(&path).unwrap();
        assert!(native.valid().unwrap());
        assert_eq!(Solid::step_solids(&path).unwrap().len(), members.len());
        let read = native.volume().unwrap();
        assert!(
            (read - expected).abs() / expected < 1e-6,
            "{read} {expected}"
        );
        println!(
            "{{\"case\":\"worm pair compound STEP\",\"members\":{},\"rust_decoded_volume\":{volume},\"occt_step_volume\":{read}}}",
            members.len()
        );
    }
}

/// Largest distance from preview points of `edges` with `role` to the
/// nearest preview segment of `reference` edges in `roles`.
fn preview_distance(
    edges: &[multicad_kernel::ProjectedVector],
    role: multicad_core::drawings::VectorRole,
    reference: &[multicad_kernel::ProjectedVector],
    roles: &[multicad_core::drawings::VectorRole],
) -> f64 {
    let segments: Vec<[[f64; 2]; 2]> = reference
        .iter()
        .filter(|edge| roles.contains(&edge.role))
        .flat_map(|edge| edge.preview_mm.windows(2).map(|w| [w[0], w[1]]))
        .collect();
    edges
        .iter()
        .filter(|edge| edge.role == role)
        .flat_map(|edge| edge.preview_mm.iter())
        .map(|&p| {
            segments
                .iter()
                .map(|&[a, b]| {
                    let d = [b[0] - a[0], b[1] - a[1]];
                    let w = [p[0] - a[0], p[1] - a[1]];
                    let t = ((w[0] * d[0] + w[1] * d[1]) / (d[0] * d[0] + d[1] * d[1]).max(1e-300))
                        .clamp(0., 1.);
                    (w[0] - t * d[0]).hypot(w[1] - t * d[1])
                })
                .fold(f64::INFINITY, f64::min)
        })
        .fold(0., f64::max)
}

#[test]
fn rust_worm_drawings_follow_exact_axial_visibility_and_sections() {
    use multicad_core::drawings::{VectorCurve, VectorRole};
    use multicad_kernel::{
        DrawingHatch, DrawingProjection, DrawingSolid, GeometryErrorKind, project_drawing,
    };
    let policy = GeometryPolicy::default();
    let spec = ZaWorm {
        axial_module_mm: 1.,
        threaded_length_mm: 20.,
        bore_radius_mm: 2.,
        axial_thinning_mm: 0.04,
    };
    let feature = EntityId::new();
    let worm = Solid::rust_za_worm_named(spec, feature, policy).unwrap();
    let draw = |solid: &Solid, options: DrawingProjection| {
        project_drawing(&[DrawingSolid { solid, hatch: None }], options)
    };
    let view = DrawingProjection {
        hidden_lines: true,
        chord_tolerance_mm: 0.002,
        ..Default::default()
    };
    let projected = draw(&worm, view).unwrap();
    let radii = |role: VectorRole| {
        let mut radii: Vec<f64> = projected
            .iter()
            .filter(|edge| edge.role == role)
            .filter_map(|edge| match edge.curve {
                VectorCurve::Ellipse { major_mm, .. } => Some(major_mm),
                _ => None,
            })
            .collect();
        radii.sort_by(f64::total_cmp);
        radii
    };
    // Visible crest circle and near bore; hidden root circle outside the
    // near root land.
    assert_eq!(radii(VectorRole::Visible), [2., 6.]);
    assert_eq!(radii(VectorRole::Hidden), [3.75]);
    let lines = |role: VectorRole| {
        projected
            .iter()
            .filter(|edge| edge.role == role && matches!(edge.curve, VectorCurve::Line { .. }))
            .count()
    };
    assert!(lines(VectorRole::Visible) > 20 && lines(VectorRole::Hidden) > 20);
    // Every visible near-boundary point lies on the exact thread radius at
    // the near cap, between the root and crest circles.
    let rust = multicad_kernel::rust_kernel::worm_gear::Worm::new(
        multicad_kernel::rust_kernel::worm_gear::WormSpec {
            axial_module_mm: 1.,
            threaded_length_mm: 20.,
            bore_radius_mm: 2.,
            axial_thinning_mm: 0.04,
        },
        policy.minimum_feature_mm,
    )
    .unwrap();
    let at = |angle: f64, height: f64| rust.thread().radius(angle - rust.phase_rad(), height);
    let thread = |angle: f64| at(angle, 20.);
    for edge in projected.iter().filter(|e| e.role == VectorRole::Visible) {
        if let VectorCurve::Line { a, .. } = edge.curve {
            let angle = a[1].atan2(a[0]);
            assert!((a[0].hypot(a[1]) - thread(angle)).abs() < 1e-9, "{a:?}");
        }
    }
    // Looking from the other end swaps which cap is visible.
    let reversed = draw(
        &worm,
        DrawingProjection {
            frame: multicad_kernel::PlaneFrame {
                v: [0., -1., 0.],
                ..Default::default()
            },
            ..view
        },
    )
    .unwrap();
    assert_eq!(reversed.len(), projected.len());
    // Section only, retained sections and unsupported requests.
    let cut = draw(
        &worm,
        DrawingProjection {
            section_offset_mm: Some(1.3),
            section_only: true,
            ..view
        },
    )
    .unwrap();
    assert!(cut.iter().all(|edge| edge.role == VectorRole::Section));
    let area: f64 = cut
        .iter()
        .filter_map(|edge| match edge.curve {
            VectorCurve::Line { a, b } => Some((a[0] * b[1] - a[1] * b[0]) / 2.),
            _ => None,
        })
        .sum();
    // The section area integrates the exact thread radius less the bore.
    let exact = {
        let n = 20_000;
        let step = std::f64::consts::TAU / f64::from(n);
        (0..n)
            .map(|i| at(f64::from(i) * step, 11.3).powi(2) / 2. * step)
            .sum::<f64>()
            - std::f64::consts::PI * 4.
    };
    assert!((area - exact).abs() / exact < 1e-3, "{area} {exact}");
    let hatched = project_drawing(
        &[DrawingSolid {
            solid: &worm,
            hatch: Some(DrawingHatch {
                angle_rad: 0.3,
                spacing_mm: 0.5,
                line_width_mm: 0.1,
                pattern: multicad_core::materials::HatchPattern::Lines,
            }),
        }],
        DrawingProjection {
            section_offset_mm: Some(1.3),
            section_only: true,
            ..view
        },
    )
    .unwrap();
    assert!(hatched.iter().any(|edge| edge.role == VectorRole::Hatch));
    let retained = draw(
        &worm,
        DrawingProjection {
            section_offset_mm: Some(1.3),
            section_depth_mm: Some(5.),
            ..view
        },
    )
    .unwrap();
    assert!(retained.iter().any(|edge| edge.role == VectorRole::Section));
    assert!(retained.iter().any(|edge| edge.role == VectorRole::Visible));
    let shallow = DrawingProjection {
        section_offset_mm: Some(1.3),
        section_depth_mm: Some(1.),
        ..view
    };
    assert_eq!(
        draw(&worm, shallow).unwrap_err().kind,
        GeometryErrorKind::Unsupported
    );
    assert!(
        draw(
            &worm,
            DrawingProjection {
                section_offset_mm: Some(30.),
                section_only: true,
                ..view
            }
        )
        .unwrap()
        .is_empty()
    );
    let side = DrawingProjection {
        frame: multicad_kernel::PlaneFrame {
            v: [0., 0., 1.],
            ..Default::default()
        },
        ..view
    };
    assert_eq!(
        draw(&worm, side).unwrap_err().kind,
        GeometryErrorKind::Unsupported
    );
    // Wheels: exact sections, explicit refusal of views.
    let wheel = Solid::rust_generated_za_worm_wheel_named(
        GeneratedZaWormWheel {
            teeth: 40,
            axial_module_mm: 1.,
            face_width_mm: 1.5,
            bore_radius_mm: 4.,
            pitch_thinning_mm: 0.04,
        },
        EntityId::new(),
        policy,
    )
    .unwrap();
    let wheel_cut = draw(
        &wheel,
        DrawingProjection {
            section_offset_mm: Some(0.3),
            section_only: true,
            ..view
        },
    )
    .unwrap();
    assert!(
        wheel_cut
            .iter()
            .all(|edge| edge.role == VectorRole::Section)
    );
    let wheel_area: f64 = wheel_cut
        .iter()
        .filter_map(|edge| match edge.curve {
            VectorCurve::Line { a, b } => Some((a[0] * b[1] - a[1] * b[0]) / 2.),
            _ => None,
        })
        .sum();
    let disk = std::f64::consts::PI * (21_f64.powi(2) - 16.);
    assert!(
        wheel_area > 0.85 * disk && wheel_area < disk,
        "{wheel_area}"
    );
    let both = project_drawing(
        &[
            DrawingSolid {
                solid: &worm,
                hatch: None,
            },
            DrawingSolid {
                solid: &wheel,
                hatch: None,
            },
        ],
        view,
    );
    assert_eq!(both.unwrap_err().kind, GeometryErrorKind::Unsupported);
    if !multicad_kernel::OCCT_LINKED {
        return;
    }
    // Native hidden-line projection of the native worm agrees with the
    // visible and hidden curves and with both sections.
    let native = Solid::za_worm_named(spec, feature, policy).unwrap();
    let native_view = draw(&native, view).unwrap();
    let visible = preview_distance(
        &projected,
        VectorRole::Visible,
        &native_view,
        &[VectorRole::Visible],
    );
    let hidden = preview_distance(
        &projected,
        VectorRole::Hidden,
        &native_view,
        &[VectorRole::Hidden, VectorRole::Visible],
    );
    let native_cut = draw(
        &native,
        DrawingProjection {
            section_offset_mm: Some(1.3),
            section_only: true,
            ..view
        },
    )
    .unwrap();
    let section = preview_distance(
        &cut,
        VectorRole::Section,
        &native_cut,
        &[VectorRole::Section],
    );
    let native_wheel = Solid::generated_za_worm_wheel_named(
        GeneratedZaWormWheel {
            teeth: 40,
            axial_module_mm: 1.,
            face_width_mm: 1.5,
            bore_radius_mm: 4.,
            pitch_thinning_mm: 0.04,
        },
        EntityId::new(),
        policy,
    )
    .unwrap();
    let native_wheel_cut = draw(
        &native_wheel,
        DrawingProjection {
            section_offset_mm: Some(0.3),
            section_only: true,
            ..view
        },
    )
    .unwrap();
    let wheel_section = preview_distance(
        &wheel_cut,
        VectorRole::Section,
        &native_wheel_cut,
        &[VectorRole::Section],
    );
    let complete = preview_distance(
        &native_view,
        VectorRole::Visible,
        &projected,
        &[VectorRole::Visible],
    );
    let complete_hidden = preview_distance(
        &native_view,
        VectorRole::Hidden,
        &projected,
        &[VectorRole::Hidden, VectorRole::Visible],
    );
    println!(
        "{{\"case\":\"worm drawings vs OCCT\",\"native_visible_to_rust_mm\":{complete},\"native_hidden_to_rust_mm\":{complete_hidden},\"visible_mm\":{visible},\"hidden_mm\":{hidden},\"worm_section_mm\":{section},\"wheel_section_mm\":{wheel_section}}}"
    );
    assert!(visible < 0.01 && hidden < 0.01, "{visible} {hidden}");
    assert!(
        complete < 0.01 && complete_hidden < 0.01,
        "{complete} {complete_hidden}"
    );
    assert!(
        section < 0.01 && wheel_section < 0.01,
        "{section} {wheel_section}"
    );
}

#[test]
fn matched_worm_and_wheel_build_and_step_export_latency() {
    if !multicad_kernel::OCCT_LINKED {
        return;
    }
    let policy = GeometryPolicy::default();
    let worm_spec = ZaWorm {
        axial_module_mm: 1.,
        threaded_length_mm: 20.,
        bore_radius_mm: 1.,
        axial_thinning_mm: 0.05,
    };
    let wheel_spec = GeneratedZaWormWheel {
        teeth: 40,
        axial_module_mm: 1.,
        face_width_mm: 1.5,
        bore_radius_mm: 3.,
        pitch_thinning_mm: 0.05,
    };
    let directory = tempfile::tempdir().unwrap();
    let time = |run: &mut dyn FnMut()| {
        let started = std::time::Instant::now();
        run();
        started.elapsed().as_secs_f64() * 1e3
    };
    let feature = EntityId::new();
    let mut solids: Vec<(&str, Solid)> = Vec::new();
    let mut builds = Vec::new();
    for (name, rust) in [
        ("worm", true),
        ("worm", false),
        ("wheel", true),
        ("wheel", false),
    ] {
        let mut built = None;
        let ms = time(&mut || {
            built = Some(match (name, rust) {
                ("worm", true) => Solid::rust_za_worm_named(worm_spec, feature, policy),
                ("worm", false) => Solid::za_worm_named(worm_spec, feature, policy),
                (_, true) => Solid::rust_generated_za_worm_wheel_named(wheel_spec, feature, policy),
                _ => Solid::generated_za_worm_wheel_named(wheel_spec, feature, policy),
            });
        });
        builds.push(ms);
        solids.push((name, built.unwrap().unwrap()));
    }
    let mut exports = Vec::new();
    for (index, (name, solid)) in solids.iter().enumerate() {
        let path = directory.path().join(format!("{name}{index}.step"));
        exports.push(time(&mut || solid.write_step(&path).unwrap()));
    }
    println!(
        "{{\"case\":\"worm build and STEP latency\",\"rust_worm_build_ms\":{},\"native_worm_build_ms\":{},\"rust_wheel_build_ms\":{},\"native_wheel_build_ms\":{},\"rust_worm_step_ms\":{},\"native_worm_step_ms\":{},\"rust_wheel_step_ms\":{},\"native_wheel_step_ms\":{}}}",
        builds[0], builds[1], builds[2], builds[3], exports[0], exports[1], exports[2], exports[3]
    );
}

#[test]
fn rust_wheel_views_match_native_hidden_lines_both_ways() {
    for (spec, all) in [
        (
            GeneratedZaWormWheel {
                teeth: 40,
                axial_module_mm: 1.,
                face_width_mm: 1.5,
                bore_radius_mm: 4.,
                pitch_thinning_mm: 0.04,
            },
            true,
        ),
        (
            GeneratedZaWormWheel {
                teeth: 64,
                axial_module_mm: 2.,
                face_width_mm: 1.,
                bore_radius_mm: 6.,
                pitch_thinning_mm: 0.2,
            },
            false,
        ),
    ] {
        compare_wheel_views(spec, all);
    }
}

fn compare_wheel_views(spec: GeneratedZaWormWheel, all: bool) {
    use multicad_core::drawings::VectorRole;
    use multicad_kernel::{DrawingProjection, DrawingSolid, PlaneFrame, project_drawing};
    let policy = GeometryPolicy::default();
    let feature = EntityId::new();
    let wheel = Solid::rust_generated_za_worm_wheel_named(spec, feature, policy).unwrap();
    let draw = |solid: &Solid, options: DrawingProjection| {
        project_drawing(&[DrawingSolid { solid, hatch: None }], options).unwrap()
    };
    let base = DrawingProjection {
        hidden_lines: true,
        chord_tolerance_mm: 0.002,
        ..Default::default()
    };
    let below = PlaneFrame {
        v: [0., -1., 0.],
        ..Default::default()
    };
    let cases = [
        ("top", base),
        (
            "bottom",
            DrawingProjection {
                frame: below,
                ..base
            },
        ),
        (
            "visible only",
            DrawingProjection {
                hidden_lines: false,
                ..base
            },
        ),
        (
            "retained",
            DrawingProjection {
                section_offset_mm: Some(0.2),
                ..base
            },
        ),
        (
            "depth",
            DrawingProjection {
                section_offset_mm: Some(0.2),
                section_depth_mm: Some(0.6),
                ..base
            },
        ),
    ];
    let mut rust = Vec::new();
    for (name, options) in cases.into_iter().take(if all { 5 } else { 2 }) {
        let started = std::time::Instant::now();
        let projected = draw(&wheel, options);
        println!(
            "{{\"case\":\"wheel z{} view {name} latency\",\"rust_ms\":{}}}",
            spec.teeth,
            started.elapsed().as_secs_f64() * 1e3
        );
        assert!(
            projected
                .iter()
                .any(|edge| edge.role == VectorRole::Visible),
            "{name}"
        );
        assert_eq!(
            projected.iter().any(|edge| edge.role == VectorRole::Hidden),
            options.hidden_lines,
            "{name}"
        );
        rust.push((name, options, projected));
    }
    if !multicad_kernel::OCCT_LINKED {
        return;
    }
    let native = Solid::generated_za_worm_wheel_named(spec, feature, policy).unwrap();
    // Every Rust line lies on a native line of the same role. Native lines
    // may also lie under a coincident Rust line of another role: the far
    // bore under the near bore, or a cut edge drawn as a section line.
    let all = vec![VectorRole::Visible, VectorRole::Hidden, VectorRole::Section];
    let roles = [
        (VectorRole::Visible, vec![VectorRole::Visible], all.clone()),
        (VectorRole::Hidden, vec![VectorRole::Hidden], all.clone()),
        (VectorRole::Section, vec![VectorRole::Section], all),
    ];
    for (name, options, projected) in rust {
        let reference = draw(&native, options);
        for (role, same, any) in &roles {
            let forward = preview_distance(&projected, *role, &reference, same);
            let backward = preview_distance(&reference, *role, &projected, any);
            println!(
                "{{\"case\":\"wheel z{} view {name}\",\"role\":\"{role:?}\",\"rust_to_native_mm\":{forward},\"native_to_rust_mm\":{backward}}}",
                spec.teeth
            );
            assert!(forward < 0.005 && backward < 0.005, "{name} {role:?}");
        }
    }
}
