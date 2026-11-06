use multicad_core::{
    document::EntityId,
    references::{AnalyticGeometry, TopologyKind},
};
use multicad_kernel::{
    BooleanOperation, GearHub, GearSweep, GeometryPolicy, InvoluteGear, MeshOptions,
    RigidTransform, Solid,
};
use std::f64::consts::{PI, TAU};

fn normal(teeth: u32, beta: f64, sweep: GearSweep) -> InvoluteGear {
    // KHK normal-system basic dimensions: mt=mn/cos(beta),
    // tan(alpha_t)=tan(alpha_n)/cos(beta), ha=mn, hf=1.25*mn.
    let module = 2.;
    let pressure = (20_f64.to_radians().tan() / beta.cos()).atan();
    let pitch = module * f64::from(teeth) / (2. * beta.cos());
    InvoluteGear {
        teeth,
        pitch_radius_mm: pitch,
        base_radius_mm: pitch * pressure.cos(),
        tip_radius_mm: pitch + module,
        root_radius_mm: pitch - 1.25 * module,
        root_fillet_radius_mm: 0.2 * module,
        half_base_angle_rad: PI / (2. * f64::from(teeth)) + pressure.tan()
            - pressure
            - 0.05 / (2. * pitch),
        face_width_mm: 8.,
        bore_radius_mm: 2.,
        hub: None,
        twist_rad_per_mm: beta.tan() / pitch,
        sweep,
    }
}
fn build(spec: InvoluteGear) -> Solid {
    Solid::involute_gear_named(spec, EntityId::new(), GeometryPolicy::default()).unwrap()
}

#[test]
fn intrinsic_hubs_extend_one_solid_and_one_continuous_bore_with_exact_face_ancestry() {
    for (beta, sweep) in [
        (0., GearSweep::Spur),
        (20_f64.to_radians(), GearSweep::Helical),
        (20_f64.to_radians(), GearSweep::Chevron),
        (20_f64.to_radians(), GearSweep::GappedChevron { gap_mm: 2. }),
    ] {
        let base = normal(20, beta, sweep);
        let plain = build(base);
        let feature = EntityId::new();
        let spec = InvoluteGear {
            hub: Some(GearHub {
                radius_mm: 10.,
                axial_start_mm: -3.,
                axial_end_mm: 11.,
            }),
            ..base
        };
        let solid = Solid::involute_gear_named(spec, feature, GeometryPolicy::default()).unwrap();
        assert_eq!(solid.solid_count().unwrap(), 1);
        let added = 6. * PI * (100. - base.bore_radius_mm.powi(2));
        assert!((solid.volume_mm3().unwrap() - plain.volume_mm3().unwrap() - added).abs() < 0.005);
        let bounds = solid.bounds().unwrap();
        assert!((bounds.min[2] + 3.).abs() < 1e-6);
        assert!((bounds.max[2] - 11.).abs() < 1e-6);
        for z in [-2.3, 0.3, 4., 7.7, 10.3] {
            assert!(probe(&solid, 0., 0., z) < 1e-5);
            assert!(probe(&solid, base.bore_radius_mm - 0.1, 0., z) < 1e-5);
            assert!(probe(&solid, base.bore_radius_mm + 0.1, 0., z) > 0.999);
        }
        for z in [-2.3, 10.3] {
            assert!(probe(&solid, 9.9, 0., z) > 0.999);
            assert!(probe(&solid, 10.1, 0., z) < 1e-5);
        }
        if matches!(sweep, GearSweep::GappedChevron { .. }) {
            assert!(probe(&solid, base.pitch_radius_mm, 0., 4.) < 1e-5);
        }
        let snapshot = solid.topology_snapshot(feature, 0).unwrap();
        for role in [
            "gear/hub.side",
            "gear/hub.end.start",
            "gear/hub.end.end",
            "gear/bore",
            "gear/end.start",
            "gear/end.end",
        ] {
            assert!(
                snapshot
                    .elements
                    .iter()
                    .any(|e| e.key.kind == TopologyKind::Face
                        && e.origins
                            .iter()
                            .any(|o| o.source == feature && o.role == role)),
                "{sweep:?}: {role}"
            );
        }
        assert!(
            snapshot
                .elements
                .iter()
                .filter(|e| e.key.kind == TopologyKind::Face)
                .all(|e| !e.origins.is_empty())
        );
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("hub.step");
        solid.write_step(&path).unwrap();
        let read = Solid::read_step(&path, GeometryPolicy::default()).unwrap();
        assert_eq!(read.solid_count().unwrap(), 1);
        assert!((read.volume_mm3().unwrap() - solid.volume_mm3().unwrap()).abs() < 0.005);
        for z in [-2.3, 4., 10.3] {
            assert!(probe(&read, base.bore_radius_mm - 0.1, 0., z) < 1e-5);
            assert!(probe(&read, base.bore_radius_mm + 0.1, 0., z) > 0.999);
        }
    }
}

