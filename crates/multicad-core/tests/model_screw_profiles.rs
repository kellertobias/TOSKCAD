use multicad_core::{
    commands::{Actor, Channel, Session},
    document::Document,
    model::*,
};
fn angle(v: f64) -> multicad_core::constraint_sketch::Dimension {
    multicad_core::constraint_sketch::Dimension::Literal {
        value: v,
        unit: "deg".into(),
    }
}
fn session() -> Session {
    Session::new(Document::new("Screw profile contract").unwrap()).unwrap()
}
fn pan() -> ScrewHead {
    ScrewHead::Pan {
        profile: Box::new(CrownedScrewHeadParameters {
            diameter: length(5.6),
            height: length(2.4),
            crown_radius: length(3.),
            top_land_diameter: length(4.2),
            neck_radius: length(0.1),
        }),
    }
}
fn button() -> ScrewHead {
    ScrewHead::Button {
        profile: Box::new(CrownedScrewHeadParameters {
            diameter: length(5.7),
            height: length(1.65),
            crown_radius: length(3.351),
            top_land_diameter: length(3.6),
            neck_radius: length(0.1),
        }),
    }
}
fn csk() -> ScrewHead {
    ScrewHead::Countersunk {
        profile: Box::new(CountersunkScrewHeadParameters {
            diameter: length(5.5),
            height: length(1.65),
            included_angle: angle(90.),
            edge_chamfer: length(0.05),
            neck_radius: length(0.1),
        }),
    }
}
fn torx() -> ScrewDrive {
    ScrewDrive::Hexalobular {
        profile: Box::new(HexalobularDriveParameters {
            major_diameter: length(2.8),
            minor_diameter: length(2.05),
            outer_lobe_radius: length(0.28),
            depth: length(1.14),
            bottom_angle: angle(120.),
        }),
    }
}
fn phillips(h0: bool) -> ScrewDrive {
    ScrewDrive::PhillipsH {
        profile: Box::new(PhillipsHParameters {
            size: if h0 {
                PhillipsHSize::H0
            } else {
                PhillipsHSize::H1
            },
            mouth_diameter: length(if h0 { 1.6 } else { 3.1 }),
            mouth_radius: length(if h0 { 0.3 } else { 0.5 }),
            b_width: length(if h0 { 0.6 } else { 0.96 }),
            g_diameter: length(if h0 { 0.81 } else { 1.27 }),
            tilt_angle: angle(7.),
            outer_flank_angle: angle(26.5),
            floor_angle: angle(28.),
            outer_corner_angle: angle(92.),
            inner: if h0 {
                PhillipsInnerForm::Rounded {
                    radius: length(0.3),
                }
            } else {
                PhillipsInnerForm::ShallowV {
                    angle: angle(138.),
                    e_width: length(0.435),
                }
            },
        }),
    }
}
fn recipe(head: ScrewHead, drive: ScrewDrive) -> MechanicalPartParameters {
    let mut p = default_machine_screw();
    let MechanicalPartParameters::MachineScrew {
        head: h,
        drive: d,
        thread,
        ..
    } = &mut p
    else {
        unreachable!()
    };
    *h = head;
    *d = drive;
    thread.length = length(9.);
    p
}
#[test]
fn all_supported_combinations_resolve_and_preserve_every_typed_field() {
    let s = session();
    for (head, drive) in [
        (pan(), torx()),
        (pan(), phillips(false)),
        (pan(), phillips(true)),
        (button(), torx()),
        (
            button(),
            ScrewDrive::HexSocket {
                across_flats: length(2.55),
                depth: length(1.3),
            },
        ),
        (csk(), torx()),
        (csk(), phillips(false)),
        (
            csk(),
            ScrewDrive::HexSocket {
                across_flats: length(2.55),
                depth: length(1.8),
            },
        ),
    ] {
        let p = recipe(head, drive);
        let json = serde_json::to_value(&p).unwrap();
        assert!(
            json["head"].get("profile").is_none(),
            "Head box must not change wire nesting"
        );
        if json["drive"]["kind"] == "hexalobular" {
            assert!(json["drive"].get("profile").is_none());
        }
        for branch in ["head", "drive"] {
            let mut unknown = json.clone();
            unknown[branch]["unexpected"] = serde_json::json!(true);
            assert!(
                serde_json::from_value::<MechanicalPartParameters>(unknown).is_err(),
                "{branch} must reject unknown fields"
            );
        }

        let restored: MechanicalPartParameters = serde_json::from_value(json.clone()).unwrap();
        assert_eq!(p, restored);
        let ResolvedMechanicalPart::MachineScrew {
            head,
            drive,
            underhead_length_mm,
            thread,
            ..
        } = p.resolve(s.parameters()).unwrap()
        else {
            unreachable!()
        };
        assert!(drive.entry_radius_mm() < head.top_land_radius_mm());
        assert!(drive.removed_depth_mm() < head.height_mm() + underhead_length_mm);
        assert_eq!(thread.length_mm, 9.);
        if let ResolvedScrewDrive::PhillipsH { profile } = drive {
            assert_eq!(profile.outer_corner_angle_deg, 92.);
        }
    }
}
#[test]
fn invalid_profile_dimensions_units_kinds_and_hidden_neck_breakthrough_refuse_atomically() {
    let s = session();
    let before = s.document().clone();
    let mut bad = vec![
        recipe(pan(), ScrewDrive::ExternalHex),
        recipe(
            ScrewHead::ExternalHex {
                across_flats: length(5.5),
                height: length(2.),
            },
            torx(),
        ),
    ];
    let original = recipe(pan(), torx());
    let value = serde_json::to_value(&original).unwrap();
    for (path, v) in [
        ("/head/crown_radius", 1.),
        ("/head/top_land_diameter", 5.7),
        ("/head/neck_radius", 1.1),
        ("/drive/minor_diameter", 3.),
        ("/drive/outer_lobe_radius", 2.),
        ("/drive/depth", 20.),
        ("/drive/bottom_angle", 180.),
    ] {
        let mut vj = value.clone();
        vj.pointer_mut(path).unwrap()["value"] = serde_json::json!(v);
        bad.push(serde_json::from_value(vj).unwrap());
    }
    let mut vj = serde_json::to_value(recipe(csk(), torx())).unwrap();
    vj["head"]["included_angle"]["unit"] = serde_json::json!("mm");
    bad.push(serde_json::from_value(vj).unwrap());
    let mut vj = serde_json::to_value(recipe(pan(), phillips(false))).unwrap();
    vj["drive"]["profile"]["size"] = serde_json::json!("h0");
    bad.push(serde_json::from_value(vj).unwrap());
    for p in bad {
        let request = MechanicalPartRequest::new("Invalid", p);
        assert!(prepare_mechanical_part(&s, 0, Actor::local(Channel::Mcp), &request).is_err());
        assert_eq!(s.document(), &before);
    }
    // A 3mm diameter recess below the conical head would break the 3mm shaft;
    // the same depth as a smaller legitimate socket above is permitted.
    assert!(
        recipe(
            csk(),
            ScrewDrive::HexSocket {
                across_flats: length(2.9),
                depth: length(2.)
            }
        )
        .resolve(s.parameters())
        .is_err()
    );
}
#[test]
fn new_profile_capability_in_references_and_template_only_definitions_refuses_old_readers() {
    use multicad_core::{
        project_file::{self, ProjectFile},
        symbols::{self, SymbolAuthorRequest, SymbolKind},
    };
    let actor = Actor::local(Channel::Mcp);
    let mut s = session();
    let mut request = MechanicalPartRequest::new(
        "Legacy shape with configured profile reference",
        default_machine_screw(),
    );
    request.provenance = Some(MechanicalPartProvenance {
        catalog_id: "engineering-fixture".into(),
        revision: "1".into(),
        designation: "Configured reference only".into(),
        source_url: "https://example.invalid/reference".into(),
        reference_geometry: recipe(pan(), torx()),
        catalog_facts: None,
    });
    let p = prepare_mechanical_part(&s, 0, actor.clone(), &request).unwrap();
    s.commit(p.prepared).unwrap();
    let author = SymbolAuthorRequest::new(
        "Retained profile reference",
        request.body,
        SymbolKind::Static,
        vec![],
        None,
    );
    let p = symbols::prepare_author(&s, s.revision(), actor.clone(), &author).unwrap();
    s.commit(p.prepared).unwrap();
    let definition = symbols::read_definition(s.document(), author.definition).unwrap();
    let mut host = session();
    let p = symbols::prepare_import(&host, 0, actor, &definition).unwrap();
    host.commit(p.prepared).unwrap();
    for document in [s.document(), host.document()] {
        let project = ProjectFile::new(document.clone()).unwrap();
        assert!(project.capabilities().contains(SCREW_PROFILES_CAPABILITY));
        let mut bytes = vec![];
        project_file::write(&project, &mut bytes, Default::default()).unwrap();
        let mut old = project_file::supported_capabilities();
        old.remove(SCREW_PROFILES_CAPABILITY);
        assert!(
            project_file::read(&mut bytes.as_slice(), Default::default(), &old)
                .unwrap_err()
                .contains(SCREW_PROFILES_CAPABILITY)
        );
        let restored = project_file::read(
            &mut bytes.as_slice(),
            Default::default(),
            &project_file::supported_capabilities(),
        )
        .unwrap()
        .project;
        assert_eq!(&restored.document, document);
    }
}
#[test]
fn every_new_dimension_binds_with_its_actual_unit_and_h0_has_no_dormant_v() {
    use multicad_core::{
        commands::Request,
        constraint_sketch::Dimension,
        document::{Entity, EntityData, Mutation},
        parameters::BoundExpression,
    };
    fn collect(value: &serde_json::Value, path: String, paths: &mut Vec<String>) {
        if value.get("value").is_some_and(serde_json::Value::is_number)
            && value.get("unit").is_some()
        {
            paths.push(path);
            return;
        }
        if let Some(map) = value.as_object() {
            for (key, v) in map {
                collect(v, format!("{path}/{key}"), paths);
            }
        }
    }
    for original in [
        recipe(pan(), torx()),
        recipe(csk(), phillips(false)),
        recipe(
            button(),
            ScrewDrive::HexSocket {
                across_flats: length(2.55),
                depth: length(1.3),
            },
        ),
        recipe(pan(), phillips(true)),
    ] {
        let source = serde_json::to_value(&original).unwrap();
        let mut paths = vec![];
        collect(&source["head"], "/head".into(), &mut paths);
        collect(&source["drive"], "/drive".into(), &mut paths);
        for path in paths {
            let mut s = session();
            let d = source.pointer(&path).unwrap();
            let value = d["value"].as_f64().unwrap();
            let unit = d["unit"].as_str().unwrap();
            let parameter = Entity::new(
                "profile_input",
                EntityData::Parameter {
                    expression: value.to_string(),
                    unit: unit.into(),
                    exposed: true,
                    inputs: vec![],
                },
            );
            let id = parameter.id;
            s.apply(Request {
                expected_revision: 0,
                actor: Actor::local(Channel::Mcp),
                label: "Input".into(),
                edits: vec![Mutation::Insert(parameter)],
            })
            .unwrap();
            let bound = Dimension::Expression {
                expression: BoundExpression::bind(s.document(), "=$profile_input", unit, None)
                    .unwrap(),
            };
            let mut data = source.clone();
            *data.pointer_mut(&path).unwrap() = serde_json::to_value(bound).unwrap();
            let p: MechanicalPartParameters = serde_json::from_value(data).unwrap();
            assert_eq!(
                p.resolve(s.parameters()).unwrap(),
                original.resolve(s.parameters()).unwrap(),
                "{path}"
            );
            let request = MechanicalPartRequest::new("Bound profile", p);
            let prepared =
                prepare_mechanical_part(&s, s.revision(), Actor::local(Channel::Mcp), &request)
                    .unwrap();
            s.commit(prepared.prepared).unwrap();
            let EntityData::Feature { inputs, .. } =
                &s.document().get(request.feature).unwrap().data
            else {
                unreachable!()
            };
            assert!(inputs.iter().any(|link| link.target == id), "{path}");
        }
    }
    let s = session();
    let p = recipe(pan(), phillips(true));
    let ResolvedMechanicalPart::MachineScrew {
        drive: ResolvedScrewDrive::PhillipsH { profile },
        ..
    } = p.resolve(s.parameters()).unwrap()
    else {
        unreachable!()
    };
    assert!((profile.derived_e_width_mm() - 0.6 * 46_f64.to_radians().cos()).abs() < 1e-15);
    let json = serde_json::to_value(phillips(true)).unwrap();
    assert!(json["profile"]["inner"].get("angle").is_none());
    assert!(json["profile"]["inner"].get("e_width").is_none());
}
#[test]
fn cards_keep_angle_units_out_of_mm_and_reference_compares_every_new_physical_input() {
    use multicad_core::document::EntityId;
    let s = session();
    let p = recipe(csk(), phillips(false));
    let mut live = session();
    let request = MechanicalPartRequest::new("Configured CSK H", p.clone());
    let prepared = prepare_mechanical_part(&live, 0, Actor::local(Channel::Mcp), &request).unwrap();
    live.commit(prepared.prepared).unwrap();
    let cards = mechanical_part_cards(live.document());
    assert_eq!(cards.len(), 1);
    let card = &cards[0];
    assert!(card.profile_details.iter().any(|v| v == "Phillips H1"));
    assert!(
        card.profile_details
            .iter()
            .any(|v| v.contains("92 degrees"))
    );
    assert!(
        !card
            .dimensions_mm
            .keys()
            .any(|key| key.contains("angle") || key.contains("size"))
    );
    for original in [
        recipe(pan(), torx()),
        recipe(csk(), phillips(false)),
        recipe(
            button(),
            ScrewDrive::HexSocket {
                across_flats: length(2.55),
                depth: length(1.3),
            },
        ),
    ] {
        let reference = MechanicalPartProvenance {
            catalog_id: "independent-fixture".into(),
            revision: "1".into(),
            designation: "Engineering input comparison".into(),
            source_url: "https://example.invalid/reference".into(),
            reference_geometry: original.clone(),
            catalog_facts: None,
        };
        let def = MechanicalPartDefinition {
            output: BodyOutput {
                body: EntityId::new(),
            },
            parameters: original.clone(),
            provenance: Some(reference),
        };
        assert!(def.matches_reference_dimensions(s.parameters()).unwrap());
        let source = serde_json::to_value(&original).unwrap();
        for branch in ["head", "drive"] {
            let mut pending = vec![(format!("/{branch}"), source[branch].clone())];
            while let Some((path, value)) = pending.pop() {
                if let Some(number) = value.get("value").and_then(serde_json::Value::as_f64) {
                    let mut changed = source.clone();
                    changed.pointer_mut(&path).unwrap()["value"] =
                        serde_json::json!(number * 1.001);
                    let mut configured = def.clone();
                    configured.parameters = serde_json::from_value(changed).unwrap();
                    assert!(
                        !configured
                            .matches_reference_dimensions(s.parameters())
                            .unwrap(),
                        "Missing physical input {path}"
                    );
                } else if let Some(map) = value.as_object() {
                    for (key, v) in map {
                        pending.push((format!("{path}/{key}"), v.clone()));
                    }
                }
            }
        }
    }
}
