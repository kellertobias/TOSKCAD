use multicad_core::{
    commands::{Actor, Channel, Request, Session},
    document::{Document, EntityData, Mutation},
    model::*,
    project_file::{self, ProjectFile},
    symbols::{self, SymbolAuthorRequest, SymbolKind},
};
fn session() -> Session {
    Session::new(Document::new("Sourced round threads").unwrap()).unwrap()
}
fn actor() -> Actor {
    Actor::local(Channel::Mcp)
}
#[test]
fn round_sources_keep_actual_internal_cutter_radii_and_shared_pitch_diameter_for_both_hands() {
    let s = session();
    let rows = round_thread_presets();
    assert_eq!(rows.len(), 2);
    for row in rows {
        for mode in [
            ThreadMode::NewExternal,
            ThreadMode::CutExternal,
            ThreadMode::CutInternal,
        ] {
            for hand in [ThreadHand::Right, ThreadHand::Left] {
                let mut p = row.parameters();
                p.hand = hand;
                let r = p.resolve(s.parameters(), mode).unwrap();
                let q = r.round.as_ref().unwrap();
                let internal = mode == ThreadMode::CutInternal;
                assert_eq!(p.fit, ThreadFit::NominalBasic);
                assert!(r.fit.is_none());
                assert_eq!(r.starts, 1);
                assert_eq!(r.hand, hand);
                assert_eq!(r.profile.lower_flank_deg, 15.);
                assert_eq!(r.profile.upper_flank_deg, 15.);
                assert_eq!(r.profile.radial_taper, 0.);
                assert_eq!(r.profile.crest_width_mm, 0.);
                let major = row.diameter_mm + if internal { 0.1 * row.pitch_mm } else { 0. };
                let pd = row.diameter_mm - row.pitch_mm / 2.;
                assert!((r.major_radius_mm * 2. - major).abs() < 1e-12);
                assert!(
                    (q.nominal_construction_minor_diameter_mm - (major - row.pitch_mm)).abs()
                        < 1e-12
                );
                assert!((r.nominal_pitch_diameter_mm.unwrap() - pd).abs() < 1e-12);
                assert!((r.actual_pitch_diameter_mm.unwrap() - pd).abs() < 1e-12);
                assert!((r.profile.depth_mm - row.pitch_mm / 2.).abs() < 1e-12);
                assert!((r.lead_mm - row.pitch_mm).abs() < 1e-12);
                let coefficients = if internal {
                    (0.221047, 0.255967)
                } else {
                    (0.238507, 0.238507)
                };
                assert!(
                    (q.cutter_crest_radius_mm / row.pitch_mm - coefficients.0).abs() < 0.0000005
                );
                assert!(
                    (q.cutter_root_radius_mm / row.pitch_mm - coefficients.1).abs() < 0.0000005
                );
                assert_eq!(r.profile.rounding_radius_mm, Some(q.cutter_crest_radius_mm));
                assert_eq!(
                    q.material_crest_radius_mm,
                    if internal {
                        q.cutter_root_radius_mm
                    } else {
                        q.cutter_crest_radius_mm
                    }
                );
                assert_eq!(
                    q.material_root_radius_mm,
                    if internal {
                        q.cutter_crest_radius_mm
                    } else {
                        q.cutter_root_radius_mm
                    }
                );
                assert!(
                    (q.actual_cutter_crest_circle.center_radius_mm + q.cutter_crest_radius_mm
                        - r.major_radius_mm)
                        .abs()
                        < 1e-12
                );
                assert!(
                    (q.actual_cutter_root_circle.center_radius_mm
                        - q.cutter_root_radius_mm
                        - (r.major_radius_mm - r.profile.depth_mm))
                        .abs()
                        < 1e-12
                );
                let wire = serde_json::to_value(&p).unwrap();
                assert_eq!(wire["major_diameter"]["unit"], "mm");
                assert_eq!(wire["pitch"]["unit"], "in");
                assert!(wire.get("round").is_none());
                assert!(wire.get("buttress").is_none());
            }
        }
    }
}
#[test]
fn round_radial_fabrication_allowance_preserves_source_circles_and_changes_actual_pd() {
    let s = session();
    for row in round_thread_presets() {
        for mode in [ThreadMode::NewExternal, ThreadMode::CutInternal] {
            let mut p = row.parameters();
            let original = p.resolve(s.parameters(), mode).unwrap();
            p.radial_allowance = length(0.005);
            let changed = p.resolve(s.parameters(), mode).unwrap();
            let sign = if mode == ThreadMode::CutInternal {
                1.
            } else {
                -1.
            };
            assert_eq!(
                changed.nominal_pitch_diameter_mm,
                original.nominal_pitch_diameter_mm
            );
            assert!(
                (changed.actual_pitch_diameter_mm.unwrap()
                    - original.actual_pitch_diameter_mm.unwrap()
                    - sign * 0.01)
                    .abs()
                    < 1e-12
            );
            let a = original.round.unwrap();
            let b = changed.round.unwrap();
            assert_eq!(a.preset, b.preset);
            assert_eq!(a.cutter_crest_radius_mm, b.cutter_crest_radius_mm);
            assert_eq!(a.cutter_root_radius_mm, b.cutter_root_radius_mm);
            assert!(
                (b.actual_cutter_crest_circle.center_radius_mm
                    - a.actual_cutter_crest_circle.center_radius_mm
                    - sign * 0.005)
                    .abs()
                    < 1e-12
            );
            assert!(
                (b.actual_cutter_root_circle.center_radius_mm
                    - a.actual_cutter_root_circle.center_radius_mm
                    - sign * 0.005)
                    .abs()
                    < 1e-12
            );
        }
    }
}
#[test]
fn unsupported_round_rows_starts_fits_and_axial_allowance_fail_without_partial_commit() {
    let s = session();
    let before = s.document().clone();
    for row in round_thread_presets() {
        let good = row.parameters();
        for p in [
            ThreadParameters {
                major_diameter: length(row.diameter_mm + 0.1),
                ..good.clone()
            },
            ThreadParameters {
                pitch: length(row.pitch_mm + 0.1),
                ..good.clone()
            },
            ThreadParameters {
                fit: ThreadFit::Unified2A,
                ..good.clone()
            },
            ThreadParameters {
                starts: 2,
                ..good.clone()
            },
            ThreadParameters {
                axial_allowance: length(0.001),
                ..good.clone()
            },
            ThreadParameters {
                custom: Some(Default::default()),
                ..good.clone()
            },
        ] {
            assert!(
                prepare_thread(
                    &s,
                    0,
                    actor(),
                    &ThreadRequest::new("Rejected round thread", p)
                )
                .is_err()
            );
            assert_eq!(s.document(), &before);
            assert_eq!(s.revision(), 0);
        }
    }
}