#[test]
fn detached_thin_root_erasing_and_nonfinite_hubs_are_rejected() {
    let base = normal(20, 0., GearSweep::Spur);
    for hub in [
        GearHub {
            radius_mm: 10.,
            axial_start_mm: 9.,
            axial_end_mm: 11.,
        },
        GearHub {
            radius_mm: 10.,
            axial_start_mm: -2.,
            axial_end_mm: 0.,
        },
        GearHub {
            radius_mm: 10.,
            axial_start_mm: 2.,
            axial_end_mm: 2.,
        },
        GearHub {
            radius_mm: 1.,
            axial_start_mm: -2.,
            axial_end_mm: 2.,
        },
        GearHub {
            radius_mm: base.root_radius_mm,
            axial_start_mm: -2.,
            axial_end_mm: 2.,
        },
        GearHub {
            radius_mm: f64::NAN,
            axial_start_mm: -2.,
            axial_end_mm: 2.,
        },
        GearHub {
            radius_mm: 10.,
            axial_start_mm: f64::NEG_INFINITY,
            axial_end_mm: 2.,
        },
    ] {
        assert!(
            Solid::involute_gear_named(
                InvoluteGear {
                    hub: Some(hub),
                    ..base
                },
                EntityId::new(),
                GeometryPolicy::default()
            )
            .is_err(),
            "{hub:?}"
        );
    }
}

#[test]
fn coincident_or_hidden_hub_caps_preserve_every_surviving_face_and_gear_end() {
    let base = normal(20, 0., GearSweep::Spur);
    let plain = build(base).volume_mm3().unwrap();
    for (start, end, extension) in [(-3., 8., 3.), (0., 11., 3.), (0., 8., 0.)] {
        let feature = EntityId::new();
        let solid = Solid::involute_gear_named(
            InvoluteGear {
                hub: Some(GearHub {
                    radius_mm: 10.,
                    axial_start_mm: start,
                    axial_end_mm: end,
                }),
                ..base
            },
            feature,
            GeometryPolicy::default(),
        )
        .unwrap();
        assert_eq!(solid.solid_count().unwrap(), 1);
        assert!((solid.volume_mm3().unwrap() - plain - extension * PI * 96.).abs() < 0.005);
        let snapshot = solid.topology_snapshot(feature, 0).unwrap();
        assert!(
            snapshot
                .elements
                .iter()
                .filter(|e| e.key.kind == TopologyKind::Face)
                .all(|e| !e.origins.is_empty())
        );
        for role in ["gear/end.start", "gear/end.end", "gear/bore"] {
            assert!(
                snapshot
                    .elements
                    .iter()
                    .any(|e| e.origins.iter().any(|o| o.role == role))
            );
        }
        if extension > 0. {
            let role = if start < 0. {
                "gear/hub.end.start"
            } else {
                "gear/hub.end.end"
            };
            assert!(
                snapshot
                    .elements
                    .iter()
                    .any(|e| e.origins.iter().any(|o| o.role == role))
            );
        }
    }
}
fn probe(solid: &Solid, r: f64, a: f64, z: f64) -> f64 {
    let size = 0.005;
    let center = [r * a.cos(), r * a.sin(), z];
    let cube = Solid::box_solid([size; 3], GeometryPolicy::default())
        .unwrap()
        .transformed(RigidTransform {
            translation_mm: center.map(|v| v - size / 2.),
            ..Default::default()
        })
        .unwrap();
    solid
        .boolean(
            &cube,
            BooleanOperation::Intersect,
            GeometryPolicy::default(),
        )
        .unwrap()
        .map_or(0., |s| s.volume_mm3().unwrap())
        / size.powi(3)
}
fn overlap(a: &Solid, b: &Solid) -> f64 {
    a.boolean(b, BooleanOperation::Intersect, GeometryPolicy::default())
        .unwrap()
        .map_or(0., |s| s.volume_mm3().unwrap())
}
fn rotated(s: &Solid, angle: f64, x: f64) -> Solid {
    s.transformed(RigidTransform {
        rotation: [
            [angle.cos(), -angle.sin(), 0.],
            [angle.sin(), angle.cos(), 0.],
            [0., 0., 1.],
        ],
        translation_mm: [x, 0., 0.],
    })
    .unwrap()
}

