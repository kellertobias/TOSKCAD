//! CAD-150 incremental rebuilds: a compile through an `EvaluationCache` is
//! identical to a cold compile (the complete published snapshot, including
//! shapes, meshes, topology names, volumes, references and side results),
//! reuses every feature whose inputs are unchanged and re-runs only affected
//! features, also across randomized edit/undo sequences, interruptions and
//! memory-bounded eviction.
#[path = "../examples/makerspace_review.rs"]
#[allow(dead_code)]
mod makerspace;

#[path = "../examples/tool_case_review.rs"]
#[allow(dead_code)]
mod tool_case;

#[path = "../examples/cabinetry_table_review.rs"]
#[allow(dead_code)]
mod cabinetry_table;

#[path = "../examples/control_desk_review.rs"]
#[allow(dead_code)]
mod control_desk;

#[path = "../examples/large_scenario.rs"]
#[allow(dead_code)]
mod large;

use multicad_core::{
    commands::{Request, Session},
    document::{Document, EntityData, EntityId, Mutation},
};
use multicad_model::{Compiler, EvaluationCache, Model, is_superseded};
use std::{
    collections::BTreeSet,
    sync::atomic::{AtomicUsize, Ordering},
};

/// The complete published snapshot in an exact textual form (f64 values
/// print round-trip exactly).
fn exact(model: &Model) -> String {
    format!("{:?}", model.snapshot())
}
fn cold(document: &Document, revision: u64) -> Result<String, String> {
    Compiler::default()
        .compile(document, revision)
        .map(|m| exact(&m))
        .map_err(|e| e.to_string())
}
fn cached(
    cache: &mut EvaluationCache,
    document: &Document,
    revision: u64,
) -> Result<String, String> {
    Compiler::default()
        .compile_cached(cache, document, revision, None)
        .map(|m| exact(&m))
        .map_err(|e| e.to_string())
}

#[track_caller]
fn same(actual: &Result<String, String>, expected: &Result<String, String>, context: &str) {
    match (actual, expected) {
        (Ok(actual), Ok(expected)) if actual != expected => {
            let at = actual
                .bytes()
                .zip(expected.bytes())
                .position(|(a, b)| a != b)
                .unwrap_or(actual.len().min(expected.len()));
            let window = |s: &str| {
                let start = at.saturating_sub(400).min(s.len());
                s[start..(at + 300).min(s.len())].to_string()
            };
            panic!(
                "{context}: cached and cold snapshots differ at byte {at}\ncached: {}\ncold:   {}",
                window(actual),
                window(expected)
            );
        }
        (actual, expected) => assert_eq!(actual, expected, "{context}"),
    }
}

fn edit_parameter(session: &mut Session, name: &str, expression: &str) -> Result<(), String> {
    let mut entity = session
        .document()
        .entities()
        .find(|e| e.name == name && matches!(e.data, EntityData::Parameter { .. }))
        .ok_or_else(|| format!("Missing parameter {name}"))?
        .clone();
    let EntityData::Parameter { expression: e, .. } = &mut entity.data else {
        unreachable!()
    };
    *e = expression.into();
    let prepared = session
        .prepare(Request {
            expected_revision: session.revision(),
            actor: large::actor(),
            label: "Edit parameter".into(),
            edits: vec![Mutation::Replace(entity)],
        })
        .map_err(|e| e.to_string())?;
    session.commit(prepared).map_err(|e| e.to_string())?;
    Ok(())
}

fn features(document: &Document) -> BTreeSet<EntityId> {
    document
        .entities()
        .filter(|e| matches!(e.data, EntityData::Feature { .. }))
        .map(|e| e.id)
        .collect()
}

