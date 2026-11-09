use multicad_core::{
    document::{Document, EntityId},
    model::{
        BodyInput, CutTargetProbe, CutTargetProbeResult, CutTargetQuery, CutTargetSelection,
        EvaluationOptions, MAX_CUT_TARGET_PROBES, ModelEvaluationInfo, ModelQueryError,
    },
    project_file::ContentHash,
};
use std::collections::BTreeSet;
fn query() -> CutTargetQuery {
    CutTargetQuery {
        tools: vec![EntityId::new()],
        component: None,
        include: vec![],
        exclude: vec![],
    }
}
fn probe() -> CutTargetProbe {
    CutTargetProbe {
        id: EntityId::new(),
        query: query(),
    }
}
#[test]
fn correlation_validation_bounds_work_and_rejects_identity_aliasing_before_dispatch() {
    let maximum = EvaluationOptions {
        cut_targets: (0..MAX_CUT_TARGET_PROBES).map(|_| probe()).collect(),
        ..Default::default()
    };
    maximum.validate().unwrap();
    let wire = serde_json::to_vec(&maximum).unwrap();
    let roundtrip: EvaluationOptions = serde_json::from_slice(&wire).unwrap();
    roundtrip.validate().unwrap();
    assert_eq!(roundtrip, maximum);
    let mut duplicate = maximum.clone();
    duplicate.cut_targets[1].id = duplicate.cut_targets[0].id;
    assert_eq!(
        duplicate.validate().unwrap_err().entity,
        Some(duplicate.cut_targets[0].id)
    );
    let mut nil = maximum.clone();
    nil.cut_targets[0].id = EntityId::parse("00000000-0000-0000-0000-000000000000").unwrap();
    assert!(nil.validate().is_err());
    let mut overflow = maximum.clone();
    overflow.cut_targets.push(probe());
    assert_eq!(overflow.validate().unwrap_err().entity, None);
    assert_eq!(serde_json::to_vec(&maximum).unwrap(), wire);
}
#[test]
fn empty_options_need_no_probe_and_query_semantics_remain_native_responsibility() {
    let empty = EvaluationOptions::default();
    empty.validate().unwrap();
    assert!(empty.cut_targets.is_empty());
    assert_eq!(
        serde_json::from_str::<EvaluationOptions>("{}").unwrap(),
        empty
    );
    let mut p = probe();
    p.query.tools.clear();
    p.query.include = vec![EntityId::new()];
    p.query.exclude = p.query.include.clone();
    // Core validates job correlation only: it cannot decide native body
    // availability or geometric intersection, including exclusion precedence.
    EvaluationOptions {
        cut_targets: vec![p],
        ..Default::default()
    }
    .validate()
    .unwrap();
}
#[test]
fn mixed_probe_receipts_preserve_source_frozen_producers_and_explicit_failure() {
    let document = Document::new("Receipt source").unwrap();
    let hash = ContentHash::document(&document).unwrap();
    let defaults = BodyInput {
        body: EntityId::new(),
        producer: EntityId::new(),
    };
    let overridden = BodyInput {
        body: EntityId::new(),
        producer: EntityId::new(),
    };
    let miss = EntityId::new();
    let tool = EntityId::new();
    let ok = EntityId::new();
    let failed = EntityId::new();
    let info = ModelEvaluationInfo {
        screw_connection_checks: vec![],
        source_hash: hash,
        consumed_bodies: BTreeSet::from([tool]),
        cut_targets: vec![
            CutTargetProbeResult {
                id: ok,
                result: Ok(CutTargetSelection {
                    defaults: vec![defaults.clone()],
                    targets: vec![overridden.clone()],
                    not_intersecting: vec![miss],
                }),
            },
            CutTargetProbeResult {
                id: failed,
                result: Err(ModelQueryError {
                    entity: Some(tool),
                    message: "Tool is no longer a current producer".into(),
                }),
            },
        ],
    };
    let wire = serde_json::to_vec(&info).unwrap();
    let receipt: ModelEvaluationInfo = serde_json::from_slice(&wire).unwrap();
    assert_eq!(receipt, info);
    let selection = receipt
        .cut_targets
        .iter()
        .find(|r| r.id == ok)
        .unwrap()
        .result
        .as_ref()
        .unwrap();
    assert_eq!(selection.defaults, vec![defaults]);
    assert_eq!(selection.targets, vec![overridden]);
    assert_eq!(selection.not_intersecting, vec![miss]);
    assert!(
        receipt
            .cut_targets
            .iter()
            .find(|r| r.id == failed)
            .unwrap()
            .result
            .is_err()
    );
    let mut changed = document.clone();
    changed.rename("New source").unwrap();
    assert_ne!(
        ContentHash::document(&changed).unwrap(),
        receipt.source_hash
    );
    assert_eq!(ContentHash::document(&document).unwrap(), hash);
}
#[test]
fn unknown_wire_fields_are_rejected_at_request_and_receipt_boundaries() {
    let options = EvaluationOptions {
        cut_targets: vec![probe()],
        ..Default::default()
    };
    let mut wire = serde_json::to_value(&options).unwrap();
    wire["cut_targets"][0]["query"]["nearest_body"] = true.into();
    assert!(serde_json::from_value::<EvaluationOptions>(wire).is_err());
    let error = ModelQueryError {
        entity: None,
        message: "Unsupported query".into(),
    };
    let mut wire = serde_json::to_value(&error).unwrap();
    wire["fallback"] = true.into();
    assert!(serde_json::from_value::<ModelQueryError>(wire).is_err());
    let info = ModelEvaluationInfo {
        screw_connection_checks: vec![],
        source_hash: ContentHash::of(b"source"),
        consumed_bodies: BTreeSet::new(),
        cut_targets: vec![],
    };
    let mut wire = serde_json::to_value(&info).unwrap();
    wire["guessed"] = true.into();
    assert!(serde_json::from_value::<ModelEvaluationInfo>(wire).is_err());
}

