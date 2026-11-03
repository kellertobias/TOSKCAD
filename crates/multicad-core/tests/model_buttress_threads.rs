use multicad_core::{
    commands::{Actor, Channel, Request, Session},
    document::{Document, EntityData, Mutation},
    model::*,
};
fn session() -> Session {
    Session::new(Document::new("Named buttress").unwrap()).unwrap()
}
fn actor() -> Actor {
    Actor::local(Channel::Mcp)
}
#[test]
fn named_buttress_preserves_source_root_and_distinct_internal_clearance_for_both_hands_and_load_sides()
 {
    let s = session();
    let row = buttress_thread_presets().remove(0);
    for side in [
        ButtressLoadFlankSide::NegativeAxisSide,
        ButtressLoadFlankSide::PositiveAxisSide,
    ] {
        for hand in [ThreadHand::Right, ThreadHand::Left] {
            let mut p = row.parameters(side);
            p.hand = hand;
            let external = p.resolve(s.parameters(), ThreadMode::NewExternal).unwrap();
            let internal = p.resolve(s.parameters(), ThreadMode::CutInternal).unwrap();
            let a = external.buttress.as_ref().unwrap();
            let b = internal.buttress.as_ref().unwrap();
            assert_eq!(a.intent, b.intent);
            assert_eq!(a.intent.load_flank_side, side);
            assert_eq!(external.nominal_pitch_diameter_mm, Some(17.));
            assert_eq!(internal.nominal_pitch_diameter_mm, Some(17.));
            assert!((external.actual_pitch_diameter_mm.unwrap() - 17.).abs() < 1e-12);
            assert!((internal.actual_pitch_diameter_mm.unwrap() - 17.6351645699013).abs() < 1e-11);
            assert!((a.depth_mm - 3.4710735327306654).abs() < 1e-11);
            assert_eq!(b.depth_mm, 3.);
            assert_eq!(a.nominal_engagement_height_mm, 3.);
            assert_eq!(a.root_circle.as_ref().unwrap().radius_mm, 0.49708);
            assert!(b.root_circle.is_none());
            assert!((a.crest_width_mm - 1.0553629272909996).abs() < 1e-12);
            assert!((b.crest_width_mm - a.crest_width_mm - 0.2).abs() < 1e-12);
            let sign = if side == ButtressLoadFlankSide::NegativeAxisSide {
                1.
            } else {
                -1.
            };
            assert_eq!(b.crest_phase_axial_mm, sign * 0.1);
            assert_eq!(a.crest_phase_axial_mm, 0.);
            let hand_sign = if hand == ThreadHand::Right { 1. } else { -1. };
            assert!(
                (-hand_sign * b.phase_rotation_rad * row.pitch_mm / std::f64::consts::TAU
                    - b.crest_phase_axial_mm)
                    .abs()
                    < 1e-14
            );
            for (is_lower, direction, angle) in [
                (true, -1., external.profile.lower_flank_deg),
                (false, 1., external.profile.upper_flank_deg),
            ] {
                let t = angle.to_radians().tan();
                let drop = 10. - 8.5;
                let male = direction * (a.crest_width_mm / 2. + drop * t);
                let female =
                    b.crest_phase_axial_mm + direction * (b.crest_width_mm / 2. + drop * t);
                let load = (side == ButtressLoadFlankSide::NegativeAxisSide) == is_lower;
                assert!((female - male - if load { 0. } else { direction * 0.2 }).abs() < 1e-12);
            }
            for (actual, printed) in [
                (a.depth_mm, row.printed.external_depth_mm),
                (
                    a.nominal_construction_minor_diameter_mm,
                    row.printed.external_minor_mm,
                ),
                (
                    b.nominal_construction_pitch_diameter_mm,
                    row.printed.internal_pitch_mm,
                ),
                (
                    a.root_circle.as_ref().unwrap().radius_mm,
                    row.printed.root_radius_mm,
                ),
            ] {
                assert!((actual - printed).abs() <= 0.0005);
            }
            p.radial_allowance = length(0.005);
            let allowance = p.resolve(s.parameters(), ThreadMode::NewExternal).unwrap();
            assert!((allowance.actual_pitch_diameter_mm.unwrap() - 16.99).abs() < 1e-12);
            assert_eq!(allowance.buttress.as_ref().unwrap().intent, a.intent);
            assert!(
                (allowance
                    .buttress
                    .unwrap()
                    .root_circle
                    .unwrap()
                    .center_radius_mm
                    - a.root_circle.as_ref().unwrap().center_radius_mm
                    + 0.005)
                    .abs()
                    < 1e-12
            );
        }
    }
}
#[test]
fn incomplete_mixed_and_unsupported_buttress_inputs_fail_without_a_transaction() {
    let s = session();
    let before = s.document().clone();
    let good = buttress_thread_presets()
        .remove(0)
        .parameters(ButtressLoadFlankSide::NegativeAxisSide);
    for p in [
        ThreadParameters {
            buttress: None,
            ..good.clone()
        },
        ThreadParameters {
            family: ThreadFamily::Custom,
            ..good.clone()
        },
        ThreadParameters {
            major_diameter: length(21.),
            ..good.clone()
        },
        ThreadParameters {
            pitch: length(5.),
            ..good.clone()
        },
        ThreadParameters {
            starts: 2,
            ..good.clone()
        },
        ThreadParameters {
            fit: ThreadFit::Unified2A,
            ..good.clone()
        },
        ThreadParameters {
            axial_allowance: length(0.001),
            ..good.clone()
        },
    ] {
        assert!(
            prepare_thread(&s, 0, actor(), &ThreadRequest::new("Rejected buttress", p)).is_err()
        );
        assert_eq!(s.document(), &before);
        assert_eq!(s.revision(), 0);
    }
    let wire = serde_json::to_value(good.buttress.unwrap()).unwrap();
    for (field, value) in [("series", "din_513_2020"), ("load_flank_side", "push")] {
        let mut bad = wire.clone();
        bad[field] = value.into();
        assert!(serde_json::from_value::<ButtressIntent>(bad).is_err());
    }
}

