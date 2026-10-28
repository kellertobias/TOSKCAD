#[path = "../../multicad-app/examples/parameter_workflow_review.rs"]
mod review;
use multicad_core::{
    commands::{Channel, ErrorCode},
    constraint_sketch::source::Source,
    document::{Entity, EntityData, EntityId, Mutation},
    parameters::{ParameterErrorCode, Unit},
    project_file::ContentHash,
};
use multicad_model::Compiler;
fn edited(f: &review::Fixture, id: EntityId, change: impl FnOnce(&mut Entity)) -> Mutation {
    let mut entity = f.session.document().get(id).unwrap().clone();
    change(&mut entity);
    Mutation::Replace(entity)
}
fn expression(f: &review::Fixture, id: EntityId, value: &str) -> Mutation {
    edited(f, id, |e| {
        let EntityData::Parameter { expression, .. } = &mut e.data else {
            panic!("parameter")
        };
        *expression = value.into();
    })
}
fn order(f: &review::Fixture) -> Vec<EntityId> {
    let EntityData::Extension { definition, .. } = &f.session.document().get(f.table).unwrap().data
    else {
        panic!("order")
    };
    serde_json::from_value(definition.data.clone()).unwrap()
}
fn canonical(f: &review::Fixture) -> Source {
    Source::read_for_evaluation(f.session.document(), f.sketch, f.session.revision(), None)
        .unwrap()
        .0
}
fn near(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-5, "{a} != {b}");
}
#[test]
fn table_metadata_and_stable_bindings_drive_fully_constrained_sketch_and_exact_native_solid() {
    let mut f = review::seed().unwrap();
    review::verify(&f, [120., 80., 8.]).unwrap();
    let original = canonical(&f);
    let source_feature = f.session.document().get(f.output.feature).unwrap().clone();
    let ids = review::compile(&f).unwrap().snapshot().bodies.clone();
    // Authored guesses intentionally differ from the genuine solved dimensions.
    assert_eq!(original.sketch.points[1].position_mm, [60., 0.]);
    let edits = vec![
        expression(&f, f.width, "150"),
        expression(&f, f.height, "100"),
        expression(&f, f.stock, "8"),
    ];
    review::commit_checked(&mut f, "Change driving values", edits).unwrap();
    review::verify(&f, [150., 100., 10.]).unwrap();
    assert_eq!(canonical(&f), original);
    assert_eq!(
        f.session.document().get(f.output.feature),
        Some(&source_feature)
    );
    assert_eq!(review::compile(&f).unwrap().snapshot().bodies, ids);
    let metadata = edited(&f, f.width, |e| {
        e.name = "span".into();
        e.description = "Original span in centimeters".into();
        let EntityData::Parameter {
            expression,
            unit,
            exposed,
            ..
        } = &mut e.data
        else {
            panic!()
        };
        *expression = "15".into();
        *unit = "cm".into();
        *exposed = false;
    });
    let stock = edited(&f, f.stock, |e| e.name = "material_thickness".into());
    let decoy = Entity::new(
        "width",
        EntityData::Parameter {
            expression: "999".into(),
            unit: "mm".into(),
            exposed: true,
            inputs: vec![],
        },
    );
    let new_order = [f.depth, f.height, f.stock, f.width, decoy.id];
    let edits = vec![
        metadata,
        stock,
        Mutation::Insert(decoy.clone()),
        review::order_mutation(f.session.document(), f.table, &new_order).unwrap(),
    ];
    review::commit_checked(&mut f, "Rename, units, exposure and row order", edits).unwrap();
    review::verify(&f, [150., 100., 10.]).unwrap();
    assert_eq!(order(&f), new_order);
    assert_eq!(canonical(&f), original);
    assert_eq!(
        f.session.document().get(f.output.feature),
        Some(&source_feature)
    );
    let width = f.session.document().get(f.width).unwrap();
    assert_eq!(width.name, "span");
    assert_eq!(width.description, "Original span in centimeters");
    let EntityData::Parameter { unit, exposed, .. } = &width.data else {
        panic!()
    };
    assert_eq!(unit, "cm");
    assert!(!exposed);
    near(
        f.session
            .parameters()
            .value(f.width, Unit::Millimeter)
            .unwrap(),
        150.,
    );
    near(
        f.session
            .parameters()
            .value(decoy.id, Unit::Millimeter)
            .unwrap(),
        999.,
    );
    let edits = vec![expression(&f, f.width, "20")];
    review::commit_checked(&mut f, "Edit renamed source", edits).unwrap();
    review::verify(&f, [200., 100., 10.]).unwrap();
    assert_eq!(canonical(&f), original);
    // Deleting an unused same-name row is safe and never steals an old binding.
    let remaining = [f.depth, f.height, f.stock, f.width];
    let edits = vec![
        Mutation::Remove(decoy.id),
        review::order_mutation(f.session.document(), f.table, &remaining).unwrap(),
    ];
    review::commit_checked(&mut f, "Delete unused table row", edits).unwrap();
    review::verify(&f, [200., 100., 10.]).unwrap();
    assert_eq!(order(&f), remaining);
}
#[test]
fn source_invalid_and_discarded_native_preflight_candidates_preserve_committed_source() {
    let mut f = review::seed().unwrap();
    f.session.mark_saved(f.session.revision()).unwrap();
    let before = f.session.document().clone();
    let revision = f.session.revision();
    let hash = ContentHash::document(&before).unwrap();
    for (id, value, code) in [
        (
            f.depth,
            "=$missing + 1 mm",
            ParameterErrorCode::MissingReference,
        ),
        (f.width, "1 mm + 1 deg", ParameterErrorCode::Dimension),
        (f.width, "1 / 0", ParameterErrorCode::NonFinite),
    ] {
        let result = f.session.prepare(review::request(
            &f.session,
            "Reject invalid expression",
            vec![expression(&f, id, value)],
        ));
        let error = result.unwrap_err();
        assert_eq!(error.code, ErrorCode::InvalidParameter);
        assert_eq!(error.parameter.unwrap().code, code);
        assert_eq!(f.session.document(), &before);
    }
    let cycle = f.session.prepare(review::request(
        &f.session,
        "Reject cycle",
        vec![
            expression(&f, f.width, "=$height"),
            expression(&f, f.height, "=$width"),
        ],
    ));
    assert_eq!(
        cycle.unwrap_err().parameter.unwrap().code,
        ParameterErrorCode::Cycle
    );
    // Source-only preparation is separate from native geometry acceptance.
    // A valid parameter number can still make a dimensional sketch invalid.
    for edits in [
        vec![expression(&f, f.width, "-10")],
        vec![edited(&f, f.width, |e| {
            let EntityData::Parameter { unit, .. } = &mut e.data else {
                panic!()
            };
            *unit = "deg".into();
        })],
    ] {
        let candidate = f
            .session
            .prepare(review::request(
                &f.session,
                "Native-invalid parameter candidate",
                edits,
            ))
            .unwrap();
        assert!(
            Compiler::default()
                .compile(candidate.document(), revision)
                .is_err()
        );
        drop(candidate);
        assert_eq!(f.session.document(), &before);
    }
    let replacement = Entity::new(
        "width",
        EntityData::Parameter {
            expression: "999".into(),
            unit: "mm".into(),
            exposed: false,
            inputs: vec![],
        },
    );
    let candidate = f
        .session
        .prepare(review::request(
            &f.session,
            "Removed binding must not adopt replacement name",
            vec![
                Mutation::Remove(f.width),
                Mutation::Insert(replacement.clone()),
            ],
        ))
        .unwrap();
    assert!(
        Compiler::default()
            .compile(candidate.document(), revision)
            .is_err()
    );
    assert_ne!(replacement.id, f.width);
    drop(candidate);
    let preview = f
        .session
        .prepare(review::request(
            &f.session,
            "Discard valid but unapplied resize",
            vec![expression(&f, f.width, "180")],
        ))
        .unwrap();
    let preview_model = Compiler::default()
        .compile(preview.document(), revision)
        .unwrap();
    near(
        preview_model
            .solid(f.output.feature)
            .unwrap()
            .volume_mm3()
            .unwrap(),
        180. * 80. * 8.,
    );
    drop(preview_model);
    drop(preview);
    assert_eq!(f.session.document(), &before);
    assert_eq!(f.session.revision(), revision);
    assert!(!f.session.dirty());
    assert_eq!(ContentHash::document(f.session.document()).unwrap(), hash);
    review::verify(&f, [120., 80., 8.]).unwrap();
    let archived = review::archive(f.session.document()).unwrap();
    let reopened = review::reopen(&archived).unwrap();
    assert_eq!(reopened.document(), &before);
    let model = Compiler::default()
        .compile(reopened.document(), reopened.revision())
        .unwrap();
    near(
        model.solid(f.output.feature).unwrap().volume_mm3().unwrap(),
        120. * 80. * 8.,
    );
}
#[test]
fn undo_redo_save_and_reopen_preserve_parameter_rows_expressions_and_geometry_identities() {
    let mut f = review::seed().unwrap();
    let before = f.session.document().clone();
    let original_source = canonical(&f);
    let old_order = order(&f);
    let edit = edited(&f, f.width, |e| {
        e.name = "panel_span".into();
        e.description = "Exposed span in inches".into();
        let EntityData::Parameter {
            expression,
            unit,
            exposed,
            ..
        } = &mut e.data
        else {
            panic!()
        };
        *expression = "5".into();
        *unit = "in".into();
        *exposed = true;
    });
    let new_order = [f.depth, f.stock, f.width, f.height];
    let edits = vec![
        edit,
        expression(&f, f.stock, "8"),
        review::order_mutation(f.session.document(), f.table, &new_order).unwrap(),
    ];
    review::commit_checked(&mut f, "Atomic parameter edits", edits).unwrap();
    let after = f.session.document().clone();
    review::verify(&f, [127., 80., 10.]).unwrap();
    let changed = f.session.activity().last().unwrap();
    assert!(changed.change.invalidated.contains(&f.sketch));
    assert!(changed.change.invalidated.contains(&f.output.feature));
    f.session
        .undo(f.session.revision(), review::actor())
        .unwrap();
    assert_eq!(f.session.document(), &before);
    assert_eq!(order(&f), old_order);
    review::verify(&f, [120., 80., 8.]).unwrap();
    f.session
        .redo(f.session.revision(), review::actor())
        .unwrap();
    assert_eq!(f.session.document(), &after);
    assert_eq!(order(&f), new_order);
    review::verify(&f, [127., 80., 10.]).unwrap();
    assert_eq!(canonical(&f), original_source);
    let bytes = review::archive(f.session.document()).unwrap();
    let reopened = review::reopen(&bytes).unwrap();
    assert_eq!(reopened.document(), &after);
    let model = Compiler::default().compile(reopened.document(), 0).unwrap();
    assert_eq!(model.snapshot().bodies[&f.output.body], f.output.feature);
    near(
        model.solid(f.output.feature).unwrap().volume_mm3().unwrap(),
        127. * 80. * 10.,
    );
    let source = Source::read_for_evaluation(reopened.document(), f.sketch, 0, None)
        .unwrap()
        .0;
    assert_eq!(source, original_source);
    let EntityData::Parameter {
        expression,
        exposed,
        ..
    } = &reopened.document().get(f.depth).unwrap().data
    else {
        panic!()
    };
    assert_eq!(expression, "=$stock + 2 mm");
    assert!(!exposed);
    assert!(
        f.session
            .activity()
            .all(|entry| entry.actor.channel == Channel::CommandLine)
    );
}
#[test]
fn review_generator_writes_checked_editable_projects_without_overwriting_existing_files() {
    let tmp = tempfile::tempdir().unwrap();
    let directory = tmp.path().join("review");
    let manifest = review::generate(&directory).unwrap();
    let projects = manifest["projects"].as_array().unwrap();
    assert_eq!(projects.len(), 3);
    let mut ids = vec![];
    for (project, expected) in
        projects
            .iter()
            .zip([120. * 80. * 8., 150. * 100. * 10., 150. * 100. * 10.])
    {
        let bytes = std::fs::read(directory.join(project["file"].as_str().unwrap())).unwrap();
        let reopened = review::reopen(&bytes).unwrap();
        let feature: EntityId = serde_json::from_value(project["feature"].clone()).unwrap();
        let model = Compiler::default().compile(reopened.document(), 0).unwrap();
        near(
            model.solid(feature).unwrap().volume_mm3().unwrap(),
            expected,
        );
        ids.push((reopened.document().id(), feature));
    }
    assert!(ids.windows(2).all(|ids| ids[0] == ids[1]));
    let file = directory.join("manifest.json");
    let before = std::fs::read(&file).unwrap();
    assert!(review::generate(&directory).is_err());
    assert_eq!(std::fs::read(file).unwrap(), before);
}
#[test]
fn source_valid_parameter_edits_remain_repairable_without_relabeling_stale_native_geometry() {
    for case in 0..3 {
        let mut f = review::seed().unwrap();
        let original = f.session.document().clone();
        let old = review::compile(&f).unwrap();
        let identities = old.snapshot().bodies.clone();
        let edits = match case {
            0 => vec![expression(&f, f.width, "-10")],
            1 => vec![edited(&f, f.width, |e| {
                let EntityData::Parameter { unit, .. } = &mut e.data else {
                    panic!()
                };
                *unit = "deg".into();
            })],
            _ => vec![
                Mutation::Remove(f.width),
                Mutation::Insert(Entity::new(
                    "width",
                    EntityData::Parameter {
                        expression: "999".into(),
                        unit: "mm".into(),
                        exposed: false,
                        inputs: vec![],
                    },
                )),
            ],
        };
        let mut request = review::request(
            &f.session,
            "Apply source-valid change requiring geometry repair",
            edits,
        );
        request.actor = multicad_core::commands::Actor::local(Channel::Ui);
        let change = f.session.apply(request).unwrap();
        assert!(change.invalidated.contains(&f.sketch));
        assert!(change.invalidated.contains(&f.output.feature));
        // Production parameter Apply commits valid source independently of a
        // solve. A failed worker must keep its old native result stamped old.
        f.session.document().validate().unwrap();
        assert!(review::compile(&f).is_err());
        assert_ne!(old.snapshot().stamp.revision, f.session.revision());
        assert_ne!(
            old.snapshot().stamp.source_hash,
            ContentHash::document(f.session.document()).unwrap()
        );
        near(
            old.solid(f.output.feature).unwrap().volume_mm3().unwrap(),
            120. * 80. * 8.,
        );
        let damaged = f.session.document().clone();
        let archive = review::archive(&damaged).unwrap();
        let reopened = review::reopen(&archive).unwrap();
        assert_eq!(reopened.document(), &damaged);
        assert!(
            Compiler::default()
                .compile(reopened.document(), reopened.revision())
                .is_err()
        );
        f.session
            .undo(f.session.revision(), review::actor())
            .unwrap();
        assert_eq!(f.session.document(), &original);
        review::verify(&f, [120., 80., 8.]).unwrap();
        assert_eq!(review::compile(&f).unwrap().snapshot().bodies, identities);
        f.session
            .redo(f.session.revision(), review::actor())
            .unwrap();
        assert_eq!(f.session.document(), &damaged);
        assert!(review::compile(&f).is_err());
        f.session
            .undo(f.session.revision(), review::actor())
            .unwrap();
        review::verify(&f, [120., 80., 8.]).unwrap();
        assert_eq!(f.session.document(), &original);
    }
}
