//! Rust internal ring gears against the native OCCT ring contract.
use multicad_core::{
    document::EntityId,
    references::{AnalyticGeometry, TopologyKind},
};
use multicad_kernel::{
    BooleanOperation, GearHub, GearSweep, GeometryErrorKind, GeometryPolicy, InvoluteGear,
    MeshOptions, RigidTransform, Solid,
    rust_kernel::{spur_gear::clearance::ResultKind, step_read},
};
#[cfg(feature = "occt")]
use std::collections::BTreeMap;
use std::f64::consts::{PI, TAU};

#[derive(Clone, Copy)]
struct Case {
    teeth: u32,
    module: f64,
    fillet: f64,
    rim: f64,
    width: f64,
    thinning: f64,
}

const NOMINAL: Case = Case {
    teeth: 48,
    module: 1.,
    fillet: 0.2,
    rim: 4.,
    width: 2.,
    thinning: 0.05,
};

fn ring(case: Case) -> InvoluteGear {
    let a = 20_f64.to_radians();
    let m = case.module;
    let pitch = m * f64::from(case.teeth) / 2.;
    InvoluteGear {
        hub: None,
        teeth: case.teeth,
        pitch_radius_mm: pitch,
        base_radius_mm: pitch * a.cos(),
        tip_radius_mm: pitch - m,
        root_radius_mm: pitch + 1.25 * m,
        root_fillet_radius_mm: case.fillet,
        half_base_angle_rad: (PI * m / 2. - case.thinning) / (2. * pitch) - a.tan() + a,
        face_width_mm: case.width,
        bore_radius_mm: 0.,
        twist_rad_per_mm: 0.,
        sweep: GearSweep::InternalSpur {
            outer_radius_mm: pitch + case.rim * m,
        },
    }
}

fn pinion(case: Case, bore: f64) -> InvoluteGear {
    let a = 20_f64.to_radians();
    let m = case.module;
    let pitch = 12. * m;
    InvoluteGear {
        hub: None,
        teeth: 24,
        pitch_radius_mm: pitch,
        base_radius_mm: pitch * a.cos(),
        tip_radius_mm: pitch + m,
        root_radius_mm: pitch - 1.25 * m,
        root_fillet_radius_mm: case.fillet,
        half_base_angle_rad: (PI * m / 2. - case.thinning) / (2. * pitch) + a.tan() - a,
        face_width_mm: case.width,
        bore_radius_mm: bore,
        twist_rad_per_mm: 0.,
        sweep: GearSweep::Spur,
    }
}

/// Rotation about the oblique axis (1, 2, 3) by 0.7 rad.
fn tilt() -> [[f64; 3]; 3] {
    let n = 14_f64.sqrt();
    let [x, y, z] = [1. / n, 2. / n, 3. / n];
    let (s, c) = 0.7_f64.sin_cos();
    let t = 1. - c;
    [
        [t * x * x + c, t * x * y - s * z, t * x * z + s * y],
        [t * x * y + s * z, t * y * y + c, t * y * z - s * x],
        [t * x * z - s * y, t * y * z + s * x, t * z * z + c],
    ]
}

const OFFSET: [f64; 3] = [12.5, -40., 7.25];

/// Place a gear in the tilted pair frame: spin by `phase` about its own
/// axis and translate `x` along the pair frame's first axis.
fn pose(phase: f64, x: f64, tilted: bool) -> RigidTransform {
    let (s, c) = phase.sin_cos();
    let spin = [[c, -s, 0.], [s, c, 0.], [0., 0., 1.]];
    let (tilt, offset) = if tilted {
        (tilt(), OFFSET)
    } else {
        ([[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]], [0.; 3])
    };
    RigidTransform {
        rotation: std::array::from_fn(|i| {
            std::array::from_fn(|j| (0..3).map(|k| tilt[i][k] * spin[k][j]).sum())
        }),
        translation_mm: std::array::from_fn(|i| offset[i] + tilt[i][0] * x),
    }
}