/// First compile fills the cache and equals a cold compile; an unchanged
/// recompile re-runs only uncacheable providers; a parameter edit equals a
/// cold compile of the edited document and reuses at least `reused_floor`
/// features. Returns (evaluated, reused) of the edit rebuild.
fn scenario_edit(
    mut session: Session,
    name: &str,
    parameter: &str,
    value: &str,
    reused_floor: usize,
) -> (usize, usize) {
    let mut cache = EvaluationCache::default();
    let (document, revision) = (session.document().clone(), session.revision());
    same(
        &cached(&mut cache, &document, revision),
        &cold(&document, revision),
        name,
    );
    let first = cache.last_compile().clone();
    assert!(first.reused.is_empty(), "{name}: empty cache reused");
    same(
        &cached(&mut cache, &document, revision),
        &cold(&document, revision),
        name,
    );
    let again = cache.last_compile().clone();
    assert_eq!(
        again.evaluated, first.uncacheable,
        "{name}: an unchanged recompile re-ran a cacheable feature"
    );
    edit_parameter(&mut session, parameter, value).unwrap();
    let (edited, revision) = (session.document().clone(), session.revision());
    same(
        &cached(&mut cache, &edited, revision),
        &cold(&edited, revision),
        name,
    );
    let edit = cache.last_compile().clone();
    assert_eq!(
        edit.evaluated
            .union(&edit.reused)
            .copied()
            .collect::<BTreeSet<_>>(),
        features(&edited),
        "{name}: every feature is either evaluated or reused"
    );
    assert!(
        edit.reused.len() >= reused_floor,
        "{name}: reused {} < {reused_floor}",
        edit.reused.len()
    );
    eprintln!(
        "{name}: edit {parameter}={value} evaluated {} reused {} uncacheable {}",
        edit.evaluated.len(),
        edit.reused.len(),
        edit.uncacheable.len()
    );
    (edit.evaluated.len(), edit.reused.len())
}

#[test]
fn makerspace_edit_reuses_unaffected_features() {
    scenario_edit(
        makerspace::build().unwrap().session,
        "makerspace",
        "room_width",
        "9600",
        3,
    );
}
#[test]
fn cabinetry_table_edit_reuses_unaffected_features() {
    scenario_edit(
        cabinetry_table::build().unwrap().session,
        "cabinetry_table",
        "table_width",
        "1800",
        8,
    );
}
#[test]
fn tool_case_edit_reuses_unaffected_features() {
    scenario_edit(
        tool_case::build().unwrap().session,
        "tool_case",
        "rail_pitch",
        "80",
        16,
    );
}
#[test]
fn control_desk_edit_reuses_unaffected_features() {
    let sources = tempfile::tempdir().unwrap();
    scenario_edit(
        control_desk::build(sources.path()).unwrap().session,
        "control_desk",
        "desk_width",
        "280",
        7,
    );
}

#[test]
fn large_scenario_edits_reevaluate_only_affected_features() {
    let s = large::build(4, 6).unwrap();
    let (evaluated, reused) = scenario_edit(s.session, "large 4x6", "row_2_width", "97", 0);
    // Box and Boolean of the row's 6 plates; every Cylinder and every other
    // row is reused.
    assert_eq!((evaluated, reused), (12, 60));
    let s = large::build(4, 6).unwrap();
    let (evaluated, reused) = scenario_edit(s.session, "large 4x6", "hole_radius", "7", 0);
    // Every Cylinder and Boolean; the Boxes are reused.
    assert_eq!((evaluated, reused), (48, 24));
}

