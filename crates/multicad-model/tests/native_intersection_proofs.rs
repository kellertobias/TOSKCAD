//! Full native connection validation must stay live around cached exact None
//! predicates. These controls do not assert an operator-latency target.
#[path = "../../multicad-mcp/tests/support/screw_connection.rs"]
mod fixture;
use multicad_core::{
    commands::{Actor, Channel, Session},
    model::*,
};
use multicad_model::{Compiler, EvaluationCache};
use std::time::Instant;
fn actor() -> Actor {
    Actor::local(Channel::CommandLine)
}
fn compiler() -> Compiler {
    Compiler {
        omit_display: true,
        ..Default::default()
    }
}

#[test]
fn warm_metric_connections_reuse_none_but_recheck_engagement_and_new_foreign_geometry() {
    for (document, request, _) in [fixture::fixture(), fixture::tapped_fixture()] {
        let mut session = Session::new(document).unwrap();
        session
            .commit(
                prepare_screw_connection(&session, 0, actor(), &request)
                    .unwrap()
                    .prepared,
            )
            .unwrap();
        let compiler = compiler();
        let mut cache = EvaluationCache::default();
        let cold = Instant::now();
        let first = compiler
            .compile_cached(&mut cache, session.document(), 0, None)
            .unwrap();
        let cold = cold.elapsed();
        let first_stats = cache.native_intersection_proof_stats();
        assert!(
            first_stats.published > 0 && first_stats.native_queries > 0,
            "The first complete connection must execute and publish actual native None queries: {first_stats:?}"
        );
        let warm = Instant::now();
        let second = compiler
            .compile_cached(&mut cache, session.document(), 1, None)
            .unwrap();
        let warm = warm.elapsed();
        let warm_stats = cache.native_intersection_proof_stats();
        assert!(
            warm_stats.hits > first_stats.hits,
            "Unchanged exact geometry should reuse completed None evidence: {warm_stats:?}"
        );
        assert_eq!(
            second.snapshot().screw_connection_checks,
            first.snapshot().screw_connection_checks
        );
        assert_eq!(second.snapshot().bodies, first.snapshot().bodies);
        assert_eq!(
            second
                .snapshot()
                .leaf_occurrences
                .keys()
                .collect::<Vec<_>>(),
            first.snapshot().leaf_occurrences.keys().collect::<Vec<_>>()
        );
        eprintln!(
            "{:?} exact-native cache cold={cold:?} warm={warm:?}, {warm_stats:?}",
            request.mode
        );
        let mut invalid =
            read_screw_connection_request(session.document(), request.connection).unwrap();
        invalid.parameters.minimum_engagement = length(4.);
        let invalid =
            prepare_screw_connection(&session, session.revision(), actor(), &invalid).unwrap();
        let error = compiler
            .compile_cached(&mut cache, invalid.prepared.document(), 2, None)
            .err()
            .expect("A cached None must not skip current engagement validation");
        assert!(
            error.message.to_lowercase().contains("engagement"),
            "{error}"
        );
        assert_eq!(
            cache.native_intersection_proof_stats().published,
            warm_stats.published
        );
        if request.mode != ScrewConnectionMode::ThroughNut {
            continue;
        }
        let collider = PrimitiveRequest::new(
            "Independent foreign component",
            PrimitiveParameters::Box {
                size: std::array::from_fn(|_| length(0.5)),
                frame: PrimitiveFrame {
                    origin: std::array::from_fn(|_| length(100.)),
                    ..Default::default()
                },
            },
        );
        session
            .commit(
                prepare_primitive(&session, session.revision(), actor(), &collider)
                    .unwrap()
                    .prepared,
            )
            .unwrap();
        compiler
            .compile_cached(&mut cache, session.document(), 3, None)
            .unwrap();
        let before = cache.native_intersection_proof_stats();
        let moved = PrimitiveParameters::Box {
            size: std::array::from_fn(|_| length(0.5)),
            frame: PrimitiveFrame {
                origin: [length(9.75), length(9.75), length(4.5)],
                ..Default::default()
            },
        };
        session
            .commit(
                prepare_primitive_edit(
                    &session,
                    session.revision(),
                    actor(),
                    collider.feature,
                    &moved,
                )
                .unwrap()
                .prepared,
            )
            .unwrap();
        let source = session.document().clone();
        let error = compiler
            .compile_cached(&mut cache, session.document(), 4, None)
            .err()
            .expect(
                "A newly intersecting foreign component must invalidate full connection readiness",
            );
        assert!(
            error.message.contains("unintended component interference"),
            "{error}"
        );
        let after = cache.native_intersection_proof_stats();
        assert!(
            after.native_queries > before.native_queries,
            "The changed actual foreign solid must reach a new native query"
        );
        assert_eq!(
            after.published, before.published,
            "A later failed query cannot publish earlier staged results"
        );
        assert_eq!(session.document(), &source);
    }
}
