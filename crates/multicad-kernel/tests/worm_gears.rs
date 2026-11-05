use multicad_core::{
    document::EntityId,
    references::{AnalyticGeometry, TopologyKind},
};
use multicad_kernel::{
    BRepSelection, BooleanOperation, GeneratedZaWormWheel, GeometryPolicy, RigidTransform, Solid,
    ZaWorm,
};
use std::f64::consts::{PI, TAU};

fn worm() -> ZaWorm {
    ZaWorm {
        axial_module_mm: 1.,
        threaded_length_mm: 16.,
        bore_radius_mm: 1.,
        axial_thinning_mm: 0.05,
    }
}
fn wheel(teeth: u32) -> GeneratedZaWormWheel {
    GeneratedZaWormWheel {
        teeth,
        axial_module_mm: 1.,
        face_width_mm: 2.,
        bore_radius_mm: 2.,
        pitch_thinning_mm: 0.05,
    }
}
fn close(a: f64, b: f64, budget: f64) {
    assert!((a - b).abs() <= budget, "{a} != {b}, budget {budget}");
}
fn probe(solid: &Solid, center: [f64; 3]) -> f64 {
    let side = 0.005;
    let cube = Solid::box_solid([side; 3], Default::default())
        .unwrap()
        .transformed(RigidTransform {
            translation_mm: center.map(|x| x - side / 2.),
            ..Default::default()
        })
        .unwrap();
    solid
        .boolean(&cube, BooleanOperation::Intersect, Default::default())
        .unwrap()
        .map_or(0., |s| s.volume_mm3().unwrap())
        / side.powi(3)
}
fn expected_worm_volume(s: ZaWorm) -> f64 {
    // Independent radial integration of axial trapezoid occupancy; every
    // axial slice has this area, including a fractional final turn.
    let m = s.axial_module_mm;
    let top = 6. * m;
    let root = 3.75 * m;
    let k = 20_f64.to_radians().tan() * 1.01_f64.sqrt();
    let crest = PI * m / 2. - s.axial_thinning_mm - 2. * m * k;
    let integral = (crest + 2. * k * top) * (top * top - root * root) / 2.
        - 2. * k * (top.powi(3) - root.powi(3)) / 3.;
    s.threaded_length_mm
        * (PI * root * root + TAU / (PI * m) * integral - PI * s.bore_radius_mm.powi(2))
}
#[test]
fn native_worm_has_axial_trapezoid_centered_phase_bore_and_independent_volume() {
    let policy = GeometryPolicy::default();
    for spec in [
        worm(),
        ZaWorm {
            threaded_length_mm: 17.25,
            ..worm()
        },
        ZaWorm {
            axial_module_mm: 0.5,
            threaded_length_mm: 8.,
            bore_radius_mm: 0.,
            axial_thinning_mm: 0.,
        },
    ] {
        let length = spec.threaded_length_mm;
        let m = spec.axial_module_mm;
        let id = EntityId::new();
        let native = Solid::za_worm_named(spec, id, policy).unwrap();
        assert_eq!(native.solid_count().unwrap(), 1);
        let bounds = native.bounds().unwrap();
        close(bounds.min[2], -length / 2., 2e-6);
        close(bounds.max[2], length / 2., 2e-6);
        let volume = native.volume_mm3().unwrap();
        let area: f64 = native.faces().unwrap().iter().map(|f| f.area_mm2).sum();
        close(
            volume,
            expected_worm_volume(spec),
            policy.linear_tolerance_mm * area + 1e-7 * volume,
        );
        for z in [-2.1, 0., 2.3] {
            let z = z * m;
            let angle = PI + 2. * z / m;
            assert!(probe(&native, [5.9 * m * angle.cos(), 5.9 * m * angle.sin(), z]) > 0.999);
            assert!(probe(&native, [-5.9 * m * angle.cos(), -5.9 * m * angle.sin(), z]) < 1e-5);
        }
        let axis_material = probe(&native, [0., 0., 0.]);
        assert!(if spec.bore_radius_mm > 0. {
            axis_material < 1e-5
        } else {
            axis_material > 0.999
        });
        let topology = native.topology_snapshot(id, 0).unwrap();
        assert!(topology.elements.iter().all(|e| !e.origins.is_empty()));
        assert!(
            topology
                .elements
                .iter()
                .flat_map(|e| &e.origins)
                .all(|o| !o.role.starts_with("thread"))
        );
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("za-worm.step");
        native.write_step(&file).unwrap();
        let opened = Solid::read_step(&file, policy).unwrap();
        assert_eq!(opened.solid_count().unwrap(), 1);
        close(
            opened.volume_mm3().unwrap(),
            expected_worm_volume(spec),
            2e-3,
        );
    }
}
fn rotate(p: [f64; 3], angle: f64) -> [f64; 3] {
    let (s, c) = angle.sin_cos();
    [c * p[0] - s * p[1], s * p[0] + c * p[1], p[2]]
}
fn unit(p: [f64; 3]) -> [f64; 3] {
    let length = p.iter().map(|x| x * x).sum::<f64>().sqrt();
    p.map(|x| x / length)
}
fn flank_point(teeth: u32, r: f64, v: f64, side: f64) -> ([f64; 3], [f64; 3]) {
    // Direct screw surface + relative-velocity contact, independently written
    // in physical worm coordinates rather than the native tensor evaluator.
    let rw = f64::from(teeth) / 2.;
    let h = 0.5;
    let theta = PI - (v / r).asin();
    let k = -side * 20_f64.to_radians().tan() * 1.01_f64.sqrt();
    let z = side * (PI / 2. + 0.05) / 2. + k * (r - 5.) + h * (theta - PI);
    let (sn, cs) = theta.sin_cos();
    let nx = h * sn - k * r * cs;
    let normal = [nx, -h * cs - k * r * sn, r];
    let phi = (z * nx + (rw - (r * cs + 5. + rw)) * r) / (h * nx);
    let point = rotate([r * cs + 5. + rw, h * phi - z, v], -h / rw * phi);
    let outward =
        unit(rotate([normal[0], -normal[2], normal[1]], -h / rw * phi)).map(|x| -side * x);
    (point, outward)
}
fn witness(solid: &Solid, owner: EntityId, role: &str, point: [f64; 3], outward: [f64; 3]) {
    let face = solid
        .topology_snapshot(owner, 0)
        .unwrap()
        .elements
        .into_iter()
        .find(|e| e.key.kind == TopologyKind::Face && e.origins.iter().any(|o| o.role == role))
        .unwrap_or_else(|| panic!("missing actual native face {role}"))
        .key
        .ordinal;
    let offset = 0.01;
    let target = std::array::from_fn(|i| point[i] + offset * outward[i]);
    let cube = Solid::box_solid([0.01; 3], Default::default())
        .unwrap()
        .transformed(RigidTransform {
            translation_mm: target,
            ..Default::default()
        })
        .unwrap();
    let vertex = cube.topology_snapshot(EntityId::new(), 0).unwrap().elements.into_iter().find(|e| {
        matches!(e.geometry, Some(AnalyticGeometry::Point { position_mm }) if position_mm.iter().zip(target).all(|(a,b)|(a-b).abs()<1e-10))
    }).unwrap().key.ordinal;
    let result = solid
        .minimum_distance(
            BRepSelection::Face(face),
            &cube,
            BRepSelection::Vertex(vertex),
        )
        .unwrap();
    close(result.distance_mm, offset, 2e-5);
    let best = result
        .witnesses
        .iter()
        .map(|w| {
            w.a_mm
                .iter()
                .zip(point)
                .map(|(a, b)| (a - b).powi(2))
                .sum::<f64>()
                .sqrt()
        })
        .fold(f64::INFINITY, f64::min);
    assert!(
        best < 2e-5,
        "{role}: actual native witness missed generated surface by {best}"
    );
}
#[test]
fn generated_wheel_has_measured_throat_flanks_root_caps_bore_and_native_step() {
    let id = EntityId::new();
    let native = Solid::generated_za_worm_wheel_named(wheel(32), id, Default::default()).unwrap();
    assert_eq!(native.solid_count().unwrap(), 1);
    // Independent Green-integral oracle, not a stored native result. Its
    // explicit radius-2/diameter-4 recipe and numerical refinement receipts
    // live in artifacts/worm-review-20261105/independent-wheel-volume-summary.json.
    let expected = 1563.90212973608;
    let volume = native.volume_mm3().unwrap();
    let area: f64 = native.faces().unwrap().iter().map(|f| f.area_mm2).sum();
    let uncertainty = GeometryPolicy::default().linear_tolerance_mm * area + 1e-7 * volume;
    close(volume, expected, uncertainty);
    eprintln!(
        "generated_za_wheel native_volume_mm3={volume:.17} independent_green_mm3={expected:.17} budget_mm3={uncertainty:.17}"
    );
    let bounds = native.bounds().unwrap();
    close(bounds.min[2], -1., 2e-6);
    close(bounds.max[2], 1., 2e-6);
    assert!(probe(&native, [0., 0., 0.]) < 1e-5);
    for side in [-1., 1.] {
        for v in [-0.6, 0., 0.6] {
            let (p, n) = flank_point(32, 5., v, side);
            witness(
                &native,
                id,
                if side > 0. {
                    "worm_wheel/sector/0/flank.minus"
                } else {
                    "worm_wheel/sector/0/flank.plus"
                },
                p,
                n,
            );
        }
    }
    for v in [-0.6_f64, 0., 0.6] {
        let root = 21. - (6.25_f64.powi(2) - v * v).sqrt();
        let angle = 0.5 / 16. * (v / 6.25).asin();
        let point = rotate([root, 0., v], angle);
        let normal = unit(rotate(
            [1., 0., -v / (6.25_f64.powi(2) - v * v).sqrt()],
            angle,
        ));
        witness(&native, id, "worm_wheel/sector/0/root.torus", point, normal);
    }
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("generated-za-wheel.step");
    native.write_step(&file).unwrap();
    let opened = Solid::read_step(&file, Default::default()).unwrap();
    assert_eq!(opened.solid_count().unwrap(), 1);
    close(
        opened.volume_mm3().unwrap(),
        native.volume_mm3().unwrap(),
        2e-3,
    );
    let topology = native.topology_snapshot(id, 0).unwrap();
    assert!(topology.elements.iter().all(|e| !e.origins.is_empty()));
}
#[test]
fn generated_wheel_upper_domain_has_independent_volume_and_step_readback() {
    let spec = GeneratedZaWormWheel {
        axial_module_mm: 2.,
        face_width_mm: 4.,
        bore_radius_mm: 2.,
        pitch_thinning_mm: 0.1,
        ..wheel(64)
    };
    let native =
        Solid::generated_za_worm_wheel_named(spec, EntityId::new(), Default::default()).unwrap();
    let expected = 8. * 6393.570566838108;
    let area: f64 = native.faces().unwrap().iter().map(|f| f.area_mm2).sum();
    close(
        native.volume_mm3().unwrap(),
        expected,
        1e-7 * (area + expected),
    );
    let bounds = native.bounds().unwrap();
    close(bounds.min[2], -2., 2e-6);
    close(bounds.max[2], 2., 2e-6);
    assert!(probe(&native, [65., 0., 0.]) < 1e-5);
    let a = PI / 64.;
    assert!(probe(&native, [65. * a.cos(), 65. * a.sin(), 0.]) > 0.999);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("upper-wheel.step");
    native.write_step(&path).unwrap();
    let opened = Solid::read_step(&path, Default::default()).unwrap();
    assert_eq!(opened.solid_count().unwrap(), 1);
    close(opened.volume_mm3().unwrap(), expected, 0.02);
}
#[test]
fn conjugate_pair_phase_clears_and_half_tooth_wrong_phase_interferes() {
    let policy = GeometryPolicy::default();
    let worm_id = EntityId::new();
    let wheel_id = EntityId::new();
    let worm = Solid::za_worm_named(worm(), worm_id, policy).unwrap();
    let wheel = Solid::generated_za_worm_wheel_named(wheel(32), wheel_id, policy).unwrap();
    for drive in [0., TAU * 0.37] {
        let (s, c) = drive.sin_cos();
        let driven = worm
            .transformed(RigidTransform {
                rotation: [[c, -s, 0.], [s, c, 0.], [0., 0., 1.]],
                ..Default::default()
            })
            .unwrap();
        for wrong in [false, true] {
            let angle = drive / 32. + if wrong { PI / 32. } else { 0. };
            let (s, c) = angle.sin_cos();
            let placed = wheel
                .transformed(RigidTransform {
                    rotation: [[c, -s, 0.], [0., 0., 1.], [-s, -c, 0.]],
                    translation_mm: [-21., 0., 0.],
                })
                .unwrap();
            let before = [&driven, &placed].map(|s| (s.bounds().unwrap(), s.volume_mm3().unwrap()));
            let ancestry = [
                driven.topology_snapshot(worm_id, 0).unwrap(),
                placed.topology_snapshot(wheel_id, 0).unwrap(),
            ];
            let mut serial_volume = None;
            for parallel in [false, true] {
                let started = std::time::Instant::now();
                let intersection = if parallel {
                    driven.intersect_parallel(&placed, policy)
                } else {
                    driven.boolean(&placed, BooleanOperation::Intersect, policy)
                }
                .unwrap();
                eprintln!(
                    "worm_intersection parallel={parallel} wrong={wrong} drive={drive} elapsed_ms={}",
                    started.elapsed().as_secs_f64() * 1000.
                );
                if wrong {
                    let overlap = intersection
                        .expect("wrong phase needs a real interference solid")
                        .volume_mm3()
                        .unwrap();
                    assert!(overlap > 0.01, "wrong-phase overlap {overlap}");
                    if let Some(expected) = serial_volume {
                        close(overlap, expected, 1e-6);
                    } else {
                        serial_volume = Some(overlap);
                    }
                } else {
                    assert!(
                        intersection.is_none(),
                        "correct phase must have no intersection solid"
                    );
                }
            }
            assert_eq!(
                before,
                [&driven, &placed].map(|s| (s.bounds().unwrap(), s.volume_mm3().unwrap()))
            );
            assert_eq!(
                ancestry,
                [
                    driven.topology_snapshot(worm_id, 0).unwrap(),
                    placed.topology_snapshot(wheel_id, 0).unwrap()
                ]
            );
        }
    }
}
#[test]
fn parallel_intersection_keeps_independent_native_owners_on_their_threads() {
    let workers: Vec<_> = (0..2)
        .map(|_| {
            std::thread::spawn(|| {
                let policy = GeometryPolicy::default();
                let a = Solid::za_worm_named(worm(), EntityId::new(), policy).unwrap();
                let b = Solid::generated_za_worm_wheel_named(wheel(32), EntityId::new(), policy)
                    .unwrap()
                    .transformed(RigidTransform {
                        rotation: [[1., 0., 0.], [0., 0., 1.], [0., -1., 0.]],
                        translation_mm: [-21., 0., 0.],
                    })
                    .unwrap();
                for _ in 0..2 {
                    assert!(a.intersect_parallel(&b, policy).unwrap().is_none());
                }
            })
        })
        .collect();
    for worker in workers {
        worker.join().unwrap();
    }
}
#[test]
fn unsupported_worm_native_domains_are_refused_before_construction() {
    let p = GeometryPolicy::default();
    for bad in [
        ZaWorm {
            axial_module_mm: 0.49,
            ..worm()
        },
        ZaWorm {
            threaded_length_mm: 15.9,
            ..worm()
        },
        ZaWorm {
            bore_radius_mm: 4.,
            ..worm()
        },
        ZaWorm {
            axial_thinning_mm: 0.11,
            ..worm()
        },
    ] {
        assert!(Solid::za_worm_named(bad, EntityId::new(), p).is_err());
    }
    for bad in [
        GeneratedZaWormWheel {
            teeth: 31,
            ..wheel(32)
        },
        GeneratedZaWormWheel {
            face_width_mm: 2.01,
            ..wheel(32)
        },
        GeneratedZaWormWheel {
            bore_radius_mm: 15.,
            ..wheel(32)
        },
        GeneratedZaWormWheel {
            pitch_thinning_mm: f64::NAN,
            ..wheel(32)
        },
    ] {
        assert!(Solid::generated_za_worm_wheel_named(bad, EntityId::new(), p).is_err());
    }
}
