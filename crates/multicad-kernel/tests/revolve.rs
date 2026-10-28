use multicad_core::{
    document::EntityId,
    references::{Attachment, GeometryIntent, Origin, Resolution, TopologyKind, resolve},
};
use multicad_kernel::*;
use std::f64::consts::{PI, TAU};
fn p() -> GeometryPolicy {
    Default::default()
}
fn near(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-5, "{a} != {b}");
}
fn line(a: [f64; 2], b: [f64; 2]) -> CurvedEdge2d {
    CurvedEdge2d::Line { a, b }
}
fn rect(a: [f64; 2], b: [f64; 2]) -> Vec<CurvedEdge2d> {
    vec![
        line(a, [b[0], a[1]]),
        line([b[0], a[1]], b),
        line(b, [a[0], b[1]]),
        line([a[0], b[1]], a),
    ]
}
fn axis() -> RevolutionAxis {
    RevolutionAxis {
        origin_mm: [0.; 3],
        direction: [0., 0., 1.],
    }
}
fn frame() -> PlaneFrame {
    PlaneFrame {
        origin: [0.; 3],
        u: [1., 0., 0.],
        v: [0., 0., 1.],
    }
}
fn profile() -> CurvedProfile {
    CurvedProfile {
        frame: frame(),
        outer: rect([2., 0.], [5., 6.]),
        holes: vec![rect([3., 2.], [4., 4.])],
    }
}
fn origins(profile: &CurvedProfile) -> Vec<Origin> {
    profile
        .outer
        .iter()
        .chain(profile.holes.iter().flatten())
        .map(|_| Origin {
            source: EntityId::new(),
            role: "curve".into(),
        })
        .collect()
}
fn origin(id: EntityId, role: &str) -> Origin {
    Origin {
        source: id,
        role: role.into(),
    }
}
fn check_frames(solid: &Solid, id: EntityId) -> usize {
    let snap = solid.topology_snapshot(id, 0).unwrap();
    let faces = solid.faces().unwrap();
    let mut count = 0;
    for e in snap
        .elements
        .iter()
        .filter(|e| e.key.kind == TopologyKind::Face)
    {
        assert!(!e.origins.is_empty());
        for f in &e.frames {
            count += 1;
            f.frame.validate().unwrap();
            let (plane, n) = faces
                .iter()
                .find(|f| f.index == e.key.ordinal)
                .unwrap()
                .plane
                .unwrap();
            let dot =
                |a: [f64; 3], b: [f64; 3]| a.into_iter().zip(b).map(|(a, b)| a * b).sum::<f64>();
            near(
                dot(std::array::from_fn(|i| f.frame.origin_mm[i] - plane[i]), n),
                0.,
            );
            near(dot(f.frame.u, n), 0.);
            near(dot(f.frame.v, n), 0.);
        }
    }
    count
}
#[test]
fn exact_hollow_partial_and_full_revolution_have_named_real_support_frames() {
    let profile = profile();
    let id = EntityId::new();
    let origins = origins(&profile);
    let original = profile.clone();
    for angle in [TAU, PI / 2., -PI / 2.] {
        let solid = Solid::revolve_curved_profile_named(&profile, axis(), angle, id, &origins, p())
            .unwrap();
        near(solid.volume_mm3().unwrap(), 56. * angle.abs());
        assert_eq!(profile, original);
        let snap = solid.topology_snapshot(id, 0).unwrap();
        let caps = snap
            .elements
            .iter()
            .filter(|e| {
                e.key.kind == TopologyKind::Face
                    && (e.origins.contains(&origin(id, "cap.start"))
                        || e.origins.contains(&origin(id, "cap.end")))
            })
            .count();
        assert_eq!(caps, if angle == TAU { 0 } else { 2 });
        assert_eq!(check_frames(&solid, id), if angle == TAU { 4 } else { 6 });
        let bounds = solid.bounds().unwrap();
        near(bounds.min[2], 0.);
        near(bounds.max[2], 6.);
        near(bounds.max[0], 5.);
        if angle == TAU {
            near(bounds.min[0], -5.);
            near(bounds.min[1], -5.);
            near(bounds.max[1], 5.);
        } else if angle > 0. {
            near(bounds.min[0], 0.);
            near(bounds.min[1], 0.);
            near(bounds.max[1], 5.);
        } else {
            near(bounds.min[0], 0.);
            near(bounds.min[1], -5.);
            near(bounds.max[1], 0.);
        }
    }
}
#[test]
fn true_spline_revolution_with_hole_roundtrips_step_without_planar_side_fiction() {
    let profile = CurvedProfile {
        frame: frame(),
        outer: vec![
            CurvedEdge2d::BSpline(BSpline2d {
                degree: 2,
                poles: vec![[2., 0.], [5., 4.], [8., 0.]],
                knots: vec![0., 1.],
                multiplicities: vec![3, 3],
                weights: None,
            }),
            line([8., 0.], [2., 0.]),
        ],
        holes: vec![rect([4., 0.25], [6., 0.75])],
    };
    let id = EntityId::new();
    let origins = origins(&profile);
    let solid =
        Solid::revolve_curved_profile_named(&profile, axis(), TAU, id, &origins, p()).unwrap();
    near(solid.volume_mm3().unwrap(), 70. * PI);
    let bounds = solid.bounds().unwrap();
    near(bounds.max[0], 8.);
    near(bounds.min[0], -8.);
    near(bounds.max[2], 2.);
    let snap = solid.topology_snapshot(id, 0).unwrap();
    let curved = snap
        .elements
        .iter()
        .find(|e| e.key.kind == TopologyKind::Face && e.origins.contains(&origins[0]))
        .unwrap();
    assert!(curved.frames.is_empty());
    check_frames(&solid, id);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("exact-spline-revolve.step");
    solid.write_step(&path).unwrap();
    near(
        Solid::read_step(&path, p()).unwrap().volume_mm3().unwrap(),
        70. * PI,
    );
}
#[test]
fn hemisphere_arc_can_touch_axis_without_crossing_or_creating_fake_axis_face() {
    let profile = CurvedProfile {
        frame: frame(),
        outer: vec![
            CurvedEdge2d::Arc {
                center: [0., 0.],
                a: [0., -3.],
                b: [0., 3.],
                clockwise: false,
            },
            line([0., 3.], [0., -3.]),
        ],
        holes: vec![],
    };
    let id = EntityId::new();
    let solid =
        Solid::revolve_curved_profile_named(&profile, axis(), TAU, id, &origins(&profile), p())
            .unwrap();
    near(solid.volume_mm3().unwrap(), 36. * PI);
    let snap = solid.topology_snapshot(id, 0).unwrap();
    let faces: Vec<_> = snap
        .elements
        .iter()
        .filter(|e| e.key.kind == TopologyKind::Face)
        .collect();
    assert_eq!(faces.len(), 1);
    assert!(faces[0].frames.is_empty());
}
#[test]
fn cap_reference_follows_signed_angle_and_offset_axis_after_dimension_edit() {
    let mut profile = profile();
    profile.frame.origin = [10., 20., 30.];
    let axis = RevolutionAxis {
        origin_mm: [10., 20., 30.],
        ..axis()
    };
    let id = EntityId::new();
    let curves = origins(&profile);
    let first = Solid::revolve_curved_profile_named(&profile, axis, PI / 2., id, &curves, p())
        .unwrap()
        .topology_snapshot(id, 0)
        .unwrap();
    let cap = first
        .elements
        .iter()
        .find(|e| e.origins.contains(&origin(id, "cap.end")))
        .unwrap();
    let mut intent = GeometryIntent::capture(&first, cap.key).unwrap();
    intent.attachment = Some(Attachment {
        anchor: origin(id, "cap.end"),
        offset_mm: [2., 3., 0.],
        angle_rad: 0.,
    });
    profile.outer = rect([2., 0.], [7., 6.]);
    let next = Solid::revolve_curved_profile_named(&profile, axis, -PI / 2., id, &curves, p())
        .unwrap()
        .topology_snapshot(id, 1)
        .unwrap();
    let Resolution::Resolved { frame: Some(f), .. } = resolve(&intent, &next, 1) else {
        panic!("unresolved cap")
    };
    near(f.origin_mm[0], 10.);
    near(f.origin_mm[1], 18.);
    near(f.origin_mm[2], 33.);
    near(f.u[1], -1.);
    near(f.v[2], 1.);
}
#[test]
fn invalid_revolution_inputs_reject_without_mutating_profile_or_valid_result() {
    let profile = profile();
    let before = profile.clone();
    let valid = Solid::revolve_curved_profile(&profile, axis(), TAU, p()).unwrap();
    for a in [0., f64::NAN, f64::INFINITY, TAU + 1e-9, 1e-9, TAU - 1e-9] {
        assert!(Solid::revolve_curved_profile(&profile, axis(), a, p()).is_err());
    }
    for a in [
        RevolutionAxis {
            origin_mm: [0., 1., 0.],
            ..axis()
        },
        RevolutionAxis {
            direction: [0., 1., 0.],
            ..axis()
        },
        RevolutionAxis {
            direction: [0.; 3],
            ..axis()
        },
        RevolutionAxis {
            direction: [0., 0., 2.],
            ..axis()
        },
        RevolutionAxis {
            origin_mm: [f64::NAN, 0., 0.],
            ..axis()
        },
    ] {
        assert!(Solid::revolve_curved_profile(&profile, a, TAU, p()).is_err());
    }
    let mut cross = profile.clone();
    cross.outer = rect([-2., 0.], [5., 6.]);
    cross.holes.clear();
    assert!(
        Solid::revolve_curved_profile(&cross, axis(), TAU, p())
            .err()
            .unwrap()
            .message
            .contains("crosses")
    );
    let id = EntityId::new();
    let mut duplicate = origins(&profile);
    duplicate[1] = duplicate[0].clone();
    assert!(
        Solid::revolve_curved_profile_named(&profile, axis(), TAU, id, &duplicate, p()).is_err()
    );
    assert_eq!(profile, before);
    near(valid.volume_mm3().unwrap(), 112. * PI);
}
#[test]
fn revolve_worker_exports_only_current_owner_thread_keys() {
    let profile = profile();
    let feature = EntityId::new();
    let curves = origins(&profile);
    let worker = GeometryWorker::new().unwrap();
    let program = |revision, angle_rad| GeometryProgram {
        revision,
        policy: p(),
        operations: vec![GeometryOperation::NamedRevolve {
            profile: profile.clone(),
            axis: axis(),
            angle_rad,
            feature,
            curve_origins: curves.clone(),
        }],
        outputs: vec![0],
        mesh: Default::default(),
    };
    let reply = worker.submit(program(0, PI)).unwrap().wait().unwrap();
    let solid = reply.solids[0].as_ref().unwrap();
    near(solid.volume_mm3, 56. * PI);
    assert!(!solid.topology.is_empty());
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("worker-revolve.step");
    worker
        .write_step_to_staging(solid.key, 0, path.clone())
        .unwrap()
        .wait()
        .unwrap();
    near(
        Solid::read_step(&path, p()).unwrap().volume_mm3().unwrap(),
        56. * PI,
    );
    worker.submit(program(1, TAU)).unwrap().wait().unwrap();
    assert!(
        worker
            .write_step_to_staging(solid.key, 0, dir.path().join("stale.step"))
            .is_err()
    );
}
#[test]
fn circular_section_makes_exact_torus_and_rotated_axis_uses_world_frame() {
    let torus = CurvedProfile {
        frame: frame(),
        outer: vec![CurvedEdge2d::Circle {
            center: [3., 0.],
            radius_mm: 1.,
        }],
        holes: vec![],
    };
    let id = EntityId::new();
    let solid = Solid::revolve_curved_profile_named(&torus, axis(), TAU, id, &origins(&torus), p())
        .unwrap();
    near(solid.volume_mm3().unwrap(), 6. * PI * PI);
    let snapshot = solid.topology_snapshot(id, 0).unwrap();
    assert!(
        snapshot
            .elements
            .iter()
            .filter(|e| e.key.kind == TopologyKind::Face)
            .all(|e| e.frames.is_empty() && !e.origins.is_empty())
    );
    let mut profile = profile();
    profile.frame = PlaneFrame {
        origin: [1., 2., 3.],
        u: [0., 1., 0.],
        v: [1., 0., 0.],
    };
    let axis = RevolutionAxis {
        origin_mm: [1., 2., 3.],
        direction: [1., 0., 0.],
    };
    let id = EntityId::new();
    let solid =
        Solid::revolve_curved_profile_named(&profile, axis, TAU, id, &origins(&profile), p())
            .unwrap();
    near(solid.volume_mm3().unwrap(), 112. * PI);
    check_frames(&solid, id);
    let bounds = solid.bounds().unwrap();
    for (i, (lo, hi)) in [(1., 7.), (-3., 7.), (-2., 8.)].into_iter().enumerate() {
        near(bounds.min[i], lo);
        near(bounds.max[i], hi);
    }
}
