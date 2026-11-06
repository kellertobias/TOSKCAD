use multicad_core::{
    commands::{Actor, Channel, Session},
    constraint_sketch::Dimension,
    document::Document,
    model::*,
};
use multicad_model::{
    Compiler, EvaluationCache, GearPairCheck, PairCycleProofStore, with_pair_cycle_proofs,
};

fn actor() -> Actor {
    Actor::local(Channel::Ui)
}
fn compiler() -> Compiler {
    Compiler {
        omit_display: true,
        ..Default::default()
    }
}
fn pair() -> (Session, GearPairRequest) {
    let mut session = Session::new(Document::new("Shared native cycle").unwrap()).unwrap();
    let request = GearPairRequest::new(
        "Pair",
        std::array::from_fn(|i| {
            GearRequest::new(
                if i == 0 { "Pinion" } else { "Wheel" },
                GearParameters {
                    teeth: Dimension::Literal {
                        value: if i == 0 { 20. } else { 30. },
                        unit: "count".into(),
                    },
                    module: length(1.),
                    face_width: length(2.),
                    bore_diameter: length(4.),
                    ..Default::default()
                },
            )
        }),
    );
    let prepared = prepare_gear_pair(&session, 0, actor(), &request).unwrap();
    session.commit(prepared.prepared).unwrap();
    (session, request)
}
fn on_owner(document: Document, store: PairCycleProofStore) -> (GearPairCheck, Vec<f64>) {
    std::thread::spawn(move || {
        with_pair_cycle_proofs(store, || {
            let model = compiler().compile(&document, 0).unwrap();
            assert_eq!(model.snapshot().leaf_occurrences.len(), 2);
            let volumes = model
                .snapshot()
                .leaf_occurrences
                .keys()
                .map(|path| {
                    let solid = model.leaf_solid(path).unwrap();
                    assert_eq!(solid.solid_count().unwrap(), 1);
                    solid.volume_mm3().unwrap()
                })
                .collect();
            (
                model.gear_pair_checks().values().next().unwrap().clone(),
                volumes,
            )
        })
    })
    .join()
    .unwrap()
}
#[test]
fn independent_native_owners_share_only_cycle_evidence_and_always_check_current_pose() {
    let (mut session, request) = pair();
    let store = PairCycleProofStore::default();
    let before = session.document().clone();
    let (first, volumes) = on_owner(before.clone(), store.clone());
    assert!(!first.cycle_reused);
    assert!(first.current_pose_checked);
    assert!(first.sampled_cycle_ms > 0.);
    assert_eq!(store.stats().entries, 1);
    let (second, again) = on_owner(before.clone(), store.clone());
    assert!(second.cycle_reused);
    assert!(second.current_pose_checked);
    assert_eq!(second.sampled_cycle_ms, 0.);
    assert!(second.current_pose_ms > 0.);
    assert_eq!(second.cycle_drive_angles_rad, first.cycle_drive_angles_rad);
    for (a, b) in volumes.iter().zip(again) {
        assert!((a - b).abs() < 1e-7);
    }
    let mut parameters = request.parameters.clone();
    parameters.drive_angle = angle(12.);
    let prepared = prepare_gear_pair_edit(&session, 1, actor(), request.pair, &parameters).unwrap();
    session.commit(prepared.prepared).unwrap();
    let (driven, driven_volumes) = on_owner(session.document().clone(), store.clone());
    assert!(driven.cycle_reused && driven.current_pose_checked);
    assert!((driven.current_drive_angle_rad - 12_f64.to_radians()).abs() < 1e-12);
    assert_eq!(store.stats().entries, 1);
    // Identical entity IDs copied into a genuinely different document are cold.
    let copied = Document::new("Different document")
        .unwrap()
        .edited(
            &session
                .document()
                .entities()
                .cloned()
                .map(multicad_core::document::Mutation::Insert)
                .collect::<Vec<_>>(),
        )
        .unwrap();
    let (foreign, _) = on_owner(copied, store.clone());
    assert!(!foreign.cycle_reused);
    assert_eq!(store.stats().entries, 2);
    assert_eq!(session.revision(), 2);
    for gear in &request.gears {
        assert_eq!(
            before.get(gear.feature),
            session.document().get(gear.feature)
        );
    }
    let (_, fresh_volumes) = on_owner(session.document().clone(), PairCycleProofStore::default());
    for (a, b) in driven_volumes.iter().zip(fresh_volumes) {
        assert!((a - b).abs() < 1e-7);
    }
}
#[test]
fn injected_clear_and_owner_switch_cannot_fall_back_to_warm_local_proofs() {
    let (session, request) = pair();
    let mut cache = EvaluationCache::default();
    compiler()
        .compile_cached(&mut cache, session.document(), 1, None)
        .unwrap();
    let store = PairCycleProofStore::default();
    with_pair_cycle_proofs(store.clone(), || {
        let cold = compiler()
            .compile_cached(&mut cache, session.document(), 1, None)
            .unwrap();
        assert!(!cold.gear_pair_checks()[&request.pair].cycle_reused);
        assert!(
            compiler()
                .compile_cached(&mut cache, session.document(), 1, None)
                .unwrap()
                .gear_pair_checks()[&request.pair]
                .cycle_reused
        );
        store.clear();
        let cleared = compiler()
            .compile_cached(&mut cache, session.document(), 1, None)
            .unwrap();
        assert!(!cleared.gear_pair_checks()[&request.pair].cycle_reused);
        cache.clear();
        let native_reset = compiler()
            .compile_cached(&mut cache, session.document(), 1, None)
            .unwrap();
        assert!(native_reset.gear_pair_checks()[&request.pair].cycle_reused);
    });
    with_pair_cycle_proofs(PairCycleProofStore::default(), || {
        let other = compiler()
            .compile_cached(&mut cache, session.document(), 1, None)
            .unwrap();
        assert!(!other.gear_pair_checks()[&request.pair].cycle_reused);
    });
}
#[test]
fn source_inspection_does_not_mint_full_model_cycle_evidence() {
    let (session, request) = pair();
    let store = PairCycleProofStore::default();
    with_pair_cycle_proofs(store.clone(), || {
        let model = compiler()
            .compile_sources(session.document(), 1, &[request.gears[0].feature])
            .unwrap();
        assert!(model.gear_pair_checks().is_empty());
    });
    assert_eq!(store.stats().entries, 0);
}

#[test]
fn runtime_creation_transfers_explicit_store_to_distinct_native_workers() {
    let (session, request) = pair();
    let store = PairCycleProofStore::default();
    for expected_reuse in [false, true] {
        let mut worker = with_pair_cycle_proofs(store.clone(), || {
            multicad_model::Runtime::with_model_publisher(compiler(), move |_, model, _| {
                Ok(model.gear_pair_checks()[&request.pair].clone())
            })
            .unwrap()
        });
        worker.request(session.document(), 1).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(90);
        let report = loop {
            if let Some(reply) = worker.poll() {
                break reply.result.unwrap();
            }
            assert!(
                std::time::Instant::now() < deadline,
                "native runtime deadline"
            );
            std::thread::sleep(std::time::Duration::from_millis(2));
        };
        assert_eq!(report.cycle_reused, expected_reuse);
        assert!(report.current_pose_checked);
    }
    assert_eq!(store.stats().entries, 1);
}
