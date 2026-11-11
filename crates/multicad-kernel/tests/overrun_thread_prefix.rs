use multicad_core::{
    commands::Session,
    document::{Document, EntityId},
    model::{ThreadFamily, ThreadFit, ThreadHand, ThreadMode, ThreadParameters, length},
    references::Origin,
};
use multicad_kernel::{
    BooleanOperation, GeometryPolicy, HelicalThread, OCCT_LINKED, PlaneFrame, RigidTransform,
    Solid,
    rust_kernel::{
        Circle, Rational,
        cylinder::AxialCylinder,
        helical_thread::{Spec, Thread},
    },
};
fn point(f: PlaneFrame, p: [f64; 3]) -> [f64; 3] {
    std::array::from_fn(|i| f.origin[i] + f.u[i] * p[0] + f.v[i] * p[1] + f.normal()[i] * p[2])
}
fn stock(frame: PlaneFrame) -> Solid {
    let id = EntityId::new();
    let n = |n| Rational::new(n, 1).unwrap();
    Solid::from_axial_cylinder_sweep(
        AxialCylinder::new(
            id.uuid_bits(),
            Circle::new([n(0); 2], n(5)).unwrap(),
            n(0),
            n(4),
        )
        .unwrap(),
        frame,
        [0.; 2],
        false,
        Origin {
            source: id,
            role: "stock.circle".into(),
        },
        GeometryPolicy::default(),
    )
    .unwrap()
}
#[test]
fn sourced_overrun_tools_keep_literal_length_and_only_thread_the_stock_prefix() {
    let policy = GeometryPolicy::default();
    let session = Session::new(Document::new("Overrun proof").unwrap()).unwrap();
    let temp = tempfile::tempdir().unwrap();
    let file = temp.path().join("prefix.step");
    for (family, fit, pitch, diameter) in [
        (ThreadFamily::IsoMetric, ThreadFit::Metric6HInternal, 1., 6.),
        (ThreadFamily::Unc, ThreadFit::Unified2B, 25.4 / 20., 6.35),
        (ThreadFamily::Unf, ThreadFit::Unified2B, 25.4 / 28., 6.35),
    ] {
        for left in [false, true] {
            let parameters = ThreadParameters {
                family,
                fit,
                pitch: length(pitch),
                major_diameter: length(diameter),
                length: length(5.2),
                hand: if left {
                    ThreadHand::Left
                } else {
                    ThreadHand::Right
                },
                ..Default::default()
            };
            let resolved = parameters
                .resolve(session.parameters(), ThreadMode::CutInternal)
                .unwrap();
            let spec = HelicalThread {
                major_radius_mm: resolved.major_radius_mm,
                pitch_mm: resolved.pitch_mm,
                length_mm: resolved.length_mm,
                depth_mm: resolved.profile.depth_mm,
                crest_width_mm: resolved.profile.crest_width_mm,
                lower_flank_deg: resolved.profile.lower_flank_deg,
                upper_flank_deg: resolved.profile.upper_flank_deg,
                radial_taper: 0.,
                starts: resolved.starts,
                left_hand: left,
            };
            let analytic = Thread::new(Spec {
                major_radius_mm: spec.major_radius_mm,
                pitch_mm: spec.pitch_mm,
                length_mm: spec.length_mm,
                depth_mm: spec.depth_mm,
                crest_width_mm: spec.crest_width_mm,
                lower_flank_deg: spec.lower_flank_deg,
                upper_flank_deg: spec.upper_flank_deg,
                starts: spec.starts,
                left_hand: left,
                root_radius_mm: None,
            })
            .unwrap();
            for frame in [
                PlaneFrame::default(),
                PlaneFrame {
                    origin: [7., -3., 2.],
                    u: [0.8, 0.6, 0.],
                    v: [-0.36, 0.48, 0.8],
                },
            ] {
                for reverse in [false, true] {
                    let blank = stock(frame);
                    let before = blank.bounds().unwrap();
                    let entry = PlaneFrame {
                        origin: point(frame, [0., 0., if reverse { 4. } else { 0. }]),
                        u: frame.u,
                        v: frame.v.map(|v| if reverse { -v } else { v }),
                    };
                    let owner = EntityId::new();
                    let cut = blank
                        .rust_cut_internal_thread(spec, 0.2, owner, entry, policy)
                        .unwrap();
                    let receipt = cut.rust_receiving_thread_intervals().unwrap();
                    assert_eq!(receipt.requested_tool_length_mm, 5.2);
                    assert_eq!(receipt.physical_thread_length_mm, 4.);
                    assert_eq!(receipt.stock_length_mm, 4.);
                    assert_eq!(parameters.length, length(5.2));
                    assert_eq!(cut.faces().unwrap().len(), 7);
                    let bounds = cut.bounds().unwrap();
                    for (a, b) in bounds
                        .min
                        .into_iter()
                        .chain(bounds.max)
                        .zip(before.min.into_iter().chain(before.max))
                    {
                        assert!((a - b).abs() < 1e-12);
                    }
                    cut.manufacturing_mesh_with_receipt(Default::default())
                        .unwrap();
                    cut.write_step(&file).unwrap();
                    let text = std::fs::read_to_string(&file).unwrap();
                    assert_eq!(text.matches("ADVANCED_FACE").count(), 7);
                    assert!(
                        (cut.verify_helical_step(&text).unwrap().unwrap()
                            - cut.volume_mm3().unwrap())
                        .abs()
                            < 1e-6
                    );
                    let imported = Solid::import_rust_step(&text, EntityId::new(), policy).unwrap();
                    let recovered = imported.rust_receiving_thread_intervals().unwrap();
                    assert!((recovered.requested_tool_length_mm - 4.).abs() < 1e-12);
                    assert!((recovered.physical_thread_length_mm - 4.).abs() < 1e-12);
                    let mut longer = spec;
                    longer.length_mm = 6.2;
                    let equivalent = blank
                        .rust_cut_internal_thread(longer, 0.2, owner, entry, policy)
                        .unwrap();
                    assert_eq!(
                        cut.topology_snapshot(owner, 0).unwrap(),
                        equivalent.topology_snapshot(owner, 0).unwrap()
                    );
                    assert_eq!(cut.volume_mm3().unwrap(), equivalent.volume_mm3().unwrap());
                    let theta = 0.71_f64;
                    let r = (analytic.root_radius_mm() + spec.major_radius_mm) / 2.;
                    let breaks = analytic.profile_breaks();
                    let phases = [(0., false), ((breaks[2] + breaks[3]) / 2., true)];
                    let native = OCCT_LINKED.then(|| Solid::read_step(&file, policy).unwrap());
                    if let Some(native) = &native {
                        assert!(native.rust_receiving_thread_intervals().is_none());
                        assert!(
                            (native.volume_mm3().unwrap() - cut.volume_mm3().unwrap()).abs() < 1e-5
                        );
                    }
                    for (phase, expected) in phases {
                        let z = phase + analytic.helical_rate() * theta + pitch;
                        let local = [r * theta.cos(), r * theta.sin(), z];
                        let p = point(entry, local);
                        assert_eq!(cut.rust_thread_contains(p).unwrap(), expected);
                        assert_eq!(imported.rust_thread_contains(p).unwrap(), expected);
                        if let Some(native) = &native {
                            let probe = Solid::box_solid([0.005; 3], policy)
                                .unwrap()
                                .transformed(RigidTransform {
                                    translation_mm: p,
                                    ..Default::default()
                                })
                                .unwrap();
                            let volume = native
                                .boolean(&probe, BooleanOperation::Intersect, policy)
                                .unwrap()
                                .map_or(0., |s| s.volume_mm3().unwrap());
                            assert!(
                                (volume / 0.005_f64.powi(3) - f64::from(expected)).abs() < 1e-3
                            );
                        }
                    }
                    assert!(
                        !cut.rust_thread_contains(point(entry, [0., 0., 3.9]))
                            .unwrap()
                    );
                    assert_eq!(blank.bounds().unwrap(), before);
                }
            }
        }
    }
}
#[test]
fn prefix_restriction_keeps_original_tool_limits_taper_refusal_and_atomic_sources() {
    let policy = GeometryPolicy::default();
    let frame = PlaneFrame::default();
    let blank = stock(frame);
    let before = blank.bounds().unwrap();
    let spec = HelicalThread {
        major_radius_mm: 3.,
        pitch_mm: 1.,
        length_mm: 5.2,
        depth_mm: 5. * 3_f64.sqrt() / 16.,
        crest_width_mm: 0.125,
        lower_flank_deg: 30.,
        upper_flank_deg: 30.,
        radial_taper: 0.,
        starts: 1,
        left_hand: false,
    };
    assert!(blank.rust_receiving_thread_intervals().is_none());
    for invalid in [
        HelicalThread {
            length_mm: 129.,
            ..spec
        },
        HelicalThread {
            radial_taper: 0.01,
            ..spec
        },
    ] {
        assert!(
            blank
                .rust_cut_internal_thread(invalid, 0.2, EntityId::new(), frame, policy)
                .is_err()
        );
        assert_eq!(blank.bounds().unwrap(), before);
    }
    let id = EntityId::new();
    let n = |n| Rational::new(n, 1).unwrap();
    let short = Solid::from_axial_cylinder_sweep(
        AxialCylinder::new(
            id.uuid_bits(),
            Circle::new([n(0); 2], n(5)).unwrap(),
            n(0),
            Rational::new(1, 2).unwrap(),
        )
        .unwrap(),
        frame,
        [0.; 2],
        false,
        Origin {
            source: id,
            role: "short.circle".into(),
        },
        policy,
    )
    .unwrap();
    let prior = short.bounds().unwrap();
    let failure = short
        .rust_cut_internal_thread(spec, 0.2, EntityId::new(), frame, policy)
        .err()
        .unwrap();
    assert_eq!(failure.kind, multicad_kernel::GeometryErrorKind::Limit);
    assert_eq!(short.bounds().unwrap(), prior);
    assert!(
        blank
            .rust_cut_internal_thread(spec, 0.2, EntityId::new(), frame, policy)
            .unwrap()
            .rust_receiving_thread_intervals()
            .is_some()
    );
}
