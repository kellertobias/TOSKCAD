#[allow(dead_code)]
#[path = "../examples/mechanical_library.rs"]
mod generator;
use generator::wood_screw_catalog as catalog;
use multicad_core::{
    commands::{Actor, Channel, Session},
    document::{Document, Placement},
    model::{
        self, MechanicalPartParameters, ResolvedMechanicalPart, ScrewLengthInput,
        WoodScrewRepresentation,
    },
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
fn exact_44_row_matrix_retains_nonmetric_source_limits_and_all_128_old_project_bytes() {
    let c = catalog::curated().unwrap();
    assert_eq!(
        (
            c.entries.len(),
            c.coverage.len(),
            c.sources.len(),
            c.excluded_families.len()
        ),
        (44, 12, 3, 2)
    );
    for (d, lengths, p, core, k, af, e) in [
        (
            4.,
            vec![16., 20., 25., 30., 35., 40.],
            1.8,
            2.8,
            2.8,
            7.,
            7.50,
        ),
        (
            5.,
            vec![16., 20., 25., 30., 35., 40., 45., 50.],
            2.2,
            3.5,
            3.5,
            8.,
            8.63,
        ),
        (
            6.,
            vec![20., 25., 30., 35., 40., 45., 50., 55., 60.],
            2.6,
            4.2,
            4.,
            10.,
            10.89,
        ),
        (
            8.,
            vec![25., 30., 35., 40., 45., 50., 55., 60., 70., 80., 100.],
            3.6,
            5.6,
            5.5,
            13.,
            14.20,
        ),
        (
            10.,
            vec![30., 35., 40., 45., 50., 55., 60., 70., 80., 100.],
            4.5,
            7.,
            7.,
            17.,
            18.72,
        ),
    ] {
        let entries: Vec<_> = c.entries.iter().filter(|e| e.nominal_mm == d).collect();
        assert_eq!(
            entries
                .iter()
                .map(|e| e.underhead_length_mm)
                .collect::<Vec<_>>(),
            lengths
        );
        for row in entries {
            assert_eq!(
                (
                    row.v_reference.pitch_mm,
                    row.v_reference.core_diameter_mm,
                    row.head.height_nominal_mm,
                    row.head.across_flats_max_mm,
                    row.head.diagonal_reference_mm
                ),
                (p, core, k, af, e)
            );
            assert_eq!(
                (
                    row.v_reference.included_angle_deg,
                    row.v_reference.included_angle_tolerance_deg,
                    row.minimum_thread_extent_ratio
                ),
                (60., 6., 0.6)
            );
            assert!(!row.designation.starts_with('M'));
            assert!(
                row.length_min_mm < row.underhead_length_mm
                    && row.underhead_length_mm < row.length_max_mm
            );
        }
    }
    assert_eq!(
        c.coverage
            .iter()
            .map(|r| (r.nominal_mm * 1000.) as u64)
            .collect::<BTreeSet<_>>(),
        BTreeSet::from([
            2000, 2500, 3000, 3500, 4000, 4500, 5000, 6000, 7000, 8000, 9000, 10000
        ])
    );
    let ids: BTreeSet<_> = c.entries.iter().map(|e| e.id.clone()).collect();
    assert_eq!(
        c.coverage
            .iter()
            .flat_map(|r| r.entry_ids.clone())
            .collect::<BTreeSet<_>>(),
        ids
    );
    for row in &c.coverage {
        assert_eq!(row.status == "supported", !row.entry_ids.is_empty());
        if row.entry_ids.is_empty() {
            assert!(row.reason.as_ref().is_some_and(|s| !s.is_empty()));
        }
    }
    assert!(
        c.coverage
            .iter()
            .find(|r| r.nominal_mm == 7.)
            .unwrap()
            .reason
            .as_ref()
            .unwrap()
            .contains("no populated")
    );
    assert!(
        c.excluded_families
            .iter()
            .all(|r| r.status == "unavailable" && !r.reason.is_empty())
    );
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../library");
    for (file, hash) in LEGACY_PROJECTS {
        assert_eq!(
            project_file::ContentHash::of(&std::fs::read(root.join(file)).unwrap()).to_string(),
            *hash,
            "old asset {file}"
        );
    }
    let mut invalid = c.clone();
    invalid.entries[0].v_reference.pitch_mm = 0.7;
    assert!(catalog::build(&invalid, 0).is_err());
    let mut value = serde_json::to_value(&c).unwrap();
    value["entries"][0]["v_reference"]["metric_fit"] = serde_json::json!("6g");
    assert!(serde_json::from_value::<catalog::Curation>(value).is_err());
}

#[test]
fn five_native_envelopes_have_sourced_hex_and_flat_tip_not_a_recovered_thread_or_point() {
    let c = catalog::curated().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let mut documents = BTreeSet::new();
    for d in [4., 5., 6., 8., 10.] {
        let index = c.entries.iter().position(|e| e.nominal_mm == d).unwrap();
        let e = &c.entries[index];
        let project = catalog::build(&c, index).unwrap();
        assert!(documents.insert(project.document.id()));
        let saved = bytes(&project);
        let project = reopen(&saved);
        assert_eq!(saved, bytes(&project));
        let info = project_file::info::read(&project.document).unwrap();
        let preview = info.preview.as_ref().unwrap();
        assert!(preview.width() >= 128 && preview.height() >= 128);
        let blank =
            multicad_ui::preview_capture::preview_from(&multicad_ui::egui::ColorImage::filled(
                [preview.width() as usize, preview.height() as usize],
                multicad_ui::egui::Color32::from_rgb(244, 244, 242),
            ))
            .unwrap();
        assert_ne!(preview.png(), blank.png());
        let definitions = symbols::definitions(&project.document).unwrap();
        assert_eq!(definitions.len(), 1);
        let def = &definitions[0].1;
        assert_eq!(def.kind, SymbolKind::Dynamic);
        assert_eq!(def.exposed.len(), 7);
        assert!(def.template.entities().all(|entity|!matches!(&entity.data,multicad_core::document::EntityData::Parameter{unit,..} if unit=="count")));
        let (feature, source) = def
            .template
            .entities()
            .find_map(|e| {
                model::read_mechanical_part(&def.template, e.id)
                    .ok()
                    .map(|source| (e.id, source))
            })
            .unwrap();
        let MechanicalPartParameters::WoodScrew { parameters } = &source.parameters else {
            panic!("wood source")
        };
        assert!(matches!(
            parameters.length_input,
            ScrewLengthInput::Underhead { .. }
        ));
        assert_eq!(
            parameters.representation,
            WoodScrewRepresentation::SimplifiedEnvelope
        );
        let reference = source.provenance.as_ref().unwrap();
        let MechanicalPartParameters::WoodScrew {
            parameters: literal,
        } = &reference.reference_geometry
        else {
            panic!("literal wood reference")
        };
        assert!(matches!(
            literal.length_input,
            ScrewLengthInput::Underhead {
                value: multicad_core::constraint_sketch::Dimension::Literal { .. }
            }
        ));
        assert!(matches!(
            literal.minimum_tip_inclusive_thread_extent,
            multicad_core::constraint_sketch::Dimension::Literal { .. }
        ));
        let session = Session::new(def.template.clone()).unwrap();
        let ResolvedMechanicalPart::WoodScrew {
            parameters: resolved,
        } = source.parameters.resolve(session.parameters()).unwrap()
        else {
            panic!("resolved wood")
        };
        assert_eq!(
            (
                resolved.underhead_length_mm,
                resolved.overall_length_mm,
                resolved.minimum_tip_inclusive_thread_extent_mm,
                resolved.maximum_possible_plain_length_mm
            ),
            (
                e.underhead_length_mm,
                e.underhead_length_mm + e.head.height_nominal_mm,
                e.underhead_length_mm * 0.6,
                e.underhead_length_mm * 0.4
            )
        );
        let model = Compiler::default().compile(&def.template, 0).unwrap();
        let solid = model.solid(feature).unwrap();
        assert_eq!(solid.solid_count().unwrap(), 1);
        let bounds = solid.bounds().unwrap();
        assert!((bounds.min[2] + e.head.height_nominal_mm).abs() < 1e-6);
        assert!((bounds.max[2] - e.underhead_length_mm).abs() < 1e-6);
        let af = e.head.across_flats_max_mm;
        let expected = 3_f64.sqrt() * af * af * e.head.height_nominal_mm / 2.
            + std::f64::consts::PI * d * d * e.underhead_length_mm / 4.;
        let actual = solid.volume_mm3().unwrap();
        assert!(
            (actual - expected).abs() < expected * 2e-7,
            "d{d}: {actual} vs {expected}"
        );
        let faces = solid.faces().unwrap();
        let flats: Vec<_> = faces
            .iter()
            .filter_map(|f| f.plane)
            .filter(|(_, n)| n[2].abs() < 1e-8 && n[0].hypot(n[1]) > 0.99)
            .collect();
        assert_eq!(flats.len(), 6);
        for (o, n) in flats {
            assert!(((o[0] * n[0] + o[1] * n[1]).abs() - af / 2.).abs() < 1e-6);
        }
        let distal = faces
            .iter()
            .find(|f| {
                f.plane.is_some_and(|(o, n)| {
                    (o[2] - e.underhead_length_mm).abs() < 1e-6 && n[2].abs() > 0.99
                })
            })
            .expect("actual flat distal cap");
        assert!((distal.area_mm2 - std::f64::consts::PI * d * d / 4.).abs() < 1e-6);
        if d == 4. || d == 10. {
            let path = dir.path().join(format!("raw-envelope-{d}.step"));
            solid.write_step(&path).unwrap();
            let step = Solid::read_step(&path, GeometryPolicy::default()).unwrap();
            assert_eq!(step.solid_count().unwrap(), 1);
            assert!((step.volume_mm3().unwrap() - expected).abs() < expected * 2e-7);
        }
    }
}

#[test]
fn one_pin_keeps_independent_lengths_bound_minimum_and_literal_reference_through_reopen() {
    let c = catalog::curated().unwrap();
    let index = c
        .entries
        .iter()
        .position(|e| e.nominal_mm == 6. && e.underhead_length_mm == 40.)
        .unwrap();
    let library = catalog::build(&c, index).unwrap();
    let before_library = bytes(&library);
    let (_, definition) = symbols::definitions(&library.document)
        .unwrap()
        .pop()
        .unwrap();
    let pin = definition.pin().unwrap();
    let parameter = definition
        .template
        .entities()
        .find(|e| e.name == "underhead_length")
        .unwrap()
        .id;
    let feature = definition
        .template
        .entities()
        .find(|e| model::read_mechanical_part(&definition.template, e.id).is_ok())
        .unwrap()
        .id;
    let mut host =
        Session::new(Document::new("Independent wood envelope lengths").unwrap()).unwrap();
    let imported = symbols::prepare_import(&host, 0, actor(), &definition).unwrap();
    let embedded = imported.definition;
    host.commit(imported.prepared).unwrap();
    let before = host.document().clone();
    let bad = SymbolPlaceRequest::new(
        host.document(),
        embedded,
        "Invalid wood envelope",
        BTreeMap::from([(
            parameter,
            SymbolValue {
                value: -1.,
                unit: "mm".into(),
            },
        )]),
        Default::default(),
    )
    .unwrap();
    // Symbol preparation creates a candidate; the shared native preview
    // validates its evaluated dimensions before any transaction is committed.
    let candidate = symbols::prepare_place(&host, host.revision(), actor(), &bad).unwrap();
    assert!(
        Compiler::default()
            .compile(candidate.prepared.document(), host.revision())
            .is_err()
    );
    assert_eq!(host.document(), &before);
    let mut instances = Vec::new();
    for (l, x) in [(40., 0.), (55., 30.)] {
        let req = SymbolPlaceRequest::new(
            host.document(),
            embedded,
            "Configured wood envelope",
            BTreeMap::from([(
                parameter,
                SymbolValue {
                    value: l,
                    unit: "mm".into(),
                },
            )]),
            Placement {
                translation_mm: [x, 0., 0.],
                ..Default::default()
            },
        )
        .unwrap();
        let prepared = symbols::prepare_place(&host, host.revision(), actor(), &req).unwrap();
        host.commit(prepared.prepared).unwrap();
        let record = symbols::read_instance(host.document(), req.instance).unwrap();
        assert_eq!(record.pin, pin);
        let source = model::read_mechanical_part(host.document(), record.ids[&feature]).unwrap();
        let ResolvedMechanicalPart::WoodScrew { parameters: r } =
            source.parameters.resolve(host.parameters()).unwrap()
        else {
            panic!("wood")
        };
        assert_eq!(r.underhead_length_mm, l);
        assert!((r.minimum_tip_inclusive_thread_extent_mm - 0.6 * l).abs() < 1e-12);
        assert!((r.maximum_possible_plain_length_mm - 0.4 * l).abs() < 1e-12);
        assert_eq!(
            source
                .matches_reference_dimensions(host.parameters())
                .unwrap(),
            l == 40.
        );
        let MechanicalPartParameters::WoodScrew {
            parameters: reference,
        } = &source.provenance.as_ref().unwrap().reference_geometry
        else {
            panic!("reference")
        };
        let ScrewLengthInput::Underhead { value } = &reference.length_input else {
            panic!("underhead")
        };
        assert!(matches!(
            value,
            multicad_core::constraint_sketch::Dimension::Literal { value: 40., .. }
        ));
        instances.push(req.instance);
    }
    let saved = bytes(&ProjectFile::new(host.document().clone()).unwrap());
    let reopened = reopen(&saved);
    let model = Compiler::default().compile(&reopened.document, 0).unwrap();
    assert_eq!(model.snapshot().leaf_occurrences.len(), 2);
    for id in instances {
        assert_eq!(
            symbols::read_instance(&reopened.document, id).unwrap().pin,
            pin
        );
    }
    assert_eq!(bytes(&library), before_library);
}

#[test]
fn regeneration_accepts_unchanged_selected_wood_and_refuses_hash_identity_or_same_pin_byte_drift() {
    let c = catalog::curated().unwrap();
    let index = c
        .entries
        .iter()
        .position(|e| e.nominal_mm == 6. && e.underhead_length_mm == 40.)
        .unwrap();
    let e = &c.entries[index];
    let project = catalog::build(&c, index).unwrap();
    let saved = bytes(&project);
    let listed = serde_json::json!({"file":e.file,"document":project.document.id(),"sha256":project_file::ContentHash::of(&saved).to_string()});
    let existing = catalog::checked_existing_wood(e, &listed, &saved).unwrap();
    let rebuilt = catalog::build(&c, index).unwrap();
    catalog::verify_unchanged_wood(e, &existing, &rebuilt, &bytes(&rebuilt)).unwrap();
    assert_eq!(bytes(&rebuilt), saved);
    let mut bad = listed.clone();
    bad["sha256"] = serde_json::json!("0".repeat(64));
    assert!(catalog::checked_existing_wood(e, &bad, &saved).is_err());
    let mut bad = listed.clone();
    bad["document"] = serde_json::json!(Document::new("Wrong").unwrap().id());
    assert!(catalog::checked_existing_wood(e, &bad, &saved).is_err());
    let mut session = Session::new(project.document.clone()).unwrap();
    let mut info = project_file::info::read(session.document()).unwrap();
    info.description.push_str(" changed metadata");
    let edits = project_file::info::edits(session.document(), &info).unwrap();
    session
        .apply(multicad_core::commands::Request {
            expected_revision: session.revision(),
            actor: actor(),
            label: "Same pin changed card".into(),
            edits,
        })
        .unwrap();
    let changed = ProjectFile::new(session.document().clone()).unwrap();
    assert_eq!(
        symbols::definitions(&changed.document).unwrap()[0]
            .1
            .pin()
            .unwrap(),
        symbols::definitions(&project.document).unwrap()[0]
            .1
            .pin()
            .unwrap()
    );
    assert!(catalog::verify_unchanged_wood(e, &existing, &changed, &bytes(&changed)).is_err());
    assert_eq!(bytes(&project), saved);
}

const LEGACY_PROJECTS: &[(&str, &str)] = &[
    (
        "bearing-open-608.tcd",
        "0f541bd3e7477fb736572c73573f9d8bc055b4ce0beacf529f96d594c5e14514",
    ),
    (
        "bearing-open-6000.tcd",
        "ad2107734a22c87f17b11d3742931f95978e63e0b9762dba2aeb44bbaee3bcf9",
    ),
    (
        "bearing-open-6200.tcd",
        "4d85bc30aee327ae50eef2cc5128d9989324b46ac6f3b8cfcaaf8822230efa5a",
    ),
    (
        "bearing-open-6300.tcd",
        "a0a85864e183857d3ff0be71b97d84ab90362af9c65f833dd8795a054a1fff97",
    ),
    (
        "plant-generic-tree.tcd",
        "9b8601eb459a8181068a0e12f8b5242bda581c77458bc97ef767ce310fd8d323",
    ),
    (
        "plant-generic-shrub.tcd",
        "0eb2464b9bd7e63464d69a8d2ecdd428c3cb2e825a1a4328b4dd4239f03dd2fa",
    ),
    (
        "equipment-power-outlet-type-f.tcd",
        "9ab1d71bbe94ba347b7d9889107ecc8b8b403d363f73bb6b182f3282befc0eb0",
    ),
    (
        "equipment-double-power-outlet-type-f.tcd",
        "7aa12d6a086bb80ab8ec9ca716b8fa735fd5e0e7c87eea85420e90203957230b",
    ),
    (
        "equipment-triple-power-outlet-type-f.tcd",
        "44bd0249e00c039afce86e7f45ee710fefd9bbd86bb710c8a8c4c40b054c3bbb",
    ),
    (
        "equipment-data-outlet-rj45.tcd",
        "be49a23df30285ec816537cc38a43ca1993a186f3583199631522c4a69f22742",
    ),
    (
        "equipment-tv-antenna-outlet.tcd",
        "494409345feeab9d1d20e4a590b450b9d98ec241a9d64be063f06e129defb437",
    ),
    (
        "equipment-switch.tcd",
        "198bbecb3dcd609693a92e021489017a1d9001eb3e147b8f24a4ac0de17b9565",
    ),
    (
        "equipment-ceiling-luminaire.tcd",
        "e1940ef32f4e9202bfee6d9c5cb51e95ec2cc6a04a5e31528b6d851bef4d0213",
    ),
    (
        "equipment-wall-luminaire.tcd",
        "6c37d1553db94ead303025bcc000d08e5dfd6854aad666a88214a0af7809cbbe",
    ),
    (
        "equipment-lamp-holder-e27.tcd",
        "d57f84b699da3e9753bfa25cde4a44f0e3df72d3daebc2843e3b0667b635daa4",
    ),
    (
        "equipment-radiator.tcd",
        "5d24c6c5555aa7f912c0840c923127a9efac12119375b337bc23f86f60dcb90d",
    ),
    (
        "equipment-kitchen-sink.tcd",
        "4617a7bfac2bfb30cb45a23b263633f2fac861bbcdef029ce135dc07372912d9",
    ),
    (
        "equipment-washbasin.tcd",
        "b83649fa285562b0eec8e632bacc85c6007f5e99bedfd35964097d8789b54042",
    ),
    (
        "equipment-toilet.tcd",
        "42b8c7348cfe4c5af1ea158f734aea4a67c693b3102fd849d8979aa02075f5e0",
    ),
    (
        "equipment-bathtub.tcd",
        "c1e7c33e05d0008196a6b50dc686848ab5e2e82dc7dbadde0f85e96532350b6f",
    ),
    (
        "equipment-shower.tcd",
        "21a16b610ba35262f7728a1e11b243dd2869315ecec5c991f35e6c0aa04d37d1",
    ),
    (
        "equipment-cooker-oven.tcd",
        "02085f1a8cdc93c4ea8ee33ff1d1f8517493d205bb46e90dc5335d53646e4f08",
    ),
    (
        "equipment-cooktop.tcd",
        "ce2822390069ec765178a854d701542588932ddf1414ffca299eb4c491c429fe",
    ),
    (
        "equipment-washing-machine.tcd",
        "0ecd46dff95a67660cf7a0c88ee5d92947f47086f485346c6c104167070e7129",
    ),
    (
        "equipment-dryer.tcd",
        "10421a27d9678003c146586ccc3c1865e5868cd2f080f7c1e9b842b6fa089bb6",
    ),
    (
        "equipment-refrigerator.tcd",
        "29b056772e166627c45e13a925476bf1b2733a6fd2feb7bf5cb9de1d41d7969a",
    ),
    (
        "equipment-dishwasher.tcd",
        "f59f65068acf956d4389ae93177d112dd838418b0c5402625ab7e6622e2ce448",
    ),
    (
        "equipment-junction-box.tcd",
        "fb9a5942161c9626f5afbfa0739747de05474c08b4cf19543635576553a15308",
    ),
    (
        "washer-normal-m2.tcd",
        "a1c86653d8b025ae1464167250ea56d2ebfec1c0eb6f8e997e57a179af1bfa0e",
    ),
    (
        "washer-normal-m2-5.tcd",
        "450c4ff56cbe204e4fb9f26ff7b6b62bffb7587a4e60d2df3a117c2075f7826b",
    ),
    (
        "washer-normal-m3.tcd",
        "b6c796bff44fa60dfee6611c84c1fe118ab9a74361b982088d0a2bd1eddb882a",
    ),
    (
        "washer-normal-m3-5.tcd",
        "2e014708236d7d87858af54ffbbd81d062f33a0d3a6abcc9f653db9fae7ad539",
    ),
    (
        "washer-normal-m4.tcd",
        "df86d91e6318c20037c3dbb00405e68a7685d393f22198e9f6c69edb56d9808e",
    ),
    (
        "washer-normal-m5.tcd",
        "e94db48b05ed00788635bbcb7f6dee59cada17a3116e22d6d261a366049954b0",
    ),
    (
        "washer-normal-m6.tcd",
        "e1ee8bca4ee02b1ff4085f0df6b4bf432bd58351e561d57c9b68f0c09f5a0121",
    ),
    (
        "washer-normal-m7.tcd",
        "826b995b929c6ef5ccd5aa42f56ab6f9b0f7e194f8e8ad2b481d32b9583b1273",
    ),
    (
        "washer-normal-m8.tcd",
        "4fc0ab5cd06f7b3fd38180f55718974c72a2192416193e960a0d2ea7cd0d9f10",
    ),
    (
        "washer-normal-m10.tcd",
        "9c060915cf414bca99526b8e1201c60a87b4770e5c0096577ac3f76042ddbc6e",
    ),
    (
        "screw-socket-m2-l10.tcd",
        "c8b329ac9681580ad046e8a0bb7feee9c9e5f166f406a17d3ee9a8bf28724a80",
    ),
    (
        "screw-socket-m2-5-l12.tcd",
        "58f3922077593a57055078084cd23ba168463649a067d73894d3239ec76fea48",
    ),
    (
        "screw-socket-m3-l10.tcd",
        "1054de1af3c4e80228639dade846d0f9b64e5554cdb0534ed1fc406789bc42fe",
    ),
    (
        "screw-socket-m3-l30.tcd",
        "b5caba51f3dc449a7848197b68cc1af03ea7a5788634e5d1292e77748d6316c7",
    ),
    (
        "screw-socket-m4-l12.tcd",
        "1e56419df9544adc80b055ee54f378c46f7d08cfc1c004840cfc3e3bcd55feaa",
    ),
    (
        "screw-socket-m4-l30.tcd",
        "3a3c796920f862302ed51f913cc84cad6c8d8bc210d568ca261a4af4931f23b9",
    ),
    (
        "screw-socket-m5-l16.tcd",
        "9f0b1b75476b54a0c0732e9b4abf5547e926a7a21de2f18e41f95a2075f2c3d7",
    ),
    (
        "screw-socket-m5-l30.tcd",
        "d2d0ec2d0950e80ef77f78fd3376ef5e8ee024bec4ebb7a593ca13d7181a5723",
    ),
    (
        "screw-socket-m6-l20.tcd",
        "012226f0f581f8a702e257e1e0623ffac0299f02493f0b27ba1798f29f768974",
    ),
    (
        "screw-socket-m6-l40.tcd",
        "8c53beedd6f66448ad48758adf429f511da0176b00e0ce37814f90fa036aeb19",
    ),
    (
        "screw-socket-m8-l25.tcd",
        "dd564eead89125f2a18dbfcbe1f90a57598e1f002df1b77a93ae6270ba9212ea",
    ),
    (
        "screw-socket-m8-l40.tcd",
        "774ab50f5d42fd79c9d5e23bdea7086318e57e85ca767e525260cb3bbe12024a",
    ),
    (
        "screw-socket-m10-l30.tcd",
        "d1aea9c311fa20ac570537063eb8a5d490c2942dc961725f62dc17c95102d1fa",
    ),
    (
        "screw-socket-m10-l50.tcd",
        "d285bcdae693973fa7dd61b0e777cd1357286fffe0c2febb67c1ca5ce6e0eabe",
    ),
    (
        "screw-hex-m4-l16.tcd",
        "bd3d8f32ee1e189e0aa7829b792b0f76e9614d8a00fb1ab2fe7a456a4a62a49f",
    ),
    (
        "screw-hex-m5-l16.tcd",
        "2a61b52a6c110ce402f88e6294821993048b39d5a31421ac10928d8c02bdde2b",
    ),
    (
        "screw-hex-m6-l20.tcd",
        "4a0d7fd1371456139cad45c0f9ece136984c4f60b4b53c6d014a048f697f0f58",
    ),
    (
        "screw-hex-m8-l25.tcd",
        "9b0135cf129fe4179dce8160f4a662ebb0dad6283a4881e9ea42b353b989b6f9",
    ),
    (
        "screw-hex-m10-l30.tcd",
        "7627a834de1a4534234bd886fb162219751499df1f88e342f5c39d215cf3acda",
    ),
    (
        "nut-din934-1987-m2.tcd",
        "546111b26e90f81d58b1cc4fedcbccfb61bb89462dcd924988c8d34c0660dbc8",
    ),
    (
        "nut-din934-1987-m2-5.tcd",
        "b52ef9458a1a04017ca1d65b2286ced1f58906e8d5b7946a4901a338d351186a",
    ),
    (
        "nut-din934-1987-m3.tcd",
        "25e70b124317a6d3f0f4dec8f4d7c051c689d84b129ff9ae56d6d212fd8f19a8",
    ),
    (
        "nut-din934-1987-m3-5.tcd",
        "f1d5361a0d28a075b62e2acf9a4e3316eba3bebfe44679abcee0e15db45b6394",
    ),
    (
        "nut-din934-1987-m4.tcd",
        "95a2e1670e957f53993a20663b73b7dd05dd67fef54d348f701ecfac39bf3574",
    ),
    (
        "nut-din934-1987-m5.tcd",
        "f26c499dcfea9e28c491b350eb9532f1906f80bfbae3e04496215d9441ced207",
    ),
    (
        "nut-din934-1987-m6.tcd",
        "a87e4f6b16acf5669a63c1e27cf1000d8500a5f63b58e4f1e41cf79248a643c1",
    ),
    (
        "nut-din934-1987-m7.tcd",
        "8d1cbec4c445e6ad25278a1b06ccb9ed87a995cc8bbad71715c36085e8db5e7e",
    ),
    (
        "nut-din934-1987-m8.tcd",
        "c6fb71c73623faa99e4a5763b0ddd9ed38d89d4bda6a49ea1fab093f07afc4ce",
    ),
    (
        "nut-din934-1987-m10.tcd",
        "27b562d15a380bcc6def9e086dfe2d8700617c5f7e8cfee7a614b5cbc564e749",
    ),
    (
        "nut-iso4032-2012-m2.tcd",
        "7946bc25fe1509b95ef1b72452af6daf54ee97ae3c47bfab170dfb8fc9df8f67",
    ),
    (
        "nut-iso4032-2012-m2-5.tcd",
        "6b43151abf2a75f9a91a0ad78b835ec0f01c92fc2235039b8109a404ca7b9e8f",
    ),
    (
        "nut-iso4032-2012-m3.tcd",
        "44c58258ec540af86298b8da668d2d5501c85a3ce12cc504398d74cda88d19a7",
    ),
    (
        "nut-iso4032-2012-m3-5.tcd",
        "9f3a30e912750dd106ba284077374f037a21ee2c5740c2c242a47deccc9ad263",
    ),
    (
        "nut-iso4032-2012-m4.tcd",
        "fe62f76ad95e81e87792448767126c0413846881e0374c29f18ef57120ab5644",
    ),
    (
        "nut-iso4032-2012-m5.tcd",
        "a9d9ee84c4709474f6eec57146f9c238507dc5bda4699e8f8d4ba3e4e74cda78",
    ),
    (
        "nut-iso4032-2012-m6.tcd",
        "0782dd13b2e3e0b699b94454ceae6565532caf3b4167afb836ca5ad14c349aa3",
    ),
    (
        "nut-iso4032-2012-m8.tcd",
        "8ab94b2b0703b7dbf1639cec4a91f677ae30321616b8af354350d66ca2cc8cec",
    ),
    (
        "nut-iso4032-2012-m10.tcd",
        "3384b64006ae825e8738a2f32e1c319cc826eea257b2e17a4a2c653047e4ea00",
    ),
    (
        "screw-iso14583-2011-m2-l12.tcd",
        "fe579f405b27f5958d700d7f0c4fc424e057a3af965cc4edf2bcb84e55a39a0a",
    ),
    (
        "screw-iso14583-2011-m2-5-l12.tcd",
        "57341b62f56b1f868521a3e57d55a6f99922d0eafcbabca2aaf76e2a6155ae48",
    ),
    (
        "screw-iso14583-2011-m3-l12.tcd",
        "ebc982119717f66591344abfadba5a4660f97f8d3fdedde48468c900551cf387",
    ),
    (
        "screw-iso14583-2011-m3-5-l12.tcd",
        "aed827031b5f3179907df8ab70383cac7b6b26f16ad851bdd65db891ff472e63",
    ),
    (
        "screw-iso14583-2011-m4-l12.tcd",
        "b328c6953056ddf104bb0a493527458de70895a5af3a081388041854e4903115",
    ),
    (
        "screw-iso14583-2011-m5-l12.tcd",
        "e4db9c6867b5fb0f7f51181503fd8e89a0390bd1795958981e8161aaa7d042a0",
    ),
    (
        "screw-iso14583-2011-m6-l12.tcd",
        "1bfeb1b49ffbe245551226678b2404222248353799a1539262b8e90aff7f05d8",
    ),
    (
        "screw-iso14583-2011-m8-l12.tcd",
        "d246f245729b82dc5e67fe0ba167e2144336d53fe3e134365b637b902e94863f",
    ),
    (
        "screw-iso14583-2011-m10-l12.tcd",
        "4c67f70a19fc150e78003639fcf3a90e2d618eef4423599a6e84a26b2989479d",
    ),
    (
        "screw-iso14581-2013-m2-l12.tcd",
        "7f3b3f83f7afbf0890069414844f47291490d81115fe4e257a59b9c7dd55eded",
    ),
    (
        "screw-iso14581-2013-m2-5-l12.tcd",
        "ad677fbb134db5e65fd5950fb194cf5a8365b7f0f8e207a01e56f7461f12b0ad",
    ),
    (
        "screw-iso14581-2013-m3-l12.tcd",
        "461066b385d778ec234078d9b0cac786126ca5ad26320e2213bc9fcc008bca7e",
    ),
    (
        "screw-iso14581-2013-m3-5-l12.tcd",
        "39cadb3d79f1451bc9e87ab1fb01d97a62ec1ee74b39f7ca7bd17b7e828d9086",
    ),
    (
        "screw-iso14581-2013-m4-l12.tcd",
        "778df697014772f8701ac5b1d93cd367d8fb014bcbd27d69f33bfdefb504448a",
    ),
    (
        "screw-iso14581-2013-m5-l12.tcd",
        "b4b8e92551e4373e213c819c9941a56f3c5c692b1df64a326b652f6e2b237578",
    ),
    (
        "screw-iso14581-2013-m6-l12.tcd",
        "507d638207a9fcb155392be0ac4c7af21286de0f6e7311193c80f5e47d4405e8",
    ),
    (
        "screw-iso14581-2013-m8-l12.tcd",
        "f192fd6131fbbfe994132fc2f1ece1c4f6336b7563bfe0bbe5b6dbab0595de0f",
    ),
    (
        "screw-iso14581-2013-m10-l12.tcd",
        "cd1122bba013b29d6fe91b02c9c12d24636388e70e485a42a607afb7bf00e0fa",
    ),
    (
        "screw-iso7045-2011-h-m2-l12.tcd",
        "0fb31f9ee10b335975df841367f3a3e2ac28088f748be53da683597fcdb15e36",
    ),
    (
        "screw-iso7045-2011-h-m2-5-l12.tcd",
        "e1b7fd6be9cd3a21d9a3d1e9c7ddbb10882d5cdfe34c6a5d8a6786d0adecfd3d",
    ),
    (
        "screw-iso7045-2011-h-m3-l12.tcd",
        "31a666a4f013da4b2b52d0db8815873c4b3c3660858d4fba80e3f1ed7046c7ab",
    ),
    (
        "screw-iso7045-2011-h-m3-5-l12.tcd",
        "e847d4bb8f1c5b8e58291a2b4372aafe00430b4e09e45a75b99f3ef831859cf4",
    ),
    (
        "screw-iso7045-2011-h-m4-l12.tcd",
        "973fc531267c8d65957e1a646b8cbb9cd34d7398fea441777cddb9b4e6a4c943",
    ),
    (
        "screw-iso7045-2011-h-m5-l12.tcd",
        "91a7e69e336c36010db2efa47adbc74bb5105f17b4de780ca51810bfbfa233dd",
    ),
    (
        "screw-iso7045-2011-h-m6-l12.tcd",
        "43732f16a6be191820f2adb263ffff19463e279926ef335ac7d46c9fa3073251",
    ),
    (
        "screw-iso7045-2011-h-m8-l12.tcd",
        "f55aa3ab8b74704f9dbe64967e8a2f809f3d20fd4b4f469aa819e8dfcf9ccb6a",
    ),
    (
        "screw-iso7045-2011-h-m10-l12.tcd",
        "09bb1aae9fe8eec46cb1c0c7833708569bc80515126ca1699f9a83af265d6524",
    ),
    (
        "screw-iso7046-1-2011-h-m2-l12.tcd",
        "e801336a1e1f7e1306df1eaf6a3d86e49d442834fe80a8ef1316d7b5b9edbe51",
    ),
    (
        "screw-iso7046-1-2011-h-m2-5-l12.tcd",
        "9d102cd0e2c456ab8d4054e9039a1c02cda3f310ef25ac74806c466a4458b7b9",
    ),
    (
        "screw-iso7046-1-2011-h-m3-l12.tcd",
        "79d504d58d821fa68e38bd98d76e4c7c6bca13d05849cb121baadc0ce19d90df",
    ),
    (
        "screw-iso7046-1-2011-h-m3-5-l12.tcd",
        "7abecae059540e645a05f434ae4eb717ccedc41bfe05feb3577ad2ac903eb8a9",
    ),
    (
        "screw-iso7046-1-2011-h-m4-l12.tcd",
        "a40e737d89bdf1dbef40697f179066f37344cbf5518f21b6a9ce0e43e10123fa",
    ),
    (
        "screw-iso7046-1-2011-h-m5-l12.tcd",
        "38dd4519face725ea9c59bd81fa26ee5e10403de83fb98df7dcca2870bc50851",
    ),
    (
        "screw-iso7046-1-2011-h-m6-l12.tcd",
        "63ba2ee5453be58a71a7280a6bcfc132f943c5cb6a2d78c4b5aefbf9ac2d41b8",
    ),
    (
        "screw-iso7046-1-2011-h-m8-l12.tcd",
        "ea87e574d7af5e24919c0bd511776fb3fd20f0a75b638d9dce1f2089faa7c542",
    ),
    (
        "screw-iso7046-1-2011-h-m10-l12.tcd",
        "916403b99508d85884b4749c03552cefddb5f487d7212c25eef37c5bb0808e20",
    ),
    (
        "screw-fila-iso14579-m6-l30.tcd",
        "ffce5a2a0cf664d66fb82702cf82f9499a29bd2b2865c4d8dab9106ad49755b5",
    ),
    (
        "screw-fila-iso14579-m8-l40.tcd",
        "89fa8fab2e3c3db02ea84c3daca3b06a96f0c31a21529f5b1afbd259a5e574e0",
    ),
    (
        "screw-fila-iso14579-m10-l40.tcd",
        "5c252231721a088c210973cc4ade50e26a04a7538836e8e935b6b2edeb99bcb7",
    ),
    (
        "screw-iso7380-1-2011-m3-l12.tcd",
        "7814b83f23a726ead6dd4d2377868c3bf1eea5c755a9484c79d924fd72307ba1",
    ),
    (
        "screw-iso7380-1-2011-m4-l12.tcd",
        "275b03890b767d2794b354ddcdc08e18a980f5d822c3b327fb11565a3447bfe5",
    ),
    (
        "screw-iso7380-1-2011-m5-l12.tcd",
        "865725711529576a795fadd6b7a44cfc09f0111198aaca89dd68bf1e131fca46",
    ),
    (
        "screw-iso7380-1-2011-m6-l12.tcd",
        "7ede18eb754b88911958c9203967d62fec5d03622b1fb97ddc4087c08c5e9485",
    ),
    (
        "screw-iso7380-1-2011-m8-l12.tcd",
        "5e42cdd73c245a0fa27820c686c5a12a7f3dbe828c1035ec78c88501ccca8601",
    ),
    (
        "screw-iso7380-1-2011-m10-l16.tcd",
        "8a33a05465b97a25892b69eb86f6bd4e5a37381a80b158b4d0e43251c57021f6",
    ),
    (
        "screw-iso10642-2004-m3-l12.tcd",
        "29bb9a7611f9c8404ade89a505572573317f2f00411ac8dbd669917afc7d470a",
    ),
    (
        "screw-iso10642-2004-m4-l12.tcd",
        "e6ed1407b5a802dcef63b762ceddf2189b8f9ff636a0d7711c0ddc196297d621",
    ),
    (
        "screw-iso10642-2004-m5-l12.tcd",
        "6974103ac3146bfacd61a742e5a569e6dad3cd895bc07f6c025f1a35116a151e",
    ),
    (
        "screw-iso10642-2004-m6-l12.tcd",
        "a29df3abbc7c2f8a5b7d148066fd2e8656d4238f4c89ccb04ada638a6b11cd2e",
    ),
    (
        "screw-iso10642-2004-m8-l12.tcd",
        "3f5e120a548f118de7d261272c681bb1e1baca913438e61d5811e69f0f8f90ad",
    ),
    (
        "screw-iso10642-2004-m10-l12.tcd",
        "2aad1cfb4d3f0556904a7e1a915224f2f65e632283e2defb65cfa0b21b0942ed",
    ),
    (
        "screw-accu-shb-m3-10-a2-r360-m3-l10.tcd",
        "fe5187c1c86a4ac786b13f7bf47373de30dbe45c515ec31f90af25ef00c1697a",
    ),
];
