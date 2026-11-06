use multicad_core::{commands::Session, document::Document, model::*};

fn session() -> Session {
    Session::new(Document::new("Screw length contract").unwrap()).unwrap()
}
fn typed(overall: bool, value: f64) -> MechanicalPartParameters {
    let mut p = default_machine_screw();
    let MechanicalPartParameters::MachineScrew {
        underhead_length,
        length_input,
        ..
    } = &mut p
    else {
        unreachable!()
    };
    *underhead_length = None;
    *length_input = Some(if overall {
        ScrewLengthInput::Overall {
            value: length(value),
        }
    } else {
        ScrewLengthInput::Underhead {
            value: length(value),
        }
    });
    p
}
#[test]
fn sole_length_authority_and_physical_extent() {
    let s = session();
    for (recipe, expected, convention) in [
        (
            default_machine_screw(),
            10.,
            ScrewLengthConvention::Underhead,
        ),
        (typed(false, 10.), 10., ScrewLengthConvention::Underhead),
        (typed(true, 13.), 10., ScrewLengthConvention::Overall),
    ] {
        let ResolvedMechanicalPart::MachineScrew {
            underhead_length_mm,
            length_convention,
            ..
        } = recipe.resolve(s.parameters()).unwrap()
        else {
            unreachable!()
        };
        assert_eq!(underhead_length_mm, expected);
        assert_eq!(length_convention, convention);
    }
    for (legacy, input) in [
        (None, None),
        (
            Some(length(10.)),
            Some(ScrewLengthInput::Overall { value: length(13.) }),
        ),
    ] {
        let mut p = default_machine_screw();
        let MechanicalPartParameters::MachineScrew {
            underhead_length,
            length_input,
            ..
        } = &mut p
        else {
            unreachable!()
        };
        *underhead_length = legacy;
        *length_input = input;
        assert!(p.validate().is_err());
        assert!(p.resolve(s.parameters()).is_err());
    }
    assert!(typed(true, 3.).resolve(s.parameters()).is_err());
    assert!(
        typed(true, 12.).resolve(s.parameters()).is_err(),
        "Thread length must still fit physical underhead span"
    );
}
#[test]
fn legacy_serialization_unchanged_and_new_source_unambiguous() {
    let legacy = serde_json::to_value(default_machine_screw()).unwrap();
    assert!(legacy.get("length_input").is_none());
    assert_eq!(
        legacy["underhead_length"],
        serde_json::to_value(length(10.)).unwrap()
    );
    let round: MechanicalPartParameters = serde_json::from_value(legacy.clone()).unwrap();
    assert_eq!(serde_json::to_value(round).unwrap(), legacy);
    let new = serde_json::to_value(typed(true, 13.)).unwrap();
    assert!(new.get("underhead_length").is_none());
    assert_eq!(new["length_input"]["kind"], "overall");
    let restored: MechanicalPartParameters = serde_json::from_value(new.clone()).unwrap();
    assert_eq!(serde_json::to_value(restored).unwrap(), new);
}

