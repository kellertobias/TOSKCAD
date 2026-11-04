use multicad_core::{
    commands::{Actor, Channel, Request, Session},
    document::{Document, EntityData, Mutation},
    model::*,
    project_file::{self, ProjectFile},
    symbols::{self, SymbolAuthorRequest, SymbolKind},
};

fn parameters(diameter: f64, pitch: f64, fit: ThreadFit) -> ThreadParameters {
    ThreadParameters {
        major_diameter: length(diameter),
        pitch: length(pitch),
        length: length(diameter),
        fit,
        ..Default::default()
    }
}
#[test]
fn sourced_metric_limits_target_circles_and_allowances_are_independent() {
    let s = Session::new(Document::new("Metric fits").unwrap()).unwrap();
    for p in metric_fit_presets() {
        for (class, mode) in [
            (ThreadFit::Metric6gExternal, ThreadMode::NewExternal),
            (ThreadFit::Metric6HInternal, ThreadMode::CutInternal),
        ] {
            let mut q = parameters(p.diameter_mm, p.pitch_mm, class);
            let a = q.resolve(s.parameters(), mode).unwrap();
            let fit = a.fit.as_ref().unwrap();
            let construction = a.metric_fit.as_ref().unwrap();
            assert_eq!(fit.class, class);
            assert_eq!(fit.source, METRIC_FIT_SOURCE);
            assert_eq!(
                fit.limits,
                if mode == ThreadMode::CutInternal {
                    p.internal
                } else {
                    p.external
                }
            );
            assert!(
                (a.nominal_pitch_diameter_mm.unwrap()
                    - (p.diameter_mm - 3. * 3_f64.sqrt() / 8. * p.pitch_mm))
                    .abs()
                    < 1e-12
            );
            assert!(
                (a.actual_pitch_diameter_mm.unwrap() - construction.target_pitch_diameter_mm).abs()
                    < 1e-12
            );
            if let Some(radius) = construction.external_root_radius_mm {
                assert!(radius >= p.printed_external_minimum_root_radius_mm);
                assert!(
                    (2. * a.major_radius_mm
                        - 2. * a.profile.depth_mm
                        - (construction.target_pitch_diameter_mm - p.pitch_mm / 3_f64.sqrt()))
                    .abs()
                        < 1e-12
                );
            }
            q.radial_allowance = length(0.002);
            q.axial_allowance = length(0.002);
            let b = q.resolve(s.parameters(), mode).unwrap();
            assert_eq!(a.metric_fit, b.metric_fit);
            assert_eq!(
                a.fit.as_ref().unwrap().limits,
                b.fit.as_ref().unwrap().limits
            );
            let sign = if mode == ThreadMode::CutInternal {
                1.
            } else {
                -1.
            };
            assert!(
                (b.actual_pitch_diameter_mm.unwrap()
                    - a.actual_pitch_diameter_mm.unwrap()
                    - sign * (0.004 + 0.002 * 3_f64.sqrt()))
                .abs()
                    < 1e-12
            );
            q.radial_allowance = length(1.);
            assert!(q.resolve(s.parameters(), mode).is_err());
        }
    }
}
#[test]
fn metric_fit_engagement_boundaries_and_wrong_combinations_are_explicit() {
    let s = Session::new(Document::new("Metric limits").unwrap()).unwrap();
    for p in metric_fit_presets() {
        for (fit, mode) in [
            (ThreadFit::Metric6gExternal, ThreadMode::NewExternal),
            (ThreadFit::Metric6HInternal, ThreadMode::CutInternal),
        ] {
            let mut q = parameters(p.diameter_mm, p.pitch_mm, fit);
            q.length = length(p.engagement_over_mm);
            assert!(q.resolve(s.parameters(), mode).is_err());
            q.length = length(p.engagement_over_mm + 0.000001);
            assert!(q.resolve(s.parameters(), mode).is_ok());
            q.length = length(p.engagement_up_to_mm);
            assert!(q.resolve(s.parameters(), mode).is_ok());
            q.length = length(p.engagement_up_to_mm + 0.000001);
            assert!(q.resolve(s.parameters(), mode).is_err());
            q.length = length(p.diameter_mm);
            q.starts = 2;
            assert!(q.resolve(s.parameters(), mode).is_err());
        }
    }
    assert!(
        parameters(3., 0.5, ThreadFit::Metric6gExternal)
            .resolve(s.parameters(), ThreadMode::CutInternal)
            .is_err()
    );
    assert!(
        parameters(3., 0.5, ThreadFit::Metric6HInternal)
            .resolve(s.parameters(), ThreadMode::NewExternal)
            .is_err()
    );
    assert!(
        parameters(8., 1.25, ThreadFit::Metric6gExternal)
            .resolve(s.parameters(), ThreadMode::NewExternal)
            .is_err()
    );
    let basic = parameters(3., 0.5, ThreadFit::NominalBasic)
        .resolve(s.parameters(), ThreadMode::NewExternal)
        .unwrap();
    assert!(basic.metric_fit.is_none());
    assert_eq!(basic.profile.depth_mm, 5. * 3_f64.sqrt() / 16. * 0.5);
    assert_eq!(basic.profile.crest_width_mm, 0.5 / 8.);
}

