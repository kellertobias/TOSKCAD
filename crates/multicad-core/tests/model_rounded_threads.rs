use multicad_core::{
    commands::{Actor, Channel, Request, Session},
    document::{Document, EntityData, Mutation},
    model::*,
    project_file::{self, ProjectFile},
    symbols::{self, SymbolAuthorRequest, SymbolKind},
};
fn actor() -> Actor {
    Actor::local(Channel::Mcp)
}
fn session() -> Session {
    Session::new(Document::new("Rounded thread contracts").unwrap()).unwrap()
}
fn parameters(p: &WhitworthThreadPreset, internal: bool) -> ThreadParameters {
    ThreadParameters {
        family: p.family,
        major_diameter: length(p.diameter_mm),
        pitch: length(p.pitch_mm),
        length: length(3. * p.pitch_mm),
        fit: match (p.family, internal) {
            (ThreadFamily::Bspp, false) => ThreadFit::BsppClassBExternal,
            (ThreadFamily::Bspp, true) => ThreadFit::BsppInternal,
            (_, false) => ThreadFit::WhitworthMediumExternal,
            (_, true) => ThreadFit::WhitworthMediumInternal,
        },
        ..Default::default()
    }
}
fn assert_range(range: ThreadDiameterRange, value: f64) {
    assert!(range.min_mm.is_none_or(|v| value >= v - 1e-9));
    assert!(range.max_mm.is_none_or(|v| value <= v + 1e-9));
}

#[test]
fn six_presets_retain_exact_independent_source_bounds_and_pipe_labels_are_not_diameters() {
    // BS84 values in inches; G table values in mm. No profile formula is used
    // to reconstruct these independently transcribed tolerance-table bounds.
    let expected = [
        (
            ThreadFamily::Bsw,
            "1/4-20 BSW",
            0.25 * 25.4,
            20,
            25.4,
            [[0.2427, 0.2488], [0.2129, 0.2168], [0.1764, 0.1848]],
            [[0.25, f64::INFINITY], [0.218, 0.2219], [0.186, 0.203]],
        ),
        (
            ThreadFamily::Bsw,
            "5/16-18 BSW",
            0.3125 * 25.4,
            18,
            25.4,
            [[0.3046, 0.3112], [0.2714, 0.2756], [0.2311, 0.24]],
            [[0.3125, f64::INFINITY], [0.2769, 0.2811], [0.2413, 0.2594]],
        ),
        (
            ThreadFamily::Bsf,
            "1/4-26 BSF",
            0.25 * 25.4,
            26,
            25.4,
            [[0.2432, 0.2489], [0.2206, 0.2243], [0.1921, 0.1997]],
            [[0.25, f64::INFINITY], [0.2254, 0.2291], [0.2008, 0.2125]],
        ),
        (
            ThreadFamily::Bsf,
            "5/16-22 BSF",
            0.3125 * 25.4,
            22,
            25.4,
            [[0.3051, 0.3113], [0.2781, 0.2822], [0.2447, 0.2531]],
            [[0.3125, f64::INFINITY], [0.2834, 0.2875], [0.2543, 0.2684]],
        ),
        (
            ThreadFamily::Bspp,
            "G1/8",
            9.728,
            28,
            1.,
            [[9.514, 9.728], [8.933, 9.147], [8.298, 8.566]],
            [[9.728, f64::INFINITY], [9.147, 9.254], [8.566, 8.848]],
        ),
        (
            ThreadFamily::Bspp,
            "G1/4",
            13.157,
            19,
            1.,
            [[12.907, 13.157], [12.051, 12.301], [11.133, 11.445]],
            [[13.157, f64::INFINITY], [12.301, 12.426], [11.445, 11.89]],
        ),
    ];
    let presets = whitworth_thread_presets();
    assert_eq!(presets.len(), expected.len());
    for (family, name, diameter, tpi, scale, external, internal) in expected {
        let matches: Vec<_> = presets.iter().filter(|p| p.designation == name).collect();
        assert_eq!(matches.len(), 1);
        let p = matches[0];
        assert_eq!(p.family, family);
        assert_eq!(p.diameter_mm, diameter);
        assert_eq!(p.threads_per_inch, tpi);
        assert_eq!(p.pitch_mm, 25.4 / f64::from(tpi));
        if family == ThreadFamily::Bspp {
            assert!(p.nominal_diameter_inch.is_none());
            assert!(p.diameter_mm > 9.);
        } else {
            assert_eq!(p.nominal_diameter_inch, Some(diameter / 25.4));
        }
        for (limits, expected) in [(p.external, external), (p.internal, internal)] {
            for (range, [minimum, maximum]) in [limits.major, limits.pitch, limits.minor]
                .into_iter()
                .zip(expected)
            {
                assert_eq!(range.min_mm, Some(minimum * scale));
                assert_eq!(range.max_mm, maximum.is_finite().then_some(maximum * scale));
            }
        }
        assert!(!p.source.is_empty());
        assert!(p.source_url.starts_with("https://"));
        assert!(p.profile_source_url.starts_with("https://"));
    }
}

