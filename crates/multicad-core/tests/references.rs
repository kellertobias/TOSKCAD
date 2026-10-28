use multicad_core::{commands::*, document::*, references::*};
use std::collections::{BTreeMap, BTreeSet};

fn source(name: &str, inputs: Vec<Link>) -> Entity {
    Entity::new(
        name,
        EntityData::Feature {
            definition: Definition {
                type_id: "test.feature".into(),
                schema_version: 1,
                data: serde_json::json!({}),
            },
            inputs,
            suppressed: false,
        },
    )
}
fn origin(source: EntityId, role: &str) -> Origin {
    Origin {
        source,
        role: role.into(),
    }
}
fn frame(z: f64) -> ReferenceFrame {
    ReferenceFrame {
        origin_mm: [0., 0., z],
        u: [1., 0., 0.],
        v: [0., 1., 0.],
    }
}
fn face(ordinal: u32, origin: &Origin, z: f64) -> TopologyElement {
    TopologyElement {
        geometry: None,
        key: TopologyKey {
            kind: TopologyKind::Face,
            ordinal,
        },
        origins: BTreeSet::from([origin.clone()]),
        boundary: BTreeSet::new(),
        frames: vec![AnchoredFrame {
            origin: origin.clone(),
            frame: frame(z),
        }],
    }
}
fn attached(snapshot: &TopologySnapshot) -> GeometryIntent {
    let mut intent = GeometryIntent::capture(snapshot, snapshot.elements[0].key).unwrap();
    intent.attachment = Some(Attachment {
        anchor: snapshot.elements[0].origins.first().unwrap().clone(),
        offset_mm: [5., 7., 2.],
        angle_rad: std::f64::consts::FRAC_PI_2,
    });
    intent
}
#[test]
fn source_identity_survives_ordinal_reordering_and_attachment_is_not_accumulated() {
    let id = EntityId::new();
    let top = origin(id, "cap.end");
    let mut snapshot = TopologySnapshot {
        owner: id,
        revision: 0,
        elements: vec![face(5, &top, 10.)],
    };
    let intent = attached(&snapshot);
    for (revision, height, ordinal) in [(0, 10., 5), (1, 20., 1), (2, 40., 9)] {
        snapshot.revision = revision;
        snapshot.elements = vec![face(ordinal, &top, height)];
        let Resolution::Resolved {
            key,
            frame: Some(frame),
            ..
        } = resolve(&intent, &snapshot, revision)
        else {
            panic!("source must remain attached")
        };
        assert_eq!(key.ordinal, ordinal);
        assert_eq!(frame.origin_mm, [5., 7., height + 2.]);
        assert!((frame.u[1] - 1.).abs() < 1e-12);
        assert!((frame.v[0] + 1.).abs() < 1e-12);
    }
    assert!(matches!(
        resolve(&intent, &snapshot, 1),
        Resolution::Stale {
            snapshot_revision: 2
        }
    ));
}
#[test]
fn split_deleted_and_geometrically_identical_unrelated_faces_do_not_silently_rebind() {
    let id = EntityId::new();
    let top = origin(id, "cap.end");
    let mut snapshot = TopologySnapshot {
        owner: id,
        revision: 0,
        elements: vec![face(0, &top, 10.)],
    };
    let intent = attached(&snapshot);
    snapshot.elements.push(face(1, &top, 10.));
    assert_eq!(
        resolve(&intent, &snapshot, 0),
        Resolution::Ambiguous {
            candidates: snapshot.elements.iter().map(|e| e.key).collect()
        }
    );
    assert!(GeometryIntent::capture(&snapshot, snapshot.elements[0].key).is_err());
    snapshot.elements = vec![face(0, &origin(EntityId::new(), "cap.end"), 10.)];
    assert_eq!(resolve(&intent, &snapshot, 0), Resolution::Missing);
    snapshot.elements.clear();
    assert_eq!(resolve(&intent, &snapshot, 0), Resolution::Missing);
}
fn fixture() -> (Session, TopologySnapshot, ReferenceAddress, EntityId) {
    let feature = source("Panel", vec![]);
    let top = origin(feature.id, "cap.end");
    let snapshot = TopologySnapshot {
        owner: feature.id,
        revision: 0,
        elements: vec![face(0, &top, 18.)],
    };
    let intent = attached(&snapshot);
    let mut link = Link::new(feature.id, "face");
    link.geometry = Some(intent.clone());
    let plane = Entity::new(
        "Face plane",
        EntityData::Plane {
            origin_mm: [0.; 3],
            normal: [0., 0., 1.],
            anchor: Some(link),
        },
    );
    let hole = source("Dependent hole", vec![Link::new(plane.id, "plane")]);
    let address = ReferenceAddress {
        owner: plane.id,
        reference: intent.id,
    };
    let mut doc = Document::new("References").unwrap();
    for entity in [feature, plane, hole.clone()] {
        doc.insert(entity).unwrap();
    }
    (Session::new(doc).unwrap(), snapshot, address, hole.id)
}
#[test]
fn repair_is_explicit_previewable_attributable_and_one_undoable_transaction() {
    let (mut session, mut snapshot, address, hole) = fixture();
    let before = session.document().clone();
    let mut fragment = snapshot.elements[0].clone();
    fragment.key.ordinal = 7;
    fragment
        .boundary
        .insert(origin(EntityId::new(), "cut.wall"));
    snapshot.elements.push(fragment.clone());
    let report = inspect(
        session.document(),
        0,
        &BTreeMap::from([(snapshot.owner, snapshot.clone())]),
    );
    let diag = report.get(address).unwrap();
    assert!(diag.affected.contains(&hole));
    assert!(matches!(diag.resolution, Resolution::Ambiguous { .. }));
    let actor = Actor {
        id: "external-designer".into(),
        channel: Channel::Mcp,
    };
    let preview = prepare_repair(
        &session,
        0,
        actor.clone(),
        address,
        &snapshot,
        fragment.key,
        None,
    )
    .unwrap();
    assert!(preview.affected.contains(&hole));
    assert_eq!(session.document(), &before);
    drop(preview);
    assert!(!session.can_undo());
    let preview = prepare_repair(
        &session,
        0,
        actor.clone(),
        address,
        &snapshot,
        fragment.key,
        None,
    )
    .unwrap();
    let stale = preview.prepared.clone();
    session.commit(preview.prepared).unwrap();
    let after = session.document().clone();
    snapshot.revision = 1;
    let report = inspect(
        &after,
        1,
        &BTreeMap::from([(snapshot.owner, snapshot.clone())]),
    );
    assert!(
        matches!(report.get(address).unwrap().resolution, Resolution::Resolved { key, .. } if key == fragment.key)
    );
    assert_eq!(
        report.get(address).unwrap().intent.boundary.as_ref(),
        Some(&fragment.boundary)
    );
    assert_eq!(report.get(address).unwrap().intent.id, address.reference);
    assert_eq!(session.activity().next().unwrap().actor, actor);
    assert_eq!(
        session.commit(stale).unwrap_err().code,
        ErrorCode::StaleRevision
    );
    session.undo(1, Actor::local(Channel::Ui)).unwrap();
    assert_eq!(session.document(), &before);
    session.redo(2, Actor::local(Channel::Ui)).unwrap();
    assert_eq!(session.document(), &after);
    assert!(
        prepare_repair(
            &session,
            3,
            Actor::local(Channel::Ui),
            address,
            &snapshot,
            fragment.key,
            None
        )
        .is_err()
    );
}
#[test]
fn repair_rejects_wrong_kind_missing_frames_and_indistinguishable_fragments() {
    let (session, mut snapshot, address, _) = fixture();
    let key = snapshot.elements[0].key;
    snapshot.elements[0].frames.clear();
    assert!(
        prepare_repair(
            &session,
            0,
            Actor::local(Channel::Ui),
            address,
            &snapshot,
            key,
            None
        )
        .is_err()
    );
    snapshot.elements[0].key.kind = TopologyKind::Edge;
    assert!(
        prepare_repair(
            &session,
            0,
            Actor::local(Channel::Ui),
            address,
            &snapshot,
            snapshot.elements[0].key,
            None
        )
        .is_err()
    );
    snapshot.elements[0].key = key;
    let mut duplicate = snapshot.elements[0].clone();
    duplicate.key.ordinal = 1;
    snapshot.elements.push(duplicate);
    assert!(
        prepare_repair(
            &session,
            0,
            Actor::local(Channel::Ui),
            address,
            &snapshot,
            key,
            None
        )
        .is_err()
    );
    assert_eq!(session.revision(), 0);
}
#[test]
fn provenance_and_diagnostics_roundtrip_and_legacy_links_still_load() {
    let (session, snapshot, address, _) = fixture();
    let loaded: Document =
        serde_json::from_slice(&serde_json::to_vec(session.document()).unwrap()).unwrap();
    let restored: TopologySnapshot =
        serde_json::from_slice(&serde_json::to_vec(&snapshot).unwrap()).unwrap();
    assert_eq!(snapshot, restored);
    let report = inspect(&loaded, 0, &BTreeMap::from([(snapshot.owner, restored)]));
    let restored: ReferenceReport =
        serde_json::from_slice(&serde_json::to_vec(&report).unwrap()).unwrap();
    assert_eq!(report, restored);
    assert!(report.all_resolved());
    assert!(report.get(address).is_some());
    let legacy: Link =
        serde_json::from_value(serde_json::json!({"target": snapshot.owner, "role": "source"}))
            .unwrap();
    assert!(legacy.geometry.is_none());
}
#[test]
fn unresolved_attempt_keeps_last_valid_presentation_and_stale_report_cannot_replace_it() {
    let (session, snapshot, _, _) = fixture();
    let mut gate = ReferenceGate::default();
    let report = inspect(
        session.document(),
        0,
        &BTreeMap::from([(snapshot.owner, snapshot)]),
    );
    assert!(gate.consider(0, "valid mesh", report).unwrap());
    let report = inspect(session.document(), 1, &BTreeMap::new());
    assert!(!gate.consider(1, "broken mesh", report.clone()).unwrap());
    assert_eq!(gate.visible(), Some(&"valid mesh"));
    assert_eq!(gate.report(), Some(&report));
    assert!(gate.consider(2, "stale", report).is_err());
    assert_eq!(gate.visible(), Some(&"valid mesh"));
}
#[test]
fn canonical_document_rejects_duplicate_reference_ids_and_invalid_attachment_values() {
    let (session, snapshot, address, _) = fixture();
    let intent = attached(&snapshot);
    let mut entity = source("Invalid", vec![]);
    let EntityData::Feature { inputs, .. } = &mut entity.data else {
        unreachable!()
    };
    let mut link = Link::new(snapshot.owner, "source");
    link.geometry = Some(intent);
    inputs.extend([link.clone(), link.clone()]);
    let mut doc = session.document().clone();
    assert!(doc.insert(entity).is_err());
    let mut plane = doc.get(address.owner).unwrap().clone();
    let EntityData::Plane {
        anchor: Some(anchor),
        ..
    } = &mut plane.data
    else {
        unreachable!()
    };
    anchor
        .geometry
        .as_mut()
        .unwrap()
        .attachment
        .as_mut()
        .unwrap()
        .offset_mm[0] = f64::NAN;
    assert!(doc.replace(plane).is_err());
    let mut invalid = snapshot.clone();
    invalid.elements[0].frames[0].frame.u = [0.; 3];
    assert!(invalid.validate().is_err());
    invalid = snapshot;
    let duplicate = invalid.elements[0].frames[0].clone();
    invalid.elements[0].frames.push(duplicate);
    assert!(invalid.validate().is_err());
}
