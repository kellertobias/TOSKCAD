use super::*;
use crate::{Compiler, EvaluationCache};
use multicad_core::{
    commands::{Actor, Channel, Session},
    document::Document,
    model::{PrimitiveParameters, PrimitiveRequest, length, prepare_primitive},
};
use multicad_kernel::RigidTransform;
use std::{cell::Cell, time::Instant};

fn separated(policy: GeometryPolicy) -> (Solid, Solid) {
    let a = Solid::box_solid([2.; 3], policy).unwrap();
    let b = a
        .transformed(RigidTransform {
            translation_mm: [5., 0., 0.],
            ..Default::default()
        })
        .unwrap();
    (a, b)
}
fn successful(store: &ProofStore, a: &Solid, b: &Solid, policy: GeometryPolicy) -> bool {
    let scope = PublicationScope::begin(Some(store.clone()));
    let result = no_intersection(a, b, policy).unwrap();
    scope.finish(true);
    result
}

#[test]
fn real_completed_none_hits_but_actual_shape_pose_and_every_policy_plane_miss() {
    let policy = GeometryPolicy::default();
    let (a, b) = separated(policy);
    let store = ProofStore::new(MAX_BYTES, MAX_ENTRIES);
    let owner_a = multicad_core::document::EntityId::new();
    let owner_b = multicad_core::document::EntityId::new();
    let source_a = (
        a.volume_mm3().unwrap(),
        a.bounds().unwrap(),
        a.topology_snapshot(owner_a, 0).unwrap(),
    );
    let source_b = (
        b.volume_mm3().unwrap(),
        b.bounds().unwrap(),
        b.topology_snapshot(owner_b, 0).unwrap(),
    );
    let before_a = a.to_brep().unwrap();
    let before_b = b.to_brep().unwrap();
    let before_digest = [
        a.exact_geometry_digest().unwrap(),
        b.exact_geometry_digest().unwrap(),
    ];
    assert_eq!(
        before_digest,
        [
            a.exact_geometry_digest().unwrap(),
            b.exact_geometry_digest().unwrap()
        ],
        "Digest must be stable without a native operation"
    );
    let before_key = key(&a, &b, policy).unwrap();
    let cold = Instant::now();
    assert!(successful(&store, &a, &b, policy));
    let cold = cold.elapsed();
    assert_eq!(
        source_a,
        (
            a.volume_mm3().unwrap(),
            a.bounds().unwrap(),
            a.topology_snapshot(owner_a, 0).unwrap()
        ),
        "COMMON must preserve the original source geometry/topology"
    );
    assert_eq!(
        source_b,
        (
            b.volume_mm3().unwrap(),
            b.bounds().unwrap(),
            b.topology_snapshot(owner_b, 0).unwrap()
        ),
        "COMMON must preserve the other source geometry/topology"
    );
    let after_digest = [
        a.exact_geometry_digest().unwrap(),
        b.exact_geometry_digest().unwrap(),
    ];
    eprintln!(
        "ordered key before={before_key} after={}",
        key(&a, &b, policy).unwrap()
    );
    eprintln!(
        "a digest {} -> {}; b digest {} -> {}",
        before_digest[0], after_digest[0], before_digest[1], after_digest[1]
    );
    for (label, before, after) in [
        ("a", before_a, a.to_brep().unwrap()),
        ("b", before_b, b.to_brep().unwrap()),
    ] {
        let differences = before
            .lines()
            .zip(after.lines())
            .enumerate()
            .filter(|(_, (a, b))| a != b)
            .take(12)
            .map(|(line, (before, after))| (line + 1, before, after))
            .collect::<Vec<_>>();
        eprintln!("{label} BRep differences {differences:?}");
    }
    assert_eq!(store.stats().entries, 1);
    let warm = Instant::now();
    assert!(successful(&store, &a, &b, policy));
    let warm = warm.elapsed();
    assert_eq!(store.stats().hits, 1);
    assert_eq!(store.stats().native_queries, 1);
    let moved = b
        .transformed(RigidTransform {
            translation_mm: [1., 0., 0.],
            ..Default::default()
        })
        .unwrap();
    assert!(successful(&store, &a, &moved, policy));
    let changed = Solid::box_solid([3., 2., 2.], policy).unwrap();
    assert!(successful(&store, &changed, &b, policy));
    let changed_request = GeometryPolicy {
        boolean_fuzzy_mm: 1e-8,
        ..policy
    };
    assert!(successful(&store, &a, &b, changed_request));
    let operand_policy = GeometryPolicy {
        minimum_feature_mm: 2e-5,
        ..policy
    };
    let operand = Solid::box_solid([2.; 3], operand_policy).unwrap();
    assert!(successful(&store, &operand, &b, policy));
    let other_operand = operand
        .transformed(RigidTransform {
            translation_mm: [5., 0., 0.],
            ..Default::default()
        })
        .unwrap();
    assert!(successful(&store, &a, &other_operand, policy));
    assert_eq!(store.stats().native_queries, 6);
    assert_ne!(
        key(&a, &b, policy).unwrap(),
        key(&b, &a, policy).unwrap(),
        "Keep native operand order explicit"
    );
    eprintln!(
        "exact None cold={cold:?}, warm={warm:?}; {:?}",
        store.stats()
    );
}