#[test]
fn allowances_hit_real_class_boundaries_without_mutating_source_limits_or_root_radius() {
    let s = Session::new(Document::new("Fit boundaries").unwrap()).unwrap();
    for row in metric_fit_presets() {
        for (class, mode) in [
            (ThreadFit::Metric6gExternal, ThreadMode::NewExternal),
            (ThreadFit::Metric6HInternal, ThreadMode::CutInternal),
        ] {
            let mut p = parameters(row.diameter_mm, row.pitch_mm, class);
            let base = p.resolve(s.parameters(), mode).unwrap();
            let fit = base.fit.as_ref().unwrap();
            let target = base.actual_pitch_diameter_mm.unwrap();
            let bound = if mode == ThreadMode::CutInternal {
                fit.limits.pitch.max_mm.unwrap() - target
            } else {
                target - fit.limits.pitch.min_mm.unwrap()
            };
            p.radial_allowance = length(bound / 2.);
            let at = p.resolve(s.parameters(), mode).unwrap();
            assert_eq!(base.metric_fit, at.metric_fit);
            assert_eq!(fit.limits, at.fit.unwrap().limits);
            p.radial_allowance = length(bound / 2. + 0.000001);
            assert!(p.resolve(s.parameters(), mode).is_err());
            p.radial_allowance = length(0.);
            p.axial_allowance = length(bound / 3_f64.sqrt());
            assert!(p.resolve(s.parameters(), mode).is_ok());
            p.axial_allowance = length(bound / 3_f64.sqrt() + 0.000001);
            assert!(p.resolve(s.parameters(), mode).is_err());
            if let Some(r) = base.metric_fit.unwrap().external_root_radius_mm {
                let td = row.external.pitch.max_mm.unwrap() - row.external.pitch.min_mm.unwrap();
                let rmin = row.printed_external_minimum_root_radius_mm;
                let h = 3_f64.sqrt() / 2. * row.pitch_mm;
                let cmax = h / 4.
                    - rmin
                        * (1. - (std::f64::consts::PI / 3. - (1. - td / (4. * rmin)).acos()).cos())
                    + td / 2.;
                assert!(r >= 0.125 * row.pitch_mm);
                assert!(r + td / 2. < cmax);
            }
        }
    }
}
fn treatments() -> ThreadEndTreatments {
    ThreadEndTreatments {
        start: ThreadStartTreatment::Chamfer45 {
            radial_depth: length(0.4),
        },
        end: ThreadEndTreatment::CylindricalRelief {
            length: length(0.3),
            radial_depth: length(0.05),
        },
    }
}
#[test]
fn treatments_preserve_total_length_and_class_but_check_full_profile_and_material() {
    let s = Session::new(Document::new("Treatment bounds").unwrap()).unwrap();
    let mut p = parameters(3., 0.5, ThreadFit::Metric6gExternal);
    p.end_treatments = Some(treatments());
    let r = p.resolve(s.parameters(), ThreadMode::NewExternal).unwrap();
    let t = r.end_treatments.unwrap();
    assert_eq!(r.length_mm, 3.);
    assert_eq!(t.full_profile_start_mm, 0.4);
    assert_eq!(t.full_profile_end_mm, 2.7);
    assert!((t.full_profile_length_mm - 2.3).abs() < 1e-12);
    p.fit = ThreadFit::Metric6HInternal;
    let r = p.resolve(s.parameters(), ThreadMode::CutInternal).unwrap();
    assert!(r.end_treatments.unwrap().maximum_cutter_radius_mm > r.major_radius_mm + 0.05);
    let before = s.document().clone();
    for bad in [
        ThreadParameters {
            length: length(1.8),
            fit: ThreadFit::Metric6gExternal,
            ..p.clone()
        },
        ThreadParameters {
            family: ThreadFamily::Custom,
            custom: Some(Default::default()),
            fit: ThreadFit::NominalBasic,
            ..p.clone()
        },
        ThreadParameters {
            representation: ThreadRepresentation::CosmeticEnvelope,
            fit: ThreadFit::Metric6gExternal,
            ..p.clone()
        },
        ThreadParameters {
            end_treatments: Some(ThreadEndTreatments {
                start: ThreadStartTreatment::Chamfer45 {
                    radial_depth: length(5.),
                },
                end: ThreadEndTreatment::FlatCut,
            }),
            fit: ThreadFit::Metric6gExternal,
            ..p.clone()
        },
    ] {
        assert!(
            prepare_thread(
                &s,
                0,
                Actor::local(Channel::Mcp),
                &ThreadRequest::new("Refused treatment", bad)
            )
            .is_err()
        );
        assert_eq!(s.document(), &before);
        assert_eq!(s.revision(), 0);
    }
    assert!(
        MechanicalPartParameters::HexNut {
            across_flats: length(6.),
            thread: Box::new(p)
        }
        .validate()
        .is_err()
    );
    assert!(
        serde_json::to_value(ThreadParameters::default())
            .unwrap()
            .get("end_treatments")
            .is_none()
    );
}
fn reader(s: &Session, required: &[&str]) {
    let p = ProjectFile::new(s.document().clone()).unwrap();
    let mut bytes = vec![];
    project_file::write(&p, &mut bytes, Default::default()).unwrap();
    for &cap in required {
        assert!(p.capabilities().contains(cap));
        let mut supported = project_file::supported_capabilities();
        supported.remove(cap);
        assert!(
            project_file::read(&mut bytes.as_slice(), Default::default(), &supported)
                .unwrap_err()
                .contains(cap)
        );
    }
    assert_eq!(
        project_file::read(
            &mut bytes.as_slice(),
            Default::default(),
            &project_file::supported_capabilities()
        )
        .unwrap()
        .project
        .document,
        *s.document()
    );
}
#[test]
fn metric_fit_and_treatment_readers_cover_saved_suppressed_and_template_sources() {
    let actor = Actor::local(Channel::Mcp);
    let mut s = Session::new(Document::new("Fit reader").unwrap()).unwrap();
    let mut p = parameters(3., 0.5, ThreadFit::Metric6gExternal);
    p.end_treatments = Some(treatments());
    let request = ThreadRequest::new("Fitted treated source", p);
    let q = prepare_thread(&s, 0, actor.clone(), &request).unwrap();
    s.commit(q.prepared).unwrap();
    let caps = [METRIC_FIT_CAPABILITY, THREAD_END_TREATMENT_CAPABILITY];
    reader(&s, &caps);
    s.undo(s.revision(), actor.clone()).unwrap();
    assert!(!required_capabilities(s.document()).contains(METRIC_FIT_CAPABILITY));
    s.redo(s.revision(), actor.clone()).unwrap();
    let mut feature = s.document().get(request.feature).unwrap().clone();
    let EntityData::Feature { suppressed, .. } = &mut feature.data else {
        panic!()
    };
    *suppressed = true;
    s.apply(Request {
        expected_revision: s.revision(),
        actor: actor.clone(),
        label: "Suppress".into(),
        edits: vec![Mutation::Replace(feature)],
    })
    .unwrap();
    reader(&s, &caps);
    s.undo(s.revision(), actor.clone()).unwrap();
    let author = SymbolAuthorRequest::new(
        "Fitted template",
        request.body,
        SymbolKind::Static,
        vec![],
        None,
    );
    let q = symbols::prepare_author(&s, s.revision(), actor.clone(), &author).unwrap();
    s.commit(q.prepared).unwrap();
    let definition = symbols::read_definition(s.document(), author.definition).unwrap();
    let mut other = Session::new(Document::new("Template only").unwrap()).unwrap();
    let q = symbols::prepare_import(&other, 0, actor, &definition).unwrap();
    other.commit(q.prepared).unwrap();
    reader(&other, &caps);
}