/// Deterministic xorshift so a failure is reproducible from its seed.
struct Random(u64);
impl Random {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

/// Random parameter edits (including values providers reject), undo/redo to
/// earlier documents and repeated compiles: after every step the cached
/// compile equals a cold compile of the same document, including errors.
fn randomized(mut session: Session, name: &str, seed: u64, steps: usize) {
    let mut random = Random(seed);
    let mut cache = EvaluationCache::default();
    let mut history = vec![session.document().clone()];
    let mut revision = session.revision();
    let numeric: Vec<(String, f64)> = session
        .document()
        .entities()
        .filter_map(|e| match &e.data {
            EntityData::Parameter { expression, .. } => expression
                .trim()
                .parse::<f64>()
                .ok()
                .map(|v| (e.name.clone(), v)),
            _ => None,
        })
        .collect();
    assert!(!numeric.is_empty(), "{name}: no numeric parameters");
    let mut current = session.document().clone();
    let mut edits = 0;
    for step in 0..steps {
        let action = random.below(10);
        let context = format!("{name} seed {seed:#x} step {step} action {action}");
        if action < 7 {
            let (parameter, base) = &numeric[random.below(numeric.len())];
            let factor = [0.9, 0.95, 1.05, 1.1, 1.2, 0.0, -1.0][random.below(7)];
            let value = (base * factor).round();
            // The session refuses some values; the document is then unchanged.
            if edit_parameter(&mut session, parameter, &format!("{value}")).is_ok() {
                edits += 1;
                history.push(session.document().clone());
                current = session.document().clone();
                revision = session.revision();
            }
        } else if action < 9 {
            current = history[random.below(history.len())].clone();
            revision += 1;
        }
        same(
            &cached(&mut cache, &current, revision),
            &cold(&current, revision),
            &context,
        );
    }
    assert!(edits > 0, "{name}: no edit applied");
}

#[test]
fn randomized_edits_match_cold_compiles_on_the_large_scenario() {
    for seed in [0x9e37_79b9_7f4a_7c15, 0x2545_f491_4f6c_dd1d] {
        randomized(large::build(3, 4).unwrap().session, "large 3x4", seed, 24);
    }
}
#[test]
fn randomized_edits_match_cold_compiles_on_the_cabinetry_table() {
    randomized(
        cabinetry_table::build().unwrap().session,
        "cabinetry_table",
        0x5851_f42d_4c95_7f2d,
        8,
    );
}
#[test]
fn randomized_edits_match_cold_compiles_on_the_tool_case() {
    randomized(
        tool_case::build().unwrap().session,
        "tool_case",
        0x1405_7b7e_f767_814f,
        6,
    );
}

/// An interrupted cached compile reports supersession, and the next cached
/// compile (which may reuse results stored before the interruption) equals
/// a cold compile: no partial result is ever reused.
#[test]
fn interrupted_cached_compiles_never_leave_stale_results() {
    let mut s = large::build(3, 4).unwrap();
    let mut cache = EvaluationCache::default();
    let count = AtomicUsize::new(0);
    let probe = || {
        count.fetch_add(1, Ordering::Relaxed);
        false
    };
    let (document, revision) = (s.session.document().clone(), s.session.revision());
    Compiler::default()
        .compile_cached(&mut cache, &document, revision, Some(&probe))
        .unwrap();
    let total = count.into_inner();
    assert!(total > 8, "{total}");
    let mut interrupted = 0;
    for (step, stop) in (1..=total).step_by((total / 8).max(1)).enumerate() {
        edit_parameter(
            &mut s.session,
            &large::row_parameter(step % 3),
            &format!("{}", 80 + step),
        )
        .unwrap();
        let (document, revision) = (s.session.document().clone(), s.session.revision());
        let calls = AtomicUsize::new(0);
        let interrupt = || calls.fetch_add(1, Ordering::Relaxed) + 1 >= stop;
        let result =
            Compiler::default().compile_cached(&mut cache, &document, revision, Some(&interrupt));
        let context = format!("interrupt at {stop} (cold compile has {total} boundaries)");
        match result {
            Err(error) => {
                assert!(is_superseded(&error), "{context}: {error}");
                interrupted += 1;
            }
            // Reused features skip per-feature boundaries, so a late stop
            // can lie beyond the end of a cached compile.
            Ok(model) => {
                assert!(calls.into_inner() < stop, "{context}: ignored");
                same(&Ok(exact(&model)), &cold(&document, revision), &context);
            }
        }
        same(
            &cached(&mut cache, &document, revision),
            &cold(&document, revision),
            &format!("after {context}"),
        );
    }
    assert!(
        interrupted >= 4,
        "only {interrupted} compiles were interrupted"
    );
}

/// Entries used by the latest compile are kept; older entries are evicted
/// to the retained-size bound. A toggled edit reuses retained entries.
#[test]
fn eviction_keeps_the_latest_compile_and_bounds_older_entries() {
    let mut s = large::build(2, 3).unwrap();
    let original = s.session.document().clone();
    edit_parameter(&mut s.session, &large::row_parameter(0), "95").unwrap();
    let documents = [original, s.session.document().clone()];
    // No retained budget: only the latest compile's entries survive.
    let mut tight = EvaluationCache::new(0, usize::MAX);
    let mut roomy = EvaluationCache::default();
    for (step, index) in [0, 1, 0, 1].into_iter().enumerate() {
        let revision = step as u64 + 1;
        let document = &documents[index];
        same(
            &cached(&mut tight, document, revision),
            &cold(document, revision),
            "tight",
        );
        let stats = tight.last_compile().clone();
        let used = stats.reused.len() + stats.evaluated.len() - stats.uncacheable.len();
        assert_eq!(tight.len(), used, "tight step {step}");
        assert_eq!(stats.entries, used);
        if step > 0 {
            // The edited row's Box and Boolean are re-evaluated and their
            // previous entries evicted.
            assert_eq!(stats.evaluated.len(), 6, "tight step {step}");
            assert_eq!(stats.evicted, 6, "tight step {step}");
        }
        same(
            &cached(&mut roomy, document, revision),
            &cold(document, revision),
            "roomy",
        );
        let stats = roomy.last_compile();
        // With room, toggling back to an earlier document reuses everything.
        let expected = match step {
            0 => features(document).len(),
            1 => 6,
            _ => 0,
        };
        assert_eq!(stats.evaluated.len(), expected, "roomy step {step}");
        assert_eq!(stats.evicted, 0);
    }
    assert_eq!(roomy.len(), features(&documents[0]).len() + 6);
    assert!(roomy.estimated_bytes() > tight.estimated_bytes());
}
