use super::*;
use crate::helical_thread::Spec;
use std::f64::consts::{FRAC_PI_2, TAU};

fn thread(starts: u32, left_hand: bool) -> Thread {
    Thread::new(Spec {
        major_radius_mm: 4.,
        pitch_mm: 1.25,
        length_mm: 12.5,
        depth_mm: 1.25 * 5. * 3_f64.sqrt() / 16.,
        crest_width_mm: 1.25 / 8.,
        lower_flank_deg: 30.,
        upper_flank_deg: 30.,
        starts,
        left_hand,
        root_radius_mm: None,
    })
    .unwrap()
}

fn socket_cap() -> ScrewHead {
    ScrewHead::SocketCap {
        radius_mm: 6.5,
        height_mm: 8.,
        socket_across_flats_mm: 6.,
        socket_depth_mm: 4.,
    }
}

fn hex_head() -> ScrewHead {
    ScrewHead::ExternalHex {
        across_flats_mm: 13.,
        height_mm: 5.3,
    }
}

#[test]
fn socket_cap_preserves_a_blind_hex_recess_and_the_helical_shank() {
    let thread = thread(1, false);
    let spec = ScrewSpec {
        head: socket_cap(),
        shank_length_mm: 3.,
        shank_radius_mm: 4.,
    };
    let screw = Screw::new(thread, spec).unwrap();
    assert_eq!(*screw.thread(), thread);
    assert_eq!(screw.spec(), spec);
    assert_eq!(screw.head_top_z(), -11.);
    assert_eq!(screw.head_seat_z(), -3.);
    assert_eq!(screw.head_radius(FRAC_PI_6), 6.5);
    assert_eq!(screw.socket_radius(0.), Some(3.));
    assert!((screw.socket_radius(FRAC_PI_6).unwrap() - 2. * 3_f64.sqrt()).abs() < 1e-12);

    for material in [
        [0., 0., -7.],
        [3., 0., -10.],
        [3.3, 0., -10.],
        [0., 3.5, -10.],
        [4., 0., -2.],
        [6.4, 0., -3.],
        [3.8, 0., 1.25],
    ] {
        assert!(screw.contains(material, 0.), "{material:?} is material");
    }
    for empty in [
        [0., 0., -10.],
        [0., 0., -11.],
        [0., 0., -11.1],
        [0., 3.3, -10.],
        [4.1, 0., -2.],
        [3.8, 0., 1.875],
        [0., 0., 12.6],
    ] {
        assert!(!screw.contains(empty, 0.), "{empty:?} is empty");
    }
    assert!(screw.contains([0., 0., -7.0005], 0.001));
    assert!(screw.contains([2.9995, 0., -10.], 0.001));
    assert!(!screw.contains([0., 0., 0.], f64::NAN));
    assert!(!screw.contains([0., 0., 0.], -0.1));
    assert!(!screw.contains([0., f64::INFINITY, 0.], 0.));

    let side = 6. / 3_f64.sqrt();
    let removed = 6. * (side * 3. / 2.) * 4.;
    let expected = thread.volume_mm3() + PI * (4_f64.powi(2) * 3. + 6.5_f64.powi(2) * 8.) - removed;
    assert!((screw.volume_mm3() - expected).abs() < 1e-10);
    assert_eq!(screw.support([1., 0., 0.]), 6.5);
    assert_eq!(screw.support([0., 0., 1.]), 12.5);
    assert_eq!(screw.support([0., 0., -2.]), 22.);
    assert_eq!(screw.support([1., 0., -2.]), 28.5);
    assert_eq!(screw.support([0.; 3]), 0.);
    assert!(screw.support([0., f64::INFINITY, 1.]).is_nan());
    assert_eq!(
        screw.faces(),
        [
            ScrewFace::ThreadShoulder,
            ScrewFace::ThreadTip,
            ScrewFace::ThreadWall(0),
            ScrewFace::ThreadWall(1),
            ScrewFace::ThreadWall(2),
            ScrewFace::ThreadWall(3),
            ScrewFace::ShankWall,
            ScrewFace::HeadSeat,
            ScrewFace::HeadTop,
            ScrewFace::HeadWall(0),
            ScrewFace::SocketFloor,
            ScrewFace::SocketWall(0),
            ScrewFace::SocketWall(1),
            ScrewFace::SocketWall(2),
            ScrewFace::SocketWall(3),
            ScrewFace::SocketWall(4),
            ScrewFace::SocketWall(5),
        ]
    );
}

