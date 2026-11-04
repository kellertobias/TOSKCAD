use multicad_core::{
    commands::{Actor, Channel, Request, Session},
    constraint_sketch::Dimension,
    document::{Document, EntityData, Mutation},
    model::*,
    project_file::{self, ProjectFile},
    symbols::{self, SymbolAuthorRequest, SymbolKind},
};
fn actor() -> Actor {
    Actor::local(Channel::Mcp)
}
fn session() -> Session {
    Session::new(Document::new("Sourced UN threads").unwrap()).unwrap()
}
fn parameters(preset: &UnifiedThreadPreset, class: ThreadFit) -> ThreadParameters {
    ThreadParameters {
        family: preset.family,
        fit: class,
        major_diameter: Dimension::Literal {
            value: preset.nominal_diameter_inch,
            unit: "in".into(),
        },
        pitch: length(preset.pitch_mm),
        length: length(4. * preset.pitch_mm),
        ..Default::default()
    }
}
#[test]
fn independently_transcribed_limits_and_mode_specific_profiles_cover_exactly_four_series_rows() {
    let s = session();
    // Independent Table2.21 values in inches, not derived from the registry.
    let rows = [
        (
            "1/4-20 UNC",
            20,
            [0.2127, 0.2164],
            [0.2175, 0.2224],
            [0.2408, 0.2489],
            [0.196, 0.207],
        ),
        (
            "1/4-28 UNF",
            28,
            [0.2225, 0.2258],
            [0.2268, 0.2311],
            [0.2425, 0.2490],
            [0.211, 0.220],
        ),
        (
            "5/16-18 UNC",
            18,
            [0.2712, 0.2752],
            [0.2764, 0.2817],
            [0.3026, 0.3113],
            [0.252, 0.265],
        ),
        (
            "5/16-24 UNF",
            24,
            [0.2806, 0.2843],
            [0.2854, 0.2902],
            [0.3042, 0.3114],
            [0.267, 0.277],
        ),
    ];
    let presets = unified_thread_presets();
    assert_eq!(presets.len(), 4);
    for (name, tpi, male_pd, female_pd, male_major, female_minor) in rows {
        let preset = presets.iter().find(|p| p.designation == name).unwrap();
        assert_eq!(preset.threads_per_inch, tpi);
        assert_eq!(preset.pitch_mm, 25.4 / f64::from(tpi));
        for (class, mode, pd) in [
            (ThreadFit::Unified2A, ThreadMode::NewExternal, male_pd),
            (ThreadFit::Unified2B, ThreadMode::CutInternal, female_pd),
        ] {
            let p = parameters(preset, class);
            let r = p.resolve(s.parameters(), mode).unwrap();
            let f = r.fit.as_ref().unwrap();
            let h = 3_f64.sqrt() * r.pitch_mm / 2.;
            assert!(
                (r.nominal_pitch_diameter_mm.unwrap() - (preset.diameter_mm - 0.75 * h)).abs()
                    < 1e-10
            );
            assert!(
                (r.actual_pitch_diameter_mm.unwrap() - (pd[0] + pd[1]) * 25.4 / 2.).abs() < 1e-10
            );
            assert!(
                (r.profile.depth_mm
                    - if mode == ThreadMode::CutInternal {
                        5. * h / 8.
                    } else {
                        17. * h / 24.
                    })
                .abs()
                    < 1e-10
            );
            assert_eq!(f.limits.pitch.min_mm, Some(pd[0] * 25.4));
            assert_eq!(f.limits.pitch.max_mm, Some(pd[1] * 25.4));
            if mode == ThreadMode::NewExternal {
                assert_eq!(f.limits.major.min_mm, Some(male_major[0] * 25.4));
                assert_eq!(f.limits.major.max_mm, Some(male_major[1] * 25.4));
                assert_eq!(f.limits.minor.min_mm, None);
                assert_eq!(f.limits.minor.max_mm, None);
            } else {
                assert_eq!(f.limits.minor.min_mm, Some(female_minor[0] * 25.4));
                assert_eq!(f.limits.minor.max_mm, Some(female_minor[1] * 25.4));
                assert_eq!(f.limits.major.min_mm, Some(preset.diameter_mm));
                assert_eq!(f.limits.major.max_mm, None);
            }
            let adjusted = ThreadParameters {
                radial_allowance: length(0.001),
                axial_allowance: length(0.001),
                ..p
            };
            let a = adjusted.resolve(s.parameters(), mode).unwrap();
            let shift = 0.002 + 0.001 / 30_f64.to_radians().tan();
            let expected = f.pitch_diameter_mm
                + if mode == ThreadMode::CutInternal {
                    shift
                } else {
                    -shift
                };
            assert!((a.actual_pitch_diameter_mm.unwrap() - expected).abs() < 1e-10);
            assert_eq!(a.nominal_pitch_diameter_mm, r.nominal_pitch_diameter_mm);
        }
    }
}
#[test]
fn unsupported_series_class_start_length_and_allowance_fail_without_partial_commit() {
    let s = session();
    let preset = unified_thread_presets().remove(0);
    let good = parameters(&preset, ThreadFit::Unified2A);
    let bad = [
        ThreadParameters {
            family: ThreadFamily::Unf,
            ..good.clone()
        },
        ThreadParameters {
            fit: ThreadFit::Unified2B,
            ..good.clone()
        },
        ThreadParameters {
            fit: ThreadFit::NominalBasic,
            ..good.clone()
        },
        ThreadParameters {
            starts: 2,
            ..good.clone()
        },
        ThreadParameters {
            length: length(preset.diameter_mm * 1.5 + 0.001),
            ..good.clone()
        },
        ThreadParameters {
            radial_allowance: length(0.2),
            ..good.clone()
        },
        ThreadParameters {
            axial_allowance: length(0.05),
            ..good.clone()
        },
        ThreadParameters {
            family: ThreadFamily::Unified,
            ..good.clone()
        },
    ];
    let before = s.document().clone();
    for p in bad {
        assert!(
            prepare_thread(
                &s,
                s.revision(),
                actor(),
                &ThreadRequest::new("Invalid UN", p)
            )
            .is_err()
        );
        assert_eq!(s.document(), &before);
    }
    let mut wire = serde_json::to_value(&good).unwrap();
    wire["fit"] = "unified_3a".into();
    assert!(serde_json::from_value::<ThreadParameters>(wire).is_err());
    let wrong = ThreadDefinition {
        output: BodyOutput {
            body: multicad_core::document::EntityId::new(),
        },
        target: None,
        mode: ThreadMode::NewExternal,
        parameters: parameters(&preset, ThreadFit::Unified2B),
    };
    assert!(
        wrong.definition().is_err(),
        "Saved wrong-mode fits must fail before native compilation"
    );
    let nut = MechanicalPartParameters::HexNut {
        across_flats: length(12.),
        thread: Box::new(good),
    };
    assert!(nut.validate().is_err(), "Nested nuts need internal 2B");
}
#[test]
fn direct_and_suppressed_nested_unified_sources_require_reader_capability_and_roundtrip_independently()
 {
    let mut s = session();
    let preset = unified_thread_presets().remove(1);
    let before = s.document().clone();
    let request = ThreadRequest::new("UNF shaft", parameters(&preset, ThreadFit::Unified2A));
    let prepared = prepare_thread(&s, s.revision(), actor(), &request).unwrap();
    assert_eq!(s.document(), &before);
    s.commit(prepared.prepared).unwrap();
    let applied = s.document().clone();
    s.undo(s.revision(), actor()).unwrap();
    assert_eq!(s.document(), &before);
    s.redo(s.revision(), actor()).unwrap();
    assert_eq!(s.document(), &applied);
    let mut bytes = vec![];
    let project = ProjectFile::new(applied.clone()).unwrap();
    assert!(project.capabilities().contains(UNIFIED_THREAD_CAPABILITY));
    project_file::write(&project, &mut bytes, Default::default()).unwrap();
    let mut older = project_file::supported_capabilities();
    older.remove(UNIFIED_THREAD_CAPABILITY);
    assert!(project_file::read(&mut &bytes[..], Default::default(), &older).is_err());
    let reopened = project_file::read(
        &mut &bytes[..],
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap()
    .project;
    let independent = Session::new(reopened.document).unwrap();
    assert_eq!(
        read_thread(independent.document(), request.feature)
            .unwrap()
            .parameters,
        request.parameters
    );

    let mut nested = session();
    let nut = MechanicalPartRequest::new(
        "UNF nut",
        MechanicalPartParameters::HexNut {
            across_flats: length(12.),
            thread: Box::new(parameters(&preset, ThreadFit::Unified2B)),
        },
    );
    nested
        .commit(
            prepare_mechanical_part(&nested, nested.revision(), actor(), &nut)
                .unwrap()
                .prepared,
        )
        .unwrap();
    let mut entity = nested.document().get(nut.feature).unwrap().clone();
    let EntityData::Feature { suppressed, .. } = &mut entity.data else {
        unreachable!()
    };
    *suppressed = true;
    nested
        .apply(Request {
            expected_revision: nested.revision(),
            actor: actor(),
            label: "Suppress nut".into(),
            edits: vec![Mutation::Replace(entity)],
        })
        .unwrap();
    let project = ProjectFile::new(nested.document().clone()).unwrap();
    assert!(
        project.capabilities().contains(UNIFIED_THREAD_CAPABILITY),
        "Suppression cannot hide a saved-source reader requirement"
    );
    let legacy = ProjectFile::new(Document::new("No Unified source").unwrap()).unwrap();
    assert!(!legacy.capabilities().contains(UNIFIED_THREAD_CAPABILITY));
}

fn assert_reader(s: &Session, capability: &str) {
    let project = ProjectFile::new(s.document().clone()).unwrap();
    assert!(
        project.capabilities().contains(capability),
        "Every saved source needs its reader even when not currently instantiated"
    );
    let mut bytes = vec![];
    project_file::write(&project, &mut bytes, Default::default()).unwrap();
    let mut older = project_file::supported_capabilities();
    older.remove(capability);
    assert!(
        project_file::read(&mut bytes.as_slice(), Default::default(), &older)
            .unwrap_err()
            .contains(capability)
    );
    let read = project_file::read(
        &mut bytes.as_slice(),
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap();
    assert_eq!(read.project.document, *s.document());
}
fn assert_unified_reader(s: &Session) {
    assert_reader(s, UNIFIED_THREAD_CAPABILITY);
}
fn embedded_only(s: &mut Session, body: multicad_core::document::EntityId) -> Session {
    let author = SymbolAuthorRequest::new(
        "Unified source template",
        body,
        SymbolKind::Static,
        vec![],
        None,
    );
    let p = symbols::prepare_author(s, s.revision(), actor(), &author).unwrap();
    s.commit(p.prepared).unwrap();
    let definition = symbols::read_definition(s.document(), author.definition).unwrap();
    let mut imported = session();
    let p = symbols::prepare_import(&imported, 0, actor(), &definition).unwrap();
    imported.commit(p.prepared).unwrap();
    assert!(
        !imported
            .document()
            .entities()
            .any(|e| matches!(e.data, EntityData::Feature { .. }))
    );
    imported
}
#[test]
fn hidden_unified_thread_templates_require_the_reader_without_active_features() {
    for family in [ThreadFamily::Unc, ThreadFamily::Unf] {
        let preset = unified_thread_presets()
            .into_iter()
            .find(|p| p.family == family)
            .unwrap();
        let mut s = session();
        let request = ThreadRequest::new(
            "Embedded Unified shaft",
            parameters(&preset, ThreadFit::Unified2A),
        );
        let p = prepare_thread(&s, 0, actor(), &request).unwrap();
        s.commit(p.prepared).unwrap();
        assert_unified_reader(&embedded_only(&mut s, request.body));
    }
}
#[test]
fn hidden_unified_retained_hardware_references_require_reader_directly_and_in_templates() {
    for family in [ThreadFamily::Unc, ThreadFamily::Unf] {
        let preset = unified_thread_presets()
            .into_iter()
            .find(|p| p.family == family)
            .unwrap();
        let mut s = session();
        let reference = MechanicalPartParameters::HexNut {
            across_flats: length(18.),
            thread: Box::new(parameters(&preset, ThreadFit::Unified2B)),
        };
        let mut request = MechanicalPartRequest::new(
            "Metric actual, Unified reference",
            MechanicalPartParameters::HexNut {
                across_flats: length(18.),
                thread: Box::default(),
            },
        );
        request.provenance = Some(MechanicalPartProvenance {
            catalog_id: "test-only-retained-unified".into(),
            revision: "1".into(),
            designation: preset.designation,
            source_url: UNIFIED_THREAD_SOURCE_URL.into(),
            reference_geometry: reference,
            catalog_facts: None,
        });
        let p = prepare_mechanical_part(&s, 0, actor(), &request).unwrap();
        s.commit(p.prepared).unwrap();
        assert_unified_reader(&s);
        assert_unified_reader(&embedded_only(&mut s, request.body));
    }
}

#[test]
fn template_only_nominal_and_custom_threads_require_the_base_thread_reader() {
    for p in [
        ThreadParameters::default(),
        ThreadParameters {
            family: ThreadFamily::Custom,
            custom: Some(Default::default()),
            ..Default::default()
        },
    ] {
        let mut s = session();
        let request = ThreadRequest::new("Embedded basic source", p);
        let p = prepare_thread(&s, 0, actor(), &request).unwrap();
        s.commit(p.prepared).unwrap();
        assert_reader(&embedded_only(&mut s, request.body), THREAD_CAPABILITY);
    }
}
#[test]
fn template_only_basic_hardware_requires_its_base_provider_reader() {
    let mut s = session();
    let request = MechanicalPartRequest::new(
        "Embedded basic nut",
        MechanicalPartParameters::HexNut {
            across_flats: length(18.),
            thread: Box::default(),
        },
    );
    let p = prepare_mechanical_part(&s, 0, actor(), &request).unwrap();
    s.commit(p.prepared).unwrap();
    let embedded = embedded_only(&mut s, request.body);
    assert_reader(&embedded, MECHANICAL_PART_CAPABILITY);
    assert!(
        !required_capabilities(embedded.document()).contains(THREAD_CAPABILITY),
        "Nested thread parameters belong to their mechanical provider, not a standalone thread feature"
    );
}
