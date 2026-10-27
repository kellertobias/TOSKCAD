use multicad_kernel::{GeometryPolicy, RigidTransform, Solid};
use std::sync::{Arc, Barrier};

#[test]
fn concurrent_step_exchange_retains_independent_owner_geometry_and_unlocks_after_failures() {
    let directory = tempfile::tempdir().unwrap();
    let start = Arc::new(Barrier::new(8));
    let workers: Vec<_> = (0..8)
        .map(|owner| {
            let directory = directory.path().to_path_buf();
            let start = start.clone();
            std::thread::spawn(move || {
                // Only paths, counters and the start barrier cross threads.
                // Every Solid is constructed, used and dropped by this owner.
                start.wait();
                for iteration in 0..12 {
                    let dimensions = [20. + owner as f64, 30. + iteration as f64, 8.];
                    let translation = [owner as f64 * 100., iteration as f64 * 5., 0.];
                    let source = Solid::box_solid(dimensions, GeometryPolicy::default())
                        .unwrap()
                        .transformed(RigidTransform {
                            translation_mm: translation,
                            ..Default::default()
                        })
                        .unwrap();
                    if iteration == 0 {
                        let bad = directory.join(format!("bad-{owner}.step"));
                        std::fs::write(&bad, b"not a STEP file").unwrap();
                        assert!(Solid::read_step(&bad, GeometryPolicy::default()).is_err());
                        assert!(
                            source
                                .write_step(&directory.join(format!("missing/failed-{owner}.step")))
                                .is_err()
                        );
                    }
                    let path = directory.join(format!("solid-{owner}-{iteration}.step"));
                    source.write_step(&path).unwrap();
                    assert!(
                        std::fs::read_to_string(&path)
                            .unwrap()
                            .contains("SI_UNIT(.MILLI.,.METRE.)")
                    );
                    let reopened = Solid::read_step(&path, GeometryPolicy::default()).unwrap();
                    let expected = dimensions.into_iter().product::<f64>();
                    assert!((reopened.volume_mm3().unwrap() - expected).abs() < 1e-6);
                    let bounds = reopened.bounds().unwrap();
                    for axis in 0..3 {
                        assert!((bounds.min[axis] - translation[axis]).abs() < 1e-6);
                        assert!(
                            (bounds.max[axis] - translation[axis] - dimensions[axis]).abs() < 1e-6
                        );
                    }
                }
            })
        })
        .collect();
    for worker in workers {
        worker.join().unwrap();
    }
}
