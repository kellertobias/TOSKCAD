use multicad_core::{
    commands::{Actor, Channel, Session},
    document::{Document, Entity, EntityData, EntityId},
    inspection::*,
    project_file::{self, ProjectFile},
    references::{AnalyticGeometry, GeometryIntent, Origin, ReferenceFrame, TopologyKind},
};
fn actor() -> Actor {
    Actor::local(Channel::Mcp)
}
fn reference(source: EntityId, role: &str) -> GeometryReference {
    GeometryReference {
        source,
        intent: GeometryIntent {
            id: EntityId::new(),
            kind: TopologyKind::Face,
            origins: [Origin {
                source: EntityId::new(),
                role: role.into(),
            }]
            .into(),
            boundary: None,
            attachment: None,
        },
    }
}
fn fixture() -> (Session, PinRequest) {
    let mut d = Document::new("Pinned measures").unwrap();
    let a = Entity::new(
        "Cabinet",
        EntityData::Part {
            features: vec![],
            material: None,
            tags: Default::default(),
        },
    );
    let id = a.id;
    d.insert(a).unwrap();
    let query = MeasurementQuery {
        a: reference(id, "left"),
        b: reference(id, "right"),
    };
    (Session::new(d).unwrap(), PinRequest::new("Width", query))
}
#[test]
fn pin_is_source_neutral_then_one_undo_without_driving_parameter_or_feature_edits() {
    let (mut s, r) = fixture();
    let before = s.document().clone();
    let p = prepare_pin(&s, 0, actor(), &r).unwrap();
    assert_eq!(s.document(), &before);
    let foreign = Session::new(before.clone()).unwrap();
    assert!(foreign.validate_preview(&p).is_err());
    s.commit(p).unwrap();
    let pin = read(s.document(), r.id).unwrap();
    assert_eq!(pin.query, r.query);
    assert_eq!(
        s.document()
            .entities()
            .filter(|e| matches!(
                e.data,
                EntityData::Feature { .. } | EntityData::Parameter { .. }
            ))
            .count(),
        0
    );
    let mut edit = r.clone();
    edit.label_offset_mm = [10., 20., 0.];
    edit.visible = false;
    let stale = prepare_pin(&s, 0, actor(), &edit).unwrap_err();
    assert!(stale.to_string().contains("revision"));
    let p = prepare_pin(&s, s.revision(), actor(), &edit).unwrap();
    s.commit(p).unwrap();
    assert!(!read(s.document(), r.id).unwrap().visible);
    s.undo(s.revision(), actor()).unwrap();
    assert!(read(s.document(), r.id).unwrap().visible);
    s.undo(s.revision(), actor()).unwrap();
    assert_eq!(s.document(), &before);
}
#[test]
fn native_container_reopen_keeps_link_owned_references_and_missing_sources_remain_repairable() {
    let (mut s, r) = fixture();
    s.commit(prepare_pin(&s, 0, actor(), &r).unwrap()).unwrap();
    let mut d = s.document().clone();
    d.remove(r.query.a.source).unwrap();
    d.validate().unwrap();
    let definition = read(&d, r.id).unwrap();
    assert_eq!(definition.query, r.query);
    let project = ProjectFile::new(d.clone()).unwrap();
    assert!(project.capabilities().contains(CAPABILITY));
    let mut bytes = vec![];
    project_file::write(&project, &mut bytes, Default::default()).unwrap();
    let loaded = project_file::read(
        &mut bytes.as_slice(),
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap();
    assert_eq!(read(&loaded.project.document, r.id).unwrap(), definition);
    let mut supported = project_file::supported_capabilities();
    supported.remove(CAPABILITY);
    assert!(project_file::read(&mut bytes.as_slice(), Default::default(), &supported).is_err());
}
#[test]
fn invalid_duplicate_capture_and_wrong_source_kind_fail_without_source_changes() {
    let (s, mut r) = fixture();
    let before = s.document().clone();
    r.query.b.intent.id = r.query.a.intent.id;
    assert!(prepare_pin(&s, 0, actor(), &r).is_err());
    r.query.b.intent.id = EntityId::new();
    r.label_offset_mm[0] = f64::INFINITY;
    assert!(prepare_pin(&s, 0, actor(), &r).is_err());
    assert_eq!(s.document(), &before);
    let (mut s, mut r) = fixture();
    let p = Entity::new("Not geometry", EntityData::Group {});
    let id = p.id;
    let mut d = s.document().clone();
    d.insert(p).unwrap();
    s = Session::new(d).unwrap();
    r.query.b.source = id;
    assert!(prepare_pin(&s, 0, actor(), &r).is_err());
}
#[test]
fn stamp_checks_same_document_revision_with_changed_bytes_and_removal_is_undoable() {
    let (mut s, r) = fixture();
    let stamp = MeasurementStamp::capture(s.document(), 0).unwrap();
    let mut d = s.document().clone();
    d.rename("Different bytes").unwrap();
    assert!(stamp.check(&d, 0).is_err());
    s.commit(prepare_pin(&s, 0, actor(), &r).unwrap()).unwrap();
    let p = prepare_remove(&s, s.revision(), actor(), r.id).unwrap();
    s.commit(p).unwrap();
    assert_eq!(pins(s.document()).count(), 0);
    s.undo(s.revision(), actor()).unwrap();
    assert_eq!(read(s.document(), r.id).unwrap().query, r.query);
}
#[test]
fn analytic_angles_and_parallel_supporting_plane_distance_are_distinct_metrics() {
    let xy = ReferenceFrame {
        origin_mm: [0.; 3],
        u: [1., 0., 0.],
        v: [0., 1., 0.],
    };
    let parallel = ReferenceFrame {
        origin_mm: [100., 0., 7.],
        ..xy
    };
    let (angle, separation) = analytic_metrics(
        Some(AnalyticGeometry::Plane { frame: xy }),
        Some(AnalyticGeometry::Plane { frame: parallel }),
    )
    .unwrap();
    assert_eq!(angle, ScalarMetric::Radians { value: 0. });
    assert_eq!(separation, ScalarMetric::Millimeters { value: 7. });
    let line = AnalyticGeometry::Line {
        a_mm: [0.; 3],
        b_mm: [1., 0., 1.],
    };
    let (angle, separation) =
        analytic_metrics(Some(line), Some(AnalyticGeometry::Plane { frame: xy })).unwrap();
    let ScalarMetric::Radians { value } = angle else {
        panic!()
    };
    assert!((value - std::f64::consts::FRAC_PI_4).abs() < 1e-12);
    assert!(matches!(separation, ScalarMetric::Unsupported { .. }));
    assert!(
        analytic_metrics(
            Some(AnalyticGeometry::Line {
                a_mm: [0.; 3],
                b_mm: [0.; 3]
            }),
            None
        )
        .is_err()
    );
}
#[test]
fn model_publication_filters_only_valid_canonical_non_driving_owners_and_keeps_full_diagnostics() {
    use multicad_core::references::{self, ReferenceReport};
    use std::collections::BTreeMap;
    let (mut s, r) = fixture();
    s.commit(prepare_pin(&s, 0, actor(), &r).unwrap()).unwrap();
    let report = references::inspect(s.document(), s.revision(), &BTreeMap::new());
    assert_eq!(report.entries.len(), 2);
    assert!(!report.all_resolved());
    let driving = report.for_model(s.document()).unwrap();
    assert!(driving.all_resolved());
    assert_eq!(report.entries.len(), 2);
    let mut forged = report.clone();
    forged.entries[0].source = EntityId::new();
    assert!(forged.for_model(s.document()).is_err());
    let mut forged = report.clone();
    forged.entries[0].role = "unrelated".into();
    assert!(forged.for_model(s.document()).is_err());
    let mut unclassified = report.entries[0].clone();
    unclassified.address.owner = r.query.a.source;
    let report = ReferenceReport {
        sketch_supports: Default::default(),
        external_results: Default::default(),
        external_errors: Default::default(),
        revision: s.revision(),
        entries: vec![unclassified],
    };
    assert!(!report.for_model(s.document()).unwrap().all_resolved());
    let mut d = s.document().clone();
    let mut pin = d.get(r.id).unwrap().clone();
    let EntityData::Extension { definition, .. } = &mut pin.data else {
        panic!()
    };
    definition.schema_version = 99;
    assert!(
        d.replace(pin).is_err(),
        "Malformed non-driving owner must fail canonical validation"
    );
}
