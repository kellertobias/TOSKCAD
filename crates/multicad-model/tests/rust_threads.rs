use multicad_core::{
    commands::{Actor, Channel, Session},
    document::Document,
    model::{
        self, BodyInput, CustomThreadProfile, PrimitiveParameters, PrimitiveRequest, ThreadFamily,
        ThreadFit, ThreadHand, ThreadMode, ThreadParameters, ThreadRequest, angle, length,
    },
    project_file::{self, ProjectFile},
    references::{self, GeometryIntent, Resolution, TopologyKind},
};
use multicad_model::{Compiler, GeometryBackend, Model};
use std::f64::consts::{PI, TAU};

fn actor() -> Actor {
    Actor::local(Channel::Ui)
}

fn rust(document: &Document, revision: u64) -> Model {
    Compiler {
        omit_display: true,
        ..Default::default()
    }
    .compile_with_backend(document, revision, GeometryBackend::Rust, None)
    .unwrap()
}

fn m8_volume(length_mm: f64) -> f64 {
    let pitch = 1.25;
    let radius: f64 = 4.;
    let root = radius - 5. * 3_f64.sqrt() / 16. * pitch;
    let crest = pitch / 8.;
    let flanks = 2. * 30_f64.to_radians().tan();
    let radius_integral = (radius * radius - root * root) / 2.;
    let ridge = crest * radius_integral
        + flanks * (radius * radius_integral - (radius.powi(3) - root.powi(3)) / 3.);
    length_mm * (PI * root * root + TAU * ridge / pitch)
}

