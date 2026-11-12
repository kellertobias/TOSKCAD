use multicad_rust_kernel::{
    helical_thread::Spec,
    radial_tensor::{Location, Surface},
    step, step_read,
    tapered_thread::{Error, Thread},
};
use std::cell::Cell;
fn source(left: bool, starts: u32, slope: f64) -> Thread {
    Thread::new(
        Spec {
            major_radius_mm: 4.,
            pitch_mm: 1.25,
            length_mm: 2.5,
            depth_mm: 0.5,
            crest_width_mm: 0.25,
            lower_flank_deg: 12.,
            upper_flank_deg: 18.,
            starts,
            left_hand: left,
            root_radius_mm: None,
        },
        slope,
        1e-7,
        1e-6,
    )
    .unwrap()
}
const IDENTITY: [[f64; 3]; 3] = [[0.; 3], [1., 0., 0.], [0., 1., 0.]];
#[test]
fn direct_authored_graph_keeps_actual_start_band_and_cap_ownership() {
    for (left, starts, slope, frame) in [
        (false, 1, 0., IDENTITY),
        (true, 2, 0.03125, IDENTITY),
        (true, 8, 0.03125, IDENTITY),
        (
            false,
            2,
            -0.03125,
            [[80., -30., 20.], [0.8, 0.6, 0.], [-0.36, 0.48, 0.8]],
        ),
    ] {
        let source = source(left, starts, slope);
        let direct = source
            .retained_boundary_with_interrupt(frame, 1e-7, 1e-6, None)
            .unwrap();
        let input = direct.boundary.input();
        assert_eq!(input.faces.len(), 2 + 4 * starts as usize);
        for &face in &direct.cap_faces {
            assert!(matches!(
                input.surfaces[input.faces[face].surface],
                Surface::Plane(_)
            ));
        }
        for faces in &direct.band_faces {
            for &face in faces {
                assert!(matches!(
                    input.surfaces[input.faces[face].surface],
                    Surface::Tensor(_)
                ));
            }
        }
        for &face in &direct.gap_faces {
            assert!(matches!(
                input.surfaces[input.faces[face].surface],
                Surface::Cone { .. }
            ));
        }
        let text = step::write_radial_boundary(&direct.boundary, 1e-7).unwrap();
        step_read::verify_tapered_thread(&text, &source, frame[0], frame[1], frame[2], 1e-7)
            .unwrap();
        let decoded = step_read::read_radial_boundary(&text, 1e-7)
            .unwrap()
            .bind_surfaces(1e-7, None)
            .unwrap()
            .certify_axial(1e-7, 1e-6, None)
            .unwrap()
            .certify_caps(None)
            .unwrap()
            .certify_ordering(None)
            .unwrap()
            .certify_material(1e-7, None)
            .unwrap();
        let normal = [
            frame[1][1] * frame[2][2] - frame[1][2] * frame[2][1],
            frame[1][2] * frame[2][0] - frame[1][0] * frame[2][2],
            frame[1][0] * frame[2][1] - frame[1][1] * frame[2][0],
        ];
        let center = std::array::from_fn(|i| frame[0][i] + normal[i] * 1.25);
        assert_eq!(
            direct.boundary.classify(center, None).unwrap(),
            Location::Inside
        );
        assert_eq!(decoded.classify(center, None).unwrap(), Location::Inside);
        assert_eq!(
            direct.boundary.cap_rims(0).unwrap().len(),
            4 * starts as usize
        );
    }
}
#[test]
fn direct_construction_cancels_and_refuses_bad_inputs_without_poisoning_source() {
    let source = source(false, 1, 0.03125);
    let original = source.spec();
    for invalid in [0., -1., f64::INFINITY, f64::NAN] {
        assert!(matches!(
            source.retained_boundary_with_interrupt(IDENTITY, invalid, 1e-6, None),
            Err(Error::Invalid)
        ));
        assert!(matches!(
            source.retained_boundary_with_interrupt(IDENTITY, 1e-7, invalid, None),
            Err(Error::Invalid)
        ));
    }
    assert!(matches!(
        source.retained_boundary_with_interrupt([[0.; 3]; 3], 1e-7, 1e-6, None),
        Err(Error::Invalid)
    ));
    assert!(matches!(
        source.retained_boundary_with_interrupt(
            [[1e7; 3], IDENTITY[1], IDENTITY[2]],
            1e-7,
            1e-6,
            None
        ),
        Err(Error::Unsupported)
    ));
    assert!(matches!(
        source.retained_boundary_with_interrupt(IDENTITY, 1e-90, 1e-6, None),
        Err(Error::Limit)
    ));
    let calls = Cell::new(0);
    let count = || {
        calls.set(calls.get() + 1);
        false
    };
    let first = source
        .retained_boundary_with_interrupt(IDENTITY, 1e-7, 1e-6, Some(&count))
        .unwrap();
    let total = calls.get();
    for stop in [1, 2, total / 2, total - 1] {
        calls.set(0);
        let cancel = || {
            calls.set(calls.get() + 1);
            calls.get() == stop
        };
        assert!(
            matches!(
                source.retained_boundary_with_interrupt(IDENTITY, 1e-7, 1e-6, Some(&cancel)),
                Err(Error::Cancelled)
            ),
            "{stop}/{total}"
        );
        assert_eq!(calls.get(), stop);
    }
    assert_eq!(source.spec(), original);
    assert_eq!(
        source
            .retained_boundary_with_interrupt(IDENTITY, 1e-7, 1e-6, None)
            .unwrap()
            .boundary
            .input(),
        first.boundary.input()
    );
}

#[test]
fn cancellation_during_placed_pole_preparation_remains_typed_and_atomic() {
    let source = source(false, 1, 0.03125);
    let calls = Cell::new(0);
    let count = || {
        calls.set(calls.get() + 1);
        false
    };
    source.manufacturing_tensors(1e-7, Some(&count)).unwrap();
    // The direct entry checks once before this unchanged source operation,
    // then checks the first placed-pole block before producing its points.
    let first_placed_block = calls.get() + 2;
    calls.set(0);
    let cancel = || {
        calls.set(calls.get() + 1);
        calls.get() == first_placed_block
    };
    assert!(matches!(
        source.retained_boundary_with_interrupt(IDENTITY, 1e-7, 1e-6, Some(&cancel)),
        Err(Error::Cancelled)
    ));
    assert_eq!(calls.get(), first_placed_block);
    assert!(
        source
            .retained_boundary_with_interrupt(IDENTITY, 1e-7, 1e-6, None)
            .is_ok()
    );
}
