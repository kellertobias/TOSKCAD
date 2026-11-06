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
fn intent(form: NptThreadForm, size: NptThreadSize) -> NptThreadIntent {
    NptThreadIntent {
        form,
        size,
        truncation: NptTruncationTarget::MidpointConeParallel,
        gauge: NptGaugeConvention::Nbs1959PlainFace,
    }
}
#[test]
fn npt_gauge_anchor_axial_profile_stock_transition_and_engagement_are_explicit() {
    let s = session();
    for row in npt_thread_presets() {
        let pitch = 25.4 / f64::from(row.threads_per_inch);
        for (form, mode) in [
            (NptThreadForm::External, ThreadMode::NewExternal),
            (NptThreadForm::Internal, ThreadMode::CutInternal),
        ] {
            let i = intent(form, row.size);
            for radial in [0., pitch / 32.] {
                let mut p = i.parameters();
                p.radial_allowance = length(radial);
                let r = p.resolve(s.parameters(), mode).unwrap();
                let n = r.npt.unwrap();
                assert_eq!(r.profile.lower_flank_deg, 30.);
                assert_eq!(r.profile.upper_flank_deg, 30.);
                assert_eq!(r.profile.rounding_radius_mm, None);
                assert_eq!(n.nominal_gauge_pitch_mm, row.published.e1_inch * 25.4);
                let actual_base = r.major_radius_mm * 2. - r.profile.depth_mm;
                let gauge = actual_base + 2. * i.radial_taper() * n.actual_gauge_offset_mm;
                assert!((gauge - n.nominal_gauge_pitch_mm).abs() < 1e-12);
                assert!((r.actual_pitch_diameter_mm.unwrap() - actual_base).abs() < 1e-12);
                let m = 3_f64.sqrt();
                let k = i.radial_taper();
                let slopes = 1. / (m - k) + 1. / (m + k);
                let fundamental = pitch / slopes;
                assert!(
                    (fundamental - r.profile.depth_mm - 2. * n.target_truncation_mm).abs() < 1e-12
                );
                assert!(
                    (2. * r.profile.crest_width_mm + r.profile.depth_mm * slopes - pitch).abs()
                        < 1e-12
                );
                assert!(n.target_truncation_mm / pitch >= row.minimum_truncation_over_pitch);
                assert!(n.target_truncation_mm / pitch <= row.maximum_truncation_over_pitch);
                if form == NptThreadForm::External {
                    assert_eq!(r.length_mm, row.published.l2_inch * 25.4);
                    let limit = n.stock_crest_radius_limit_mm.unwrap();
                    let z = n.actual_stock_intersection_mm.unwrap();
                    assert!((r.major_radius_mm + k * z - limit).abs() < 1e-12);
                    assert!(z > 0.);
                    if radial == 0. {
                        assert!(z < r.length_mm);
                    }
                    assert_eq!(n.complete_crest_length_mm, Some(z.clamp(0., r.length_mm)));
                    assert!(
                        (n.complete_crest_length_mm.unwrap()
                            + n.imperfect_crest_length_mm.unwrap()
                            - r.length_mm)
                            .abs()
                            < 1e-12
                    );
                } else {
                    assert_eq!(n.gauge_offset_mm, 0.);
                    assert_eq!(n.stock_crest_radius_limit_mm, None);
                    assert!(r.length_mm >= n.minimum_receiving_length_mm - 1e-12);
                    assert!(
                        (n.minimum_receiving_length_mm
                            - row.published.l1_inch * 25.4
                            - 3. * pitch
                            - 32. * radial)
                            .abs()
                            < 1e-12
                    );
                }
            }
        }
    }
}
#[test]
fn unsupported_npt_requests_are_atomic_and_do_not_fall_back_to_custom() {
    let s = session();
    let before = s.document().clone();
    let good = intent(NptThreadForm::External, NptThreadSize::OneEighth).parameters();
    for p in [
        ThreadParameters {
            npt: None,
            ..good.clone()
        },
        ThreadParameters {
            fit: ThreadFit::NominalBasic,
            ..good.clone()
        },
        ThreadParameters {
            family: ThreadFamily::Custom,
            ..good.clone()
        },
        ThreadParameters {
            hand: ThreadHand::Left,
            ..good.clone()
        },
        ThreadParameters {
            starts: 2,
            ..good.clone()
        },
        ThreadParameters {
            length: length(20.),
            ..good.clone()
        },
        ThreadParameters {
            major_diameter: length(10.),
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
            radial_allowance: length(-0.001),
            ..good.clone()
        },
        ThreadParameters {
            radial_allowance: length(25.4 / 27. / 32. + 0.001),
            ..good.clone()
        },
    ] {
        assert!(prepare_thread(&s, 0, actor(), &ThreadRequest::new("Invalid NPT", p)).is_err());
        assert_eq!(s.revision(), 0);
        assert_eq!(s.document(), &before);
    }
    assert!(
        good.resolve(s.parameters(), ThreadMode::CutInternal)
            .is_err()
    );
    let mut internal = intent(NptThreadForm::Internal, NptThreadSize::OneQuarter).parameters();
    internal.length =
        length(internal.npt.unwrap().preset().published.l1_inch * 25.4 + 2. * 25.4 / 18.);
    assert!(
        internal
            .resolve(s.parameters(), ThreadMode::CutInternal)
            .is_err()
    );
    let wire = serde_json::to_value(good.npt.unwrap()).unwrap();
    for (key, value) in [
        ("form", "nptf"),
        ("size", "3/8"),
        ("gauge", "current_asme"),
        ("truncation", "sharp"),
    ] {
        let mut invalid = wire.clone();
        invalid[key] = value.into();
        assert!(serde_json::from_value::<NptThreadIntent>(invalid).is_err());
    }
}
#[test]
fn nested_npt_screw_head_clearance_uses_stock_clipped_extent() {
    let s = session();
    let thread = intent(NptThreadForm::External, NptThreadSize::OneQuarter).parameters();
    let recipe = MechanicalPartParameters::MachineScrew {
        underhead_length: Some(thread.length.clone()),
        length_input: None,
        shank_diameter: length(13.6),
        head: ScrewHead::ExternalHex {
            across_flats: length(13.76),
            height: length(3.),
        },
        drive: ScrewDrive::ExternalHex,
        thread: Box::new(thread),
    };
    assert!(recipe.resolve(s.parameters()).is_ok());
}
fn assert_reader(s: &Session) {
    let p = ProjectFile::new(s.document().clone()).unwrap();
    assert!(p.capabilities().contains(NPT_THREAD_CAPABILITY));
    let mut bytes = vec![];
    project_file::write(&p, &mut bytes, Default::default()).unwrap();
    let mut supported = project_file::supported_capabilities();
    let read = project_file::read(&mut bytes.as_slice(), Default::default(), &supported).unwrap();
    assert_eq!(read.project.document, *s.document());
    supported.remove(NPT_THREAD_CAPABILITY);
    assert!(
        project_file::read(&mut bytes.as_slice(), Default::default(), &supported)
            .unwrap_err()
            .contains(NPT_THREAD_CAPABILITY)
    );
}
#[test]
fn npt_source_survives_history_reopen_suppression_and_embedded_templates() {
    let mut s = session();
    let request = ThreadRequest::new(
        "NPT 1/8",
        intent(NptThreadForm::External, NptThreadSize::OneEighth).parameters(),
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
    assert!(!required_capabilities(s.document()).contains(NPT_THREAD_CAPABILITY));
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
        label: "Suppress npt".into(),
        edits: vec![Mutation::Replace(e)],
    })
    .unwrap();
    assert_reader(&s);
    // Author from the unsuppressed history so the template remains executable.
    s.undo(s.revision(), actor()).unwrap();
    let author = SymbolAuthorRequest::new(
        "Reusable npt",
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
fn legacy_source_omits_npt_and_new_reader_requirement() {
    let p = ThreadParameters::default();
    let wire = serde_json::to_value(&p).unwrap();
    assert!(wire.get("npt").is_none());
    assert_eq!(serde_json::from_value::<ThreadParameters>(wire).unwrap(), p);
    let mut s = session();
    let p = prepare_thread(&s, 0, actor(), &ThreadRequest::new("Metric", p)).unwrap();
    s.commit(p.prepared).unwrap();
    assert!(!required_capabilities(s.document()).contains(NPT_THREAD_CAPABILITY));
}
#[test]
fn nested_hardware_and_template_only_npt_sources_require_the_npt_reader() {
    for form in [NptThreadForm::External, NptThreadForm::Internal] {
        let mut s = session();
        let thread = intent(form, NptThreadSize::OneEighth).parameters();
        let parameters = if form == NptThreadForm::Internal {
            MechanicalPartParameters::HexNut {
                across_flats: length(16.),
                thread: Box::new(thread),
            }
        } else {
            MechanicalPartParameters::MachineScrew {
                underhead_length: Some(thread.length.clone()),
                length_input: None,
                shank_diameter: thread.major_diameter.clone(),
                head: ScrewHead::ExternalHex {
                    across_flats: length(16.),
                    height: length(3.),
                },
                drive: ScrewDrive::ExternalHex,
                thread: Box::new(thread),
            }
        };
        let request = MechanicalPartRequest::new("Npt reference hardware", parameters);
        let p = prepare_mechanical_part(&s, 0, actor(), &request).unwrap();
        s.commit(p.prepared).unwrap();
        assert_reader(&s);
        let author = SymbolAuthorRequest::new(
            "Reusable npt hardware",
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
fn retained_npt_reference_requires_the_reader_after_the_authored_family_changes() {
    let mut s = session();
    let mut request = MechanicalPartRequest::new(
        "Edited npt reference nut",
        MechanicalPartParameters::HexNut {
            across_flats: length(16.),
            thread: Box::default(),
        },
    );
    request.provenance = Some(MechanicalPartProvenance {
        catalog_id: "test-only-npt-reference".into(),
        revision: "1".into(),
        designation: "Retained NPT reference".into(),
        source_url: NPT_THREAD_SOURCE_URL.into(),
        reference_geometry: MechanicalPartParameters::HexNut {
            across_flats: length(16.),
            thread: Box::new(
                intent(NptThreadForm::Internal, NptThreadSize::OneEighth).parameters(),
            ),
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
        "Reusable edited npt nut",
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