#[test]
fn intersection_errors_and_nonempty_results_never_become_receipts() {
    let policy = GeometryPolicy::default();
    let (a, _) = separated(policy);
    let overlap = a
        .transformed(RigidTransform {
            translation_mm: [1., 0., 0.],
            ..Default::default()
        })
        .unwrap();
    let store = ProofStore::new(MAX_BYTES, MAX_ENTRIES);
    assert!(!successful(&store, &a, &overlap, policy));
    assert!(!successful(&store, &a, &overlap, policy));
    let scope = PublicationScope::begin(Some(store.clone()));
    assert!(
        no_intersection(
            &a,
            &overlap,
            GeometryPolicy {
                linear_tolerance_mm: 0.,
                ..policy
            }
        )
        .is_err()
    );
    scope.finish(true);
    assert_eq!(store.stats().entries, 0);
    assert_eq!(store.stats().published, 0);
    assert_eq!(store.stats().hits, 0);
}

#[test]
fn later_failed_pair_and_nested_failure_discard_completed_none() {
    let policy = GeometryPolicy::default();
    let (a, b) = separated(policy);
    let overlap = a
        .transformed(RigidTransform {
            translation_mm: [1., 0., 0.],
            ..Default::default()
        })
        .unwrap();
    let store = ProofStore::new(MAX_BYTES, MAX_ENTRIES);
    let outer = PublicationScope::begin(None);
    let child = PublicationScope::begin(Some(store.clone()));
    assert!(no_intersection(&a, &b, policy).unwrap());
    child.finish(true);
    assert_eq!(
        store.stats().entries,
        0,
        "Child completion cannot publish through its parent"
    );
    assert!(!no_intersection(&a, &overlap, policy).unwrap());
    outer.finish(false);
    assert_eq!(store.stats().published, 0);
    let outer = PublicationScope::begin(None);
    let child = PublicationScope::begin(Some(store.clone()));
    assert!(no_intersection(&a, &b, policy).unwrap());
    child.finish(true);
    outer.finish(true);
    assert_eq!(store.stats().published, 1);
}

#[test]
fn clear_epoch_unwind_and_source_scope_cannot_publish_or_reuse_stale_work() {
    let policy = GeometryPolicy::default();
    let (a, b) = separated(policy);
    let store = ProofStore::new(MAX_BYTES, MAX_ENTRIES);
    let scope = PublicationScope::begin(Some(store.clone()));
    assert!(no_intersection(&a, &b, policy).unwrap());
    store.clear();
    scope.finish(true);
    assert_eq!(store.stats().entries, 0);
    assert_eq!(store.stats().stale_publications, 1);
    let aborted = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _scope = PublicationScope::begin(Some(store.clone()));
        assert!(no_intersection(&a, &b, policy).unwrap());
        panic!("controlled abandoned compile");
    }));
    assert!(aborted.is_err());
    assert_eq!(store.stats().entries, 0);
    let source_scope = PublicationScope::begin(None);
    let child = PublicationScope::begin(Some(store.clone()));
    assert!(no_intersection(&a, &b, policy).unwrap());
    child.finish(true);
    source_scope.finish(false);
    assert_eq!(store.stats().entries, 0);
    assert!(
        FRAMES.with(|state| state.borrow().frames.is_empty() && state.borrow().pending.is_empty())
    );
}

#[test]
fn bounded_lru_and_zero_capacity_preserve_native_results() {
    let policy = GeometryPolicy::default();
    let (a, b) = separated(policy);
    let store = ProofStore::new(MAX_BYTES, 2);
    assert!(successful(&store, &a, &b, policy));
    for x in [8., 11.] {
        let other = b
            .transformed(RigidTransform {
                translation_mm: [x, 0., 0.],
                ..Default::default()
            })
            .unwrap();
        assert!(successful(&store, &a, &other, policy));
    }
    assert_eq!(store.stats().entries, 2);
    assert_eq!(store.stats().evicted, 1);
    let before = store.stats().native_queries;
    assert!(successful(&store, &a, &b, policy));
    assert_eq!(store.stats().native_queries, before + 1);
    let disabled = ProofStore::new(0, MAX_ENTRIES);
    assert!(successful(&disabled, &a, &b, policy));
    assert!(successful(&disabled, &a, &b, policy));
    assert_eq!(disabled.stats().entries, 0);
    assert_eq!(disabled.stats().native_queries, 2);
}

