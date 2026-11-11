use multicad_core::{
    commands::Session,
    document::{Document, EntityId},
    model::{self, ThreadHand, ThreadMode, length},
    references::*,
};
use multicad_kernel::{
    rust_kernel::{Circle, Rational, cylinder::AxialCylinder},
    *,
};
fn near(a: f64, b: f64, tol: f64) {
    assert!((a - b).abs() < tol, "{a} != {b}");
}
#[test]
fn sourced_multiple_start_threads_keep_geometry_references_and_independent_native_step() {
    let policy = GeometryPolicy::default();
    let session = Session::new(Document::new("Multistart exchange").unwrap()).unwrap();
    let row = &model::lead_screw_presets()[0];
    let dir = tempfile::tempdir().unwrap();
    for starts in [2, 3] {
        for hand in [ThreadHand::Right, ThreadHand::Left] {
            for frame in [
                PlaneFrame::default(),
                PlaneFrame {
                    origin: [17., -8., 13.],
                    u: [0.8, 0.6, 0.],
                    v: [-0.36, 0.48, 0.8],
                },
            ] {
                for internal in [false, true] {
                    let mut parameters = row.parameters();
                    parameters.starts = starts;
                    parameters.hand = hand;
                    parameters.length = length(3.37 * row.pitch_mm);
                    let r = parameters
                        .resolve(
                            session.parameters(),
                            if internal {
                                ThreadMode::CutInternal
                            } else {
                                ThreadMode::NewExternal
                            },
                        )
                        .unwrap();
                    let spec = HelicalThread {
                        major_radius_mm: r.major_radius_mm,
                        pitch_mm: r.pitch_mm,
                        length_mm: r.length_mm,
                        depth_mm: r.profile.depth_mm,
                        crest_width_mm: r.profile.crest_width_mm,
                        lower_flank_deg: r.profile.lower_flank_deg,
                        upper_flank_deg: r.profile.upper_flank_deg,
                        radial_taper: 0.,
                        starts,
                        left_hand: hand == ThreadHand::Left,
                    };
                    let id = EntityId::new();
                    let source = if internal {
                        let q = |v: f64| Rational::parse_decimal(&v.to_string()).unwrap();
                        let stock_id = EntityId::new();
                        let stock = Solid::from_axial_cylinder_sweep(
                            AxialCylinder::new(
                                stock_id.uuid_bits(),
                                Circle::new([q(0.); 2], q(8.)).unwrap(),
                                q(0.),
                                q(spec.length_mm),
                            )
                            .unwrap(),
                            frame,
                            [0.; 2],
                            false,
                            Origin {
                                source: stock_id,
                                role: "receiver".into(),
                            },
                            policy,
                        )
                        .unwrap();
                        stock
                            .rust_cut_internal_thread(spec, 0.5, id, frame, policy)
                            .unwrap()
                    } else {
                        Solid::rust_helical_thread(spec, None, id, frame, policy).unwrap()
                    };
                    let before = source.topology_snapshot(id, 0).unwrap();
                    let path = dir.path().join("thread.step");
                    source.write_step(&path).unwrap();
                    let text = std::fs::read_to_string(&path).unwrap();
                    assert_eq!(
                        text.matches("B_SPLINE_SURFACE_WITH_KNOTS").count(),
                        4 * starts as usize
                    );
                    let imported = Solid::import_rust_step(&text, EntityId::new(), policy).unwrap();
                    near(
                        imported.volume_mm3().unwrap(),
                        source.volume_mm3().unwrap(),
                        1e-5,
                    );
                    for z in [0.31, 1.27, 2.71] {
                        for angle in [0.13_f64, 1.18, 3.21] {
                            let radius = spec.major_radius_mm - spec.depth_mm / 2.;
                            let p = std::array::from_fn(|i| {
                                frame.origin[i]
                                    + frame.u[i] * radius * angle.cos()
                                    + frame.v[i] * radius * angle.sin()
                                    + frame.normal()[i] * z * spec.pitch_mm
                            });
                            assert_eq!(
                                source.rust_thread_contains(p).unwrap(),
                                imported.rust_thread_contains(p).unwrap()
                            );
                        }
                    }
                    for element in &before.elements {
                        let intent = GeometryIntent::capture(&before, element.key).unwrap();
                        assert!(matches!(
                            resolve(&intent, &before, 0),
                            Resolution::Resolved { .. }
                        ));
                    }
                    let b = source.bounds().unwrap();
                    if OCCT_LINKED {
                        let native = Solid::read_step(&path, policy).unwrap();
                        assert_eq!(native.solid_count().unwrap(), 1);
                        // Independent native point/solid classification at each
                        // distinct start's crest and its neighboring root.
                        let rate = spec.pitch_mm * f64::from(starts) / std::f64::consts::TAU
                            * if spec.left_hand { -1. } else { 1. };
                        for start in 0..starts {
                            for (phase, crest) in [
                                (f64::from(start) * spec.pitch_mm, true),
                                ((f64::from(start) + 0.5) * spec.pitch_mm, false),
                            ] {
                                let z = spec.length_mm / 2.;
                                let angle = (z - phase) / rate;
                                let radius = spec.major_radius_mm - spec.depth_mm / 2.;
                                let point = std::array::from_fn(|i| {
                                    frame.origin[i]
                                        + frame.u[i] * radius * angle.cos()
                                        + frame.v[i] * radius * angle.sin()
                                        + frame.normal()[i] * z
                                });
                                let probe = Solid::box_solid([0.01; 3], policy)
                                    .unwrap()
                                    .transformed(RigidTransform {
                                        translation_mm: point,
                                        ..Default::default()
                                    })
                                    .unwrap();
                                let vertex = probe
                                    .display_vertices()
                                    .unwrap()
                                    .into_iter()
                                    .find(|v| {
                                        v.position_mm
                                            .into_iter()
                                            .zip(point)
                                            .all(|(a, b)| (a - b).abs() < 1e-9)
                                    })
                                    .unwrap()
                                    .index;
                                let distance = native
                                    .minimum_distance(
                                        BRepSelection::Whole,
                                        &probe,
                                        BRepSelection::Vertex(vertex),
                                    )
                                    .unwrap();
                                assert_eq!(distance.inner_solution, crest != internal);
                            }
                        }
                        near(
                            native.volume_mm3().unwrap(),
                            source.volume_mm3().unwrap(),
                            2e-5,
                        );
                        for (a, b) in native
                            .bounds()
                            .unwrap()
                            .min
                            .into_iter()
                            .chain(native.bounds().unwrap().max)
                            .zip(b.min.into_iter().chain(b.max))
                        {
                            near(a, b, 2e-6);
                        }
                    }
                    if frame == PlaneFrame::default() {
                        let mesh = imported
                            .manufacturing_mesh_with_receipt(MeshOptions {
                                linear_deflection_mm: 0.05,
                                ..Default::default()
                            })
                            .unwrap();
                        assert_eq!(mesh.edge_pairs as usize * 2, mesh.mesh.indices.len());
                    }
                    assert_eq!(source.topology_snapshot(id, 0).unwrap(), before);
                }
            }
        }
    }
}