#[cfg(feature = "occt")]
fn role_areas(solid: &Solid, feature: EntityId) -> BTreeMap<String, f64> {
    let faces: BTreeMap<_, _> = solid
        .faces()
        .unwrap()
        .into_iter()
        .map(|face| (face.index, face.area_mm2))
        .collect();
    let mut areas = BTreeMap::new();
    for element in solid.topology_snapshot(feature, 0).unwrap().elements {
        if element.key.kind != TopologyKind::Face {
            continue;
        }
        assert!(!element.origins.is_empty(), "{:?}", element.key);
        for origin in element.origins {
            assert_eq!(origin.source, feature);
            *areas.entry(origin.role).or_insert(0.) += faces[&element.key.ordinal];
        }
    }
    areas
}

fn json_f64(value: f64) -> String {
    format!("{value:.12e}")
}

#[test]
#[cfg(feature = "occt")]
fn rust_rings_match_native_volume_role_areas_bounds_and_step() {
    let policy = GeometryPolicy::default();
    let directory = tempfile::tempdir().unwrap();
    for (index, case) in [
        NOMINAL,
        Case {
            teeth: 60,
            ..NOMINAL
        },
        Case {
            teeth: 72,
            module: 1.5,
            fillet: 0.38 * 1.5,
            rim: 3.,
            width: 5.,
            thinning: 0.08,
        },
        Case {
            teeth: 40,
            module: 2.,
            fillet: 0.3,
            rim: 6.,
            width: 8.,
            thinning: 0.1,
        },
    ]
    .into_iter()
    .enumerate()
    {
        let spec = ring(case);
        let feature = EntityId::new();
        let native = Solid::involute_gear_named(spec, feature, policy).unwrap();
        let rust = Solid::rust_internal_ring_named(spec, feature, policy).unwrap();
        for tilted in [false, true] {
            let transform = pose(0.3, 0., tilted);
            let [native, rust] = [&native, &rust].map(|s| s.transformed(transform).unwrap());
            assert!(rust.uses_rust_geometry());
            assert_eq!(native.solid_count().unwrap(), 1);
            assert_eq!(rust.solid_count().unwrap(), 1);
            let [vn, vr] = [&native, &rust].map(|s| s.volume_mm3().unwrap());
            assert!((vn - vr).abs() < 1e-6 * vn, "{vn} {vr}");
            let [an, ar] = [&native, &rust].map(|s| role_areas(s, feature));
            assert_eq!(an.keys().collect::<Vec<_>>(), ar.keys().collect::<Vec<_>>());
            assert!(ar.contains_key("gear/rim.outer") && ar.contains_key("gear/end.end"));
            let mut worst: f64 = 0.;
            for (role, area) in &an {
                worst = worst.max((area - ar[role]).abs() / area.max(1.));
            }
            assert!(worst < 1e-6, "{worst}");
            let [bn, br] = [&native, &rust].map(|s| s.bounds().unwrap());
            let bounds_error = (0..3)
                .map(|i| {
                    (bn.min[i] - br.min[i])
                        .abs()
                        .max((bn.max[i] - br.max[i]).abs())
                })
                .fold(0., f64::max);
            assert!(bounds_error < 1e-5, "{bounds_error}");
            // Exact Rust STEP: independent Rust readback against the source
            // topology, then OCCT reads one valid solid of the same volume.
            let path = directory.path().join(format!("ring-{index}-{tilted}.step"));
            rust.write_step(&path).unwrap();
            let text = std::fs::read_to_string(&path).unwrap();
            assert_eq!(text.matches("MANIFOLD_SOLID_BREP").count(), 1);
            let decoded = step_read::read_analytic_compound_with_helical_cuts(
                &text,
                policy.linear_tolerance_mm,
            )
            .unwrap();
            let topology = rust.topology_snapshot(feature, 0).unwrap();
            assert!(rust.step_readback_matches(
                &topology,
                &decoded,
                policy.linear_tolerance_mm * 2.
            ));
            let rust_readback = decoded[0].0.volume_mm3;
            assert!((rust_readback - vr).abs() < 1e-8 * vr);
            let occt = multicad_kernel::Shape::read_step(&path).unwrap();
            assert!(occt.valid().unwrap());
            let occt_volume = occt.volume().unwrap();
            assert!((occt_volume - vr).abs() < 1e-7 * vr);
            let read = Solid::read_step(&path, policy).unwrap();
            assert_eq!(read.solid_count().unwrap(), 1);
            println!(
                "{{\"case\":\"internal ring parity\",\"teeth\":{},\"module\":{},\"tilted\":{tilted},\"native_volume_mm3\":{},\"rust_volume_mm3\":{},\"worst_role_area_relative\":{},\"bounds_error_mm\":{},\"rust_step_readback_mm3\":{},\"occt_step_readback_mm3\":{},\"roles\":{}}}",
                case.teeth,
                case.module,
                json_f64(vn),
                json_f64(vr),
                json_f64(worst),
                json_f64(bounds_error),
                json_f64(rust_readback),
                json_f64(occt_volume),
                ar.len()
            );
        }
    }
}