#[test]
fn exact_rounded_profile_uses_feasible_three_diameter_intersection_for_every_mode() {
    let s = session();
    for preset in whitworth_thread_presets() {
        for internal in [false, true] {
            let p = parameters(&preset, internal);
            let mode = if internal {
                ThreadMode::CutInternal
            } else {
                ThreadMode::NewExternal
            };
            let r = p.resolve(s.parameters(), mode).unwrap();
            let a = 27.5_f64.to_radians();
            let depth = preset.pitch_mm / (3. * a.tan());
            let radius = preset.pitch_mm * a.cos() / (12. * (1. - a.sin()));
            assert!((r.profile.depth_mm - depth).abs() < 1e-12);
            assert!((r.profile.rounding_radius_mm.unwrap() - radius).abs() < 1e-12);
            assert_eq!(r.profile.crest_width_mm, 0.);
            assert_eq!(r.profile.lower_flank_deg, 27.5);
            assert_eq!(r.profile.upper_flank_deg, 27.5);
            assert_eq!(r.profile.radial_taper, 0.);
            assert_eq!(r.nominal_major_radius_mm, preset.diameter_mm / 2.);
            assert!(
                (r.nominal_pitch_diameter_mm.unwrap() - (preset.diameter_mm - depth)).abs() < 1e-12
            );
            let fit = r.fit.unwrap();
            let limits = fit.limits;
            let mut low = f64::NEG_INFINITY;
            let mut high = f64::INFINITY;
            for (range, shift) in [
                (limits.major, -depth),
                (limits.pitch, 0.),
                (limits.minor, depth),
            ] {
                if let Some(v) = range.min_mm {
                    low = low.max(v + shift);
                }
                if let Some(v) = range.max_mm {
                    high = high.min(v + shift);
                }
            }
            assert!(low <= high);
            assert!((fit.pitch_diameter_mm - (low + high) / 2.).abs() < 1e-12);
            assert!((fit.major_diameter_mm - fit.pitch_diameter_mm - depth).abs() < 1e-12);
            assert!((fit.pitch_diameter_mm - fit.minor_diameter_mm - depth).abs() < 1e-12);
            assert_range(limits.major, fit.major_diameter_mm);
            assert_range(limits.pitch, fit.pitch_diameter_mm);
            assert_range(limits.minor, fit.minor_diameter_mm);
            assert_eq!(
                fit.engagement_limit_mm,
                if preset.family == ThreadFamily::Bspp {
                    None
                } else {
                    Some(15. * preset.pitch_mm)
                }
            );
            assert_eq!(r.actual_pitch_diameter_mm, Some(fit.pitch_diameter_mm));
            if !internal {
                let cut = p.resolve(s.parameters(), ThreadMode::CutExternal).unwrap();
                assert_eq!(cut.major_radius_mm, r.major_radius_mm);
            }
        }
    }
}

#[test]
fn radial_allowance_checks_all_three_sourced_bounds_and_preserves_true_rounding() {
    let s = session();
    for preset in whitworth_thread_presets() {
        for internal in [false, true] {
            let mut p = parameters(&preset, internal);
            let mode = if internal {
                ThreadMode::CutInternal
            } else {
                ThreadMode::NewExternal
            };
            let before = p.resolve(s.parameters(), mode).unwrap();
            let fit = before.fit.as_ref().unwrap();
            let margin = [
                (fit.limits.major, fit.major_diameter_mm),
                (fit.limits.pitch, fit.pitch_diameter_mm),
                (fit.limits.minor, fit.minor_diameter_mm),
            ]
            .into_iter()
            .filter_map(|(range, value)| {
                if internal {
                    range.max_mm.map(|max| (max - value) / 2.)
                } else {
                    range.min_mm.map(|min| (value - min) / 2.)
                }
            })
            .fold(f64::INFINITY, f64::min);
            assert!(margin > 0. && margin.is_finite());
            p.radial_allowance = length(margin / 2.);
            let after = p.resolve(s.parameters(), mode).unwrap();
            let sign = if internal { 1. } else { -1. };
            assert!(
                (after.major_radius_mm - before.major_radius_mm - sign * margin / 2.).abs() < 1e-12
            );
            assert_eq!(after.profile, before.profile);
            p.radial_allowance = length(margin + 0.001);
            assert!(
                p.resolve(s.parameters(), mode).is_err(),
                "{} {:?}",
                preset.designation,
                mode
            );
        }
    }
}