#[test]
fn typed_length_dependency_capability_and_history_are_retained() {
    use multicad_core::{
        commands::{Actor, Channel, Request},
        constraint_sketch::Dimension,
        document::{Entity, EntityData, Mutation},
        parameters::BoundExpression,
        project_file::{self, ProjectFile},
    };
    let mut s = session();
    let actor = Actor::local(Channel::Mcp);
    let parameter = Entity::new(
        "overall",
        EntityData::Parameter {
            expression: "13".into(),
            unit: "mm".into(),
            exposed: true,
            inputs: vec![],
        },
    );
    let parameter_id = parameter.id;
    s.apply(Request {
        expected_revision: 0,
        actor: actor.clone(),
        label: "Length".into(),
        edits: vec![Mutation::Insert(parameter)],
    })
    .unwrap();
    let mut p = typed(true, 13.);
    let MechanicalPartParameters::MachineScrew { length_input, .. } = &mut p else {
        unreachable!()
    };
    *length_input = Some(ScrewLengthInput::Overall {
        value: Dimension::Expression {
            expression: BoundExpression::bind(s.document(), "=$overall", "mm", None).unwrap(),
        },
    });
    let request = MechanicalPartRequest::new("Bound overall screw", p.clone());
    let prepared = prepare_mechanical_part(&s, s.revision(), actor.clone(), &request).unwrap();
    s.commit(prepared.prepared).unwrap();
    let EntityData::Feature { inputs, .. } = &s.document().get(request.feature).unwrap().data
    else {
        unreachable!()
    };
    assert!(inputs.iter().any(|link| link.target == parameter_id));
    let project = ProjectFile::new(s.document().clone()).unwrap();
    assert!(
        project
            .capabilities()
            .contains(MACHINE_SCREW_LENGTH_INPUT_CAPABILITY)
    );
    let mut bytes = vec![];
    project_file::write(&project, &mut bytes, Default::default()).unwrap();
    let mut old = project_file::supported_capabilities();
    old.remove(MACHINE_SCREW_LENGTH_INPUT_CAPABILITY);
    assert!(
        project_file::read(&mut bytes.as_slice(), Default::default(), &old)
            .unwrap_err()
            .contains(MACHINE_SCREW_LENGTH_INPUT_CAPABILITY)
    );
    let reopened = project_file::read(
        &mut bytes.as_slice(),
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap()
    .project;
    assert_eq!(
        read_mechanical_part(&reopened.document, request.feature)
            .unwrap()
            .parameters,
        p
    );
    let saved = s.document().clone();
    s.undo(s.revision(), actor.clone()).unwrap();
    s.redo(s.revision(), actor).unwrap();
    assert_eq!(s.document(), &saved);
}

#[test]
fn all_nineteen_shipped_legacy_recipes_preserve_files_pins_and_source_json() {
    use multicad_core::{
        project_file::{self, ContentHash},
        symbols,
    };
    let library = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../library");
    let catalog: serde_json::Value = serde_json::from_slice(
        &std::fs::read(library.join("machine-screws-curation.json")).unwrap(),
    )
    .unwrap();
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(library.join("library.json")).unwrap()).unwrap();
    let entries = catalog["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 19);
    for entry in entries {
        let name = entry["file"].as_str().unwrap();
        let bytes = std::fs::read(library.join(name)).unwrap();
        let manifest_row = manifest["entries"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["file"] == name)
            .unwrap();
        assert_eq!(
            ContentHash::of(&bytes).to_string(),
            manifest_row["sha256"].as_str().unwrap()
        );
        let project = project_file::read(
            &mut bytes.as_slice(),
            Default::default(),
            &project_file::supported_capabilities(),
        )
        .unwrap()
        .project;
        assert!(
            !project
                .capabilities()
                .contains(MACHINE_SCREW_LENGTH_INPUT_CAPABILITY)
        );
        for (id, definition) in symbols::definitions(&project.document).unwrap() {
            let before = definition.pin().unwrap();
            let canonical = definition.definition().unwrap();
            let clone = project.document.clone();
            // Re-decoding the stored immutable definition must preserve its content pin.
            let original = symbols::read_definition(&clone, id).unwrap();
            assert_eq!(original.pin().unwrap(), before);
            assert_eq!(canonical, original.definition().unwrap());
            for entity in definition.template.entities() {
                if let multicad_core::document::EntityData::Feature { definition, .. } =
                    &entity.data
                    && definition.type_id == MECHANICAL_PART_TYPE
                {
                    let source: MechanicalPartDefinition =
                        serde_json::from_value(definition.data.clone()).unwrap();
                    assert_eq!(
                        serde_json::to_value(source).unwrap(),
                        definition.data,
                        "{name} legacy source changed"
                    );
                }
            }
        }
    }
}
