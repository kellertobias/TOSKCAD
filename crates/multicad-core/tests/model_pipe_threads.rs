use multicad_core::{
    commands::{Actor, Channel, Request, Session},
    document::{Document, EntityData, Mutation},
    model::*,
    project_file::{self, ProjectFile},
    symbols::{self, SymbolAuthorRequest, SymbolKind},
};
fn session() -> Session {
    Session::new(Document::new("Pipe thread intent").unwrap()).unwrap()
}
fn actor() -> Actor {
    Actor::local(Channel::Mcp)
}
fn intent(form: PipeThreadForm, size: PipeThreadSize) -> PipeThreadIntent {
    PipeThreadIntent {
        form,
        size,
        gauge: PipeGaugeConvention::Iso7Nominal1994,
    }
}
#[test]
fn sourced_pipe_gauge_datums_tolerances_and_base_diameters_are_distinct() {
    let s = session();
    for (size, tpi, major, pitch_d, minor, offset, external_len, internal_len) in [
        (
            PipeThreadSize::OneEighth,
            28,
            9.728,
            9.147,
            8.566,
            4.,
            6.5,
            7.4,
        ),
        (
            PipeThreadSize::OneQuarter,
            19,
            13.157,
            12.301,
            11.445,
            6.,
            9.7,
            11.,
        ),
    ] {
        let pitch = 25.4 / f64::from(tpi);
        for (form, mode, slope, gauge_z, len) in [
            (
                PipeThreadForm::R,
                ThreadMode::NewExternal,
                1. / 32.,
                offset,
                external_len,
            ),
            (
                PipeThreadForm::Rc,
                ThreadMode::CutInternal,
                -1. / 32.,
                pitch / 2.,
                internal_len,
            ),
        ] {
            let p = intent(form, size).parameters();
            let r = p.resolve(s.parameters(), mode).unwrap();
            let pipe = r.pipe.unwrap();
            assert_eq!(r.length_mm, len);
            assert_eq!(r.profile.radial_taper, slope);
            assert_eq!(pipe.gauge_offset_mm, gauge_z);
            assert!((2. * r.major_radius_mm + 2. * slope * gauge_z - major).abs() < 1e-12);
            assert_eq!(pipe.printed.gauge_major_mm, major);
            assert_eq!(pipe.printed.gauge_pitch_mm, pitch_d);
            assert_eq!(pipe.printed.gauge_minor_mm, minor);
            assert!((pipe.actual_gauge_pitch_mm - pitch_d).abs() < 0.0005);
            assert!((pipe.actual_gauge_minor_mm - minor).abs() < 0.0005);
            assert_eq!(pipe.printed.external_gauge_tolerance_turns, 1.);
            assert_eq!(pipe.printed.internal_gauge_tolerance_turns, 1.25);
            assert_eq!(pipe.printed.fitting_allowance_turns, 2.75);
            assert!((r.profile.rounding_radius_mm.unwrap() / pitch - 0.137278).abs() < 0.0000005);
        }
    }
}
#[test]
fn invalid_pipe_intent_and_dimensions_fail_before_any_transaction() {
    let s = session();
    let before = s.document().clone();
    let good = intent(PipeThreadForm::R, PipeThreadSize::OneEighth).parameters();
    for p in [
        ThreadParameters {
            pipe: None,
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
            hand: ThreadHand::Left,
            ..good.clone()
        },
        ThreadParameters {
            length: length(3.),
            ..good.clone()
        },
        ThreadParameters {
            major_diameter: length(9.728),
            ..good.clone()
        },
        ThreadParameters {
            pitch: length(1.),
            ..good.clone()
        },
        ThreadParameters {
            axial_allowance: length(0.001),
            ..good.clone()
        },
        ThreadParameters {
            radial_allowance: length(0.9071428571428571 / 32. + 0.001),
            ..good.clone()
        },
        ThreadParameters {
            family: ThreadFamily::Bspp,
            ..good.clone()
        },
    ] {
        assert!(prepare_thread(&s, 0, actor(), &ThreadRequest::new("Rejected pipe", p)).is_err());
        assert_eq!(s.revision(), 0);
        assert_eq!(s.document(), &before);
    }
    assert!(
        good.resolve(s.parameters(), ThreadMode::CutInternal)
            .is_err()
    );
    let mut at_limit = good;
    at_limit.radial_allowance = length((25.4 / 28.) / 32.);
    assert!(
        at_limit
            .resolve(s.parameters(), ThreadMode::NewExternal)
            .is_err()
    );
    at_limit.length = length(4. + (1. + 2.75) * (25.4 / 28.));
    let r = at_limit
        .resolve(s.parameters(), ThreadMode::NewExternal)
        .unwrap();
    let pipe = r.pipe.unwrap();
    assert!((pipe.gauge_displacement_turns - 1.).abs() < 1e-12);
    assert!((pipe.actual_gauge_offset_mm - (4. + 25.4 / 28.)).abs() < 1e-12);
    let mut internal = intent(PipeThreadForm::Rc, PipeThreadSize::OneQuarter).parameters();
    internal.length = length(7.4); // longer than gauge length; shorter than col16.
    assert!(
        internal
            .resolve(s.parameters(), ThreadMode::CutInternal)
            .is_err()
    );
    let mut wire =
        serde_json::to_value(intent(PipeThreadForm::R, PipeThreadSize::OneQuarter)).unwrap();
    wire["form"] = "rp".into();
    assert!(serde_json::from_value::<PipeThreadIntent>(wire).is_err());
}
fn assert_reader(s: &Session) {
    let p = ProjectFile::new(s.document().clone()).unwrap();
    assert!(p.capabilities().contains(BSPT_THREAD_CAPABILITY));
    let mut bytes = vec![];
    project_file::write(&p, &mut bytes, Default::default()).unwrap();
    let mut supported = project_file::supported_capabilities();
    let read = project_file::read(&mut bytes.as_slice(), Default::default(), &supported).unwrap();
    assert_eq!(read.project.document, *s.document());
    supported.remove(BSPT_THREAD_CAPABILITY);
    assert!(
        project_file::read(&mut bytes.as_slice(), Default::default(), &supported)
            .unwrap_err()
            .contains(BSPT_THREAD_CAPABILITY)
    );
}
#[test]
fn pipe_source_survives_history_reopen_suppression_and_embedded_templates() {
    let mut s = session();
    let request = ThreadRequest::new(
        "R 1/8",
        intent(PipeThreadForm::R, PipeThreadSize::OneEighth).parameters(),
    );
    let p = prepare_thread(&s, 0, actor(), &request).unwrap();
    s.commit(p.prepared).unwrap();
    assert_eq!(
        read_thread(s.document(), request.feature)
            .unwrap()
            .parameters,
        request.parameters
    );
    assert_reader(&s);
    s.undo(s.revision(), actor()).unwrap();
    assert!(!required_capabilities(s.document()).contains(BSPT_THREAD_CAPABILITY));
    s.redo(s.revision(), actor()).unwrap();
    assert_reader(&s);
    let mut e = s.document().get(request.feature).unwrap().clone();
    let EntityData::Feature { suppressed, .. } = &mut e.data else {
        panic!()
    };
    *suppressed = true;
    s.apply(Request {
        expected_revision: s.revision(),
        actor: actor(),
        label: "Suppress pipe".into(),
        edits: vec![Mutation::Replace(e)],
    })
    .unwrap();
    assert_reader(&s);
    // Author from the unsuppressed history so the template remains executable.
    s.undo(s.revision(), actor()).unwrap();
    let author = SymbolAuthorRequest::new(
        "Reusable pipe",
        request.body,
        SymbolKind::Static,
        vec![],
        None,
    );
    let p = symbols::prepare_author(&s, s.revision(), actor(), &author).unwrap();
    s.commit(p.prepared).unwrap();
    let definition = symbols::read_definition(s.document(), author.definition).unwrap();
    let mut embedded = session();
    let p = symbols::prepare_import(&embedded, 0, actor(), &definition).unwrap();
    embedded.commit(p.prepared).unwrap();
    assert_reader(&embedded);
    assert!(
        !embedded
            .document()
            .entities()
            .any(|e| matches!(e.data, EntityData::Feature { .. }))
    );
}
#[test]
fn legacy_source_omits_pipe_and_new_reader_requirement() {
    let p = ThreadParameters::default();
    let wire = serde_json::to_value(&p).unwrap();
    assert!(wire.get("pipe").is_none());
    assert_eq!(serde_json::from_value::<ThreadParameters>(wire).unwrap(), p);
    let mut s = session();
    let p = prepare_thread(&s, 0, actor(), &ThreadRequest::new("Metric", p)).unwrap();
    s.commit(p.prepared).unwrap();
    assert!(!required_capabilities(s.document()).contains(BSPT_THREAD_CAPABILITY));
}
#[test]
fn nested_hardware_and_template_only_pipe_sources_require_the_pipe_reader() {
    for form in [PipeThreadForm::R, PipeThreadForm::Rc] {
        let mut s = session();
        let thread = intent(form, PipeThreadSize::OneEighth).parameters();
        let parameters = if form == PipeThreadForm::Rc {
            MechanicalPartParameters::HexNut {
                across_flats: length(16.),
                thread: Box::new(thread),
            }
        } else {
            MechanicalPartParameters::MachineScrew {
                underhead_length: thread.length.clone(),
                shank_diameter: thread.major_diameter.clone(),
                head: ScrewHead::ExternalHex {
                    across_flats: length(16.),
                    height: length(3.),
                },
                drive: ScrewDrive::ExternalHex,
                thread: Box::new(thread),
            }
        };
        let request = MechanicalPartRequest::new("Pipe reference hardware", parameters);
        let p = prepare_mechanical_part(&s, 0, actor(), &request).unwrap();
        s.commit(p.prepared).unwrap();
        assert_reader(&s);
        let author = SymbolAuthorRequest::new(
            "Reusable pipe hardware",
            request.body,
            SymbolKind::Static,
            vec![],
            None,
        );
        let p = symbols::prepare_author(&s, s.revision(), actor(), &author).unwrap();
        s.commit(p.prepared).unwrap();
        let definition = symbols::read_definition(s.document(), author.definition).unwrap();
        let mut embedded = session();
        let p = symbols::prepare_import(&embedded, 0, actor(), &definition).unwrap();
        embedded.commit(p.prepared).unwrap();
        assert_reader(&embedded);
    }
}
#[test]
fn retained_pipe_reference_requires_the_reader_after_the_authored_family_changes() {
    let mut s = session();
    let mut request = MechanicalPartRequest::new(
        "Edited pipe reference nut",
        MechanicalPartParameters::HexNut {
            across_flats: length(16.),
            thread: Box::default(),
        },
    );
    request.provenance = Some(MechanicalPartProvenance {
        catalog_id: "test-only-pipe-reference".into(),
        revision: "1".into(),
        designation: "Retained Rc reference".into(),
        source_url: BSPT_THREAD_SOURCE_URL.into(),
        reference_geometry: MechanicalPartParameters::HexNut {
            across_flats: length(16.),
            thread: Box::new(intent(PipeThreadForm::Rc, PipeThreadSize::OneEighth).parameters()),
        },
        catalog_facts: None,
    });
    let p = prepare_mechanical_part(&s, 0, actor(), &request).unwrap();
    s.commit(p.prepared).unwrap();
    assert!(
        !read_mechanical_part(s.document(), request.feature)
            .unwrap()
            .matches_reference_dimensions(s.parameters())
            .unwrap()
    );
    assert_reader(&s);
    let author = SymbolAuthorRequest::new(
        "Reusable edited pipe nut",
        request.body,
        SymbolKind::Static,
        vec![],
        None,
    );
    let p = symbols::prepare_author(&s, s.revision(), actor(), &author).unwrap();
    s.commit(p.prepared).unwrap();
    let definition = symbols::read_definition(s.document(), author.definition).unwrap();
    let mut embedded = session();
    let p = symbols::prepare_import(&embedded, 0, actor(), &definition).unwrap();
    embedded.commit(p.prepared).unwrap();
    assert_reader(&embedded);
}
