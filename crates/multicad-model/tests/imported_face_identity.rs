use multicad_core::{
    commands::{Actor, Channel, Request, Session},
    document::{Document, Entity, EntityData, EntityId, Link, Mutation},
    model::{
        self, BodyInput,
        imports::{prepare_step_import, prepare_step_update},
        length,
    },
    project_file::{self, ProjectFile},
    references::{
        self, AnalyticGeometry, Attachment, GeometryIntent, ReferenceAddress, ReferenceFrame,
        Resolution, TopologyKind,
    },
};
use multicad_kernel::{
    Solid,
    rust_kernel::{
        planar_boolean::Union,
        step,
        step_read::{ImportProfile, read_import_solid},
    },
};
use multicad_model::{Compiler, GeometryBackend, Model, imports::read_step_file_with_backend};

fn actor() -> Actor {
    Actor::local(Channel::Ui)
}
fn compile(session: &Session) -> Model {
    Compiler::default()
        .compile_with_backend(
            session.document(),
            session.revision(),
            GeometryBackend::Rust,
            None,
        )
        .unwrap()
}
fn reopen(session: &Session) -> Session {
    let mut bytes = vec![];
    project_file::write(
        &ProjectFile::new(session.document().clone()).unwrap(),
        &mut bytes,
        Default::default(),
    )
    .unwrap();
    Session::new(
        project_file::read(
            &mut bytes.as_slice(),
            Default::default(),
            &project_file::supported_capabilities(),
        )
        .unwrap()
        .project
        .document,
    )
    .unwrap()
}
fn near(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-7, "{a} != {b}");
}
fn tetra(split: Option<f64>) -> String {
    let [a, b, c, d] = [[0., 0., 0.], [4., 0., 0.], [0., 4., 0.], [0., 0., 4.]];
    let rings = if let Some(y) = split {
        let e = [0., y, 4. - y];
        vec![
            vec![a, c, b],
            vec![a, b, d],
            vec![a, d, e],
            vec![a, e, c],
            vec![b, c, e, d],
        ]
    } else {
        vec![vec![a, c, b], vec![a, b, d], vec![a, d, c], vec![b, c, d]]
    };
    let geometry = Union::from_planar_rings(&rings, 1e-7).unwrap();
    let text = step::write_planar_union(&geometry, [0.; 3], [1., 0., 0.], [0., 1., 0.]).unwrap();
    assert!(matches!(
        read_import_solid(&text, 1e-7).unwrap().1.profile,
        ImportProfile::PlanarBoundary { .. }
    ));
    text
}
fn reverse_shell(text: &str) -> String {
    let marker = "=CLOSED_SHELL('',(";
    let start = text.find(marker).unwrap() + marker.len();
    let end = start + text[start..].find("))").unwrap();
    let mut result = text.to_owned();
    result.replace_range(
        start..end,
        &text[start..end]
            .split(',')
            .rev()
            .collect::<Vec<_>>()
            .join(","),
    );
    result
}
fn rotate_wires(text: &str) -> String {
    text.lines()
        .map(|line| {
            let marker = "=EDGE_LOOP('',(";
            let Some(start) = line.find(marker).map(|i| i + marker.len()) else {
                return line.to_owned();
            };
            let end = start + line[start..].find("))").unwrap();
            let mut refs = line[start..end].split(',').collect::<Vec<_>>();
            refs.rotate_left(1);
            let mut result = line.to_owned();
            result.replace_range(start..end, &refs.join(","));
            result
        })
        .collect::<Vec<_>>()
        .join("\n")
}
fn reverse_declarations(text: &str) -> String {
    let mut records = text.lines().filter(|line| line.starts_with('#')).rev();
    text.lines()
        .map(|line| {
            if line.starts_with('#') {
                records.next().unwrap()
            } else {
                line
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}
fn renumber(text: &str) -> String {
    let maximum = text
        .lines()
        .filter_map(|line| {
            line.strip_prefix('#')?
                .split_once('=')?
                .0
                .parse::<u32>()
                .ok()
        })
        .max()
        .unwrap();
    let mut result = String::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        result.push(c);
        if c != '#' {
            continue;
        }
        let mut digits = String::new();
        while chars.peek().is_some_and(char::is_ascii_digit) {
            digits.push(chars.next().unwrap());
        }
        let id = digits.parse::<u32>().unwrap();
        result.push_str(&(maximum + 1 - id).to_string());
    }
    result
}
struct Fixture {
    session: Session,
    path: std::path::PathBuf,
    source: EntityId,
    body: EntityId,
    producer: EntityId,
    occurrence: EntityId,
    faces: Vec<GeometryIntent>,
    datums: Vec<ReferenceAddress>,
}
fn fixture(directory: &std::path::Path, text: &str) -> Fixture {
    let path = directory.join("retained-tetrahedron.step");
    std::fs::write(&path, text).unwrap();
    let file = read_step_file_with_backend(&path, GeometryBackend::Rust).unwrap();
    let mut session = Session::new(Document::new("Stable imported faces").unwrap()).unwrap();
    let import = prepare_step_import(&session, 0, actor(), &file).unwrap();
    let source = import.source;
    let part = import.model.outputs[0].clone();
    session.commit(import.model.prepared).unwrap();
    let instance = model::InstanceCopyRequest::new(
        model::InstanceCopySource::Part {
            source: BodyInput {
                body: part.body,
                producer: part.feature,
            },
        },
        model::NumericTransform {
            translation: [20., 5., -3.].map(length),
            axis: [1., 2., 3.],
            angle: model::angle(37.),
            ..Default::default()
        },
    );
    let occurrence = instance.instance;
    session
        .commit(
            model::prepare_instance_copies(&session, session.revision(), actor(), &[instance])
                .unwrap()
                .prepared,
        )
        .unwrap();
    let initial = compile(&session);
    let topology = &initial.snapshot().shapes[&part.feature].topology;
    let mut faces = vec![];
    let mut datums = vec![];
    let mut edits = vec![];
    for face in topology
        .elements
        .iter()
        .filter(|element| element.key.kind == TopologyKind::Face)
    {
        assert!(
            face.origins
                .iter()
                .all(|origin| origin.role.starts_with("imported-plane.v1.")),
            "unsafe ordinal alias {face:?}"
        );
        let mut intent = GeometryIntent::capture(topology, face.key).unwrap();
        intent.boundary = Some(face.boundary.clone());
        faces.push(intent.clone());
        intent.attachment = Some(Attachment {
            anchor: face.frames[0].origin.clone(),
            offset_mm: [0.25, -0.5, 0.75],
            angle_rad: 0.3,
        });
        let reference = intent.id;
        let entity = Entity::new(
            format!("Finite face datum {}", face.key.ordinal),
            EntityData::Plane {
                origin_mm: [0.; 3],
                normal: [0., 0., 1.],
                anchor: Some(Link {
                    target: part.feature,
                    role: "face_attachment".into(),
                    geometry: Some(intent),
                }),
            },
        );
        datums.push(ReferenceAddress {
            owner: entity.id,
            reference,
        });
        edits.push(Mutation::Insert(entity));
    }
    session
        .apply(Request {
            expected_revision: session.revision(),
            actor: actor(),
            label: "Attach every imported finite face".into(),
            edits,
        })
        .unwrap();
    Fixture {
        session,
        path,
        source,
        body: part.body,
        producer: part.feature,
        occurrence,
        faces,
        datums,
    }
}
fn datum_frames(model: &Model, addresses: &[ReferenceAddress]) -> Vec<ReferenceFrame> {
    addresses
        .iter()
        .map(|&address| {
            let Resolution::Resolved {
                frame: Some(frame), ..
            } = model.snapshot().references.get(address).unwrap().resolution
            else {
                panic!("datum needs explicit repair")
            };
            frame
        })
        .collect()
}
fn check_equivalent(
    fixture: &Fixture,
    session: &Session,
    baseline: &Model,
    frames: &[ReferenceFrame],
) {
    let model = compile(session);
    assert!(model.snapshot().references.all_resolved());
    assert_eq!(datum_frames(&model, &fixture.datums), frames);
    near(
        model.solid(fixture.body).unwrap().volume_mm3().unwrap(),
        32. / 3.,
    );
    near(
        model
            .solid(fixture.occurrence)
            .unwrap()
            .volume_mm3()
            .unwrap(),
        32. / 3.,
    );
    let before = &baseline.snapshot().shapes[&fixture.producer].topology;
    let after = &model.snapshot().shapes[&fixture.producer].topology;
    for intent in &fixture.faces {
        let Resolution::Resolved { key: old_key, .. } =
            references::resolve(intent, before, before.revision)
        else {
            panic!("old face")
        };
        let Resolution::Resolved { key, .. } =
            references::resolve(intent, after, session.revision())
        else {
            panic!("face declaration order changed identity")
        };
        let old = &baseline.solid(fixture.body).unwrap().faces().unwrap()[old_key.ordinal as usize];
        let new = &model.solid(fixture.body).unwrap().faces().unwrap()[key.ordinal as usize];
        near(old.area_mm2, new.area_mm2);
        for (a, b) in old.center_mm.into_iter().zip(new.center_mm) {
            near(a, b);
        }
    }
}

#[test]
fn equivalent_step_permutations_preserve_every_finite_face_and_full_datum_chart() {
    for split in [None, Some(2.)] {
        let directory = tempfile::tempdir().unwrap();
        let original = tetra(split);
        let mut fixture = fixture(directory.path(), &original);
        let baseline = compile(&fixture.session);
        let frames = datum_frames(&baseline, &fixture.datums);
        let legacy = GeometryIntent {
            id: EntityId::new(),
            kind: TopologyKind::Face,
            origins: std::collections::BTreeSet::from([references::Origin {
                source: fixture.producer,
                role: "planar-boolean.face.2".into(),
            }]),
            boundary: None,
            attachment: None,
        };
        assert!(matches!(
            references::resolve(
                &legacy,
                &baseline.snapshot().shapes[&fixture.producer].topology,
                fixture.session.revision()
            ),
            Resolution::Missing
        ));
        let permutations = [
            reverse_shell(&original),
            reverse_declarations(&original),
            rotate_wires(&original),
            renumber(&original),
            reverse_declarations(&renumber(&rotate_wires(&reverse_shell(&original)))),
        ];
        for changed in permutations {
            std::fs::write(&fixture.path, changed).unwrap();
            let file = read_step_file_with_backend(&fixture.path, GeometryBackend::Rust).unwrap();
            let before = fixture.session.document().clone();
            let revision = fixture.session.revision();
            let (plan, update) =
                prepare_step_update(&fixture.session, revision, actor(), fixture.source, &file)
                    .unwrap();
            assert_eq!(plan.matched.len(), 1);
            assert!(plan.missing.is_empty());
            assert!(plan.added.is_empty());
            let preview = Compiler::default()
                .compile_with_backend(
                    update.model.prepared.document(),
                    revision + 1,
                    GeometryBackend::Rust,
                    None,
                )
                .unwrap();
            assert_eq!(datum_frames(&preview, &fixture.datums), frames);
            assert_eq!(fixture.session.document(), &before);
            assert_eq!(fixture.session.revision(), revision);
            fixture.session.commit(update.model.prepared).unwrap();
            let after = fixture.session.document().clone();
            check_equivalent(&fixture, &fixture.session, &baseline, &frames);
            check_equivalent(&fixture, &reopen(&fixture.session), &baseline, &frames);
            fixture
                .session
                .undo(fixture.session.revision(), actor())
                .unwrap();
            assert_eq!(fixture.session.document(), &before);
            check_equivalent(&fixture, &fixture.session, &baseline, &frames);
            fixture
                .session
                .redo(fixture.session.revision(), actor())
                .unwrap();
            assert_eq!(fixture.session.document(), &after);
            check_equivalent(&fixture, &fixture.session, &baseline, &frames);
        }
    }
}

#[test]
fn changed_or_split_coplanar_regions_require_repair_without_mutating_the_valid_project() {
    for (initial, next) in [(None, Some(2.)), (Some(2.), Some(1.)), (Some(2.), None)] {
        let directory = tempfile::tempdir().unwrap();
        let original = tetra(initial);
        let fixture = fixture(directory.path(), &original);
        let before = fixture.session.document().clone();
        let revision = fixture.session.revision();
        let activity = fixture.session.activity().count();
        let model = compile(&fixture.session);
        let topology = &model.snapshot().shapes[&fixture.producer].topology;
        let selected:Vec<_>=fixture.faces.iter().filter(|intent|matches!(references::resolve(intent,topology,revision),Resolution::Resolved {geometry:Some(AnalyticGeometry::Plane {frame}),..}if frame.normal()[0] < -0.99)).collect();
        assert_eq!(selected.len(), if initial.is_some() { 2 } else { 1 });
        let export = directory.path().join("prior-valid.step");
        model
            .solid(fixture.body)
            .unwrap()
            .write_step(&export)
            .unwrap();
        let exported = std::fs::read(&export).unwrap();
        std::fs::write(&fixture.path, tetra(next)).unwrap();
        let file = read_step_file_with_backend(&fixture.path, GeometryBackend::Rust).unwrap();
        let (_, candidate) =
            prepare_step_update(&fixture.session, revision, actor(), fixture.source, &file)
                .unwrap();
        let scoped = Compiler::default()
            .compile_sources_with_backend(
                candidate.model.prepared.document(),
                revision + 1,
                &[fixture.producer],
                GeometryBackend::Rust,
                None,
            )
            .unwrap();
        let updated = &scoped.snapshot().shapes[&fixture.producer].topology;
        for intent in selected {
            assert!(
                matches!(
                    references::resolve(intent, updated, revision + 1),
                    Resolution::Missing
                ),
                "unproved coplanar finite region was silently rebound"
            );
        }
        let candidate_model = Compiler::default()
            .compile_with_backend(
                candidate.model.prepared.document(),
                revision + 1,
                GeometryBackend::Rust,
                None,
            )
            .unwrap();
        let error = multicad_model::imports::validate_references(
            candidate.model.prepared.document(),
            &candidate_model,
        )
        .expect_err("source update must require explicit datum repair");
        assert!(
            error.message.to_lowercase().contains("reference")
                || error.message.to_lowercase().contains("missing")
        );
        assert_eq!(fixture.session.document(), &before);
        assert_eq!(fixture.session.revision(), revision);
        assert_eq!(fixture.session.activity().count(), activity);
        assert!(fixture.session.can_undo());
        assert!(!fixture.session.can_redo());
        assert_eq!(std::fs::read(&export).unwrap(), exported);
        let retained = compile(&fixture.session);
        near(
            retained.solid(fixture.body).unwrap().volume_mm3().unwrap(),
            32. / 3.,
        );
        assert_eq!(
            datum_frames(&retained, &fixture.datums),
            datum_frames(&model, &fixture.datums)
        );
        let read = Solid::import_rust_step(
            &String::from_utf8(exported).unwrap(),
            EntityId::new(),
            Default::default(),
        )
        .unwrap();
        near(read.volume_mm3().unwrap(), 32. / 3.);
    }
}

#[test]
fn changed_non_driving_measurement_remains_inspectable_without_blocking_import() {
    use multicad_core::inspection::{GeometryReference, MeasurementQuery, PinRequest, prepare_pin};
    let directory = tempfile::tempdir().unwrap();
    let mut fixture = fixture(directory.path(), &tetra(None));
    fixture
        .session
        .apply(Request {
            expected_revision: fixture.session.revision(),
            actor: actor(),
            label: "Remove driving datum attachments".into(),
            edits: fixture
                .datums
                .iter()
                .map(|address| Mutation::Remove(address.owner))
                .collect(),
        })
        .unwrap();
    let pin = PinRequest::new(
        "Imported face distance",
        MeasurementQuery {
            a: GeometryReference {
                source: fixture.producer,
                intent: fixture.faces[0].clone(),
            },
            b: GeometryReference {
                source: fixture.producer,
                intent: fixture.faces[1].clone(),
            },
        },
    );
    fixture
        .session
        .commit(prepare_pin(&fixture.session, fixture.session.revision(), actor(), &pin).unwrap())
        .unwrap();
    std::fs::write(&fixture.path, tetra(Some(2.))).unwrap();
    let file = read_step_file_with_backend(&fixture.path, GeometryBackend::Rust).unwrap();
    let (_, candidate) = prepare_step_update(
        &fixture.session,
        fixture.session.revision(),
        actor(),
        fixture.source,
        &file,
    )
    .unwrap();
    let model = Compiler::default()
        .compile_with_backend(
            candidate.model.prepared.document(),
            fixture.session.revision() + 1,
            GeometryBackend::Rust,
            None,
        )
        .unwrap();
    // Retain the complete diagnostic for repairing the pin, while the shared
    // import gate follows the canonical non-driving exclusion policy.
    assert!(!model.snapshot().references.all_resolved());
    multicad_model::imports::validate_references(candidate.model.prepared.document(), &model)
        .unwrap();
    fixture.session.commit(candidate.model.prepared).unwrap();
}
