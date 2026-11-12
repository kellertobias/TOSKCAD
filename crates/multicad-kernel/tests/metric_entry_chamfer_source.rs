//! Independent native material oracle for the pure entry-treatment descriptor.
#![cfg(feature = "occt")]
use multicad_core::{
    commands::Session,
    document::{Document, EntityId},
    model::{ThreadFit, ThreadHand, ThreadMode, ThreadParameters, length},
    references::Origin,
};
use multicad_kernel::{
    BooleanOperation, CurvedEdge2d, CurvedProfile, GeometryPolicy, HelicalThread, PlaneFrame,
    RevolutionAxis, RigidTransform, RootRoundedHelicalThread, Solid,
    rust_kernel::{
        helical_thread::{Spec, Thread},
        thread_end_relief::Intent,
        thread_entry_chamfer::{Chamfer, QueryOptions},
    },
};
#[test]
fn metric_cone_clipping_matches_native_material_at_shallow_root_contact_and_deep_entries() {
    let session = Session::new(Document::new("Conical source characterization").unwrap()).unwrap();
    let policy = GeometryPolicy::default();
    for (diameter, pitch, left, factor, terminal, posed) in [
        (3., 0.5, false, 0.1, false, false),
        (3., 0.5, true, 1., true, false),
        (6., 1., false, 0.9, true, true),
        (6., 1., true, 1.2, false, true),
    ] {
        let resolved = ThreadParameters {
            major_diameter: length(diameter),
            pitch: length(pitch),
            length: length(1.1 * diameter),
            fit: ThreadFit::Metric6gExternal,
            hand: if left {
                ThreadHand::Left
            } else {
                ThreadHand::Right
            },
            ..Default::default()
        }
        .resolve(session.parameters(), ThreadMode::NewExternal)
        .unwrap();
        let root = resolved
            .metric_fit
            .as_ref()
            .unwrap()
            .external_root_radius_mm
            .unwrap();
        let spec = Spec {
            major_radius_mm: resolved.major_radius_mm,
            pitch_mm: resolved.pitch_mm,
            length_mm: resolved.length_mm,
            depth_mm: resolved.profile.depth_mm,
            crest_width_mm: resolved.profile.crest_width_mm,
            lower_flank_deg: resolved.profile.lower_flank_deg,
            upper_flank_deg: resolved.profile.upper_flank_deg,
            starts: 1,
            left_hand: left,
            root_radius_mm: Some(root),
        };
        let source = Thread::new(spec).unwrap();
        let c = factor * spec.depth_mm;
        let body = Chamfer::new(
            source,
            c,
            terminal.then_some(Intent {
                width_mm: 0.6 * pitch,
                radial_depth_mm: 0.1 * pitch,
            }),
        )
        .unwrap();
        let tool = Solid::root_rounded_helical_thread_named(
            RootRoundedHelicalThread {
                profile: HelicalThread {
                    major_radius_mm: spec.major_radius_mm,
                    pitch_mm: spec.pitch_mm,
                    length_mm: spec.length_mm,
                    depth_mm: spec.depth_mm,
                    crest_width_mm: spec.crest_width_mm,
                    lower_flank_deg: spec.lower_flank_deg,
                    upper_flank_deg: spec.upper_flank_deg,
                    radial_taper: 0.,
                    starts: 1,
                    left_hand: left,
                },
                root_radius_mm: root,
            },
            EntityId::new(),
            policy,
        )
        .unwrap();
        assert!(!tool.uses_rust_geometry());
        let mut points = vec![
            [0., 0.],
            [body.entry_radius_mm(), 0.],
            [spec.major_radius_mm, c],
            [spec.major_radius_mm, body.material_length_mm()],
        ];
        if let Some(tail) = body.terminal_relief() {
            points.extend([
                [tail.neck_radius_mm(), tail.material_length_mm()],
                [tail.neck_radius_mm(), tail.full_length_mm()],
            ]);
        }
        points.push([0., spec.length_mm]);
        points.dedup();
        let id = EntityId::new();
        let edges = (0..points.len())
            .map(|i| CurvedEdge2d::Line {
                a: points[i],
                b: points[(i + 1) % points.len()],
            })
            .collect::<Vec<_>>();
        let origins = (0..edges.len())
            .map(|i| Origin {
                source: id,
                role: format!("native cone mask/{i}"),
            })
            .collect::<Vec<_>>();
        let mask = Solid::revolve_curved_profile_named(
            &CurvedProfile {
                frame: PlaneFrame {
                    origin: [0.; 3],
                    u: [1., 0., 0.],
                    v: [0., 0., 1.],
                },
                outer: edges,
                holes: vec![],
            },
            RevolutionAxis {
                origin_mm: [0.; 3],
                direction: [0., 0., 1.],
            },
            std::f64::consts::TAU,
            id,
            &origins,
            policy,
        )
        .unwrap();
        assert!(!mask.uses_rust_geometry());
        let native = tool
            .boolean(&mask, BooleanOperation::Intersect, policy)
            .unwrap()
            .unwrap();
        let pose = if posed {
            RigidTransform {
                rotation: [[0.8, -0.36, 0.48], [0.6, 0.48, -0.64], [0., 0.8, 0.6]],
                translation_mm: [2., -3., 4.],
            }
        } else {
            RigidTransform::default()
        };
        let native = native.transformed(pose).unwrap();
        assert!(!native.uses_rust_geometry());
        let volume = body.volume_mm3(QueryOptions::default()).unwrap();
        let actual = native.volume_mm3().unwrap();
        assert!(
            actual >= volume.lower - actual * 1e-6 && actual <= volume.upper + actual * 1e-6,
            "{diameter} factor{factor} left{left}: {volume:?} native{actual}"
        );
        for (phase, ratio) in [
            (0., 0.5),
            (0.2 * pitch, 0.5),
            (0.5 * pitch, 0.1),
            (0.8 * pitch, 0.5),
        ] {
            let z = ratio * c;
            let theta = (z - phase) / source.helical_rate();
            let radius = body.radius(theta, z);
            for inside in [false, true] {
                let r = radius
                    + if inside {
                        -0.001 * pitch
                    } else {
                        0.001 * pitch
                    };
                let p = [r * theta.cos(), r * theta.sin(), z];
                let edge = 0.0002 * pitch;
                let probe = Solid::box_solid([edge; 3], policy)
                    .unwrap()
                    .transformed(RigidTransform {
                        translation_mm: p.map(|v| v - edge / 2.),
                        ..Default::default()
                    })
                    .unwrap()
                    .transformed(pose)
                    .unwrap();
                assert!(!probe.uses_rust_geometry());
                let occupied = native
                    .boolean(&probe, BooleanOperation::Intersect, policy)
                    .unwrap()
                    .map_or(0., |s| s.volume_mm3().unwrap());
                assert!(
                    if inside {
                        occupied > 0.99 * edge.powi(3)
                    } else {
                        occupied < 1e-4 * edge.powi(3)
                    },
                    "inside{inside} occupied{occupied}"
                );
            }
        }
    }
}