#[test]
fn long_custom_m8_keeps_geometry_and_cap_intent_through_edit_history_and_reopen() {
    let mut session = Session::new(Document::new("Rust M8 threads").unwrap()).unwrap();
    let before = session.document().clone();
    let initial = ThreadParameters {
        family: ThreadFamily::Custom,
        custom: Some(CustomThreadProfile {
            depth: length(5. * 3_f64.sqrt() / 16. * 1.25),
            crest_width: length(1.25 / 8.),
            lower_flank: angle(30.),
            upper_flank: angle(30.),
            ..Default::default()
        }),
        length: length(50.),
        ..Default::default()
    };
    let request = ThreadRequest::new("M8 × 1.25", initial.clone());
    let candidate = model::prepare_thread(&session, 0, actor(), &request).unwrap();
    let checked = rust(candidate.prepared.document(), 1);
    let shape = &checked.snapshot().shapes[&request.feature];
    assert!(checked.solid(request.feature).unwrap().uses_rust_geometry());
    assert_eq!(shape.solid_count, 1);
    assert!((shape.volume_mm3 - m8_volume(50.)).abs() < 1e-7);
    for (actual, expected) in shape.bounds.min.into_iter().zip([-4., -4., 0.]) {
        assert!((actual - expected).abs() < 1e-7);
    }
    for (actual, expected) in shape.bounds.max.into_iter().zip([4., 4., 50.]) {
        assert!((actual - expected).abs() < 1e-7);
    }
    let cap = shape
        .topology
        .elements
        .iter()
        .find(|face| {
            face.key.kind == TopologyKind::Face
                && face.origins.iter().any(|origin| origin.role == "cap.start")
        })
        .unwrap();
    let intent = GeometryIntent::capture(&shape.topology, cap.key).unwrap();
    assert_eq!(session.document(), &before);
    session.commit(candidate.prepared).unwrap();
    let original = session.document().clone();

    let edited = ThreadParameters {
        length: length(49.8),
        hand: ThreadHand::Left,
        ..initial
    };
    let candidate =
        model::prepare_thread_edit(&session, 1, actor(), request.feature, &edited).unwrap();
    let checked = rust(candidate.prepared.document(), 2);
    let shape = &checked.snapshot().shapes[&request.feature];
    assert!((shape.volume_mm3 - m8_volume(49.8)).abs() < 1e-7);
    assert!(matches!(
        references::resolve(&intent, &shape.topology, 2),
        Resolution::Resolved { key, .. } if key == cap.key
    ));
    assert_eq!(session.document(), &original);
    session.commit(candidate.prepared).unwrap();

    let saved = session.document().clone();
    session.undo(2, actor()).unwrap();
    assert_eq!(session.document(), &original);
    session.redo(3, actor()).unwrap();
    assert_eq!(session.document(), &saved);
    let mut bytes = Vec::new();
    project_file::write(
        &ProjectFile::new(saved).unwrap(),
        &mut bytes,
        Default::default(),
    )
    .unwrap();
    let reopened = project_file::read(
        &mut bytes.as_slice(),
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap()
    .project;
    let source = model::read_thread(&reopened.document, request.feature).unwrap();
    assert_eq!(source.parameters, edited);
    let rebuilt = rust(&reopened.document, 0);
    assert_eq!(rebuilt.snapshot().bodies[&request.body], request.feature);
    assert!(
        (rebuilt.snapshot().shapes[&request.feature].volume_mm3 - m8_volume(49.8)).abs() < 1e-7
    );
}

#[test]
fn rust_6g_m6_matches_native_fitted_profile_mass_and_major_extent() {
    let session = Session::new(Document::new("Rust 6g profile").unwrap()).unwrap();
    let request = ThreadRequest::new(
        "M6 × 1 6g left",
        ThreadParameters {
            major_diameter: length(6.),
            pitch: length(1.),
            length: length(6.6),
            hand: ThreadHand::Left,
            fit: ThreadFit::Metric6gExternal,
            ..Default::default()
        },
    );
    let candidate = model::prepare_thread(&session, 0, actor(), &request).unwrap();
    let document = candidate.prepared.document();
    let start = std::time::Instant::now();
    let actual = rust(document, 1);
    let rust_elapsed = start.elapsed();
    let start = std::time::Instant::now();
    let native = Compiler {
        omit_display: true,
        ..Default::default()
    }
    .compile(document, 1)
    .unwrap();
    eprintln!(
        "M6 × 1 left 6g source-to-exact model (without display): Rust {rust_elapsed:?}, OpenCascade {:?}",
        start.elapsed()
    );
    let actual = actual.solid(request.feature).unwrap();
    let native = native.solid(request.feature).unwrap();
    assert!(actual.uses_rust_geometry());
    assert!(!native.uses_rust_geometry());
    let area: f64 = native
        .faces()
        .unwrap()
        .iter()
        .map(|face| face.area_mm2)
        .sum();
    let expected_volume = native.volume_mm3().unwrap();
    let uncertainty = multicad_kernel::GeometryPolicy::default().linear_tolerance_mm * area
        + 1e-7 * expected_volume;
    assert!(
        (actual.volume_mm3().unwrap() - expected_volume).abs() <= uncertainty,
        "fitted volume Rust {} vs native {expected_volume}, tolerance {uncertainty}",
        actual.volume_mm3().unwrap()
    );
    let exact = actual.bounds().unwrap();
    let comparison = native.bounds().unwrap();
    for (actual, expected) in exact
        .min
        .into_iter()
        .chain(exact.max)
        .zip(comparison.min.into_iter().chain(comparison.max))
    {
        assert!(
            (actual - expected).abs() < 5e-6,
            "bounds {actual} vs {expected}"
        );
    }
}

#[test]
fn rust_internal_6h_matches_native_cut_and_keeps_the_cylinder_face_intents() {
    let mut session = Session::new(Document::new("Rust 6H receiver").unwrap()).unwrap();
    let stock = PrimitiveRequest::new(
        "Stock",
        PrimitiveParameters::Cylinder {
            radius: length(5.),
            height: length(6.6),
            frame: Default::default(),
        },
    );
    let prepared = model::prepare_primitive(&session, 0, actor(), &stock).unwrap();
    session.commit(prepared.prepared).unwrap();
    let original = rust(session.document(), 1);
    let original = &original.snapshot().shapes[&stock.feature];
    let surviving: Vec<_> = original
        .topology
        .elements
        .iter()
        .filter(|element| element.key.kind == TopologyKind::Face)
        .map(|element| GeometryIntent::capture(&original.topology, element.key).unwrap())
        .collect();
    assert_eq!(surviving.len(), 3);
    let target = BodyInput {
        body: stock.body,
        producer: stock.feature,
    };
    let parameters = ThreadParameters {
        major_diameter: length(6.),
        pitch: length(1.),
        length: length(6.6),
        minimum_wall: length(1.),
        fit: ThreadFit::Metric6HInternal,
        ..Default::default()
    };
    let request = ThreadRequest::receiving(
        "M6 × 1 6H",
        ThreadMode::CutInternal,
        target.clone(),
        parameters.clone(),
    )
    .unwrap();
    let candidate = model::prepare_thread(&session, 1, actor(), &request).unwrap();
    let before = session.document().clone();
    let document = candidate.prepared.document();
    let start = std::time::Instant::now();
    let actual = rust(document, 2);
    let rust_elapsed = start.elapsed();
    let start = std::time::Instant::now();
    let native = Compiler {
        omit_display: true,
        ..Default::default()
    }
    .compile(document, 2)
    .unwrap();
    eprintln!(
        "M6 × 1 6H through-cut source-to-exact model (without display): Rust {rust_elapsed:?}, OpenCascade {:?}",
        start.elapsed()
    );
    let native = native.solid(request.feature).unwrap();
    assert!(!native.uses_rust_geometry());
    let shape = &actual.snapshot().shapes[&request.feature];
    let solid = actual.solid(request.feature).unwrap();
    assert!(solid.uses_rust_geometry());
    assert_eq!(actual.snapshot().bodies[&stock.body], request.feature);
    assert_eq!(shape.solid_count, 1);
    assert_eq!(shape.bounds, original.bounds);
    let native_volume = native.volume_mm3().unwrap();
    let uncertainty = multicad_kernel::GeometryPolicy::default().linear_tolerance_mm
        * native
            .faces()
            .unwrap()
            .iter()
            .map(|face| face.area_mm2)
            .sum::<f64>()
        + 1e-7 * native_volume;
    assert!(
        (shape.volume_mm3 - native_volume).abs() <= uncertainty,
        "6H volume Rust {} vs OpenCascade {native_volume}, tolerance {uncertainty}",
        shape.volume_mm3
    );
    for z in [0., 0.25, 3.3, 6.35, 6.6] {
        assert!(!solid.rust_thread_contains([0., 0., z]).unwrap());
        assert!(solid.rust_thread_contains([4., 0., z]).unwrap());
    }
    assert!(!solid.rust_thread_contains([2.75, 0., 2.]).unwrap());
    assert!(solid.rust_thread_contains([2.75, 0., 2.5]).unwrap());
    let retained: Vec<_> = surviving
        .iter()
        .map(|intent| references::resolve(intent, &shape.topology, 2))
        .collect();
    assert!(
        retained
            .iter()
            .all(|result| matches!(result, Resolution::Resolved { .. }))
    );
    assert_eq!(session.document(), &before);
    session.commit(candidate.prepared).unwrap();

    let changed = ThreadParameters {
        hand: ThreadHand::Left,
        ..parameters
    };
    let edited =
        model::prepare_thread_edit(&session, 2, actor(), request.feature, &changed).unwrap();
    let checked = rust(edited.prepared.document(), 3);
    let shape = &checked.snapshot().shapes[&request.feature];
    let solid = checked.solid(request.feature).unwrap();
    assert!(!solid.rust_thread_contains([0., 2.75, 1.75]).unwrap());
    assert!(solid.rust_thread_contains([0., 2.75, 2.25]).unwrap());
    assert!(
        surviving.iter().zip(retained).all(|(intent, previous)| {
            references::resolve(intent, &shape.topology, 3) == previous
        })
    );
    session.commit(edited.prepared).unwrap();
    assert_eq!(
        model::read_thread(session.document(), request.feature)
            .unwrap()
            .target,
        Some(target)
    );
}

#[test]
fn rust_thread_compiler_refuses_unimplemented_variants_without_changing_source() {
    use multicad_core::{
        constraint_sketch::Dimension,
        model::{
            ThreadEndTreatment, ThreadEndTreatments, ThreadRepresentation, ThreadStartTreatment,
        },
    };

    let mut session =
        Session::new(Document::new("Rust thread provider boundaries").unwrap()).unwrap();
    let stock = PrimitiveRequest::new(
        "Stock",
        PrimitiveParameters::Cylinder {
            radius: length(5.),
            height: length(20.),
            frame: Default::default(),
        },
    );
    let prepared = model::prepare_primitive(&session, 0, actor(), &stock).unwrap();
    session.commit(prepared.prepared).unwrap();
    let before = session.document().clone();
    let receiving = ThreadRequest::receiving(
        "Receiving cut",
        ThreadMode::CutExternal,
        BodyInput {
            body: stock.body,
            producer: stock.feature,
        },
        ThreadParameters::default(),
    )
    .unwrap();
    let cosmetic = ThreadRequest::new(
        "Cosmetic thread",
        ThreadParameters {
            representation: ThreadRepresentation::CosmeticEnvelope,
            family: ThreadFamily::Custom,
            custom: Some(CustomThreadProfile {
                radial_taper: Dimension::Literal {
                    value: 0.01,
                    unit: "ratio".into(),
                },
                ..Default::default()
            }),
            ..Default::default()
        },
    );
    let chamfer = ThreadRequest::new(
        "Chamfered thread",
        ThreadParameters {
            end_treatments: Some(ThreadEndTreatments {
                start: ThreadStartTreatment::Chamfer45 {
                    radial_depth: length(0.5),
                },
                end: ThreadEndTreatment::FlatCut,
            }),
            ..Default::default()
        },
    );
    let tapered = ThreadRequest::new(
        "Tapered custom thread",
        ThreadParameters {
            family: ThreadFamily::Custom,
            custom: Some(CustomThreadProfile {
                radial_taper: Dimension::Literal {
                    value: 0.01,
                    unit: "ratio".into(),
                },
                ..Default::default()
            }),
            ..Default::default()
        },
    );
    for (request, diagnostic) in [
        (receiving, "receiving cuts"),
        (cosmetic, "untapered envelope"),
        (chamfer, "end treatments"),
        (tapered, "untapered profile"),
    ] {
        let candidate = model::prepare_thread(&session, 1, actor(), &request).unwrap();
        let failure = Compiler {
            omit_display: true,
            ..Default::default()
        }
        .compile_with_backend(
            candidate.prepared.document(),
            2,
            GeometryBackend::Rust,
            None,
        )
        .err()
        .expect("unsupported variant must never fall back to OpenCascade");
        assert_eq!(failure.entity, Some(request.feature));
        assert!(failure.message.contains(diagnostic), "{failure}");
        assert_eq!(session.revision(), 1);
        assert_eq!(session.document(), &before);
    }
}
