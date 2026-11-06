use multicad_core::{
    document::EntityId,
    references::{AnalyticGeometry, TopologyKind},
};
use multicad_kernel::{
    BooleanOperation, GearSweep, GeometryPolicy, InvoluteGear, RigidTransform, Solid,
};
use std::f64::consts::PI;

fn ring(teeth: u32, thinning: f64) -> InvoluteGear {
    let a = 20_f64.to_radians();
    let pitch = f64::from(teeth) / 2.;
    InvoluteGear {
        hub: None,
        teeth,
        pitch_radius_mm: pitch,
        base_radius_mm: pitch * a.cos(),
        tip_radius_mm: pitch - 1.,
        root_radius_mm: pitch + 1.25,
        root_fillet_radius_mm: 0.2,
        half_base_angle_rad: (PI / 2. - thinning) / (2. * pitch) - a.tan() + a,
        face_width_mm: 2.,
        bore_radius_mm: 0.,
        twist_rad_per_mm: 0.,
        sweep: GearSweep::InternalSpur {
            outer_radius_mm: pitch + 4.,
        },
    }
}
fn probe(s: &Solid, r: f64, a: f64) -> f64 {
    let size = 0.002;
    let cube = Solid::box_solid([size; 3], GeometryPolicy::default())
        .unwrap()
        .transformed(RigidTransform {
            translation_mm: [
                r * a.cos() - size / 2.,
                r * a.sin() - size / 2.,
                0.91 - size / 2.,
            ],
            ..Default::default()
        })
        .unwrap();
    s.boolean(
        &cube,
        BooleanOperation::Intersect,
        GeometryPolicy::default(),
    )
    .unwrap()
    .map_or(0., |v| v.volume_mm3().unwrap() / size.powi(3))
}
#[test]
fn inward_involutes_connected_rim_root_circles_and_step_are_actual_native_geometry() {
    for teeth in [48, 60] {
        let spec = ring(teeth, 0.05);
        let feature = EntityId::new();
        let solid = Solid::involute_gear_named(spec, feature, GeometryPolicy::default()).unwrap();
        assert_eq!(solid.solid_count().unwrap(), 1);
        // Independent radial integration of the material-width equation,
        // split at the circular-fillet foot (64/128-point quadrature agreed).
        let band_area = if teeth == 48 {
            181.940372898279
        } else {
            225.536388250187
        };
        let expected = (band_area
            + PI * ((spec.pitch_radius_mm + 4.).powi(2) - spec.root_radius_mm.powi(2)))
            * spec.face_width_mm;
        assert!(
            (solid.volume_mm3().unwrap() - expected).abs() < 1e-4,
            "native area/volume diverged: {} vs {expected}",
            solid.volume_mm3().unwrap()
        );
        for r in [
            spec.tip_radius_mm + 0.1,
            spec.pitch_radius_mm,
            spec.root_radius_mm - 0.5,
        ] {
            // Independently reconstruct the material width from pitch thickness.
            let alpha = (spec.base_radius_mm / r).acos();
            let reference = 20_f64.to_radians();
            let half = (PI / 2. - 0.05) / (2. * spec.pitch_radius_mm) + alpha.tan()
                - alpha
                - reference.tan()
                + reference;
            for sign in [-1., 1.] {
                assert!(probe(&solid, r, sign * (half - 0.002)) > 0.999);
                assert!(probe(&solid, r, sign * (half + 0.002)) < 1e-5);
            }
        }
        assert!(probe(&solid, spec.tip_radius_mm - 0.1, 0.) < 1e-5);
        assert!(probe(&solid, spec.root_radius_mm + 0.1, PI / f64::from(teeth)) > 0.999);
        let topology = solid.topology_snapshot(feature, 0).unwrap();
        let faces = solid.faces().unwrap();
        for tooth in 0..teeth {
            let phase = 2. * PI * f64::from(tooth) / f64::from(teeth);
            for side in [
                "tip",
                "involute.left",
                "involute.right",
                "fillet.left",
                "fillet.right",
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
                if side == "tip" {
                    assert!(
                        offset.abs() < 1e-9,
                        "{role} is not centered on its material tooth"
                    );
                } else if side.ends_with("left") {
                    assert!(
                        offset < 0. && offset > -PI / f64::from(teeth),
                        "{role} wrong material side"
                    );
                } else {
                    assert!(
                        offset > 0. && offset < PI / f64::from(teeth),
                        "{role} wrong material side"
                    );
                }
            }
        }
        assert!(
            topology
                .elements
                .iter()
                .filter(|e| e.key.kind == TopologyKind::Face)
                .all(|e| !e.origins.is_empty())
        );
        for expected in [
            0.2,
            spec.tip_radius_mm,
            spec.root_radius_mm,
            spec.pitch_radius_mm + 4.,
        ] {
            assert!(topology.elements.iter().any(|e| matches!(e.geometry,Some(AnalyticGeometry::CircularArc{radius_mm,..}) if (radius_mm-expected).abs()<1e-6)),"circle {expected}");
        }
        let bounds = solid.bounds().unwrap();
        assert!((bounds.max[0] - (spec.pitch_radius_mm + 4.)).abs() < 1e-6);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ring.step");
        solid.write_step(&path).unwrap();
        let read = Solid::read_step(&path, GeometryPolicy::default()).unwrap();
        assert_eq!(read.solid_count().unwrap(), 1);
        assert!((solid.volume_mm3().unwrap() - read.volume_mm3().unwrap()).abs() < 1e-4);
        assert!((read.volume_mm3().unwrap() - expected).abs() < 1e-4);
    }
}

#[test]
fn invalid_internal_radii_fillets_rim_and_nonfinite_inputs_are_refused() {
    let valid = ring(48, 0.05);
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
            half_base_angle_rad: 1.,
            ..valid
        },
    ] {
        assert!(
            Solid::involute_gear_named(bad, EntityId::new(), GeometryPolicy::default()).is_err()
        );
    }
}