fn reader(s: &Session) {
    let project = ProjectFile::new(s.document().clone()).unwrap();
    assert!(project.capabilities().contains(ROUND_THREAD_CAPABILITY));
    let mut bytes = vec![];
    project_file::write(&project, &mut bytes, Default::default()).unwrap();
    let mut supported = project_file::supported_capabilities();
    let read = project_file::read(&mut bytes.as_slice(), Default::default(), &supported).unwrap();
    assert_eq!(read.project.document, *s.document());
    supported.remove(ROUND_THREAD_CAPABILITY);
    assert!(
        project_file::read(&mut bytes.as_slice(), Default::default(), &supported)
            .unwrap_err()
            .contains(ROUND_THREAD_CAPABILITY)
    );
}
fn embedded_reader(s: &mut Session, body: multicad_core::document::EntityId) {
    let author = SymbolAuthorRequest::new(
        "Round thread template",
        body,
        SymbolKind::Static,
        vec![],
        None,
    );
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
fn round_sources_survive_history_reopen_suppression_and_template_only_reader_gates() {
    for row in round_thread_presets() {
        let mut s = session();
        let mut parameters = row.parameters();
        parameters.hand = ThreadHand::Left;
        parameters.starts = 1;
        let request = ThreadRequest::new("LH nominal round", parameters);
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
        assert!(!required_capabilities(s.document()).contains(ROUND_THREAD_CAPABILITY));
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
fn nested_and_retained_reference_round_profiles_require_the_reader() {
    for row in round_thread_presets() {
        for retained_only in [false, true] {
            let mut s = session();
            let reference = MechanicalPartParameters::HexNut {
                across_flats: length(30.),
                thread: Box::new(row.parameters()),
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
                source_url: row.profile_source_url.into(),
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