#[test]
fn rust_ring_names_queries_meshes_and_draws_the_native_tooth_roles() {
    let policy = GeometryPolicy::default();
    let spec = ring(NOMINAL);
    let feature = EntityId::new();
    let solid = Solid::rust_internal_ring_named(spec, feature, policy).unwrap();
    let topology = solid.topology_snapshot(feature, 0).unwrap();
    let faces = solid.faces().unwrap();
    assert_eq!(faces.len(), 48 * 6 + 3);
    for tooth in 0..48 {
        let phase = TAU * f64::from(tooth) / 48.;
        for side in [
            "tip",
            "involute.left",
            "involute.right",
            "fillet.left",
            "fillet.right",
            "root",
        ] {
            let role = format!("gear/whole/tooth/{tooth}/{side}");
            let element = topology
                .elements
                .iter()
                .find(|e| {
                    e.key.kind == TopologyKind::Face
                        && e.origins
                            .iter()
                            .any(|o| o.source == feature && o.role == role)
                })
                .unwrap();
            let face = faces
                .iter()
                .find(|f| f.index == element.key.ordinal)
                .unwrap();
            let angle = face.center_mm[1].atan2(face.center_mm[0]) - phase;
            let offset = angle.sin().atan2(angle.cos());
            match side {
                "tip" => assert!(offset.abs() < 1e-9),
                "root" => assert!((offset - PI / 48.).abs() < 1e-9),
                _ if side.ends_with("left") => assert!(offset < 0. && offset > -PI / 48.),
                _ => assert!(offset > 0. && offset < PI / 48.),
            }
        }
    }
    for expected in [
        0.2,
        spec.tip_radius_mm,
        spec.root_radius_mm,
        spec.pitch_radius_mm + 4.,
    ] {
        assert!(
            topology.elements.iter().any(|e| matches!(e.geometry,
                Some(AnalyticGeometry::CircularArc { radius_mm, .. }) if (radius_mm - expected).abs() < 1e-9)),
            "circle {expected}"
        );
    }
    assert_eq!(
        topology
            .elements
            .iter()
            .filter(|e| matches!(e.geometry, Some(AnalyticGeometry::InvoluteArc { .. })))
            .count(),
        2 * 2 * 48
    );
    // Display and certified manufacturing meshes, edges and vertices.
    let options = MeshOptions {
        linear_deflection_mm: 0.002,
        angular_deflection_rad: 0.2,
        maximum_triangles: 400_000,
    };
    let mesh = solid.mesh(options).unwrap();
    assert_eq!(
        mesh.face_indices.iter().copied().max().unwrap() as usize,
        faces.len() - 1
    );
    let receipt = solid.manufacturing_mesh_with_receipt(options).unwrap();
    assert!(!receipt.mesh.indices.is_empty() && receipt.edge_pairs > 0);
    let edges = solid.display_edges(0.01).unwrap();
    assert_eq!(edges.len(), 2 * (48 * 6 + 1) + 48 * 6);
    assert_eq!(solid.display_vertices().unwrap().len(), 2 * 48 * 6);
    // The exact axial drawing publishes every flank as a cubic spline and
    // the outside rim as one full circle.
    use multicad_core::drawings::{VectorCurve, VectorRole};
    use multicad_kernel::{DrawingProjection, DrawingSolid, PlaneFrame, project_drawing};
    let inputs = [DrawingSolid {
        solid: &solid,
        hatch: None,
    }];
    let options = DrawingProjection {
        hidden_lines: true,
        ..Default::default()
    };
    let vectors = project_drawing(&inputs, options).unwrap();
    assert!(vectors.iter().all(|v| v.role == VectorRole::Visible));
    assert_eq!(
        vectors
            .iter()
            .filter(|v| matches!(v.curve, VectorCurve::BSpline { degree: 3, .. }))
            .count(),
        96
    );
    assert_eq!(
        vectors
            .iter()
            .filter(
                |v| matches!(v.curve, VectorCurve::Ellipse { major_mm, end_rad, .. }
                if (major_mm - 28.).abs() < 1e-12 && end_rad == TAU)
            )
            .count(),
        1
    );
    let oblique = DrawingProjection {
        frame: PlaneFrame {
            v: [0., 0., 1.],
            ..Default::default()
        },
        ..options
    };
    assert_eq!(
        project_drawing(&inputs, oblique).unwrap_err().kind,
        GeometryErrorKind::Unsupported
    );
    // Sections take the shared planar-section path; record its result.
    let section = project_drawing(
        &inputs,
        DrawingProjection {
            section_offset_mm: Some(1.),
            ..options
        },
    );
    let spur = Solid::rust_spur_gear_named(pinion(NOMINAL, 2.), EntityId::new(), policy).unwrap();
    let spur_section = project_drawing(
        &[DrawingSolid {
            solid: &spur,
            hatch: None,
        }],
        DrawingProjection {
            section_offset_mm: Some(1.),
            ..options
        },
    );
    assert_eq!(
        section.as_ref().err().map(|e| e.kind),
        spur_section.as_ref().err().map(|e| e.kind)
    );
    println!(
        "{{\"case\":\"ring section\",\"ring\":{:?},\"spur\":{:?}}}",
        section.map(|v| v.len()),
        spur_section.map(|v| v.len())
    );
}