#[test]
fn nested_and_retained_only_metric_fit_sources_gate_their_reader() {
    for retained_only in [false, true] {
        let actor = Actor::local(Channel::Mcp);
        let mut s = Session::new(Document::new("Nested fitted source").unwrap()).unwrap();
        let fitted = parameters(3., 0.5, ThreadFit::Metric6HInternal);
        let reference = MechanicalPartParameters::HexNut {
            across_flats: length(6.),
            thread: Box::new(fitted.clone()),
        };
        let mut request = MechanicalPartRequest::new(
            "Fitted nut",
            if retained_only {
                MechanicalPartParameters::HexNut {
                    across_flats: length(6.),
                    thread: Box::new(ThreadParameters {
                        fit: ThreadFit::NominalBasic,
                        ..fitted
                    }),
                }
            } else {
                reference.clone()
            },
        );
        request.provenance = Some(MechanicalPartProvenance {
            catalog_id: "test-only-fit".into(),
            revision: "1".into(),
            designation: "HistoricalM3fit".into(),
            source_url: METRIC_FIT_SOURCE_URL.into(),
            reference_geometry: reference,
            catalog_facts: None,
        });
        let q = prepare_mechanical_part(&s, 0, actor.clone(), &request).unwrap();
        s.commit(q.prepared).unwrap();
        reader(&s, &[METRIC_FIT_CAPABILITY]);
        assert_eq!(
            read_mechanical_part(s.document(), request.feature)
                .unwrap()
                .matches_reference_dimensions(s.parameters())
                .unwrap(),
            !retained_only
        );
        let author = SymbolAuthorRequest::new(
            "Fitted nut template",
            request.body,
            SymbolKind::Static,
            vec![],
            None,
        );
        let q = symbols::prepare_author(&s, s.revision(), actor.clone(), &author).unwrap();
        s.commit(q.prepared).unwrap();
        let definition = symbols::read_definition(s.document(), author.definition).unwrap();
        let mut other = Session::new(Document::new("Retained fit template").unwrap()).unwrap();
        let q = symbols::prepare_import(&other, 0, actor, &definition).unwrap();
        other.commit(q.prepared).unwrap();
        reader(&other, &[METRIC_FIT_CAPABILITY]);
    }
}