#[test]
fn rounded_unsupported_classes_modes_allowances_starts_rows_and_engagement_fail_atomically() {
    let s = session();
    let before = s.document().clone();
    for preset in whitworth_thread_presets() {
        let good = parameters(&preset, false);
        let mut invalid = vec![
            ThreadParameters {
                fit: ThreadFit::NominalBasic,
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
                pitch: length(preset.pitch_mm * 1.01),
                ..good.clone()
            },
        ];
        if preset.family != ThreadFamily::Bspp {
            invalid.push(ThreadParameters {
                length: length(16. * preset.pitch_mm),
                ..good.clone()
            });
        }
        for p in invalid {
            let request = ThreadRequest::new("Invalid rounded shaft", p);
            assert!(prepare_thread(&s, 0, actor(), &request).is_err());
            assert_eq!(s.document(), &before);
            assert_eq!(s.revision(), 0);
        }
        assert!(
            good.resolve(s.parameters(), ThreadMode::CutInternal)
                .is_err()
        );
        assert!(
            parameters(&preset, true)
                .resolve(s.parameters(), ThreadMode::NewExternal)
                .is_err()
        );
        let mut boundary = good.clone();
        boundary.length = length(15. * preset.pitch_mm);
        assert!(
            boundary
                .resolve(s.parameters(), ThreadMode::NewExternal)
                .is_ok()
        );
        if preset.family == ThreadFamily::Bspp {
            boundary.length = length(16. * preset.pitch_mm);
            assert!(
                boundary
                    .resolve(s.parameters(), ThreadMode::NewExternal)
                    .is_ok()
            );
        }
        let mut wrong = good;
        wrong.fit = if preset.family == ThreadFamily::Bspp {
            ThreadFit::WhitworthMediumExternal
        } else {
            ThreadFit::BsppClassBExternal
        };
        assert!(wrong.validate().is_err());
    }
}

fn assert_reader(s: &Session, required: bool) {
    let project = ProjectFile::new(s.document().clone()).unwrap();
    assert_eq!(
        project.capabilities().contains(ROUNDED_THREAD_CAPABILITY),
        required
    );
    let mut bytes = vec![];
    project_file::write(&project, &mut bytes, Default::default()).unwrap();
    let supported = project_file::supported_capabilities();
    let read = project_file::read(&mut bytes.as_slice(), Default::default(), &supported).unwrap();
    assert_eq!(read.project.document, *s.document());
    let mut older = supported;
    older.remove(ROUNDED_THREAD_CAPABILITY);
    let result = project_file::read(&mut bytes.as_slice(), Default::default(), &older);
    if required {
        assert!(result.unwrap_err().contains(ROUNDED_THREAD_CAPABILITY));
    } else {
        assert!(result.is_ok());
    }
}
#[test]
fn rounded_reader_gate_covers_direct_suppressed_template_only_and_nested_nuts_and_screws() {
    for preset in whitworth_thread_presets() {
        let mut s = session();
        let request = ThreadRequest::new("Rounded shaft", parameters(&preset, false));
        let p = prepare_thread(&s, 0, actor(), &request).unwrap();
        s.commit(p.prepared).unwrap();
        assert_reader(&s, true);
        let mut suppressed = Session::new(s.document().clone()).unwrap();
        let mut e = suppressed.document().get(request.feature).unwrap().clone();
        let EntityData::Feature {
            suppressed: flag, ..
        } = &mut e.data
        else {
            unreachable!()
        };
        *flag = true;
        suppressed
            .apply(Request {
                expected_revision: 0,
                actor: actor(),
                label: "Suppress rounded source".into(),
                edits: vec![Mutation::Replace(e)],
            })
            .unwrap();
        assert_reader(&suppressed, true);
        let author = SymbolAuthorRequest::new(
            "Reusable rounded shaft",
            request.body,
            SymbolKind::Static,
            vec![],
            None,
        );
        let p = symbols::prepare_author(&s, s.revision(), actor(), &author).unwrap();
        s.commit(p.prepared).unwrap();
        let definition = symbols::read_definition(s.document(), author.definition).unwrap();
        let mut host = session();
        let p = symbols::prepare_import(&host, 0, actor(), &definition).unwrap();
        host.commit(p.prepared).unwrap();
        assert!(
            !host
                .document()
                .entities()
                .any(|e| matches!(e.data, EntityData::Feature { .. }))
        );
        assert_reader(&host, true);
        for internal in [false, true] {
            let mut host = session();
            let thread = parameters(&preset, internal);
            let recipe = if internal {
                MechanicalPartParameters::HexNut {
                    across_flats: length(2. * preset.diameter_mm),
                    thread: Box::new(thread),
                }
            } else {
                MechanicalPartParameters::MachineScrew {
                    underhead_length: thread.length.clone(),
                    shank_diameter: length(preset.diameter_mm),
                    head: ScrewHead::ExternalHex {
                        across_flats: length(2. * preset.diameter_mm),
                        height: length(3.),
                    },
                    drive: ScrewDrive::ExternalHex,
                    thread: Box::new(thread),
                }
            };
            let request = MechanicalPartRequest::new("Rounded hardware", recipe);
            let p = prepare_mechanical_part(&host, 0, actor(), &request).unwrap();
            host.commit(p.prepared).unwrap();
            assert_reader(&host, true);
            let author = SymbolAuthorRequest::new(
                "Reusable rounded hardware",
                request.body,
                SymbolKind::Static,
                vec![],
                None,
            );
            let p = symbols::prepare_author(&host, host.revision(), actor(), &author).unwrap();
            host.commit(p.prepared).unwrap();
            let definition = symbols::read_definition(host.document(), author.definition).unwrap();
            let mut embedded = session();
            let p = symbols::prepare_import(&embedded, 0, actor(), &definition).unwrap();
            embedded.commit(p.prepared).unwrap();
            assert_reader(&embedded, true);
        }
    }
}

