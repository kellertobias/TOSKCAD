//! Real pinned mechanical Library roles remain associative connection sources.
#[path = "../../multicad-mcp/tests/support/screw_connection.rs"]
mod fixture;
use multicad_core::{
    commands::{Actor, Channel, Request, Session},
    constraint_sketch::Dimension,
    document::{Document, Entity, EntityData, EntityId, Mutation, Placement},
    model::*,
    parameters::BoundExpression,
    project_file::{self, ProjectFile},
    symbols::{
        self, SymbolAuthorRequest, SymbolDefinition, SymbolEditRequest, SymbolKind,
        SymbolPlaceRequest, SymbolValue,
    },
};
use multicad_model::{Compiler, EvaluationCache, Model};
use std::collections::BTreeMap;
fn actor() -> Actor {
    Actor::local(Channel::Mcp)
}
fn compiler() -> Compiler {
    Compiler {
        omit_display: true,
        ..Default::default()
    }
}
fn author(
    mut parameters: MechanicalPartParameters,
    dynamic: bool,
) -> (Document, SymbolDefinition, Option<EntityId>) {
    let mut s = Session::new(Document::new("Authored connection Library source").unwrap()).unwrap();
    let parameter = if dynamic {
        let e = Entity::new(
            "washer_thickness",
            EntityData::Parameter {
                expression: "0.2".into(),
                unit: "mm".into(),
                exposed: true,
                inputs: vec![],
            },
        );
        let id = e.id;
        s.apply(Request {
            expected_revision: 0,
            actor: actor(),
            label: "Author washer input".into(),
            edits: vec![Mutation::Insert(e)],
        })
        .unwrap();
        let MechanicalPartParameters::Washer { thickness, .. } = &mut parameters else {
            unreachable!()
        };
        *thickness = Dimension::Expression {
            expression: BoundExpression::bind(s.document(), "=$washer_thickness", "mm", None)
                .unwrap(),
        };
        Some(id)
    } else {
        None
    };
    let part = MechanicalPartRequest::new("Authored mechanical provider", parameters);
    let q = prepare_mechanical_part(&s, s.revision(), actor(), &part).unwrap();
    s.commit(q.prepared).unwrap();
    let request = SymbolAuthorRequest::new(
        "Test-authored exact mechanical pin",
        part.body,
        if dynamic {
            SymbolKind::Dynamic
        } else {
            SymbolKind::Static
        },
        parameter.into_iter().collect(),
        None,
    );
    let q = symbols::prepare_author(&s, s.revision(), actor(), &request).unwrap();
    s.commit(q.prepared).unwrap();
    let definition = symbols::read_definition(s.document(), request.definition).unwrap();
    (s.document().clone(), definition, parameter)
}
fn library_row(
    definition: &SymbolDefinition,
    role: ScrewConnectionHardwareRole,
    name: &str,
) -> ScrewConnectionHardwareRequest {
    let placement = SymbolPlaceRequest::from_definition(
        EntityId::new(),
        definition,
        name,
        BTreeMap::new(),
        Placement::default(),
    )
    .unwrap();
    ScrewConnectionHardwareRequest {
        role_id: EntityId::new(),
        role,
        name: name.into(),
        instance: placement.instance,
        driver: EntityId::new(),
        source: ScrewConnectionHardwareSource::Library {
            definition: Box::new(definition.clone()),
            placement: Box::new(placement),
        },
    }
}
fn placement(row: &ScrewConnectionHardwareRequest) -> &SymbolPlaceRequest {
    let ScrewConnectionHardwareSource::Library { placement, .. } = &row.source else {
        unreachable!()
    };
    placement
}
fn leaf(native: &Model, instance: EntityId) -> &multicad_model::LeafOccurrenceSnapshot {
    let leaves = native
        .snapshot()
        .leaf_occurrences
        .values()
        .filter(|leaf| leaf.path.root == instance)
        .collect::<Vec<_>>();
    assert_eq!(
        leaves.len(),
        1,
        "A Library role is one actual physical mechanical leaf"
    );
    leaves[0]
}
fn assert_native(native: &Model, request: &ScrewConnectionRequest, washer: f64) -> RigidPose {
    assert_eq!(native.snapshot().leaf_occurrences.len(), 4);
    let [check] = native.snapshot().screw_connection_checks.as_slice() else {
        panic!("One full native connection receipt")
    };
    assert_eq!(check.connection, request.connection);
    assert!(check.native_component_collision_checked);
    assert!((check.actual_engagement_mm.unwrap() - 2.4).abs() < 1e-7);
    assert!((check.grip_mm - 4.1).abs() < 1e-7);
    let screw = leaf(native, request.hardware[0].instance);
    let ring = leaf(native, request.hardware[1].instance);
    let nut = leaf(native, request.hardware[2].instance);
    assert!((screw.pose.translation_mm[2] - (4.1 + washer)).abs() < 1e-7);
    assert!((ring.pose.translation_mm[2] - (4.1 + washer)).abs() < 1e-7);
    assert!(nut.pose.translation_mm[2].abs() < 1e-7);
    assert!(
        native
            .leaf_solid(&screw.path)
            .unwrap()
            .boolean(
                native.leaf_solid(&nut.path).unwrap(),
                multicad_kernel::BooleanOperation::Intersect,
                Default::default()
            )
            .unwrap()
            .is_none()
    );
    nut.pose
}
fn reopened(document: &Document) -> Document {
    let mut bytes = vec![];
    project_file::write(
        &ProjectFile::new(document.clone()).unwrap(),
        &mut bytes,
        Default::default(),
    )
    .unwrap();
    project_file::read(
        &mut bytes.as_slice(),
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap()
    .project
    .document
}
#[test]
fn pinned_metric_nut_and_dynamic_library_washer_follow_overrides_with_native_proof_and_reopen() {
    let (document, mut request, _) = fixture::fixture();
    let ScrewConnectionHardwareSource::Generated { part } = &request.hardware[1].source else {
        unreachable!()
    };
    let nut_parameters = part.parameters.clone();
    let (nut_author, nut_definition, _) = author(nut_parameters.clone(), false);
    let (washer_author, washer_definition, thickness) = author(
        MechanicalPartParameters::Washer {
            bore_diameter: length(3.2),
            outside_diameter: length(7.),
            thickness: length(0.2),
            frame: Default::default(),
        },
        true,
    );
    let thickness = thickness.unwrap();
    let nut_author_bytes = reopened(&nut_author);
    let washer_author_bytes = reopened(&washer_author);
    request.hardware[1] = library_row(
        &nut_definition,
        ScrewConnectionHardwareRole::Nut,
        "Pinned 6H nut",
    );
    request.hardware.insert(
        1,
        library_row(
            &washer_definition,
            ScrewConnectionHardwareRole::HeadWasher,
            "Pinned parametric washer",
        ),
    );
    let ring = placement(&request.hardware[1]).clone();
    let nut = placement(&request.hardware[2]).clone();
    let mut s = Session::new(document).unwrap();
    let before = s.document().clone();
    let q = prepare_screw_connection(&s, 0, actor(), &request).unwrap();
    let native = compiler().compile(q.prepared.document(), 1).unwrap();
    let initial_pose = assert_native(&native, &request, 0.2);
    let initial_hash = native.snapshot().stamp.source_hash;
    drop(native);
    assert_eq!(s.document(), &before, "Preview is source-neutral");
    s.commit(q.prepared).unwrap();
    let source = s.document().get(request.connection).unwrap().clone();
    let initial = s.document().clone();
    let ring_before = symbols::read_instance(&initial, ring.instance).unwrap();
    let nut_before = symbols::read_instance(&initial, nut.instance).unwrap();
    assert_eq!(ring_before.pin, washer_definition.pin().unwrap());
    assert_eq!(nut_before.pin, nut_definition.pin().unwrap());
    assert_eq!(ring_before.ids, ring.ids);
    assert_eq!(nut_before.ids, nut.ids);
    assert_ne!(ring.ids[&thickness], thickness);
    assert!(
        symbols::owned_entities(&initial)
            .unwrap()
            .contains(&ring.ids[&thickness])
    );
    let reread = read_screw_connection_request(&initial, request.connection).unwrap();
    let ResolvedMechanicalPart::HexNut { thread, .. } =
        resolve_screw_connection_hardware(&initial, s.parameters(), &reread.hardware[2]).unwrap()
    else {
        unreachable!()
    };
    assert!(thread.metric_fit.is_some());
    let EntityData::Part { features, .. } =
        &initial.get(nut.ids[&nut_definition.root]).unwrap().data
    else {
        unreachable!()
    };
    let MechanicalPartParameters::HexNut {
        thread: authored, ..
    } = read_mechanical_part(&initial, features[0])
        .unwrap()
        .parameters
    else {
        unreachable!()
    };
    assert_eq!(authored.fit, ThreadFit::Metric6HInternal);
    assert_eq!(thread.representation, ThreadRepresentation::Modeled);
    let on_disk = reopened(&initial);
    assert_eq!(on_disk, initial);
    let mut cache = EvaluationCache::default();
    assert_native(
        &compiler()
            .compile_cached(&mut cache, &on_disk, s.revision(), None)
            .unwrap(),
        &request,
        0.2,
    );
    let edit = SymbolEditRequest {
        record: ring.instance,
        overrides: BTreeMap::from([(
            thickness,
            SymbolValue {
                value: 0.3,
                unit: "mm".into(),
            },
        )]),
    };
    let q = symbols::prepare_edit(&s, s.revision(), actor(), &edit).unwrap();
    let after_native = compiler()
        .compile_cached(&mut cache, q.prepared.document(), s.revision() + 1, None)
        .unwrap();
    assert_ne!(
        assert_native(&after_native, &request, 0.3),
        initial_pose,
        "Current washer thickness changes the actual mating phase"
    );
    assert_ne!(after_native.snapshot().stamp.source_hash, initial_hash);
    drop(after_native);
    assert_eq!(s.document(), &initial);
    s.commit(q.prepared).unwrap();
    assert_eq!(
        s.document().get(request.connection),
        Some(&source),
        "No connection recomposition"
    );
    let ring_after = symbols::read_instance(s.document(), ring.instance).unwrap();
    assert_eq!(ring_after.pin, ring_before.pin);
    assert_eq!(ring_after.ids, ring_before.ids);
    assert_eq!(
        symbols::read_instance(s.document(), nut.instance).unwrap(),
        nut_before
    );
    let changed = initial
        .entities()
        .filter(|e| s.document().get(e.id) != Some(*e))
        .map(|e| e.id)
        .collect::<Vec<_>>();
    assert_eq!(
        changed.len(),
        2,
        "Only override record and mapped scalar may change: {changed:?}"
    );
    assert!(changed.contains(&ring.record));
    assert!(changed.contains(&ring.ids[&thickness]));
    assert_eq!(initial.entities().count(), s.document().entities().count());
    let final_document = reopened(s.document());
    assert_eq!(&final_document, s.document());
    assert_native(
        &compiler()
            .compile_cached(&mut cache, &final_document, s.revision(), None)
            .unwrap(),
        &request,
        0.3,
    );
    // A private mapped ID is not a public exposed source key.
    let bad_private = SymbolEditRequest {
        record: ring.instance,
        overrides: BTreeMap::from([(
            ring.ids[&thickness],
            SymbolValue {
                value: 0.4,
                unit: "mm".into(),
            },
        )]),
    };
    assert!(symbols::prepare_edit(&s, s.revision(), actor(), &bad_private).is_err());
    // A well-typed override can still make native geometry invalid; never commit that candidate.
    let bad_geometry = SymbolEditRequest {
        record: ring.instance,
        overrides: BTreeMap::from([(
            thickness,
            SymbolValue {
                value: -0.1,
                unit: "mm".into(),
            },
        )]),
    };
    let invalid = symbols::prepare_edit(&s, s.revision(), actor(), &bad_geometry).unwrap();
    assert!(
        compiler()
            .compile(invalid.prepared.document(), s.revision() + 1)
            .is_err()
    );
    assert_eq!(s.document(), &final_document);
    let mut incompatible_parameters = nut_parameters;
    let MechanicalPartParameters::HexNut {
        thread,
        across_flats,
    } = &mut incompatible_parameters
    else {
        unreachable!()
    };
    thread.major_diameter = length(6.);
    thread.pitch = length(1.);
    thread.length = length(4.);
    thread.fit = ThreadFit::Metric6HInternal;
    thread.metric_fit_engagement = Some(MetricFitEngagement::NormalGroupAssumed {});
    *across_flats = length(10.);
    let (_, incompatible_definition, _) = author(incompatible_parameters, false);
    let mut incompatible = request.clone();
    incompatible.hardware[2] = library_row(
        &incompatible_definition,
        ScrewConnectionHardwareRole::Nut,
        "Incompatible supported M6 pin",
    );
    let scratch = Session::new(before.clone()).unwrap();
    let candidate = prepare_screw_connection(&scratch, 0, actor(), &incompatible).unwrap();
    let failure = compiler()
        .compile(candidate.prepared.document(), 1)
        .err()
        .expect("Different sourced nominal/pitch must not mate");
    assert!(
        failure.to_string().contains("mating")
            || failure.to_string().contains("nominal")
            || failure.to_string().contains("pitch"),
        "{failure}"
    );
    assert_eq!(scratch.document(), &before);
    assert_eq!(s.document(), &final_document);
    assert_eq!(nut_author, nut_author_bytes);
    assert_eq!(washer_author, washer_author_bytes);
    assert_eq!(symbols::read_definition(&nut_author,nut_author.entities().find(|e| matches!(&e.data,EntityData::Extension{definition,..} if definition.type_id==symbols::DEFINITION_TYPE)).unwrap().id).unwrap(),nut_definition);
}