#[test]
fn spur_has_measured_involute_width_native_circular_fillets_and_complete_provenance() {
    let spec = normal(20, 0., GearSweep::Spur);
    let feature = EntityId::new();
    let solid = Solid::involute_gear_named(spec, feature, GeometryPolicy::default()).unwrap();
    assert_eq!(solid.solid_count().unwrap(), 1);
    let bounds = solid.bounds().unwrap();
    for axis in 0..2 {
        assert!((bounds.max[axis] - 22.).abs() < 1e-5);
        assert!((bounds.min[axis] + 22.).abs() < 1e-5);
    }
    assert!(bounds.min[2].abs() < 1e-6);
    assert!((bounds.max[2] - 8.).abs() < 1e-6);
    for r in [
        (spec.base_radius_mm + spec.pitch_radius_mm) / 2.,
        spec.pitch_radius_mm,
        21.,
    ] {
        let t = (r * r / spec.base_radius_mm.powi(2) - 1.).sqrt();
        let half = spec.half_base_angle_rad - (t - t.atan());
        for sign in [-1., 1.] {
            assert!(probe(&solid, r, sign * (half - 0.003), 3.37) > 0.999);
            assert!(probe(&solid, r, sign * (half + 0.003), 3.37) < 1e-5);
        }
    }
    assert!(probe(&solid, 0., 0., 3.37) < 1e-5);
    assert!(probe(&solid, spec.root_radius_mm - 0.1, 0., 3.37) > 0.999);
    let topology = solid.topology_snapshot(feature, 0).unwrap();
    assert!(
        topology
            .elements
            .iter()
            .filter(|e| e.key.kind == TopologyKind::Face)
            .all(|e| !e.origins.is_empty())
    );
    for tooth in 0..spec.teeth {
        for side in [
            "involute.left",
            "involute.right",
            "fillet.left",
            "fillet.right",
        ] {
            let role = format!("gear/whole/tooth/{tooth}/{side}");
            assert!(
                topology
                    .elements
                    .iter()
                    .any(|e| e.key.kind == TopologyKind::Face
                        && e.origins
                            .iter()
                            .any(|o| o.source == feature && o.role == role)),
                "{role}"
            );
        }
    }
    for expected in [0.4, 17.5, 22.] {
        assert!(topology.elements.iter().any(|e|matches!(e.geometry,Some(AnalyticGeometry::CircularArc{radius_mm,..})if(radius_mm-expected).abs()<1e-6)),"native circle radius {expected}");
    }
}

#[test]
fn helix_hand_chevron_reversal_and_center_gap_are_real_native_sections() {
    for (beta, sweep) in [
        (20_f64.to_radians(), GearSweep::Helical),
        (-20_f64.to_radians(), GearSweep::Helical),
        (20_f64.to_radians(), GearSweep::Chevron),
        (
            -20_f64.to_radians(),
            GearSweep::GappedChevron { gap_mm: 2. },
        ),
    ] {
        let spec = normal(20, beta, sweep);
        let solid = build(spec);
        for z in [1.13_f64, 6.87] {
            let progress = if matches!(sweep, GearSweep::Helical) {
                z
            } else {
                z.min(spec.face_width_mm - z)
            };
            let phase = spec.twist_rad_per_mm * progress;
            assert!(
                probe(&solid, spec.pitch_radius_mm, phase, z) > 0.999,
                "{sweep:?}"
            );
            assert!(
                probe(
                    &solid,
                    spec.pitch_radius_mm,
                    phase + PI / f64::from(spec.teeth),
                    z
                ) < 1e-5
            );
        }
        let mid = spec.face_width_mm / 2.;
        if matches!(sweep, GearSweep::GappedChevron { .. }) {
            assert!(probe(&solid, spec.pitch_radius_mm, 0., mid) < 1e-5);
            assert!(probe(&solid, spec.pitch_radius_mm, 0.11, mid) < 1e-5);
            assert!(probe(&solid, spec.root_radius_mm - 0.1, 0., mid) > 0.999);
        } else {
            assert!(
                probe(
                    &solid,
                    spec.pitch_radius_mm,
                    spec.twist_rad_per_mm * mid,
                    mid
                ) > 0.999
            );
        }
    }
}

