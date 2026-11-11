use multicad_core::{
    document::EntityId,
    references::{AnalyticGeometry, TopologyKind},
};
use multicad_kernel::{
    DrawingProjection, DrawingSolid, GearSweep, GeometryErrorKind, GeometryPolicy, InvoluteGear,
    MeshOptions, RigidTransform, Solid, project_drawing,
    rust_kernel::spur_gear::clearance::ResultKind,
};
use std::f64::consts::PI;

fn specification(teeth: u32) -> InvoluteGear {
    let beta = 15_f64.to_radians();
    let at = (20_f64.to_radians().tan() / beta.cos()).atan();
    let transverse_module = 2. / beta.cos();
    let pitch = transverse_module * f64::from(teeth) / 2.;
    InvoluteGear {
        teeth,
        pitch_radius_mm: pitch,
        base_radius_mm: pitch * at.cos(),
        tip_radius_mm: pitch + 2.,
        root_radius_mm: pitch - 2.5,
        root_fillet_radius_mm: 0.4,
        half_base_angle_rad: (PI * transverse_module / 2. - 0.05) / (2. * pitch) + at.tan() - at,
        face_width_mm: 4.,
        bore_radius_mm: 4.,
        hub: None,
        twist_rad_per_mm: if teeth == 24 { 1. } else { -1. } * beta.tan() / pitch,
        sweep: GearSweep::Chevron,
    }
}

#[test]
fn mixed_twisted_compounds_keep_cavity_shells_in_one_logical_step_member() {
    use multicad_kernel::{
        BooleanOperation, PlaneFrame,
        rust_kernel::{Rational, step_read},
    };
    let policy = GeometryPolicy::default();
    let stock = |size: [i64; 3], origin| {
        Solid::rust_box(
            EntityId::new().uuid_bits(),
            size.map(|n| Rational::new(n, 1).unwrap()),
            PlaneFrame {
                origin,
                ..Default::default()
            },
            policy,
        )
        .unwrap()
    };
    for pose in [
        RigidTransform {
            translation_mm: [80., 60., 20.],
            ..Default::default()
        },
        RigidTransform {
            rotation: [[0.8, -0.36, 0.48], [0.6, 0.48, -0.64], [0., 0.8, 0.6]],
            translation_mm: [80., 60., 20.],
        },
    ] {
        let exterior = stock([15, 10, 8], [0.; 3]).transformed(pose).unwrap();
        let probe = stock([5, 4, 3], [2., 3., 1.]).transformed(pose).unwrap();
        let cavity = exterior
            .rust_subtract_planar_tools(&[&probe], EntityId::new(), policy)
            .unwrap()
            .unwrap();
        assert_eq!(cavity.solid_count().unwrap(), 1);
        assert!((cavity.volume_mm3().unwrap() - 1140.).abs() < 1e-7);
        assert_eq!(
            cavity.rust_occupied_overlap_volume(&probe, policy).unwrap(),
            0.
        );
        let temporary = tempfile::tempdir().unwrap();
        let path = temporary.path().join("cavity.step");
        cavity.write_step(&path).unwrap();
        let source = std::fs::read_to_string(&path).unwrap();
        let decoded = step_read::read_analytic_compound_with_helical_cuts(&source, 1e-7).unwrap();
        assert_eq!(decoded.len(), 1);
        assert!(cavity.step_readback_matches(
            &cavity.topology_snapshot(EntityId::new(), 0).unwrap(),
            &decoded,
            2e-7
        ));
        let imported = Solid::import_rust_step(&source, EntityId::new(), policy).unwrap();
        assert!((imported.volume_mm3().unwrap() - 1140.).abs() < 1e-7);
        assert_eq!(
            imported
                .rust_occupied_overlap_volume(&probe, policy)
                .unwrap(),
            0.
        );
        let ordinary = stock([2; 3], [140., 0., 0.]);
        let gear =
            Solid::rust_herringbone_named(specification(24), EntityId::new(), policy).unwrap();
        let compound =
            Solid::compound_named(&[&cavity, &ordinary, &gear], EntityId::new(), policy).unwrap();
        let mixed = temporary.path().join("mixed-void-gear.step");
        compound.write_step(&mixed).unwrap();
        let source = std::fs::read_to_string(&mixed).unwrap();
        assert_eq!(source.matches("BREP_WITH_VOIDS(").count(), 1);
        assert_eq!(source.matches("MANIFOLD_SOLID_BREP(").count(), 2);
        let (count, volume, bounds) = compound
            .verify_twisted_step_compound(&source, 1e-7)
            .unwrap()
            .unwrap();
        assert_eq!(count, 3);
        assert!((volume - compound.volume_mm3().unwrap()).abs() < 1e-5);
        let expected = compound.bounds().unwrap();
        assert!(
            bounds
                .into_iter()
                .flatten()
                .zip(expected.min.into_iter().chain(expected.max))
                .all(|(a, b)| (a - b).abs() < 1e-6)
        );
        let void_shell = source
            .lines()
            .find(|line| line.contains("ORIENTED_CLOSED_SHELL("))
            .unwrap();
        assert!(void_shell.contains(".F."));
        let inverted = source.replacen(void_shell, &void_shell.replacen(".F.", ".T.", 1), 1);
        assert!(
            compound
                .verify_twisted_step_compound(&inverted, 1e-7)
                .unwrap()
                .is_err()
        );
        if multicad_kernel::OCCT_LINKED {
            let native = multicad_kernel::Shape::read_step(&mixed).unwrap();
            assert!(native.valid().unwrap());
            assert_eq!(Solid::step_solids(&mixed).unwrap().len(), 3);
            assert!((native.volume().unwrap() - volume).abs() < 2e-4);
            let probe_path = temporary.path().join("probe.step");
            probe.write_step(&probe_path).unwrap();
            assert!(
                Solid::read_step(&path, policy)
                    .unwrap()
                    .boolean(
                        &Solid::read_step(&probe_path, policy).unwrap(),
                        BooleanOperation::Intersect,
                        policy
                    )
                    .unwrap()
                    .is_none()
            );
        }
    }
}