#[test]
fn rust_ring_refuses_the_native_envelope_with_explicit_errors() {
    let policy = GeometryPolicy::default();
    let valid = ring(NOMINAL);
    let feature = EntityId::new();
    let refused = |spec: InvoluteGear| {
        let rust = Solid::rust_internal_ring_named(spec, feature, policy).map(|_| ());
        let native = Solid::involute_gear_named(spec, feature, policy).map(|_| ());
        assert!(native.is_err(), "native accepted {spec:?}");
        rust.unwrap_err()
    };
    for bad in [
        InvoluteGear {
            sweep: GearSweep::InternalSpur {
                outer_radius_mm: f64::NAN,
            },
            ..valid
        },
        InvoluteGear {
            sweep: GearSweep::InternalSpur {
                outer_radius_mm: valid.root_radius_mm,
            },
            ..valid
        },
        InvoluteGear {
            tip_radius_mm: valid.base_radius_mm,
            ..valid
        },
        InvoluteGear {
            root_fillet_radius_mm: 10.,
            ..valid
        },
        InvoluteGear {
            bore_radius_mm: 1.,
            ..valid
        },
        InvoluteGear {
            twist_rad_per_mm: 0.01,
            ..valid
        },
        InvoluteGear {
            half_base_angle_rad: 1.,
            ..valid
        },
        InvoluteGear { teeth: 3, ..valid },
        InvoluteGear {
            hub: Some(GearHub {
                radius_mm: 20.,
                axial_start_mm: 0.,
                axial_end_mm: 2.,
            }),
            ..valid
        },
    ] {
        assert_eq!(refused(bad).kind, GeometryErrorKind::Input, "{bad:?}");
    }
    // Flanks crossing before the outward root leave no separated space.
    let start = {
        let t = ((valid.tip_radius_mm / valid.base_radius_mm).powi(2) - 1.).sqrt();
        t - t.atan()
    };
    let crossing = refused(InvoluteGear {
        half_base_angle_rad: PI / 48. - start - 1e-3,
        ..valid
    });
    assert!(crossing.message.contains("Internal fillet"), "{crossing}");
    // A flank beyond one half turn is a Rust limit; the native cubic budget.
    let deep = InvoluteGear {
        teeth: 4,
        pitch_radius_mm: 1.1,
        base_radius_mm: 1.,
        tip_radius_mm: 1.05,
        root_radius_mm: 4.5,
        root_fillet_radius_mm: 0.1,
        half_base_angle_rad: 0.3,
        face_width_mm: 1.,
        sweep: GearSweep::InternalSpur {
            outer_radius_mm: 5.,
        },
        ..valid
    };
    assert_eq!(
        Solid::rust_internal_ring_named(deep, feature, policy)
            .err()
            .unwrap()
            .kind,
        GeometryErrorKind::Limit
    );
    // A large ring whose certified cubic flank needs more than 127 spans
    // at the policy tolerance: refused by both kernels.
    let huge = ring(Case {
        module: 2e4,
        fillet: 0.2 * 2e4,
        thinning: 0.05 * 2e4,
        ..NOMINAL
    });
    assert_eq!(
        Solid::rust_internal_ring_named(huge, feature, policy)
            .err()
            .unwrap()
            .kind,
        GeometryErrorKind::Limit
    );
    assert!(Solid::involute_gear_named(huge, feature, policy).is_err());
    for (spec, id, kind) in [
        (
            InvoluteGear {
                sweep: GearSweep::Spur,
                ..valid
            },
            feature,
            GeometryErrorKind::Unsupported,
        ),
        (valid, EntityId::nil(), GeometryErrorKind::Input),
    ] {
        assert_eq!(
            Solid::rust_internal_ring_named(spec, id, policy)
                .err()
                .unwrap()
                .kind,
            kind
        );
    }
    assert!(
        Solid::rust_internal_ring_named(
            valid,
            feature,
            GeometryPolicy {
                linear_tolerance_mm: 1.,
                ..policy
            }
        )
        .is_err()
    );
}