#[test]
fn compatible_pairs_have_no_sampled_interference_and_wrong_phase_is_detected() {
    for (beta, sweep) in [
        (0., GearSweep::Spur),
        (20_f64.to_radians(), GearSweep::Helical),
    ] {
        let a = normal(20, beta, sweep);
        let b = normal(30, -beta, sweep);
        let one = build(a);
        let two = build(b);
        let center = a.pitch_radius_mm + b.pitch_radius_mm;
        assert!(overlap(&one, &rotated(&two, PI, center)) > 1.);
        for fraction in [0., 0.173, 0.791] {
            let angle = TAU / f64::from(a.teeth) * fraction;
            let partner =
                PI + PI / f64::from(b.teeth) - angle * f64::from(a.teeth) / f64::from(b.teeth);
            assert!(overlap(&rotated(&one, angle, 0.), &rotated(&two, partner, center)) < 1e-5);
        }
    }
}

#[test]
fn larger_teeth_root_fillets_and_helical_step_roundtrip_keep_manufacturing_geometry() {
    let large = build(normal(60, 0., GearSweep::Spur));
    assert_eq!(large.solid_count().unwrap(), 1);
    assert!(large.volume_mm3().unwrap() > 89_000.);
    let spec = normal(20, 20_f64.to_radians(), GearSweep::Chevron);
    let solid = build(spec);
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("herringbone.step");
    solid.write_step(&path).unwrap();
    let reopened = Solid::read_step(&path, GeometryPolicy::default()).unwrap();
    assert_eq!(reopened.solid_count().unwrap(), 1);
    assert!((reopened.volume_mm3().unwrap() - solid.volume_mm3().unwrap()).abs() < 1e-3);
    let phase = spec.twist_rad_per_mm * 1.13;
    assert!(probe(&reopened, spec.pitch_radius_mm, phase, 1.13) > 0.999);
    assert!(
        probe(
            &reopened,
            spec.pitch_radius_mm,
            phase + PI / f64::from(spec.teeth),
            1.13
        ) < 1e-5
    );
    let mesh = reopened
        .mesh(MeshOptions {
            linear_deflection_mm: 0.05,
            ..Default::default()
        })
        .unwrap();
    assert!(mesh.indices.len() > 3000);
}

#[test]
fn invalid_inputs_and_excessive_twist_are_refused_without_a_solid() {
    let base = normal(20, 0., GearSweep::Spur);
    for spec in [
        InvoluteGear { teeth: 0, ..base },
        InvoluteGear { teeth: 129, ..base },
        InvoluteGear {
            base_radius_mm: f64::NAN,
            ..base
        },
        InvoluteGear {
            bore_radius_mm: base.root_radius_mm,
            ..base
        },
        InvoluteGear {
            half_base_angle_rad: 0.,
            ..base
        },
        InvoluteGear {
            root_fillet_radius_mm: 100.,
            ..base
        },
        InvoluteGear {
            twist_rad_per_mm: 1.,
            ..base
        },
        InvoluteGear {
            sweep: GearSweep::Helical,
            ..base
        },
        InvoluteGear {
            sweep: GearSweep::GappedChevron { gap_mm: 8. },
            twist_rad_per_mm: 0.01,
            ..base
        },
    ] {
        assert!(
            Solid::involute_gear_named(spec, EntityId::new(), GeometryPolicy::default()).is_err()
        );
    }
    assert!(Solid::involute_gear_named(base, EntityId::nil(), GeometryPolicy::default()).is_err());
}
