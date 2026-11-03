use multicad_core::{
    commands::{Actor, Channel, Request, Session},
    document::{Document, EntityData, Mutation},
    model::*,
    project_file::{self, ProjectFile},
    symbols::{self, SymbolAuthorRequest, SymbolKind},
};
fn session() -> Session {
    Session::new(Document::new("Basic lead screws").unwrap()).unwrap()
}
fn actor() -> Actor {
    Actor::local(Channel::Mcp)
}
#[test]
fn distinct_sourced_axial_profiles_keep_shared_basic_pitch_and_mode_specific_clearance() {
    let s = session();
    let rows = lead_screw_presets();
    assert_eq!(rows.len(), 4);
    for row in rows {
        let angle = if row.family == ThreadFamily::Trapezoidal {
            30.
        } else {
            29.
        };
        let c = if row.family == ThreadFamily::Trapezoidal {
            0.25
        } else if row.threads_per_inch == Some(10) {
            0.010 * 25.4
        } else {
            0.005 * 25.4
        };
        for mode in [
            ThreadMode::NewExternal,
            ThreadMode::CutExternal,
            ThreadMode::CutInternal,
        ] {
            for (hand, starts) in [
                (ThreadHand::Right, 1),
                (ThreadHand::Left, 2),
                (ThreadHand::Left, 8),
            ] {
                let mut p = row.parameters();
                p.hand = hand;
                p.starts = starts;
                let r = p.resolve(s.parameters(), mode).unwrap();
                let lead = r.lead_screw.unwrap();
                assert_eq!(r.fit, None);
                assert_eq!(p.fit, ThreadFit::NominalBasic);
                assert_eq!(r.profile.lower_flank_deg, angle / 2.);
                assert_eq!(r.profile.upper_flank_deg, angle / 2.);
                assert_eq!(r.profile.radial_taper, 0.);
                assert_eq!(r.profile.rounding_radius_mm, None);
                assert!((r.profile.depth_mm - row.pitch_mm / 2. - c).abs() < 1e-12);
                assert_eq!(lead.preset.basic_radial_clearance_mm, c);
                assert_eq!(
                    r.nominal_pitch_diameter_mm,
                    Some(row.diameter_mm - row.pitch_mm / 2.)
                );
                assert!(
                    (r.actual_pitch_diameter_mm.unwrap() - row.basic_pitch_diameter_mm).abs()
                        < 1e-12
                );
                let major = if mode == ThreadMode::CutInternal {
                    row.diameter_mm + 2. * c
                } else {
                    row.diameter_mm
                };
                let minor = if mode == ThreadMode::CutInternal {
                    row.diameter_mm - row.pitch_mm
                } else {
                    row.diameter_mm - row.pitch_mm - 2. * c
                };
                assert!((2. * r.major_radius_mm - major).abs() < 1e-12);
                assert!((2. * (r.major_radius_mm - r.profile.depth_mm) - minor).abs() < 1e-12);
                assert!(
                    (r.profile.crest_width_mm
                        - (row.pitch_mm / 2.
                            - (major - row.basic_pitch_diameter_mm)
                                * (angle / 2.).to_radians().tan()))
                    .abs()
                        < 1e-12
                );
                assert!((r.lead_mm - row.pitch_mm * f64::from(starts)).abs() < 1e-12);
                assert!((lead.lead_mm - r.lead_mm).abs() < 1e-12);
                let wire = serde_json::to_value(&p).unwrap();
                assert!(wire.get("custom").unwrap().is_null());
                assert!(wire.get("pipe").is_none());
                assert!(wire.get("npt").is_none());
                if row.family == ThreadFamily::Acme {
                    assert_eq!(wire["pitch"]["unit"], "in");
                    assert_eq!(wire["major_diameter"]["unit"], "in");
                }
            }
        }
    }
}
#[test]
fn fabrication_allowances_change_actual_pitch_diameter_without_rewriting_source_basics() {
    let s = session();
    for row in lead_screw_presets() {
        for mode in [ThreadMode::NewExternal, ThreadMode::CutInternal] {
            let mut p = row.parameters();
            let basic = p.resolve(s.parameters(), mode).unwrap();
            let radial = 0.005;
            let axial = 0.006;
            p.radial_allowance = length(radial);
            p.axial_allowance = length(axial);
            let r = p.resolve(s.parameters(), mode).unwrap();
            let sign = if mode == ThreadMode::CutInternal {
                1.
            } else {
                -1.
            };
            let expected = row.basic_pitch_diameter_mm
                + sign * (2. * radial + axial / (row.included_angle_deg / 2.).to_radians().tan());
            assert!((r.actual_pitch_diameter_mm.unwrap() - expected).abs() < 1e-12);
            assert_eq!(r.lead_screw, basic.lead_screw);
            assert_eq!(r.nominal_pitch_diameter_mm, basic.nominal_pitch_diameter_mm);
            assert!((r.major_radius_mm - basic.major_radius_mm - sign * radial).abs() < 1e-12);
            assert!(
                (r.profile.crest_width_mm - basic.profile.crest_width_mm - sign * axial).abs()
                    < 1e-12
            );
            assert_eq!(r.profile.depth_mm, basic.profile.depth_mm);
        }
    }
}
#[test]
fn unsupported_profile_and_fit_requests_fail_atomically() {
    let s = session();
    let before = s.document().clone();
    for row in lead_screw_presets() {
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
                starts: 0,
                ..good.clone()
            },
            ThreadParameters {
                starts: 9,
                ..good.clone()
            },
            ThreadParameters {
                custom: Some(Default::default()),
                ..good.clone()
            },
            ThreadParameters {
                axial_allowance: length(row.pitch_mm),
                ..good.clone()
            },
        ] {
            assert!(
                prepare_thread(
                    &s,
                    0,
                    actor(),
                    &ThreadRequest::new("Rejected lead screw", p)
                )
                .is_err()
            );
            assert_eq!(s.revision(), 0);
            assert_eq!(s.document(), &before);
        }
    }
}
fn reader(s: &Session) {
    let project = ProjectFile::new(s.document().clone()).unwrap();
    assert!(project.capabilities().contains(LEAD_SCREW_CAPABILITY));
    let mut bytes = vec![];
    project_file::write(&project, &mut bytes, Default::default()).unwrap();
    let mut supported = project_file::supported_capabilities();
    let read = project_file::read(&mut bytes.as_slice(), Default::default(), &supported).unwrap();
    assert_eq!(read.project.document, *s.document());
    supported.remove(LEAD_SCREW_CAPABILITY);
    assert!(
        project_file::read(&mut bytes.as_slice(), Default::default(), &supported)
            .unwrap_err()
            .contains(LEAD_SCREW_CAPABILITY)
    );
}
fn embedded_reader(s: &mut Session, body: multicad_core::document::EntityId) {
    let author = SymbolAuthorRequest::new(
        "Lead screw template",
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
fn lead_sources_survive_history_reopen_suppression_and_template_only_reader_gates() {
    for row in lead_screw_presets() {
        let mut s = session();
        let mut parameters = row.parameters();
        parameters.hand = ThreadHand::Left;
        parameters.starts = 2;
        let request = ThreadRequest::new("LH two-start", parameters);
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
        assert!(!required_capabilities(s.document()).contains(LEAD_SCREW_CAPABILITY));
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
fn nested_and_retained_reference_lead_profiles_require_the_reader() {
    for row in lead_screw_presets() {
        for retained_only in [false, true] {
            let mut s = session();
            let reference = MechanicalPartParameters::HexNut {
                across_flats: length(20.),
                thread: Box::new(row.parameters()),
            };
            let mut request = MechanicalPartRequest::new(
                "Lead reference nut",
                if retained_only {
                    MechanicalPartParameters::HexNut {
                        across_flats: length(20.),
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