#[test]
fn helical_gears_exchange_both_hands_with_placed_bores_and_solid_caps() {
    let transform = RigidTransform {
        rotation: [[0.8, 0., 0.6], [0., 1., 0.], [-0.6, 0., 0.8]],
        translation_mm: [10., -20., 30.],
    };
    for sweep in [GearSweep::Helical, GearSweep::Chevron] {
        for hub_span in [[-2., 2.], [2., 6.], [-2., 6.], [1., 3.]] {
            for hand in [-1., 1.] {
                for bore in [0., 4.] {
                    let mut spec = specification(24);
                    spec.sweep = sweep;
                    spec.hub = Some(multicad_kernel::GearHub {
                        radius_mm: 8.,
                        axial_start_mm: hub_span[0],
                        axial_end_mm: hub_span[1],
                    });
                    spec.twist_rad_per_mm *= hand;
                    spec.bore_radius_mm = bore;
                    let feature = EntityId::new();
                    let construct = if sweep == GearSweep::Helical {
                        Solid::rust_helical_gear_named
                    } else {
                        Solid::rust_herringbone_named
                    };
                    let solid = construct(spec, feature, Default::default())
                        .unwrap()
                        .transformed(transform)
                        .unwrap();
                    solid
                        .topology_snapshot(feature, 1)
                        .unwrap()
                        .validate()
                        .unwrap();
                    let directory = tempfile::tempdir().unwrap();
                    let path = directory.path().join("placed-helical.step");
                    solid.write_step(&path).unwrap();
                    let text = std::fs::read_to_string(&path).unwrap();
                    let volume = solid.verify_herringbone_step(&text).unwrap().unwrap();
                    assert!((volume - solid.volume_mm3().unwrap()).abs() < 1e-5);
                    if multicad_kernel::OCCT_LINKED {
                        let native = Solid::involute_gear_named(spec, feature, Default::default())
                            .unwrap()
                            .transformed(transform)
                            .unwrap();
                        let imported = multicad_kernel::Shape::read_step(&path).unwrap();
                        assert!(imported.valid().unwrap());
                        assert_eq!(Solid::step_solids(&path).unwrap().len(), 1);
                        assert!((imported.volume().unwrap() - volume).abs() < 2e-4);
                        assert!((native.volume_mm3().unwrap() - volume).abs() < 2e-4);
                        let area = |solid: &Solid| {
                            solid
                                .faces()
                                .unwrap()
                                .iter()
                                .map(|face| face.area_mm2)
                                .sum::<f64>()
                        };
                        assert!((area(&solid) - area(&native)).abs() < 2e-4);
                        for (a, b) in [solid.bounds().unwrap().min, solid.bounds().unwrap().max]
                            .into_iter()
                            .zip([native.bounds().unwrap().min, native.bounds().unwrap().max])
                        {
                            assert!(a.into_iter().zip(b).all(|(a, b)| (a - b).abs() < 1e-5));
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn herringbone_world_topology_manufacturing_and_verified_standalone_step() {
    let id = EntityId::new();
    let solid = Solid::rust_herringbone_named(specification(24), id, Default::default()).unwrap();
    let transform = RigidTransform {
        rotation: [[0.8, 0., 0.6], [0., 1., 0.], [-0.6, 0., 0.8]],
        translation_mm: [10., -20., 30.],
    };
    let world = solid.transformed(transform).unwrap();
    let display = world.topology_with_display(id, 7, Some(0.005)).unwrap();
    display.topology.validate().unwrap();
    let original = solid.topology_snapshot(id, 7).unwrap();
    let mut rails = 0;
    for (a, b) in original.elements.iter().zip(&display.topology.elements) {
        assert_eq!(a.key, b.key);
        assert_eq!(a.origins, b.origins);
        assert_eq!(a.boundary, b.boundary);
        if let Some(AnalyticGeometry::Point { position_mm }) = a.geometry {
            let [x, y, z] = position_mm;
            let Some(AnalyticGeometry::Point { position_mm }) = b.geometry else {
                panic!("placed vertex lost its geometry");
            };
            let expected = [10. + 0.8 * x + 0.6 * z, -20. + y, 30. - 0.6 * x + 0.8 * z];
            assert!(
                position_mm
                    .into_iter()
                    .zip(expected)
                    .all(|(a, b)| (a - b).abs() < 1e-12)
            );
        }
        if a.key.kind == TopologyKind::Edge && a.geometry.is_none() {
            rails += 1;
            assert!(
                display
                    .edges
                    .iter()
                    .any(|edge| edge.index == a.key.ordinal && edge.points_mm.len() >= 2)
            );
        }
    }
    assert!(rails > 250);
    assert_eq!(solid.volume_mm3().unwrap(), world.volume_mm3().unwrap());
    let faces = world.faces().unwrap();
    let area = faces.iter().map(|f| f.area_mm2).sum::<f64>();
    assert!((area - 5_072.894_843_221_397).abs() < 2e-4, "{area}");
    let result = world
        .manufacturing_mesh_with_receipt(MeshOptions {
            linear_deflection_mm: 0.005,
            ..Default::default()
        })
        .unwrap();
    assert!(!result.mesh.indices.is_empty());
    assert!(
        result
            .mesh
            .face_indices
            .iter()
            .all(|&face| (face as usize) < faces.len())
    );

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("placed-herringbone.step");
    assert!(world.supports_step_export());
    world.write_step(&path).unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    let verified = world.verify_herringbone_step(&text).unwrap().unwrap();
    assert!((verified - world.volume_mm3().unwrap()).abs() < 1e-5);
    assert!(solid.verify_herringbone_step(&text).unwrap().is_err());
    if multicad_kernel::OCCT_LINKED {
        let read = multicad_kernel::Shape::read_step(&path).unwrap();
        assert!(read.valid().unwrap());
        assert_eq!(Solid::step_solids(&path).unwrap().len(), 1);
        assert!((read.volume().unwrap() - verified).abs() < 2e-4);
    }
    let compound =
        Solid::compound_named(&[&solid, &world], EntityId::new(), Default::default()).unwrap();
    let compound_path = dir.path().join("herringbone-compound.step");
    assert!(compound.supports_step_export());
    compound.write_step(&compound_path).unwrap();
    let text = std::fs::read_to_string(&compound_path).unwrap();
    let (count, volume, bounds) = compound
        .verify_twisted_step_compound(&text, 1e-7)
        .unwrap()
        .unwrap();
    assert_eq!(count, 2);
    assert!((volume - compound.volume_mm3().unwrap()).abs() < 1e-5);
    assert_eq!(
        bounds,
        [
            compound.bounds().unwrap().min,
            compound.bounds().unwrap().max
        ]
    );
    assert!(solid.verify_herringbone_step(&text).unwrap().is_err());
    if multicad_kernel::OCCT_LINKED {
        assert_eq!(Solid::step_solids(&compound_path).unwrap().len(), 2);
        let decoded = multicad_kernel::Shape::read_step(&compound_path).unwrap();
        assert!(decoded.valid().unwrap());
        assert!((decoded.volume().unwrap() - volume).abs() < 4e-4);
    }
    assert_eq!(
        project_drawing(
            &[DrawingSolid {
                solid: &world,
                hatch: None
            }],
            DrawingProjection::default()
        )
        .unwrap_err()
        .kind,
        GeometryErrorKind::Unsupported
    );
}

#[test]
fn full_axial_certificate_rejects_a_half_tooth_phase_error_and_tilted_pairs() {
    let policy = GeometryPolicy::default();
    let a = Solid::rust_herringbone_named(specification(24), EntityId::new(), policy).unwrap();
    let b = Solid::rust_herringbone_named(specification(36), EntityId::new(), policy).unwrap();
    let center = specification(24).pitch_radius_mm + specification(36).pitch_radius_mm;
    let placed = |phase: f64| {
        let (s, c) = phase.sin_cos();
        b.transformed(RigidTransform {
            rotation: [[c, -s, 0.], [s, c, 0.], [0., 0., 1.]],
            translation_mm: [center, 0., 0.],
        })
        .unwrap()
    };
    assert_eq!(
        a.certify_rust_herringbone_pair(&placed(PI + PI / 36.), policy, None)
            .unwrap(),
        ResultKind::Clear
    );
    assert_eq!(
        a.certify_rust_herringbone_pair(&placed(PI), policy, None)
            .unwrap(),
        ResultKind::CommonInterior
    );
    let tilted = b
        .transformed(RigidTransform {
            rotation: [[0.8, 0., 0.6], [0., 1., 0.], [-0.6, 0., 0.8]],
            translation_mm: [center, 0., 0.],
        })
        .unwrap();
    assert_eq!(
        a.certify_rust_herringbone_pair(&tilted, policy, None)
            .unwrap_err()
            .kind,
        GeometryErrorKind::Unsupported
    );
}
