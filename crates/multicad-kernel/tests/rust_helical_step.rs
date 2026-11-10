#![cfg(feature = "occt")]

use multicad_kernel::{
    GeometryPolicy, Solid,
    rust_kernel::{
        helical_thread::{Spec, Thread},
        step::write_helical_thread,
        step_read::verify_helical_thread,
    },
};
use std::f64::consts::PI;

#[test]
fn occt_reads_bounded_spline_threads_as_real_solids() {
    let scratch = tempfile::tempdir().unwrap();
    let angle = 0.3_f64;
    let u = [angle.cos(), 0., angle.sin()];
    let v = [0., 1., 0.];
    let origin = [19., -7., 17.];
    let height = 3.7;
    for (i, (left_hand, stock_radius)) in [(false, None), (true, Some(4.5))].into_iter().enumerate()
    {
        let thread = Thread::new(Spec {
            major_radius_mm: 3.,
            pitch_mm: 1.,
            length_mm: height,
            depth_mm: 5. * 3_f64.sqrt() / 16.,
            crest_width_mm: 0.125,
            lower_flank_deg: 30.,
            upper_flank_deg: 30.,
            starts: 1,
            left_hand,
            root_radius_mm: None,
        })
        .unwrap();
        let encoded = write_helical_thread(&thread, stock_radius, origin, u, v, 1e-7).unwrap();
        let measured =
            verify_helical_thread(&encoded, &thread, stock_radius, origin, u, v, 1e-7).unwrap();
        let path = scratch.path().join(format!("thread-{i}.step"));
        std::fs::write(&path, encoded).unwrap();
        let read = Solid::read_step(&path, GeometryPolicy::default()).unwrap();
        assert_eq!(read.solid_count().unwrap(), 1);
        let actual = read.volume_mm3().unwrap();
        let expected = stock_radius.map_or(thread.volume_mm3(), |radius| {
            PI * radius * radius * height - thread.volume_mm3()
        });
        assert!((actual - expected).abs() < 1e-5, "{actual} != {expected}");
        assert!((actual - measured).abs() < 1e-5, "{actual} != {measured}");
        assert_ne!(measured, expected);
    }
}

#[test]
fn occt_reads_placed_headed_screw_as_one_connected_solid() {
    use multicad_kernel::rust_kernel::{
        headed_screw::{Screw, ScrewHead, ScrewSpec},
        step::write_helical_screw,
        step_read::verify_helical_screw,
    };
    let thread = Thread::new(Spec {
        major_radius_mm: 3.,
        pitch_mm: 1.,
        length_mm: 8.,
        depth_mm: 5. * 3_f64.sqrt() / 16.,
        crest_width_mm: 0.125,
        lower_flank_deg: 30.,
        upper_flank_deg: 30.,
        starts: 1,
        left_hand: true,
        root_radius_mm: None,
    })
    .unwrap();
    let screw = Screw::new(
        thread,
        ScrewSpec {
            head: ScrewHead::ExternalHex {
                across_flats_mm: 10.,
                height_mm: 4.,
            },
            shank_length_mm: 0.,
            shank_radius_mm: 3.,
        },
    )
    .unwrap();
    let scratch = tempfile::tempdir().unwrap();
    let path = scratch.path().join("tilted-m6-hex.step");
    let angle = 0.3_f64;
    let origin = [19., -7., 17.];
    let u = [angle.cos(), 0., angle.sin()];
    let v = [0., 1., 0.];
    let encoded = write_helical_screw(&screw, origin, u, v, 1e-7).unwrap();
    let measured = verify_helical_screw(&encoded, &screw, origin, u, v, 1e-7).unwrap();
    std::fs::write(&path, encoded).unwrap();
    assert!(
        multicad_kernel::rust_kernel::step_read::read_analytic_solid(
            &std::fs::read_to_string(&path).unwrap(),
            1e-7
        )
        .is_err()
    );
    // The legacy Solid wrapper also runs its exact trimmed-bounds solver.
    // Test the independently decoded OCCT B-rep directly for this oblique case.
    let read = multicad_kernel::Shape::read_step(&path).unwrap();
    assert!(read.valid().unwrap());
    assert_eq!(Solid::step_solids(&path).unwrap().len(), 1);
    let actual = read.volume().unwrap();
    assert!((actual - screw.volume_mm3()).abs() < 1e-5);
    assert!((actual - measured).abs() < 1e-5);
}

#[test]
fn occt_reads_nominal_tangent_shank_socket_screw() {
    use multicad_kernel::rust_kernel::{
        headed_screw::{Screw, ScrewHead, ScrewSpec},
        step::write_helical_screw,
        step_read::verify_helical_screw,
    };
    let scratch = tempfile::tempdir().unwrap();
    for left_hand in [false, true] {
        let thread = Thread::new(Spec {
            major_radius_mm: 3.,
            pitch_mm: 1.,
            length_mm: 3.7,
            depth_mm: 5. * 3_f64.sqrt() / 16.,
            crest_width_mm: 0.125,
            lower_flank_deg: 30.,
            upper_flank_deg: 30.,
            starts: 1,
            left_hand,
            root_radius_mm: None,
        })
        .unwrap();
        let screw = Screw::new(
            thread,
            ScrewSpec {
                head: ScrewHead::SocketCap {
                    radius_mm: 5.,
                    height_mm: 6.,
                    socket_across_flats_mm: 5.,
                    socket_depth_mm: 3.,
                },
                shank_length_mm: 2.,
                shank_radius_mm: 3.,
            },
        )
        .unwrap();
        let step = write_helical_screw(&screw, [0.; 3], [1., 0., 0.], [0., 1., 0.], 1e-7).unwrap();
        let path = scratch.path().join(format!("tangent-{left_hand}.step"));
        let measured =
            verify_helical_screw(&step, &screw, [0.; 3], [1., 0., 0.], [0., 1., 0.], 1e-7).unwrap();
        std::fs::write(&path, step).unwrap();
        let shape = multicad_kernel::Shape::read_step(&path).unwrap();
        assert!(shape.valid().unwrap());
        assert_eq!(Solid::step_solids(&path).unwrap().len(), 1);
        let actual = shape.volume().unwrap();
        assert!((actual - measured).abs() < 1e-5);
        assert!(
            (actual - screw.volume_mm3()).abs() < 1e-5,
            "{actual} vs {}",
            screw.volume_mm3()
        );
    }
}