/// Ring-first canonical phases: the pinion turns by pi / z_pinion plus the
/// pitch-line travel of the ring drive and sits at the pitch difference.
fn placed_pair(
    case: Case,
    drive: f64,
    phase_error: f64,
    center_error: f64,
    tilted: bool,
    solids: [&Solid; 2],
) -> [Solid; 2] {
    let center = case.module * f64::from(case.teeth - 24) / 2.;
    let teeth = f64::from(case.teeth);
    [
        solids[0].transformed(pose(drive, 0., tilted)).unwrap(),
        solids[1]
            .transformed(pose(
                PI / 24. + drive * teeth / 24. + phase_error,
                center + center_error,
                tilted,
            ))
            .unwrap(),
    ]
}

#[test]
fn rust_ring_pinion_certificates_agree_with_native_common_volume() {
    let policy = GeometryPolicy::default();
    for (case, tilted) in [
        (NOMINAL, true),
        (
            Case {
                teeth: 60,
                ..NOMINAL
            },
            false,
        ),
    ] {
        let feature = EntityId::new();
        let rust = [
            Solid::rust_internal_ring_named(ring(case), feature, policy).unwrap(),
            Solid::rust_spur_gear_named(pinion(case, 3.), EntityId::new(), policy).unwrap(),
        ];
        let native = multicad_kernel::OCCT_LINKED.then(|| {
            [
                Solid::involute_gear_named(ring(case), feature, policy).unwrap(),
                Solid::involute_gear_named(pinion(case, 3.), EntityId::new(), policy).unwrap(),
            ]
        });
        let certify = |phase_error, center_error, drive| {
            let [r, p] = placed_pair(
                case,
                drive,
                phase_error,
                center_error,
                tilted,
                [&rust[0], &rust[1]],
            );
            r.certify_rust_internal_pair(&p, policy, None).unwrap()
        };
        let overlap = |phase_error, center_error, drive| {
            let native = native.as_ref()?;
            let [r, p] = placed_pair(
                case,
                drive,
                phase_error,
                center_error,
                tilted,
                [&native[0], &native[1]],
            );
            Some(
                r.boolean(&p, BooleanOperation::Intersect, policy)
                    .unwrap()
                    .map_or(0., |common| common.volume_mm3().unwrap()),
            )
        };
        for i in 0..9 {
            let drive = TAU / f64::from(case.teeth) * f64::from(i) / 8.;
            assert_eq!(certify(0., 0., drive), ResultKind::Clear, "{i}");
            if let Some(volume) = overlap(0., 0., drive) {
                assert_eq!(volume, 0., "{i}");
            }
        }
        let half = certify(PI / 24., 0., 0.);
        assert_eq!(half, ResultKind::CommonInterior);
        let half_volume = overlap(PI / 24., 0., 0.);
        if let Some(volume) = half_volume {
            assert!(volume > 0.01);
        }
        let pushed = certify(0., 0.4, 0.);
        assert_ne!(pushed, ResultKind::Clear);
        let pushed_volume = overlap(0., 0.4, 0.);
        if let Some(volume) = pushed_volume {
            assert!(volume > 0.);
        }
        println!(
            "{{\"case\":\"internal pair poses\",\"teeth\":{},\"tilted\":{tilted},\"correct\":\"Clear x9\",\"half_pitch\":\"{half:?}\",\"native_half_pitch_common_mm3\":{},\"center_plus_0_4\":\"{pushed:?}\",\"native_center_plus_0_4_common_mm3\":{}}}",
            case.teeth,
            half_volume.map_or_else(|| "null".into(), json_f64),
            pushed_volume.map_or_else(|| "null".into(), json_f64)
        );
        // An assembly compound of the placed pair exports one verified STEP
        // with two members that OCCT also reads as two valid solids.
        let [r, p] = placed_pair(case, 0.1, 0., 0., tilted, [&rust[0], &rust[1]]);
        let compound = Solid::compound_named(&[&r, &p], EntityId::new(), policy).unwrap();
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("pair.step");
        compound.write_step(&path).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        let members = step_read::read_analytic_compound(&text, policy.linear_tolerance_mm).unwrap();
        assert_eq!(members.len(), 2);
        let expected = compound.volume_mm3().unwrap();
        let decoded: f64 = members.iter().map(|m| m.volume_mm3).sum();
        assert!((decoded - expected).abs() < 1e-8 * expected);
        let topology = compound.topology_snapshot(EntityId::new(), 0).unwrap();
        assert!(Solid::step_compound_topology_matches(
            &topology,
            &members,
            policy.linear_tolerance_mm * 2.
        ));
        if multicad_kernel::OCCT_LINKED {
            let read = Solid::read_step(&path, policy).unwrap();
            assert_eq!(read.solid_count().unwrap(), 2);
            assert!((read.volume_mm3().unwrap() - expected).abs() < 1e-7 * expected);
        }
    }
    // Explicit refusals for unsupported operands and placements.
    let ring_solid =
        Solid::rust_internal_ring_named(ring(NOMINAL), EntityId::new(), policy).unwrap();
    let pinion_solid =
        Solid::rust_spur_gear_named(pinion(NOMINAL, 0.), EntityId::new(), policy).unwrap();
    let hubbed = Solid::rust_spur_gear_named(
        InvoluteGear {
            hub: Some(GearHub {
                radius_mm: 8.,
                axial_start_mm: -2.,
                axial_end_mm: 2.,
            }),
            ..pinion(NOMINAL, 2.)
        },
        EntityId::new(),
        policy,
    )
    .unwrap();
    let skew = pinion_solid
        .transformed(RigidTransform {
            rotation: [[1., 0., 0.], [0., 0., -1.], [0., 1., 0.]],
            translation_mm: [12., 0., 0.],
        })
        .unwrap();
    let apart = pinion_solid
        .transformed(RigidTransform {
            translation_mm: [12., 0., 50.],
            ..Default::default()
        })
        .unwrap();
    for (a, b, fuzzy) in [
        (&pinion_solid, &ring_solid, 0.),
        (&ring_solid, &ring_solid, 0.),
        (&ring_solid, &hubbed, 0.),
        (&ring_solid, &skew, 0.),
        (&ring_solid, &apart, 0.),
        (&ring_solid, &pinion_solid, 0.),
        (&ring_solid, &pinion_solid, 1e-8),
    ] {
        let policy = GeometryPolicy {
            boolean_fuzzy_mm: fuzzy,
            ..policy
        };
        assert_eq!(
            a.certify_rust_internal_pair(b, policy, None)
                .unwrap_err()
                .kind,
            GeometryErrorKind::Unsupported
        );
    }
    let stop = || true;
    let placed = pinion_solid
        .transformed(RigidTransform {
            translation_mm: [12., 0., 0.],
            ..Default::default()
        })
        .unwrap();
    assert_eq!(
        ring_solid
            .certify_rust_internal_pair(&placed, policy, Some(&stop))
            .unwrap_err()
            .kind,
        GeometryErrorKind::Cancelled
    );
}