#[test]
fn legacy_metric_custom_unified_source_and_numeric_engagement_wire_remain_unchanged() {
    let metric = ThreadParameters::default();
    let custom = ThreadParameters {
        family: ThreadFamily::Custom,
        custom: Some(Default::default()),
        ..metric.clone()
    };
    let preset = unified_thread_presets().remove(0);
    let unified = ThreadParameters {
        family: preset.family,
        major_diameter: length(preset.diameter_mm),
        pitch: length(preset.pitch_mm),
        length: length(3. * preset.pitch_mm),
        fit: ThreadFit::Unified2A,
        ..metric.clone()
    };
    for parameters in [metric, custom, unified] {
        let value = serde_json::to_value(&parameters).unwrap();
        let decoded: ThreadParameters = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(serde_json::to_value(&decoded).unwrap(), value);
        assert!(value.get("rounding_radius_mm").is_none());
        let mut s = session();
        let resolved = parameters
            .resolve(s.parameters(), ThreadMode::NewExternal)
            .unwrap();
        assert!(resolved.profile.rounding_radius_mm.is_none());
        if let Some(fit) = resolved.fit {
            assert!(serde_json::to_value(fit).unwrap()["engagement_limit_mm"].is_number());
        }
        let request = ThreadRequest::new("Legacy thread", parameters);
        let p = prepare_thread(&s, 0, actor(), &request).unwrap();
        s.commit(p.prepared).unwrap();
        assert_reader(&s, false);
    }
    for family in [ThreadFamily::Whitworth, ThreadFamily::Unified] {
        assert!(!family.supported());
        assert!(
            ThreadParameters {
                family,
                ..Default::default()
            }
            .validate()
            .is_err()
        );
    }
    assert_eq!(
        serde_json::to_value(ThreadFit::WhitworthMediumExternal).unwrap(),
        "whitworth_medium_external"
    );
    assert_eq!(
        serde_json::to_value(ThreadFit::BsppClassBExternal).unwrap(),
        "bspp_class_b_external"
    );
}

#[test]
fn rounded_literal_reference_still_requires_reader_after_authored_thread_changes_family() {
    let preset = whitworth_thread_presets().remove(0);
    let reference = MechanicalPartParameters::HexNut {
        across_flats: length(13.),
        thread: Box::new(parameters(&preset, true)),
    };
    let actual = MechanicalPartParameters::HexNut {
        across_flats: length(13.),
        thread: Box::default(),
    };
    let mut s = session();
    let mut request = MechanicalPartRequest::new("Edited sourced nut", actual);
    request.provenance = Some(MechanicalPartProvenance {
        catalog_id: "test-only-rounded-reference".into(),
        revision: "1".into(),
        designation: "Retained rounded nut reference".into(),
        source_url: WHITWORTH_THREAD_SOURCE_URL.into(),
        reference_geometry: reference,
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
    assert_reader(&s, true);
    let author = SymbolAuthorRequest::new(
        "Reusable edited nut",
        request.body,
        SymbolKind::Static,
        vec![],
        None,
    );
    let p = symbols::prepare_author(&s, s.revision(), actor(), &author).unwrap();
    s.commit(p.prepared).unwrap();
    let definition = symbols::read_definition(s.document(), author.definition).unwrap();
    let mut host = session();
    let p = symbols::prepare_import(&host, 0, actor(), &definition).unwrap();
    host.commit(p.prepared).unwrap();
    assert_reader(&host, true);
}