#[test]
fn nested_pending_accumulator_is_aggregate_bounded_and_does_not_keep_owner_alive() {
    let policy = GeometryPolicy::default();
    let (a, b) = separated(policy);
    let store = ProofStore::new(MAX_BYTES, MAX_ENTRIES);
    let outer = PublicationScope::begin(Some(store.clone()));
    for start in [0, 129] {
        let child = PublicationScope::begin(Some(store.clone()));
        for index in start..start + 129 {
            let other = b
                .transformed(RigidTransform {
                    translation_mm: [index as f64, 0., 0.],
                    ..Default::default()
                })
                .unwrap();
            assert!(no_intersection(&a, &other, policy).unwrap());
            assert!(
                FRAMES.with(|state| state.borrow().pending.len() <= MAX_ENTRIES),
                "Bound applies while the nested child is still active"
            );
            assert!(FRAMES.with(|state| state.borrow().pending.capacity() <= MAX_ENTRIES));
        }
        child.finish(true);
        assert!(FRAMES.with(|frames| { frames.borrow().pending.len() <= MAX_ENTRIES }));
    }
    assert_eq!(store.stats().entries, 0);
    assert!(FRAMES.with(|frames| frames.borrow().pending.len() == MAX_ENTRIES));
    outer.finish(true);
    assert!(store.stats().entries <= MAX_ENTRIES);
    assert!(store.stats().retained_bytes <= MAX_BYTES);
    let owner = ProofStore::new(MAX_BYTES, MAX_ENTRIES);
    let weak = Rc::downgrade(&owner.0);
    let parent = PublicationScope::begin(None);
    let child = PublicationScope::begin(Some(owner.clone()));
    assert!(no_intersection(&a, &b, policy).unwrap());
    child.finish(true);
    drop(owner);
    assert!(
        weak.upgrade().is_none(),
        "Staged proofs must not extend a discarded cache owner lifetime"
    );
    parent.finish(true);
}

fn document() -> Document {
    let mut session = Session::new(Document::new("Final compile checkpoint").unwrap()).unwrap();
    let body = PrimitiveRequest::new(
        "Stock",
        PrimitiveParameters::Box {
            size: std::array::from_fn(|_| length(2.)),
            frame: Default::default(),
        },
    );
    session
        .commit(
            prepare_primitive(&session, 0, Actor::local(Channel::Ui), &body)
                .unwrap()
                .prepared,
        )
        .unwrap();
    session.document().clone()
}

#[test]
fn real_compiler_final_cancellation_and_scoped_compile_do_not_publish() {
    let document = document();
    let policy = GeometryPolicy::default();
    let (a, b) = separated(policy);
    let compiler = Compiler::default();
    let mut baseline = EvaluationCache::default();
    let calls = Cell::new(0);
    compiler
        .compile_cached(
            &mut baseline,
            &document,
            0,
            Some(&|| {
                if calls.get() == 0 {
                    assert!(no_intersection(&a, &b, policy).unwrap());
                }
                calls.set(calls.get() + 1);
                false
            }),
        )
        .unwrap();
    let final_checkpoint = calls.get();
    assert!(final_checkpoint > 1 && baseline.native_intersection_proof_stats().published == 1);
    let mut cancelled = EvaluationCache::default();
    let calls = Cell::new(0);
    let result = compiler.compile_cached(
        &mut cancelled,
        &document,
        1,
        Some(&|| {
            if calls.get() == 0 {
                assert!(no_intersection(&a, &b, policy).unwrap());
            }
            calls.set(calls.get() + 1);
            calls.get() >= final_checkpoint
        }),
    );
    assert!(result.is_err());
    assert_eq!(
        cancelled.native_intersection_proof_stats().native_queries,
        1
    );
    assert_eq!(cancelled.native_intersection_proof_stats().published, 0);
    assert_eq!(cancelled.native_intersection_proof_stats().entries, 0);
    let feature = document
        .entities()
        .find(|entity| {
            matches!(
                entity.data,
                multicad_core::document::EntityData::Feature { .. }
            )
        })
        .unwrap()
        .id;
    let calls = Cell::new(0);
    compiler
        .compile_sources_cached(
            &mut cancelled,
            &document,
            2,
            &[feature],
            Some(&|| {
                if calls.get() == 0 {
                    assert!(no_intersection(&a, &b, policy).unwrap());
                }
                calls.set(calls.get() + 1);
                false
            }),
        )
        .unwrap();
    assert_eq!(cancelled.native_intersection_proof_stats().entries, 0);
}
