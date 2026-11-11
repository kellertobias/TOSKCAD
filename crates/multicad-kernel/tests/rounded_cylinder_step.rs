#![cfg(feature = "occt")]
use multicad_core::{
    document::EntityId,
    references::{AnalyticGeometry, Origin, TopologyKind},
};
use multicad_kernel::{
    rust_kernel::{rounded_cylinder::RoundedCylinder, step, step_read},
    *,
};

fn close(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-6, "{a} != {b}");
}
fn check(solid: &Solid, geometry: &RoundedCylinder, frame: PlaneFrame) {
    close(solid.volume_mm3().unwrap(), geometry.volume_mm3());
    close(
        solid
            .faces()
            .unwrap()
            .iter()
            .map(|face| face.area_mm2)
            .sum(),
        geometry.surface_area_mm2(),
    );
    assert_eq!(solid.solid_count().unwrap(), 1);
    let bounds = solid.bounds().unwrap();
    let normal = frame.normal();
    for (i, axis) in normal.into_iter().enumerate() {
        let direction = [frame.u[i], frame.v[i], axis];
        close(
            bounds.min[i],
            frame.origin[i] - geometry.support(direction.map(|v| -v)),
        );
        close(bounds.max[i], frame.origin[i] + geometry.support(direction));
    }
}

#[test]
fn independent_native_step_and_native_fillets_match_exact_quarter_toroidal_material() {
    let policy = GeometryPolicy::default();
    for radii in [
        [Some(1.), None],
        [None, Some(3.)],
        [Some(1.), Some(3.)],
        [Some(4.9), Some(4.8)],
    ] {
        let geometry = RoundedCylinder::new(5., 12., radii, 1e-6, 1e7).unwrap();
        for frame in [
            PlaneFrame::default(),
            PlaneFrame {
                origin: [17., -8., 13.],
                u: [0.8, 0.6, 0.],
                v: [-0.36, 0.48, 0.8],
            },
            PlaneFrame {
                origin: [2., 3., 4.],
                u: [1., 0., 0.],
                v: [0., -1., 0.],
            },
        ] {
            let text =
                step::write_rounded_cylinder(&geometry, frame.origin, frame.u, frame.v, 1e-7)
                    .unwrap();
            let directory = tempfile::tempdir().unwrap();
            let path = directory.path().join("rounded-cylinder.step");
            std::fs::write(&path, &text).unwrap();
            let native = Shape::read_step(&path).unwrap();
            assert!(native.valid().unwrap());
            close(native.volume().unwrap(), geometry.volume_mm3());
            check(&Solid::read_step(&path, policy).unwrap(), &geometry, frame);
            let (read, body) = step_read::read_rounded_cylinder(&text, 1e-7).unwrap();
            close(read.volume_mm3, geometry.volume_mm3());
            assert_eq!(body.geometry.fillet_radii_mm(), radii);
            // Independent OCCT construction uses its actual Fillet operation,
            // without reusing any of the new STEP geometry or reader proof.
            let mut constructed = Solid::cylinder(frame, 5., 12., policy).unwrap();
            for (i, f) in radii.into_iter().enumerate() {
                if let Some(radius_mm) = f {
                    let feature = EntityId::new();
                    let snapshot = constructed.topology_snapshot(feature, 0).unwrap();
                    let center = std::array::from_fn::<_, 3, _>(|axis| {
                        frame.origin[axis]
                            + if i == 0 {
                                0.
                            } else {
                                12. * frame.normal()[axis]
                            }
                    });
                    let key=snapshot.elements.iter().find(|element|element.key.kind==TopologyKind::Edge && matches!(element.geometry,Some(AnalyticGeometry::CircularArc {frame:circle,radius_mm,..}) if (radius_mm-5.).abs()<1e-6 && circle.origin_mm.into_iter().zip(center).all(|(a,b)|(a-b).abs()<1e-6))).unwrap().key;
                    constructed = constructed
                        .treat_edges_named(
                            &[TreatmentEdge {
                                key,
                                origin: Origin {
                                    source: feature,
                                    role: "fillet".into(),
                                },
                            }],
                            EdgeTreatment::Fillet { radius_mm },
                            feature,
                            policy,
                        )
                        .unwrap();
                }
            }
            check(&constructed, &geometry, frame);
        }
    }
}

#[test]
fn native_step_preserves_small_features_and_large_quarter_torus_radius_ratios() {
    for (radius, height, fillets) in [
        (2e-6, 2e-6, [Some(1e-6), None]),
        (1e6, 1e6, [Some(1e6 - 2e-6), None]),
    ] {
        let geometry = RoundedCylinder::new(radius, height, fillets, 1e-6, 1e7).unwrap();
        let text =
            step::write_rounded_cylinder(&geometry, [0.; 3], [1., 0., 0.], [0., 1., 0.], 1e-7)
                .unwrap();
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("numeric-seams.step");
        std::fs::write(&path, text).unwrap();
        let shape = Shape::read_step(&path).unwrap();
        assert!(shape.valid().unwrap());
        let solid = Solid::read_step(&path, GeometryPolicy::default()).unwrap();
        assert_eq!(solid.solid_count().unwrap(), 1);
        let volume = shape.volume().unwrap();
        let area = solid
            .faces()
            .unwrap()
            .iter()
            .map(|face| face.area_mm2)
            .sum::<f64>();
        assert!(
            (volume - geometry.volume_mm3()).abs() <= geometry.volume_mm3() * 1e-8,
            "volume {volume} != {}",
            geometry.volume_mm3()
        );
        assert!(
            (area - geometry.surface_area_mm2()).abs() <= geometry.surface_area_mm2() * 1e-8,
            "area {area} != {}",
            geometry.surface_area_mm2()
        );
        let bounds = solid.bounds().unwrap();
        for (actual, expected) in bounds
            .min
            .into_iter()
            .chain(bounds.max)
            .zip([-radius, -radius, 0., radius, radius, height])
        {
            assert!((actual - expected).abs() <= 1e-6 + expected.abs() * 1e-12);
        }
    }
}