#[test]
fn receipt_budgets_allow_shared_producers_explicit_overrides_and_large_nonhit_expansions() {
    use multicad_core::model::{
        MAX_BOOLEAN_TARGETS, MAX_CUT_NON_INTERSECTING, MAX_CUT_TARGET_POOL,
    };
    let producer = EntityId::new();
    let make_body = || BodyInput {
        body: EntityId::new(),
        producer,
    };
    let selection = CutTargetSelection {
        defaults: (0..MAX_CUT_TARGET_POOL).map(|_| make_body()).collect(),
        targets: (0..MAX_BOOLEAN_TARGETS).map(|_| make_body()).collect(),
        not_intersecting: (0..MAX_CUT_NON_INTERSECTING)
            .map(|_| EntityId::new())
            .collect(),
    };
    let info = ModelEvaluationInfo {
        screw_connection_checks: vec![],
        source_hash: ContentHash::of(b"receipt source"),
        consumed_bodies: BTreeSet::new(),
        cut_targets: vec![CutTargetProbeResult {
            id: EntityId::new(),
            result: Ok(selection),
        }],
    };
    // Defaults and final targets are deliberately disjoint: excludes can
    // remove defaults while explicit includes supply other intersecting Parts.
    info.validate().unwrap();
    for list in 0..3 {
        let mut invalid = info.clone();
        let selection = invalid.cut_targets[0].result.as_mut().unwrap();
        match list {
            0 => selection.defaults.push(make_body()),
            1 => selection.targets.push(make_body()),
            _ => selection.not_intersecting.push(EntityId::new()),
        }
        assert!(invalid.validate().is_err());
    }
    let mut duplicate = info.clone();
    let selection = duplicate.cut_targets[0].result.as_mut().unwrap();
    selection.targets[1].body = selection.targets[0].body;
    assert!(duplicate.validate().is_err());
    let mut invalid = info.clone();
    let target = &mut invalid.cut_targets[0].result.as_mut().unwrap().targets[0];
    target.producer = target.body;
    assert!(invalid.validate().is_err());
    let mut duplicate = info.clone();
    duplicate.cut_targets.push(duplicate.cut_targets[0].clone());
    assert!(duplicate.validate().is_err());
    let nil = EntityId::parse("00000000-0000-0000-0000-000000000000").unwrap();
    let mut invalid = info.clone();
    invalid.cut_targets[0].result.as_mut().unwrap().targets[0].producer = nil;
    assert!(invalid.validate().is_err());
    let mut invalid = info;
    invalid.consumed_bodies.insert(nil);
    assert!(invalid.validate().is_err());
}
#[test]
fn failed_probe_receipt_can_report_the_nil_input_without_becoming_current_geometry() {
    let nil = EntityId::parse("00000000-0000-0000-0000-000000000000").unwrap();
    let error = ModelQueryError {
        entity: Some(nil),
        message: "Tool producer ID must be non-nil".into(),
    };
    assert!(error.to_string().contains(&nil.to_string()));
    let info = ModelEvaluationInfo {
        screw_connection_checks: vec![],
        source_hash: ContentHash::of(b"failed source"),
        consumed_bodies: BTreeSet::new(),
        cut_targets: vec![CutTargetProbeResult {
            id: EntityId::new(),
            result: Err(error),
        }],
    };
    info.validate().unwrap();
    let roundtrip: ModelEvaluationInfo =
        serde_json::from_slice(&serde_json::to_vec(&info).unwrap()).unwrap();
    assert!(roundtrip.cut_targets[0].result.is_err());
}