#[test]
fn hex_head_has_its_six_native_flats_and_no_separate_shank_when_fully_threaded() {
    let thread = thread(2, true);
    let spec = ScrewSpec {
        head: hex_head(),
        shank_length_mm: 0.,
        shank_radius_mm: 3.5,
    };
    let screw = Screw::new(thread, spec).unwrap();
    assert_eq!(screw.head_top_z(), -5.3);
    assert_eq!(screw.head_seat_z(), 0.);
    assert_eq!(screw.socket_radius(FRAC_PI_2), None);
    assert_eq!(screw.head_radius(0.), 6.5);
    assert!((screw.head_radius(FRAC_PI_2) - 13. / 3_f64.sqrt()).abs() < 1e-12);
    assert!(screw.contains([0., 7., -5.3], 0.));
    assert!(!screw.contains([7., 0., -5.3], 0.));
    assert!(screw.contains([6.5, 0., 0.], 0.));
    assert!(!screw.contains([6.5, 0., 0.01], 0.));
    assert_eq!(screw.support([1., 0., 0.]), 6.5);
    assert!((screw.support([0., 1., 0.]) - 13. / 3_f64.sqrt()).abs() < 1e-12);
    let vertices = hex_vertices(13.);
    let area = vertices
        .iter()
        .zip(vertices.iter().cycle().skip(1))
        .map(|(&[x, y], &[u, v])| x * v - y * u)
        .sum::<f64>()
        / 2.;
    assert!((screw.volume_mm3() - thread.volume_mm3() - area * 5.3).abs() < 1e-10);
    for (index, [x, y]) in vertices.into_iter().enumerate() {
        let theta = FRAC_PI_6 + index as f64 * TAU / 6.;
        assert!((screw.head_radius(theta) - x.hypot(y)).abs() < 1e-12);
    }
    let faces = screw.faces();
    assert_eq!(faces[0], ScrewFace::ThreadShoulder);
    assert_eq!(faces[1], ScrewFace::ThreadTip);
    assert_eq!(
        &faces[2..10],
        &(0..8).map(ScrewFace::ThreadWall).collect::<Vec<_>>()
    );
    assert_eq!(faces[10], ScrewFace::HeadTop);
    assert_eq!(
        &faces[11..],
        &(0..6).map(ScrewFace::HeadWall).collect::<Vec<_>>()
    );
}

#[test]
fn rejects_disconnected_cross_sections_unbounded_models_and_incompatible_recesses() {
    let thread = thread(1, false);
    let spec = ScrewSpec {
        head: socket_cap(),
        shank_length_mm: 3.,
        shank_radius_mm: 4.,
    };
    for (shank_length_mm, shank_radius_mm) in [
        (f64::NAN, 4.),
        (-0.1, 4.),
        (3., f64::INFINITY),
        (3., 0.),
        (3., 3.9),
    ] {
        assert_eq!(
            Screw::new(
                thread,
                ScrewSpec {
                    shank_length_mm,
                    shank_radius_mm,
                    ..spec
                }
            ),
            Err(ThreadError::Invalid)
        );
    }
    for (radius_mm, height_mm, socket_across_flats_mm, socket_depth_mm) in [
        (6.5, f64::INFINITY, 6., 4.),
        (6.5, 0., 6., 4.),
        (f64::NAN, 8., 6., 4.),
        (4., 8., 6., 4.),
        (6.5, 8., f64::NAN, 4.),
        (6.5, 8., 0., 4.),
        (6.5, 8., 12., 4.),
        (6.5, 8., 6., f64::NAN),
        (6.5, 8., 6., 0.),
        (6.5, 8., 6., 8.),
    ] {
        let head = ScrewHead::SocketCap {
            radius_mm,
            height_mm,
            socket_across_flats_mm,
            socket_depth_mm,
        };
        assert_eq!(
            Screw::new(thread, ScrewSpec { head, ..spec }),
            Err(ThreadError::Invalid)
        );
    }
    for across_flats_mm in [f64::NAN, 8.] {
        let head = ScrewHead::ExternalHex {
            across_flats_mm,
            height_mm: 5.3,
        };
        assert_eq!(
            Screw::new(thread, ScrewSpec { head, ..spec }),
            Err(ThreadError::Invalid)
        );
    }
    assert_eq!(
        Screw::new(
            thread,
            ScrewSpec {
                shank_length_mm: MAX_DIMENSION_MM,
                ..spec
            }
        ),
        Err(ThreadError::Limit)
    );
    let head = ScrewHead::ExternalHex {
        across_flats_mm: 2. * MAX_DIMENSION_MM,
        height_mm: 5.3,
    };
    assert_eq!(
        Screw::new(thread, ScrewSpec { head, ..spec }),
        Err(ThreadError::Limit)
    );
}
