use multicad_core::document::EntityId;
use multicad_kernel::{
    GeometryErrorKind, GeometryPolicy, HelicalThread, PlaneFrame, Solid,
    rust_kernel::{Circle, Rational, cylinder::AxialCylinder},
};
fn request() -> HelicalThread {
    HelicalThread {
        major_radius_mm: 10_000.,
        pitch_mm: 0.001,
        length_mm: 0.01,
        depth_mm: 0.001 * 5. * 3_f64.sqrt() / 16.,
        crest_width_mm: 0.001 / 8.,
        lower_flank_deg: 30.,
        upper_flank_deg: 30.,
        radial_taper: 0.,
        starts: 1,
        left_hand: false,
    }
}
#[test]
fn internal_receiving_clocking_certifies_radial_tool_extent_not_only_its_short_axis() {
    let policy = GeometryPolicy::default();
    let feature = EntityId::new();
    let n = |v| Rational::new(v, 1).unwrap();
    let stock = Solid::from_axial_cylinder(
        AxialCylinder::new(
            feature.uuid_bits(),
            Circle::new([n(0); 2], n(10_001)).unwrap(),
            n(0),
            n(1),
        )
        .unwrap(),
        PlaneFrame::default(),
        policy,
    )
    .unwrap();
    let before = stock.topology_snapshot(feature, 0).unwrap();
    for (angle, admitted) in [(1e-6_f64, false), (1e-7, false), (1e-14, true), (0., true)] {
        let frame = PlaneFrame {
            u: [angle.cos(), 0., angle.sin()],
            ..Default::default()
        };
        assert!(
            Solid::rust_helical_thread(request(), None, EntityId::new(), frame, policy).is_ok()
        );
        let result = stock.rust_cut_internal_thread(request(), 1., EntityId::new(), frame, policy);
        if admitted {
            let shape = result.unwrap();
            assert_eq!(
                shape
                    .rust_receiving_thread_intervals()
                    .unwrap()
                    .requested_tool_length_mm,
                0.01
            );
        } else {
            assert_eq!(
                result.err().expect("uncertified tilt refused").kind,
                GeometryErrorKind::Unsupported
            );
        }
        assert_eq!(stock.topology_snapshot(feature, 0).unwrap(), before);
    }
}