use multicad_core::{
    project_file::{self, ProjectFile},
    symbols::{self, SymbolAuthorRequest, SymbolKind},
};
fn reader(s: &Session) {
    let project = ProjectFile::new(s.document().clone()).unwrap();
    assert!(project.capabilities().contains(BUTTRESS_THREAD_CAPABILITY));
    let mut bytes = vec![];
    project_file::write(&project, &mut bytes, Default::default()).unwrap();
    let mut supported = project_file::supported_capabilities();
    let read = project_file::read(&mut bytes.as_slice(), Default::default(), &supported).unwrap();
    assert_eq!(read.project.document, *s.document());
    supported.remove(BUTTRESS_THREAD_CAPABILITY);
    assert!(
        project_file::read(&mut bytes.as_slice(), Default::default(), &supported)
            .unwrap_err()
            .contains(BUTTRESS_THREAD_CAPABILITY)
    );
}
fn embedded_reader(s: &mut Session, body: multicad_core::document::EntityId) {
    let author =
        SymbolAuthorRequest::new("Buttress template", body, SymbolKind::Static, vec![], None);
    let p = symbols::prepare_author(s, s.revision(), actor(), &author).unwrap();
    s.commit(p.prepared).unwrap();
    let definition = symbols::read_definition(s.document(), author.definition).unwrap();
    let mut embedded = session();
    let p = symbols::prepare_import(&embedded, 0, actor(), &definition).unwrap();
    embedded.commit(p.prepared).unwrap();
    reader(&embedded);
    assert!(
        !embedded
            .document()
            .entities()
            .any(|e| matches!(e.data, EntityData::Feature { .. }))
    );
}
#[test]
fn buttress_sources_survive_history_reopen_suppression_and_template_only_reader_gates() {
    for row in buttress_thread_presets() {
        let mut s = session();
        let mut parameters = row.parameters(ButtressLoadFlankSide::NegativeAxisSide);
        parameters.hand = ThreadHand::Left;
        parameters.starts = 1;
        let request = ThreadRequest::new("LH buttress", parameters);
        let p = prepare_thread(&s, 0, actor(), &request).unwrap();
        s.commit(p.prepared).unwrap();
        reader(&s);
        assert_eq!(
            read_thread(s.document(), request.feature)
                .unwrap()
                .parameters,
            request.parameters
        );
        s.undo(s.revision(), actor()).unwrap();
        assert!(!required_capabilities(s.document()).contains(BUTTRESS_THREAD_CAPABILITY));
        s.redo(s.revision(), actor()).unwrap();
        reader(&s);
        let mut feature = s.document().get(request.feature).unwrap().clone();
        let EntityData::Feature { suppressed, .. } = &mut feature.data else {
            panic!()
        };
        *suppressed = true;
        s.apply(Request {
            expected_revision: s.revision(),
            actor: actor(),
            label: "Suppress lead screw".into(),
            edits: vec![Mutation::Replace(feature)],
        })
        .unwrap();
        reader(&s);
        s.undo(s.revision(), actor()).unwrap();
        embedded_reader(&mut s, request.body);
    }
}
#[test]
fn nested_and_retained_reference_buttress_profiles_require_the_reader() {
    for row in buttress_thread_presets() {
        for retained_only in [false, true] {
            let mut s = session();
            let reference = MechanicalPartParameters::HexNut {
                across_flats: length(30.),
                thread: Box::new(row.parameters(ButtressLoadFlankSide::NegativeAxisSide)),
            };
            let mut request = MechanicalPartRequest::new(
                "Lead reference nut",
                if retained_only {
                    MechanicalPartParameters::HexNut {
                        across_flats: length(30.),
                        thread: Box::default(),
                    }
                } else {
                    reference.clone()
                },
            );
            request.provenance = Some(MechanicalPartProvenance {
                catalog_id: "test-only-basic-lead".into(),
                revision: "1".into(),
                designation: row.designation.into(),
                source_url: row.source_url.into(),
                reference_geometry: reference,
                catalog_facts: None,
            });
            let p = prepare_mechanical_part(&s, 0, actor(), &request).unwrap();
            s.commit(p.prepared).unwrap();
            reader(&s);
            assert_eq!(
                read_mechanical_part(s.document(), request.feature)
                    .unwrap()
                    .matches_reference_dimensions(s.parameters())
                    .unwrap(),
                !retained_only
            );
            embedded_reader(&mut s, request.body);
        }
    }
}
#[test]
fn changing_only_buttress_load_side_breaks_a_retained_reference_match() {
    let mut s = session();
    let row = buttress_thread_presets().remove(0);
    let reference = MechanicalPartParameters::HexNut {
        across_flats: length(30.),
        thread: Box::new(row.parameters(ButtressLoadFlankSide::NegativeAxisSide)),
    };
    let authored = MechanicalPartParameters::HexNut {
        across_flats: length(30.),
        thread: Box::new(row.parameters(ButtressLoadFlankSide::PositiveAxisSide)),
    };
    let mut request = MechanicalPartRequest::new("Reversed load face", authored);
    request.provenance = Some(MechanicalPartProvenance {
        catalog_id: "test-only-buttress-reference".into(),
        revision: "1".into(),
        designation: row.designation.into(),
        source_url: row.source_url.into(),
        reference_geometry: reference,
        catalog_facts: None,
    });
    let prepared = prepare_mechanical_part(&s, 0, actor(), &request).unwrap();
    s.commit(prepared.prepared).unwrap();
    assert!(
        !read_mechanical_part(s.document(), request.feature)
            .unwrap()
            .matches_reference_dimensions(s.parameters())
            .unwrap()
    );
    reader(&s);
}