#[test]
#[cfg(feature = "occt")]
fn matched_internal_ring_build_pair_and_step_export_latency() {
    let policy = GeometryPolicy::default();
    let directory = tempfile::tempdir().unwrap();
    let time = |run: &mut dyn FnMut()| {
        let started = std::time::Instant::now();
        run();
        started.elapsed().as_secs_f64() * 1e3
    };
    let feature = EntityId::new();
    let spec = ring(NOMINAL);
    let mut built = [None, None];
    let builds = [true, false].map(|rust| {
        time(&mut || {
            built[usize::from(!rust)] = Some(
                if rust {
                    Solid::rust_internal_ring_named(spec, feature, policy)
                } else {
                    Solid::involute_gear_named(spec, feature, policy)
                }
                .unwrap(),
            );
        })
    });
    let [rust_ring, native_ring] = built.map(Option::unwrap);
    let rust_pinion =
        Solid::rust_spur_gear_named(pinion(NOMINAL, 3.), EntityId::new(), policy).unwrap();
    let native_pinion =
        Solid::involute_gear_named(pinion(NOMINAL, 3.), EntityId::new(), policy).unwrap();
    // Nine sampled phases plus the current pose, as the pair validators run.
    let drives: Vec<f64> = (0..9)
        .map(|i| TAU / 48. * f64::from(i) / 8.)
        .chain([0.37])
        .collect();
    let rust_pair_ms = time(&mut || {
        for &drive in &drives {
            let [r, p] = placed_pair(NOMINAL, drive, 0., 0., false, [&rust_ring, &rust_pinion]);
            assert_eq!(
                r.certify_rust_internal_pair(&p, policy, None).unwrap(),
                ResultKind::Clear
            );
        }
    });
    let native_pair_ms = time(&mut || {
        for &drive in &drives {
            let [r, p] = placed_pair(
                NOMINAL,
                drive,
                0.,
                0.,
                false,
                [&native_ring, &native_pinion],
            );
            assert!(
                r.boolean(&p, BooleanOperation::Intersect, policy)
                    .unwrap()
                    .is_none()
            );
        }
    });
    let exports = [&rust_ring, &native_ring].map(|solid| {
        let path = directory
            .path()
            .join(format!("{}.step", solid.uses_rust_geometry()));
        time(&mut || solid.write_step(&path).unwrap())
    });
    println!(
        "{{\"case\":\"internal ring build, pair and STEP latency\",\"profile\":\"{}\",\"rust_ring_build_ms\":{},\"native_ring_build_ms\":{},\"rust_pair_nine_phases_and_current_ms\":{},\"native_pair_nine_phases_and_current_ms\":{},\"rust_ring_step_ms\":{},\"native_ring_step_ms\":{}}}",
        if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        },
        builds[0],
        builds[1],
        rust_pair_ms,
        native_pair_ms,
        exports[0],
        exports[1]
    );
}
