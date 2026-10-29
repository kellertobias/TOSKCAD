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

#[test]
fn step_solids_split_a_file_into_located_exact_breps_that_round_trip() {
    let directory = tempfile::tempdir().unwrap();
    let policy = GeometryPolicy::default();
    let a = Solid::box_solid([10., 20., 30.], policy).unwrap();
    let b = Solid::box_solid([5., 5., 5.], policy)
        .unwrap()
        .transformed(RigidTransform {
            translation_mm: [100., 0., 0.],
            ..Default::default()
        })
        .unwrap();
    let both = a
        .boolean(&b, multicad_kernel::BooleanOperation::Union, policy)
        .unwrap()
        .unwrap();
    assert_eq!(both.solid_count().unwrap(), 2);
    let path = directory.path().join("two.step");
    both.write_step(&path).unwrap();
    let solids = Solid::step_solids(&path).unwrap();
    assert_eq!(solids.len(), 2);
    let mut volumes: Vec<f64> = solids
        .iter()
        .map(|(_, brep)| {
            Solid::from_brep(brep, policy)
                .unwrap()
                .volume_mm3()
                .unwrap()
        })
        .collect();
    volumes.sort_by(f64::total_cmp);
    assert!((volumes[0] - 125.).abs() < 1e-6 && (volumes[1] - 6000.).abs() < 1e-6);
    // Located solids keep their position, and BRep text round-trips exactly.
    let far = solids
        .iter()
        .map(|(_, brep)| Solid::from_brep(brep, policy).unwrap())
        .find(|s| s.bounds().unwrap().min[0] > 50.)
        .unwrap();
    assert!((far.bounds().unwrap().min[0] - 100.).abs() < 1e-6);
    let again = Solid::from_brep(&far.to_brep().unwrap(), policy).unwrap();
    assert!((again.volume_mm3().unwrap() - 125.).abs() < 1e-9);
    assert!(Solid::from_brep("not brep", policy).is_err());
}
