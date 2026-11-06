#[allow(dead_code)]
#[path = "../examples/mechanical_library.rs"]
mod generator;
use generator::nut_catalog;
use multicad_core::{
    commands::{Actor, Channel, Session},
    document::{Document, EntityData, Placement},
    model::{self, MechanicalPartParameters, ResolvedMechanicalPart, ThreadRepresentation},
    project_file::{self, ProjectFile},
    symbols::{self, SymbolKind, SymbolPlaceRequest, SymbolValue},
};
use multicad_kernel::{GeometryPolicy, Solid};
use multicad_model::Compiler;
use std::collections::{BTreeMap, BTreeSet};

fn actor() -> Actor {
    Actor::local(Channel::Ui)
}
fn bytes(project: &ProjectFile) -> Vec<u8> {
    let mut value = Vec::new();
    project_file::write(project, &mut value, Default::default()).unwrap();
    value
}
fn reopen(value: &[u8]) -> ProjectFile {
    project_file::read(
        &mut &value[..],
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap()
    .project
}
#[test]
fn edition_matrix_retains_licensed_rows_and_explicit_unavailable_nominals() {
    let c = nut_catalog::curated().unwrap();
    assert_eq!(c.entries.len(), 19);
    assert_eq!(c.coverage.len(), 24);
    let expected = [
        (2., 0.4, 4., 1.6, 1.6),
        (2.5, 0.45, 5., 2., 2.),
        (3., 0.5, 5.5, 2.4, 2.4),
        (3.5, 0.6, 6., 2.8, 2.8),
        (4., 0.7, 7., 3.2, 3.2),
        (5., 0.8, 8., 4., 4.7),
        (6., 1., 10., 5., 5.2),
        (7., 1., 11., 5.5, 0.),
        (8., 1.25, 13., 6.5, 6.8),
        (10., 1.5, 17., 8., 8.4),
    ];
    for (nominal, pitch, din_af, din_height, iso_height) in expected {
        for (family, af, height) in [
            ("din934-1987", din_af, din_height),
            (
                "iso4032-2012",
                if nominal == 10. { 16. } else { din_af },
                iso_height,
            ),
        ] {
            let entries: Vec<_> = c
                .entries
                .iter()
                .filter(|e| e.family == family && e.nominal_mm == nominal)
                .collect();
            if nominal == 7. && family == "iso4032-2012" {
                assert!(entries.is_empty());
            } else {
                assert_eq!(entries.len(), 1);
                let e = entries[0];
                assert_eq!(
                    (e.pitch_mm, e.across_flats_mm, e.height_mm),
                    (pitch, af, height)
                );
                assert_eq!(&e.source_row[..3], &[nominal, af, height]);
                assert_eq!(e.correction, None);
                assert!((af - nominal) / 2. >= 1.);
            }
        }
    }
    for family in ["din934-1987", "iso4032-2012"] {
        let rows: Vec<_> = c.coverage.iter().filter(|r| r.family == family).collect();
        assert_eq!(rows.len(), 12);
        assert_eq!(
            rows.iter()
                .map(|r| (r.nominal_mm * 1000.) as u64)
                .collect::<BTreeSet<_>>()
                .len(),
            12
        );
        for row in rows {
            let absent = row.nominal_mm == 4.5
                || row.nominal_mm == 9.
                || (family == "iso4032-2012" && row.nominal_mm == 7.);
            assert_eq!(row.status, if absent { "unavailable" } else { "supported" });
            assert_eq!(row.entry_id.is_none(), absent);
            assert!(!row.reason.is_empty());
        }
    }
    let mut malformed = c.clone();
    malformed.entries[0].pitch_mm = 0.5;
    assert!(nut_catalog::build(&malformed, 0).is_err());
    malformed.entries[0] = c.entries[0].clone();
    malformed.entries[0].height_mm = 2.;
    assert!(nut_catalog::build(&malformed, 0).is_err());
}

#[test]
fn native_m10_families_keep_distinct_maximum_references_and_real_threaded_solids() {
    let c = nut_catalog::curated().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let mut identities = BTreeSet::new();
    for (family, af, height) in [("din934-1987", 17., 8.), ("iso4032-2012", 16., 8.4)] {
        let index = c
            .entries
            .iter()
            .position(|e| e.family == family && e.nominal_mm == 10.)
            .unwrap();
        let project = nut_catalog::build(&c, index).unwrap();
        assert!(identities.insert(project.document.id()));
        assert!(project.document.id().to_string().starts_with("cad23400-"));
        let saved = bytes(&project);
        let project = reopen(&saved);
        assert_eq!(saved, bytes(&project));
        let info = project_file::info::read(&project.document).unwrap();
        assert!(info.preview.is_some());
        assert!(info.description.contains("source maxima"));
        let defs = symbols::definitions(&project.document).unwrap();
        assert_eq!(defs.len(), 1);
        let definition = &defs[0].1;
        assert_eq!(definition.kind, SymbolKind::Dynamic);
        assert_eq!(definition.exposed.len(), 5);
        let source_feature = definition
            .template
            .entities()
            .find_map(|e| {
                model::read_mechanical_part(&definition.template, e.id)
                    .ok()
                    .map(|source| (e.id, source))
            })
            .unwrap();
        let source_session = Session::new(definition.template.clone()).unwrap();
        let MechanicalPartParameters::HexNut { thread, .. } = &source_feature.1.parameters else {
            panic!("expected nut")
        };
        let choice = thread.representation_parameter.unwrap();
        assert!(definition.exposed.contains(&choice));
        assert!(
            matches!(&definition.template.get(choice).unwrap().data, EntityData::Parameter{expression,unit,exposed:true,..} if expression=="1" && unit=="count")
        );
        let reference = &source_feature
            .1
            .provenance
            .as_ref()
            .unwrap()
            .reference_geometry;
        let MechanicalPartParameters::HexNut {
            thread: reference_thread,
            ..
        } = reference
        else {
            panic!("expected literal nut reference")
        };
        assert_eq!(reference_thread.representation_parameter, None);
        assert_eq!(
            reference_thread.representation,
            ThreadRepresentation::Modeled
        );
        assert!(
            source_feature
                .1
                .matches_reference_dimensions(source_session.parameters())
                .unwrap()
        );
        let ResolvedMechanicalPart::HexNut {
            across_flats_mm,
            thread,
        } = source_feature
            .1
            .parameters
            .resolve(source_session.parameters())
            .unwrap()
        else {
            panic!("expected resolved nut")
        };
        assert_eq!(
            (across_flats_mm, thread.length_mm, thread.pitch_mm),
            (af, height, 1.5)
        );
        assert_eq!(thread.minimum_wall_mm, 0.5);
        let cards = model::mechanical_part_cards(&project.document);
        assert_eq!(cards.len(), 1);
        let model = Compiler::default().compile(&project.document, 0).unwrap();
        let solid = model.solid(cards[0].feature).unwrap();
        assert_eq!(solid.solid_count().unwrap(), 1);
        let bounds = solid.bounds().unwrap();
        assert!(bounds.min[2].abs() < 1e-6);
        assert!((bounds.max[2] - height).abs() < 1e-6);
        let flats: Vec<_> = solid
            .faces()
            .unwrap()
            .into_iter()
            .filter_map(|face| {
                let (origin, normal) = face.plane?;
                let planar_length = normal[0].hypot(normal[1]);
                (normal[2].abs() < 1e-8 && planar_length > 0.99).then_some(
                    (origin[0] * normal[0] + origin[1] * normal[1]).abs() / planar_length,
                )
            })
            .collect();
        assert_eq!(flats.len(), 6, "six actual native hex flats");
        for distance in flats {
            assert!((distance - af / 2.).abs() < 1e-6);
        }
        let blank_volume = 3_f64.sqrt() * af * af * height / 2.;
        assert!(solid.volume_mm3().unwrap() < blank_volume);
        assert!(
            solid.volume_mm3().unwrap()
                > blank_volume - std::f64::consts::PI * 5.1_f64.powi(2) * height
        );
        let path = dir.path().join(format!("{family}.step"));
        solid.write_step(&path).unwrap();
        let returned = Solid::read_step(&path, GeometryPolicy::default()).unwrap();
        assert_eq!(returned.solid_count().unwrap(), 1);
        let a = solid.volume_mm3().unwrap();
        let b = returned.volume_mm3().unwrap();
        let area = solid
            .faces()
            .unwrap()
            .iter()
            .map(|f| f.area_mm2)
            .sum::<f64>();
        let budget = GeometryPolicy::default().linear_tolerance_mm * area + 1e-7 * a;
        assert!(
            (a - b).abs() <= budget,
            "{family} STEP volume delta {} exceeds {budget}",
            (a - b).abs()
        );
        assert_eq!(
            definition.pin().unwrap(),
            symbols::definitions(&reopen(&saved).document).unwrap()[0]
                .1
                .pin()
                .unwrap()
        );
    }
}

#[test]
fn one_retained_nut_pin_allows_independent_typed_representations_and_dimension_overrides() {
    let c = nut_catalog::curated().unwrap();
    let index = c
        .entries
        .iter()
        .position(|e| e.family == "iso4032-2012" && e.nominal_mm == 3.)
        .unwrap();
    let library = nut_catalog::build(&c, index).unwrap();
    let library_bytes = bytes(&library);
    let (_, definition) = symbols::definitions(&library.document)
        .unwrap()
        .pop()
        .unwrap();
    let pin = definition.pin().unwrap();
    let parameter = |name| {
        definition
            .template
            .entities()
            .find(|e| e.name == name)
            .unwrap()
            .id
    };
    let choice = parameter("thread_representation");
    let af = parameter("across_flats");
    let feature = definition
        .template
        .entities()
        .find(|e| model::read_mechanical_part(&definition.template, e.id).is_ok())
        .unwrap()
        .id;
    let mut host = Session::new(Document::new("Independent nut choices").unwrap()).unwrap();
    let imported = symbols::prepare_import(&host, 0, actor(), &definition).unwrap();
    let embedded = imported.definition;
    host.commit(imported.prepared).unwrap();
    let before = host.document().clone();
    let invalid = SymbolPlaceRequest::new(
        host.document(),
        embedded,
        "Invalid representation",
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
    for (value, expected, x, custom) in [
        (1., ThreadRepresentation::Modeled, 0., false),
        (0., ThreadRepresentation::CosmeticEnvelope, 12., false),
        (0., ThreadRepresentation::CosmeticEnvelope, 24., true),
    ] {
        let mut overrides = BTreeMap::from([(
            choice,
            SymbolValue {
                value,
                unit: "count".into(),
            },
        )]);
        if custom {
            overrides.insert(
                af,
                SymbolValue {
                    value: 6.,
                    unit: "mm".into(),
                },
            );
        }
        let request = SymbolPlaceRequest::new(
            host.document(),
            embedded,
            "Configured nut",
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
        let MechanicalPartParameters::HexNut { thread, .. } = &source.parameters else {
            panic!("expected nut")
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
        occurrences.push(request.instance);
    }
    let saved = bytes(&ProjectFile::new(host.document().clone()).unwrap());
    let reopened = reopen(&saved);
    let model = Compiler::default().compile(&reopened.document, 0).unwrap();
    assert_eq!(model.snapshot().leaf_occurrences.len(), 3);
    for instance in occurrences {
        assert_eq!(
            symbols::read_instance(&reopened.document, instance)
                .unwrap()
                .pin,
            pin
        );
    }
    for leaf in model.snapshot().leaf_occurrences.values() {
        assert_eq!(
            model.leaf_solid(&leaf.path).unwrap().solid_count().unwrap(),
            1
        );
    }
    assert_eq!(bytes(&library), library_bytes);
    assert_eq!(definition.pin().unwrap(), pin);
}

fn nut_probe_fraction(solid: &Solid, center: [f64; 3], pitch: f64) -> f64 {
    use multicad_kernel::{BooleanOperation, RigidTransform};
    let policy = GeometryPolicy::default();
    let size = 0.002 * pitch;
    let probe = Solid::box_solid([size; 3], policy)
        .unwrap()
        .transformed(RigidTransform {
            translation_mm: center.map(|value| value - size / 2.),
            ..Default::default()
        })
        .unwrap();
    solid
        .boolean(&probe, BooleanOperation::Intersect, policy)
        .unwrap()
        .map_or(0., |intersection| intersection.volume_mm3().unwrap())
        / size.powi(3)
}

#[test]
fn all_shipped_nuts_have_checked_typed_sources_and_independent_native_profile_witnesses() {
    use multicad_core::model::THREAD_REPRESENTATION_PARAMETER_CAPABILITY;
    use multicad_ui::egui::{Color32, ColorImage};
    use std::{f64::consts::PI, path::Path};
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../library");
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("library.json")).unwrap()).unwrap();
    let curation = nut_catalog::curated().unwrap();
    let policy = GeometryPolicy::default();
    let mut pins = BTreeSet::new();
    for entry in &curation.entries {
        let path = root.join(&entry.file);
        let saved = std::fs::read(&path).unwrap();
        let listed: Vec<_> = manifest["entries"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|row| row["file"] == entry.file)
            .collect();
        assert_eq!(listed.len(), 1, "{} needs one manifest entry", entry.file);
        assert_eq!(
            listed[0]["sha256"].as_str().unwrap(),
            project_file::ContentHash::of(&saved).to_string()
        );
        let project = reopen(&saved);
        assert_eq!(
            listed[0]["document"].as_str().unwrap(),
            project.document.id().to_string()
        );
        assert!(
            project
                .capabilities()
                .contains(THREAD_REPRESENTATION_PARAMETER_CAPABILITY)
        );
        let mut old_reader = project_file::supported_capabilities();
        assert!(old_reader.remove(THREAD_REPRESENTATION_PARAMETER_CAPABILITY));
        assert!(
            project_file::read(&mut saved.as_slice(), Default::default(), &old_reader)
                .unwrap_err()
                .contains(THREAD_REPRESENTATION_PARAMETER_CAPABILITY)
        );
        let info = project_file::info::read(&project.document).unwrap();
        let preview = info.preview.as_ref().expect("native preview is required");
        assert!(preview.width() >= 128 && preview.height() >= 128);
        let blank = multicad_ui::preview_capture::preview_from(&ColorImage::filled(
            [preview.width() as usize, preview.height() as usize],
            Color32::from_rgb(244, 244, 242),
        ))
        .unwrap();
        assert_ne!(
            preview.png(),
            blank.png(),
            "{} stored the renderer's empty background",
            entry.file
        );
        let definitions = symbols::definitions(&project.document).unwrap();
        assert_eq!(definitions.len(), 1);
        let definition = &definitions[0].1;
        assert_eq!(definition.kind, SymbolKind::Dynamic);
        assert_eq!(definition.revision, "1");
        assert_eq!(definition.exposed.len(), 5);
        let pin = definition.pin().unwrap();
        assert!(
            pins.insert(pin),
            "each family/nominal has an independent pin"
        );
        assert_eq!(
            symbols::definitions(&reopen(&bytes(&project)).document).unwrap()[0]
                .1
                .pin()
                .unwrap(),
            pin
        );
        let template_session = Session::new(definition.template.clone()).unwrap();
        let sources: Vec<_> = definition
            .template
            .entities()
            .filter_map(|entity| model::read_mechanical_part(&definition.template, entity.id).ok())
            .collect();
        assert_eq!(sources.len(), 1);
        let source = &sources[0];
        let MechanicalPartParameters::HexNut { thread, .. } = &source.parameters else {
            panic!("nut source")
        };
        let representation = thread
            .representation_parameter
            .expect("typed representation binding");
        assert!(definition.exposed.contains(&representation));
        assert_eq!(thread.representation, ThreadRepresentation::Modeled);
        assert!(
            matches!(&definition.template.get(representation).unwrap().data,
            EntityData::Parameter{expression,unit,exposed:true,..} if expression=="1" && unit=="count")
        );
        let names: BTreeSet<_> = definition
            .exposed
            .iter()
            .map(|id| definition.template.get(*id).unwrap().name.as_str())
            .collect();
        assert_eq!(
            names,
            [
                "nominal_thread_diameter",
                "pitch",
                "across_flats",
                "height",
                "thread_representation"
            ]
            .into()
        );
        let provenance = source.provenance.as_ref().unwrap();
        assert_eq!(provenance.catalog_id, format!("toskcad.bolts.{}", entry.id));
        assert_eq!(provenance.source_url, curation.data_source);
        let MechanicalPartParameters::HexNut {
            thread: reference, ..
        } = &provenance.reference_geometry
        else {
            panic!("literal nut reference")
        };
        assert_eq!(reference.representation_parameter, None);
        assert_eq!(reference.representation, ThreadRepresentation::Modeled);
        assert!(
            source
                .matches_reference_dimensions(template_session.parameters())
                .unwrap()
        );
        let ResolvedMechanicalPart::HexNut {
            across_flats_mm,
            thread,
        } = source
            .parameters
            .resolve(template_session.parameters())
            .unwrap()
        else {
            panic!("resolved nut")
        };
        assert_eq!(
            (
                across_flats_mm,
                thread.length_mm,
                thread.pitch_mm,
                thread.major_radius_mm
            ),
            (
                entry.across_flats_mm,
                entry.height_mm,
                entry.pitch_mm,
                entry.nominal_mm / 2.
            )
        );
        assert_eq!(thread.representation, ThreadRepresentation::Modeled);
        let cards = model::mechanical_part_cards(&project.document);
        assert_eq!(cards.len(), 1);
        let built = Compiler {
            omit_display: true,
            ..Default::default()
        }
        .compile(&project.document, 0)
        .unwrap();
        let original = built.solid(cards[0].feature).unwrap();
        let original_brep = original.to_brep().unwrap();
        let independent = Solid::from_brep(&original_brep, policy).unwrap();
        assert_eq!(independent.solid_count().unwrap(), 1);
        let bounds = independent.bounds().unwrap();
        assert!(bounds.min[2].abs() < 1e-6);
        assert!((bounds.max[2] - entry.height_mm).abs() < 1e-6);
        let faces = independent.faces().unwrap();
        let flats: Vec<_> = faces
            .iter()
            .filter_map(|face| {
                let (origin, normal) = face.plane?;
                let horizontal = normal[0].hypot(normal[1]);
                (normal[2].abs() < 1e-8 && horizontal > 0.99)
                    .then_some((origin[0] * normal[0] + origin[1] * normal[1]).abs() / horizontal)
            })
            .collect();
        assert_eq!(flats.len(), 6);
        for distance in flats {
            assert!((distance - entry.across_flats_mm / 2.).abs() < 1e-6);
        }
        // Integrate the independent nominal-basic radial internal cutter profile
        // over one angular period. Rotation with Z preserves this section area.
        let radius = entry.nominal_mm / 2.;
        let pitch = entry.pitch_mm;
        let root = radius - 5. * 3_f64.sqrt() * pitch / 16.;
        let integral = (radius * radius - root * root) / 16.
            + 2. / (3_f64.sqrt() * pitch)
                * (radius * (radius * radius - root * root) / 2.
                    - (radius.powi(3) - root.powi(3)) / 3.);
        let expected = entry.height_mm
            * (entry.across_flats_mm.powi(2) * 3_f64.sqrt() / 2.
                - PI * root * root
                - 2. * PI * integral);
        let area: f64 = faces.iter().map(|face| face.area_mm2).sum();
        let budget = policy.linear_tolerance_mm * area + 1e-7 * expected.abs();
        let volume = independent.volume_mm3().unwrap();
        assert!(
            (volume - expected).abs() <= budget,
            "{} native {volume} differs from independent {expected}, budget {budget}",
            entry.id
        );
        // At phi=0 the pitched ridge crosses the pitch radius at these two
        // phases. Both turns are inside every selected row's axial extent.
        let pitch_radius = radius - 3. * 3_f64.sqrt() * pitch / 16.;
        for turn in [1., 2.] {
            assert!(
                nut_probe_fraction(
                    &independent,
                    [pitch_radius, 0., (turn + 0.2) * pitch],
                    pitch
                ) < 1e-5,
                "{} missing cutter crest",
                entry.id
            );
            assert!(
                nut_probe_fraction(
                    &independent,
                    [pitch_radius, 0., (turn + 0.3) * pitch],
                    pitch
                ) > 0.999,
                "{} lost material between cutter ridges",
                entry.id
            );
        }
        assert_eq!(
            original.to_brep().unwrap(),
            original_brep,
            "metrology mutated {}",
            entry.id
        );
        assert_eq!(
            std::fs::read(path).unwrap(),
            saved,
            "readback mutated {}",
            entry.file
        );
    }
    assert_eq!(pins.len(), 19);
}
