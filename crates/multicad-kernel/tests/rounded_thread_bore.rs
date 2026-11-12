use multicad_core::{
    commands::Session,
    document::Document,
    model::{self, ThreadFamily, ThreadFit, ThreadHand, ThreadMode, ThreadParameters, length},
};
use multicad_kernel::rust_kernel::{
    display::Tessellation,
    rounded_thread::{Spec, Thread, ThroughBore},
    rounded_thread_mesh,
};
fn canonical(family: ThreadFamily, left: bool) -> ThroughBore {
    let mut p = if family == ThreadFamily::Round {
        model::round_thread_presets()[1].parameters()
    } else {
        let row = model::whitworth_thread_presets()
            .into_iter()
            .find(|p| p.family == family)
            .unwrap();
        ThreadParameters {
            family,
            major_diameter: length(row.diameter_mm),
            pitch: length(row.pitch_mm),
            fit: if family == ThreadFamily::Bspp {
                ThreadFit::BsppInternal
            } else {
                ThreadFit::WhitworthMediumInternal
            },
            ..Default::default()
        }
    };
    p.hand = if left {
        ThreadHand::Left
    } else {
        ThreadHand::Right
    };
    let s = Session::new(Document::new("Literal rounded receiver").unwrap()).unwrap();
    let r = p.resolve(s.parameters(), ThreadMode::CutInternal).unwrap();
    p.length = length(2.37 * r.pitch_mm);
    let r = p.resolve(s.parameters(), ThreadMode::CutInternal).unwrap();
    let (crest, root) = if let Some(round) = r.round {
        (round.cutter_crest_radius_mm, round.cutter_root_radius_mm)
    } else {
        let q = r.profile.rounding_radius_mm.unwrap();
        (q, q)
    };
    let t = Thread::new(Spec {
        major_radius_mm: r.major_radius_mm,
        pitch_mm: r.pitch_mm,
        length_mm: r.length_mm,
        depth_mm: r.profile.depth_mm,
        crest_radius_mm: crest,
        root_radius_mm: root,
        flank_angle_deg: r.profile.lower_flank_deg,
        starts: r.starts,
        left_hand: left,
    })
    .unwrap();
    ThroughBore::new(t, r.major_radius_mm + 2.).unwrap()
}
#[test]
fn canonical_rounded_receiver_keeps_literal_internal_cutter_and_closed_mesh() {
    for family in [
        ThreadFamily::Bsw,
        ThreadFamily::Bsf,
        ThreadFamily::Bspp,
        ThreadFamily::Round,
    ] {
        for left in [false, true] {
            let bore = canonical(family, left);
            let t = bore.thread();
            let s = t.spec();
            assert_eq!(s.left_hand, left);
            assert!(bore.volume_mm3() > 0.);
            let mesh = rounded_thread_mesh::tessellate_bore(
                &bore,
                Tessellation {
                    deflection: 0.05,
                    angle: 0.3,
                    triangle_limit: 500_000,
                },
            )
            .unwrap();
            assert_eq!(
                mesh.face_indices
                    .iter()
                    .copied()
                    .collect::<std::collections::BTreeSet<_>>(),
                (0..8).collect()
            );
            if family == ThreadFamily::Round {
                assert_ne!(s.crest_radius_mm, s.root_radius_mm);
            }
            #[cfg(feature = "occt")]
            native_reference(family, &bore);
        }
    }
}
#[cfg(feature = "occt")]
fn native_reference(family: ThreadFamily, bore: &ThroughBore) {
    use multicad_core::document::EntityId;
    use multicad_kernel::{
        BooleanOperation, GeometryPolicy, KnuckleHelicalThread, PlaneFrame, RigidTransform,
        RoundedHelicalThread, Solid,
    };
    let policy = GeometryPolicy::default();
    let t = bore.thread();
    let s = t.spec();
    println!("Native rounded bore {family:?} left={}", s.left_hand);
    let tool = if family == ThreadFamily::Round {
        Solid::knuckle_helical_thread_named(
            KnuckleHelicalThread {
                major_radius_mm: s.major_radius_mm,
                pitch_mm: s.pitch_mm,
                length_mm: s.length_mm,
                crest_radius_mm: s.crest_radius_mm,
                root_radius_mm: s.root_radius_mm,
                left_hand: s.left_hand,
            },
            EntityId::new(),
            policy,
        )
        .unwrap()
    } else {
        Solid::rounded_helical_thread_named(
            RoundedHelicalThread {
                major_radius_mm: s.major_radius_mm,
                pitch_mm: s.pitch_mm,
                length_mm: s.length_mm,
                depth_mm: s.depth_mm,
                rounding_radius_mm: s.crest_radius_mm,
                flank_angle_deg: s.flank_angle_deg,
                radial_taper: 0.,
                starts: 1,
                left_hand: s.left_hand,
            },
            EntityId::new(),
            policy,
        )
        .unwrap()
    };
    let stock = Solid::cylinder(
        PlaneFrame::default(),
        bore.radius_mm(),
        bore.length_mm(),
        policy,
    )
    .unwrap();
    assert!(!stock.uses_rust_geometry());
    let native = stock
        .boolean(&tool, BooleanOperation::Subtract, policy)
        .unwrap()
        .unwrap();
    assert!(!native.uses_rust_geometry());
    assert_eq!(native.solid_count().unwrap(), 1);
    let area = native
        .faces()
        .unwrap()
        .iter()
        .map(|f| f.area_mm2)
        .sum::<f64>();
    let budget = policy.linear_tolerance_mm * area + 1e-10 * bore.volume_mm3();
    assert!(
        (native.volume_mm3().unwrap() - bore.volume_mm3()).abs() <= budget,
        "{family:?}: native {} descriptor {} budget {budget}",
        native.volume_mm3().unwrap(),
        bore.volume_mm3()
    );
    for phase in [
        t.profile_breaks()[1] / 2.,
        t.profile_breaks()[1].midpoint(t.profile_breaks()[2]),
        s.pitch_mm / 2.,
        t.profile_breaks()[3].midpoint(t.profile_breaks()[4]),
    ] {
        let theta = 0.8_f64;
        let z = phase + s.pitch_mm + t.helical_rate() * theta;
        let radius = t.radius_at_phase(phase);
        for delta in [-0.001, 0.001] {
            let point = [
                (radius + delta) * theta.cos(),
                (radius + delta) * theta.sin(),
                z,
            ];
            assert_eq!(bore.contains(point, 0.), delta > 0.);
            let edge = 0.00004;
            let probe = Solid::box_solid([edge; 3], policy)
                .unwrap()
                .transformed(RigidTransform {
                    translation_mm: point.map(|v| v - edge / 2.),
                    ..Default::default()
                })
                .unwrap();
            let occupied = native
                .boolean(&probe, BooleanOperation::Intersect, policy)
                .unwrap()
                .map_or(0., |s| s.volume_mm3().unwrap())
                / edge.powi(3);
            assert!(
                (occupied - f64::from(delta > 0.)).abs() < 0.01,
                "{family:?} phase {phase} delta {delta} occupied {occupied}"
            );
        }
    }
}
