use multicad_kernel::*;
fn program(revision: u64) -> GeometryProgram {
    GeometryProgram {
        revision,
        policy: GeometryPolicy::default(),
        operations: vec![GeometryOperation::Box {
            size_mm: [10., 10., 10.],
        }],
        outputs: vec![0],
        mesh: MeshOptions::default(),
    }
}
#[test]
fn background_program_preserves_exact_geometry_behind_revision_bound_keys() {
    let worker = GeometryWorker::new().unwrap();
    let mut p = program(7);
    p.operations.push(GeometryOperation::Transform {
        source: 0,
        placement: RigidTransform {
            translation_mm: [5., 0., 0.],
            ..Default::default()
        },
    });
    p.operations.push(GeometryOperation::Boolean {
        a: 0,
        b: 1,
        operation: BooleanOperation::Subtract,
    });
    p.outputs = vec![0, 1, 2];
    let mut snapshot = worker.submit(p).unwrap().wait().unwrap();
    assert_eq!(snapshot.revision, 7);
    let final_solid = snapshot.solids[2].as_mut().unwrap();
    assert!((final_solid.volume_mm3 - 500.).abs() < 1e-7);
    assert_eq!(final_solid.key.revision(), 7);
    let key = final_solid.key;
    final_solid.mesh.vertices.fill(99999.);
    final_solid.mesh.indices.clear();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("exact.step");
    worker
        .write_step_to_staging(key, 7, path.clone())
        .unwrap()
        .wait()
        .unwrap();
    let exact = Solid::read_step(&path, GeometryPolicy::default()).unwrap();
    assert!((exact.volume_mm3().unwrap() - 500.).abs() < 1e-7);
    assert!(
        worker
            .write_step_to_staging(key, 8, directory.path().join("wrong.step"))
            .is_err()
    );
    assert!(!directory.path().join("wrong.step").exists());
    let foreign = GeometryWorker::new().unwrap();
    assert!(
        foreign
            .write_step_to_staging(key, 7, directory.path().join("foreign.step"))
            .is_err()
    );
}
#[test]
fn superseded_and_cancelled_jobs_are_never_adopted() {
    let worker = GeometryWorker::new().unwrap();
    let old = worker.submit(program(1)).unwrap();
    let latest = worker.submit(program(2)).unwrap();
    assert_eq!(old.wait().unwrap_err().kind, GeometryErrorKind::Stale);
    assert_eq!(latest.wait().unwrap().revision, 2);
    let cancelled = worker.submit(program(3)).unwrap();
    cancelled.cancel();
    assert_eq!(
        cancelled.wait().unwrap_err().kind,
        GeometryErrorKind::Cancelled
    );
    let key = worker.submit(program(4)).unwrap().wait().unwrap().solids[0]
        .as_ref()
        .unwrap()
        .key;
    let newer = worker.submit(program(5)).unwrap().wait().unwrap();
    assert_eq!(newer.revision, 5);
    let dir = tempfile::tempdir().unwrap();
    assert!(
        worker
            .write_step_to_staging(key, 4, dir.path().join("stale.step"))
            .is_err()
    );
    assert_eq!(
        worker.submit(program(4)).err().unwrap().kind,
        GeometryErrorKind::Stale
    );
}
#[test]
fn invalid_source_and_failed_geometry_allow_a_later_valid_rebuild() {
    let worker = GeometryWorker::new().unwrap();
    let mut bad = program(1);
    bad.operations.push(GeometryOperation::Transform {
        source: 1,
        placement: Default::default(),
    });
    assert!(worker.submit(bad).is_err());
    let mut invalid = program(2);
    invalid.operations[0] = GeometryOperation::Box {
        size_mm: [f64::NAN, 10., 10.],
    };
    assert_eq!(
        worker.submit(invalid).unwrap().wait().unwrap_err().kind,
        GeometryErrorKind::Input
    );
    assert_eq!(
        worker
            .submit(program(3))
            .unwrap()
            .wait()
            .unwrap()
            .solids
            .len(),
        1
    );
    let mut duplicate = program(4);
    duplicate.outputs = vec![0, 0];
    assert!(worker.submit(duplicate).is_err());
}
#[test]
fn empty_material_and_an_empty_document_are_explicit_results() {
    let worker = GeometryWorker::new().unwrap();
    let mut p = program(1);
    p.operations.push(GeometryOperation::Boolean {
        a: 0,
        b: 0,
        operation: BooleanOperation::Subtract,
    });
    p.outputs = vec![1];
    assert!(worker.submit(p.clone()).unwrap().wait().unwrap().solids[0].is_none());
    p.revision = 2;
    p.operations.push(GeometryOperation::Transform {
        source: 1,
        placement: Default::default(),
    });
    p.outputs = vec![2];
    assert_eq!(
        worker.submit(p).unwrap().wait().unwrap_err().kind,
        GeometryErrorKind::Input
    );
    let mut empty = program(3);
    empty.operations.clear();
    empty.outputs.clear();
    assert!(
        worker
            .submit(empty)
            .unwrap()
            .wait()
            .unwrap()
            .solids
            .is_empty()
    );
}
#[test]
fn whole_model_triangle_budget_and_worker_shutdown_are_enforced() {
    let worker = GeometryWorker::new().unwrap();
    let mut p = program(1);
    p.operations.push(GeometryOperation::Box {
        size_mm: [10., 10., 10.],
    });
    p.outputs = vec![0, 1];
    p.mesh.maximum_triangles = 20;
    assert!(worker.submit(p).unwrap().wait().is_err());
    let job = worker.submit(program(2)).unwrap();
    drop(worker);
    assert_eq!(job.wait().unwrap_err().kind, GeometryErrorKind::Cancelled);
}
