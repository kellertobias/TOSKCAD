#[allow(dead_code)]
#[path = "../examples/mechanical_library.rs"]
mod generator;
use generator::screw_head_catalog as catalog;
use multicad_core::{
    commands::{Actor, Channel, Session},
    document::{Document, EntityData, Placement},
    model::{self, MechanicalPartParameters, ResolvedMechanicalPart, ThreadRepresentation},
    project_file::{self, ProjectFile},
    symbols::{self, SymbolKind, SymbolPlaceRequest, SymbolValue},
};
use multicad_kernel::{GeometryPolicy, Solid};
use multicad_model::Compiler;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};
fn actor() -> Actor {
    Actor::local(Channel::Ui)
}
fn bytes(p: &ProjectFile) -> Vec<u8> {
    let mut b = Vec::new();
    project_file::write(p, &mut b, Default::default()).unwrap();
    b
}
fn reopen(b: &[u8]) -> ProjectFile {
    project_file::read(
        &mut &b[..],
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap()
    .project
}
#[test]
fn complete_source_matrix_keeps_editions_limits_lengths_and_old_projects_distinct() {
    let c = catalog::curated().unwrap();
    assert_eq!((c.entries.len(), c.coverage.len()), (52, 96));
    let expected = [
        ("iso14583-2011", 9),
        ("iso14581-2013", 9),
        ("iso7045-2011-h", 9),
        ("iso7046-1-2011-h", 9),
        ("fila-iso14579", 3),
        ("iso7380-1-2011", 6),
        ("iso10642-2004", 6),
        ("accu-shb-m3-10-a2-r360", 1),
    ];
    for (family, count) in expected {
        assert_eq!(
            c.entries.iter().filter(|e| e.family == family).count(),
            count
        );
        let rows: Vec<_> = c.coverage.iter().filter(|r| r.family == family).collect();
        assert_eq!(rows.len(), 12);
        assert_eq!(
            rows.iter()
                .map(|r| (r.nominal_mm * 1000.) as u64)
                .collect::<BTreeSet<_>>()
                .len(),
            12
        );
        for r in rows {
            let selected = c
                .entries
                .iter()
                .find(|e| e.id == r.entry_id.clone().unwrap_or_default());
            assert_eq!(selected.is_some(), r.status == "supported");
            assert!(!r.reason.is_empty());
        }
    }
    assert_eq!(
        c.coverage
            .iter()
            .filter(|r| r.status == "unavailable")
            .count(),
        44
    );
    let iso = c
        .entries
        .iter()
        .find(|e| e.family == "iso10642-2004" && e.nominal_mm == 3.)
        .unwrap();
    assert_eq!(
        iso.source_facts.source_head.theoretical_diameter_max_mm,
        Some(6.72)
    );
    assert_eq!(
        iso.source_facts.source_head.actual_diameter_min_mm,
        Some(5.54)
    );
    assert_eq!(iso.source_facts.source_head.height_max_mm, Some(1.86));
    assert_eq!(
        iso.source_facts.source_drive.straight_depth_min_mm,
        Some(1.1)
    );
    assert!(
        !c.entries
            .iter()
            .any(|e| e.family == "iso10642-2004" && e.nominal_mm < 3.)
    );
    let button = c
        .entries
        .iter()
        .find(|e| e.family == "iso7380-1-2011" && e.nominal_mm == 10.)
        .unwrap();
    assert_eq!(button.length_mm, 16.);
    assert_eq!(
        button
            .source_facts
            .source_head
            .fully_threaded_underhead_radius_min_mm,
        Some(0.7)
    );
    assert_eq!(
        button
            .source_facts
            .source_head
            .flat_land_diameter_reference_mm,
        Some(10.)
    );
    let t45 = c
        .entries
        .iter()
        .find(|e| e.family == "iso14583-2011" && e.nominal_mm == 8.)
        .unwrap();
    assert_eq!(t45.source_facts.source_drive.a_reference_mm, Some(7.95));
    let catalog::ProfileFacts::Hexalobular(f) = t45.profile_facts.as_ref().unwrap() else {
        panic!("hexalobular facts")
    };
    assert_eq!(f.a_basic_mm, 7.93);
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../library");
    for (file, hash) in LEGACY_PROJECTS {
        let raw = std::fs::read(root.join(file)).unwrap();
        assert_eq!(
            project_file::ContentHash::of(&raw).to_string(),
            *hash,
            "legacy {file} changed"
        );
    }
    let mut invalid = c.clone();
    invalid.entries[0].pitch_mm = 0.123;
    assert!(catalog::build(&invalid, 0).is_err());
    let mut invalid = c.clone();
    invalid.entries[0].family = "iso7380-1-2022".into();
    assert!(catalog::build(&invalid, 0).is_err());
    let raw: serde_json::Value = serde_json::from_str(include_str!(
        "../../../library/machine-screws-curation.json"
    ))
    .unwrap();
    let mut bad = raw["profile_entries"][0].clone();
    bad["unexpected_dimension"] = serde_json::json!(1.);
    assert!(serde_json::from_value::<catalog::Entry>(bad).is_err());
    let mut bad = raw["profile_entries"][0].clone();
    bad["source_facts"]["source_head"]["invented_limit"] = serde_json::json!(1.);
    assert!(serde_json::from_value::<catalog::Entry>(bad).is_err());
}
#[test]
fn eight_real_family_builds_keep_physical_top_length_choices_literal_references_and_native_solids()
{
    let c = catalog::curated().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let mut documents = BTreeSet::new();
    for (family, nominal) in [
        ("iso14583-2011", 3.),
        ("iso14581-2013", 2.5),
        ("iso7045-2011-h", 2.),
        ("iso7046-1-2011-h", 3.),
        ("fila-iso14579", 6.),
        ("iso7380-1-2011", 3.),
        ("iso10642-2004", 3.),
        ("accu-shb-m3-10-a2-r360", 3.),
    ] {
        let index = c
            .entries
            .iter()
            .position(|e| e.family == family && e.nominal_mm == nominal)
            .unwrap();
        let entry = &c.entries[index];
        let p = catalog::build(&c, index).unwrap();
        assert!(documents.insert(p.document.id()));
        assert!(p.document.id().to_string().starts_with("cad23600-"));
        let saved = bytes(&p);
        let p = reopen(&saved);
        assert_eq!(saved, bytes(&p));
        let info = project_file::info::read(&p.document).unwrap();
        let preview = info.preview.as_ref().expect("real native thumbnail");
        assert!(preview.width() >= 128 && preview.height() >= 128);
        let blank =
            multicad_ui::preview_capture::preview_from(&multicad_ui::egui::ColorImage::filled(
                [preview.width() as usize, preview.height() as usize],
                multicad_ui::egui::Color32::from_rgb(244, 244, 242),
            ))
            .unwrap();
        assert_ne!(preview.png(), blank.png(), "empty thumbnail for {family}");
        let defs = symbols::definitions(&p.document).unwrap();
        assert_eq!(defs.len(), 1);
        let def = &defs[0].1;
        assert_eq!(def.kind, SymbolKind::Dynamic);
        let session = Session::new(def.template.clone()).unwrap();
        let source = def
            .template
            .entities()
            .find_map(|e| model::read_mechanical_part(&def.template, e.id).ok())
            .unwrap();
        let MechanicalPartParameters::MachineScrew {
            underhead_length,
            length_input,
            thread,
            ..
        } = &source.parameters
        else {
            panic!("machine screw")
        };
        assert!(underhead_length.is_none());
        assert!(length_input.is_some());
        assert_eq!(thread.representation, ThreadRepresentation::Modeled);
        let choice = thread.representation_parameter.unwrap();
        assert!(def.exposed.contains(&choice));
        assert!(
            matches!(&def.template.get(choice).unwrap().data,EntityData::Parameter{expression,unit,exposed:true,..} if expression=="1"&&unit=="count")
        );
        let expression = thread.length.expression().unwrap();
        assert!(expression.text.contains("$thread_runout"));
        assert_eq!(
            expression.bindings.len(),
            if entry.length_convention == "overall" {
                3
            } else {
                2
            }
        );
        let MechanicalPartParameters::MachineScrew {
            thread: reference_thread,
            ..
        } = &source.provenance.as_ref().unwrap().reference_geometry
        else {
            panic!("literal reference")
        };
        assert!(reference_thread.representation_parameter.is_none());
        assert!(reference_thread.length.expression().is_none());
        assert!(
            source
                .matches_reference_dimensions(session.parameters())
                .unwrap()
        );
        let ResolvedMechanicalPart::MachineScrew {
            head,
            thread,
            underhead_length_mm,
            authored_length_mm,
            ..
        } = source.parameters.resolve(session.parameters()).unwrap()
        else {
            panic!("resolved screw")
        };
        assert_eq!(authored_length_mm, entry.length_mm);
        assert!(underhead_length_mm - thread.length_mm > 0.);
        assert!((underhead_length_mm - thread.length_mm - entry.thread_runout_mm).abs() < 1e-10);
        let model = Compiler::default().compile(&p.document, 0).unwrap();
        let cards = model::mechanical_part_cards(&p.document);
        assert_eq!(cards.len(), 1);
        let solid = model.solid(cards[0].feature).unwrap();
        assert_eq!(solid.solid_count().unwrap(), 1);
        assert!(solid.volume_mm3().unwrap() > 0.);
        let b = solid.bounds().unwrap();
        assert!(
            (b.min[2] + head.height_mm()).abs() < 1e-6,
            "physical top {family}: {:?}",
            b
        );
        assert!((b.max[2] - underhead_length_mm).abs() < 1e-6);
        assert!((b.max[2] - b.min[2] - (underhead_length_mm + head.height_mm())).abs() < 2e-6);
        let mut old = project_file::supported_capabilities();
        assert!(old.remove("model.mechanical_part.screw_profiles.1"));
        assert!(project_file::read(&mut saved.as_slice(), Default::default(), &old).is_err());
        let mut old = project_file::supported_capabilities();
        assert!(old.remove("model.mechanical_part.machine_screw.length_input.1"));
        assert!(project_file::read(&mut saved.as_slice(), Default::default(), &old).is_err());
        if matches!(family, "iso14581-2013" | "iso7045-2011-h") {
            let path = dir.path().join(format!("{family}.step"));
            solid.write_step(&path).unwrap();
            let step = Solid::read_step(&path, GeometryPolicy::default()).unwrap();
            assert_eq!(step.solid_count().unwrap(), 1);
            let v = solid.volume_mm3().unwrap();
            let area = solid
                .faces()
                .unwrap()
                .iter()
                .map(|f| f.area_mm2)
                .sum::<f64>();
            let budget = GeometryPolicy::default().linear_tolerance_mm * area + 1e-7 * v;
            assert!(
                (step.volume_mm3().unwrap() - v).abs() <= budget,
                "STEP {family}"
            );
        }
        assert_eq!(
            def.pin().unwrap(),
            symbols::definitions(&reopen(&saved).document).unwrap()[0]
                .1
                .pin()
                .unwrap()
        );
    }
}
#[test]
fn one_sourced_pin_retains_independent_choices_and_length_head_expressions_after_reopen() {
    let c = catalog::curated().unwrap();
    let index = c
        .entries
        .iter()
        .position(|e| e.family == "iso14581-2013" && e.nominal_mm == 3.)
        .unwrap();
    let e = &c.entries[index];
    let library = catalog::build(&c, index).unwrap();
    let original = bytes(&library);
    let (_, definition) = symbols::definitions(&library.document)
        .unwrap()
        .pop()
        .unwrap();
    let pin = definition.pin().unwrap();
    let parameter = |name: &str| {
        definition
            .template
            .entities()
            .find(|e| e.name == name)
            .unwrap()
            .id
    };
    let choice = parameter("thread_representation");
    let overall = parameter("overall_length");
    let height = parameter("head_height");
    let feature = definition
        .template
        .entities()
        .find(|e| model::read_mechanical_part(&definition.template, e.id).is_ok())
        .unwrap()
        .id;
    let mut host =
        Session::new(Document::new("Independent sourced screw choices").unwrap()).unwrap();
    let imported = symbols::prepare_import(&host, 0, actor(), &definition).unwrap();
    let embedded = imported.definition;
    host.commit(imported.prepared).unwrap();
    let before = host.document().clone();
    let invalid = SymbolPlaceRequest::new(
        host.document(),
        embedded,
        "Invalid choice",
        BTreeMap::from([(
            choice,
            SymbolValue {
                value: 2.,
                unit: "count".into(),
            },
        )]),
        Default::default(),
    )
    .unwrap();
    assert!(symbols::prepare_place(&host, host.revision(), actor(), &invalid).is_err());
    assert_eq!(host.document(), &before);
    let mut occurrences = Vec::new();
    let mut base_thread_length: Option<f64> = None;
    for (count, expected, x, custom) in [
        (1., ThreadRepresentation::Modeled, 0., false),
        (0., ThreadRepresentation::CosmeticEnvelope, 12., false),
        (0., ThreadRepresentation::CosmeticEnvelope, 24., true),
    ] {
        let mut overrides = BTreeMap::from([(
            choice,
            SymbolValue {
                value: count,
                unit: "count".into(),
            },
        )]);
        if custom {
            overrides.insert(
                overall,
                SymbolValue {
                    value: e.length_mm + 2.,
                    unit: "mm".into(),
                },
            );
            overrides.insert(
                height,
                SymbolValue {
                    value: 1.75,
                    unit: "mm".into(),
                },
            );
        }
        let request = SymbolPlaceRequest::new(
            host.document(),
            embedded,
            "Configured sourced screw",
            overrides,
            Placement {
                translation_mm: [x, 0., 0.],
                ..Default::default()
            },
        )
        .unwrap();
        let prepared = symbols::prepare_place(&host, host.revision(), actor(), &request).unwrap();
        host.commit(prepared.prepared).unwrap();
        let record = symbols::read_instance(host.document(), request.instance).unwrap();
        assert_eq!(record.pin, pin);
        let source = model::read_mechanical_part(host.document(), record.ids[&feature]).unwrap();
        let MechanicalPartParameters::MachineScrew { thread, .. } = &source.parameters else {
            panic!("screw")
        };
        assert_eq!(thread.representation_parameter, Some(record.ids[&choice]));
        assert_eq!(
            thread.resolved_representation(host.parameters()).unwrap(),
            expected
        );
        assert_eq!(
            source
                .matches_reference_dimensions(host.parameters())
                .unwrap(),
            !custom
        );
        let ResolvedMechanicalPart::MachineScrew {
            thread,
            authored_length_mm,
            head,
            ..
        } = source.parameters.resolve(host.parameters()).unwrap()
        else {
            panic!("screw")
        };
        if custom {
            assert_eq!(authored_length_mm, e.length_mm + 2.);
            assert_eq!(head.height_mm(), 1.75);
            assert!((thread.length_mm - (base_thread_length.unwrap() + 1.9)).abs() < 1e-9);
        } else {
            base_thread_length = Some(thread.length_mm);
        }
        occurrences.push(request.instance);
    }
    let saved = bytes(&ProjectFile::new(host.document().clone()).unwrap());
    let p = reopen(&saved);
    let model = Compiler::default().compile(&p.document, 0).unwrap();
    assert_eq!(model.snapshot().leaf_occurrences.len(), 3);
    for i in occurrences {
        assert_eq!(symbols::read_instance(&p.document, i).unwrap().pin, pin);
    }
    for leaf in model.snapshot().leaf_occurrences.values() {
        assert_eq!(
            model.leaf_solid(&leaf.path).unwrap().solid_count().unwrap(),
            1
        );
    }
    assert_eq!(bytes(&library), original);
    assert_eq!(definition.pin().unwrap(), pin);
}

const LEGACY_PROJECTS: &[(&str, &str)] = &[
    (
        "screw-socket-m2-l10.tcd",
        "6e912341adedfa7f87a6c430f760cbc6c1dd7973f275054a4dba924ef12b42e0",
    ),
    (
        "screw-socket-m2-5-l12.tcd",
        "1bf2301a37369ee00b35885d9fcbc152015338305cfced3dd750477c6316fa8c",
    ),
    (
        "screw-socket-m3-l10.tcd",
        "8c5731668dd1b2f64d94cf2704d4cfc5466bf85e1c8f02bd3f48e65e193a30ed",
    ),
    (
        "screw-socket-m3-l30.tcd",
        "d06fe92d354dd546daf0f8bbae4f124cf1fbbb8fa97f6f4369f2a6d6c4041418",
    ),
    (
        "screw-socket-m4-l12.tcd",
        "cbd1d829423c2bc054e55308e81b385c6680ebf269b2d1c7c78a3e2dc6a27af0",
    ),
    (
        "screw-socket-m4-l30.tcd",
        "9e9e765da5a04fd7d9853f38bb3c724567173b19556d5216735be0fe82137bb3",
    ),
    (
        "screw-socket-m5-l16.tcd",
        "7f24df8aaae6373428b098dd864546fbd97dcf4726ff3e8dcad894edefff9edf",
    ),
    (
        "screw-socket-m5-l30.tcd",
        "1d8917dc65ed7ffd48cd6e0682f545ca7f93ee8931c14d41eeca98075b581454",
    ),
    (
        "screw-socket-m6-l20.tcd",
        "d742adb7b3fe455637f2d0c6f0f37a052e44a1792d3d801175739c82c71a85e3",
    ),
    (
        "screw-socket-m6-l40.tcd",
        "045d8f397ab1cf4fca43c145cc281e1c2e62ec6e816e8550d2e687803c1d3abf",
    ),
    (
        "screw-socket-m8-l25.tcd",
        "83a2f1432670003c9329b6cb59e2ab793571c23878e7b9b16681683642cdda33",
    ),
    (
        "screw-socket-m8-l40.tcd",
        "560ea64199624ecb4ac733a65ad3fd2d94037b44c94fd9d9defe06cfc971ab35",
    ),
    (
        "screw-socket-m10-l30.tcd",
        "4a8179a147c1b58fe4d80512ce832bd4a3f9c407983b24bbc952f0c48547d3b1",
    ),
    (
        "screw-socket-m10-l50.tcd",
        "6e0fc8cf0e9b31fc718e0c19b9db8e05f8590b574a4ddcccea8976db27a4b3d1",
    ),
    (
        "screw-hex-m4-l16.tcd",
        "ecff4ef3d2a4fd3e0213b44aa1baab6388a78390984fb723592f4ec8115fa996",
    ),
    (
        "screw-hex-m5-l16.tcd",
        "a107ed269eb8eca2ea9da48994fa3036c2c97ad8f11d0e14cc2a463f8a0a680d",
    ),
    (
        "screw-hex-m6-l20.tcd",
        "8d98cf0c75bf87b7933c34a48ca9488af0b3198b494bf83f52f7f85aa3a299cb",
    ),
    (
        "screw-hex-m8-l25.tcd",
        "90b1f207852f05e5d806931d829430c6d7ab4f5e09be71ba623c3b74ed1b1a00",
    ),
    (
        "screw-hex-m10-l30.tcd",
        "7e9603a34ff4f331e42678ffbae7a2eca2acc6be504c5523a90feac4342af5c0",
    ),
];

#[test]
fn regeneration_accepts_unchanged_selected_profile_and_refuses_identity_hash_or_same_pin_drift() {
    let c = catalog::curated().unwrap();
    let index = c
        .entries
        .iter()
        .position(|e| e.family == "iso14583-2011" && e.nominal_mm == 3.)
        .unwrap();
    let entry = &c.entries[index];
    let project = catalog::build(&c, index).unwrap();
    let saved = bytes(&project);
    let listed = serde_json::json!({"file":entry.file,"document":project.document.id(),"sha256":project_file::ContentHash::of(&saved).to_string()});
    let existing = catalog::checked_existing_profile(entry, &listed, &saved).unwrap();
    let rebuilt = catalog::build(&c, index).unwrap();
    let rebuilt_bytes = bytes(&rebuilt);
    assert_eq!(
        saved, rebuilt_bytes,
        "unchanged real native source/preview must reproduce under this pin"
    );
    catalog::verify_unchanged_profile(entry, &existing, &rebuilt, &rebuilt_bytes).unwrap();

    let mut bad_hash = listed.clone();
    bad_hash["sha256"] = serde_json::json!("0".repeat(64));
    assert!(catalog::checked_existing_profile(entry, &bad_hash, &saved).is_err());
    let mut bad_identity = listed.clone();
    bad_identity["document"] =
        serde_json::json!(Document::new("Wrong manifest identity").unwrap().id());
    assert!(catalog::checked_existing_profile(entry, &bad_identity, &saved).is_err());
    let wrong = ProjectFile::new(Document::new("Other canonical project").unwrap()).unwrap();
    let wrong_bytes = bytes(&wrong);
    let wrong_listed = serde_json::json!({"file":entry.file,"document":wrong.document.id(),"sha256":project_file::ContentHash::of(&wrong_bytes).to_string()});
    assert!(catalog::checked_existing_profile(entry, &wrong_listed, &wrong_bytes).is_err());

    let pin = symbols::definitions(&project.document).unwrap()[0]
        .1
        .pin()
        .unwrap();
    let mut session = Session::new(project.document.clone()).unwrap();
    let mut info = project_file::info::read(session.document()).unwrap();
    info.description.push_str(" Modified card metadata");
    let edits = project_file::info::edits(session.document(), &info).unwrap();
    session
        .apply(multicad_core::commands::Request {
            expected_revision: session.revision(),
            actor: actor(),
            label: "Changed preview/card metadata".into(),
            edits,
        })
        .unwrap();
    let metadata_changed = ProjectFile::new(session.document().clone()).unwrap();
    assert_eq!(
        symbols::definitions(&metadata_changed.document).unwrap()[0]
            .1
            .pin()
            .unwrap(),
        pin
    );
    let error = catalog::verify_unchanged_profile(
        entry,
        &existing,
        &metadata_changed,
        &bytes(&metadata_changed),
    )
    .unwrap_err();
    assert!(error.to_string().contains("explicit migration"));

    let mut changed = c.clone();
    let model::ScrewHead::Pan { profile } = &mut changed.entries[index].head else {
        panic!("pan")
    };
    profile.height = model::length(2.3);
    let changed_project = catalog::build(&changed, index).unwrap();
    assert_eq!(changed_project.document.id(), project.document.id());
    assert_ne!(
        symbols::definitions(&changed_project.document).unwrap()[0]
            .1
            .pin()
            .unwrap(),
        pin
    );
    assert!(
        catalog::verify_unchanged_profile(
            entry,
            &existing,
            &changed_project,
            &bytes(&changed_project)
        )
        .is_err()
    );
    assert_eq!(bytes(&project), saved);
}
