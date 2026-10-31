use multicad_core::{
    commands::{Actor, Channel, Request, Session},
    document::{Document, Entity, EntityData, EntityId, Mutation, Placement},
    exchange::*,
    materials::{self, TagEdit, TagQuery},
    model::{
        self, BodyInput, BooleanRequest, Operation, PrimitiveParameters, PrimitiveRequest, length,
    },
    project_file::{self, ContentHash, ProjectFile},
};
use multicad_kernel::{GeometryPolicy, Solid};
use multicad_model::export::{ExportWorker, Exporter};
use std::{
    path::Path,
    time::{Duration, Instant},
};
fn actor() -> Actor {
    Actor::local(Channel::Mcp)
}
fn apply(s: &mut Session, edits: Vec<Mutation>) {
    s.apply(Request {
        expected_revision: s.revision(),
        actor: actor(),
        label: "Native export fixture".into(),
        edits,
    })
    .unwrap();
}
struct Fixture {
    session: Session,
    part: PrimitiveRequest,
    set: EntityId,
}
impl Fixture {
    fn new() -> Self {
        let mut session = Session::new(Document::new("Native export recipes").unwrap()).unwrap();
        let part = PrimitiveRequest::new(
            "Panel",
            PrimitiveParameters::Box {
                size: [length(10.), length(20.), length(30.)],
                frame: Default::default(),
            },
        );
        let p = model::prepare_primitive(&session, 0, actor(), &part).unwrap();
        session.commit(p.prepared).unwrap();
        let p = materials::prepare_tags(
            &session,
            session.revision(),
            actor(),
            &[part.body],
            &TagEdit::Add(["cnc".into()].into()),
        )
        .unwrap();
        session.commit(p).unwrap();
        let definition = SetDefinition {
            scope: Scope {
                root: ScopeRoot::Project,
                tags: Some(TagQuery {
                    all: ["cnc".into()].into(),
                    ..Default::default()
                }),
            },
            ..Default::default()
        };
        let set = EntityId::new();
        let p = prepare_set(
            &session,
            session.revision(),
            actor(),
            set,
            "Shop batch",
            &definition,
        )
        .unwrap();
        session.commit(p).unwrap();
        Self { session, part, set }
    }
    fn preview(&self) -> ExportPreview {
        Exporter::default()
            .preview(
                self.session.document(),
                self.session.revision(),
                self.set,
                &actor(),
                None,
            )
            .unwrap()
    }
    fn run(&self, key: &PreviewKey, root: &Path) -> Result<RunReceipt, ExportError> {
        Exporter::default().run(
            self.session.document(),
            self.session.revision(),
            key,
            root,
            &actor(),
            None,
        )
    }
    fn replace_set(&mut self, definition: &SetDefinition) {
        let p = prepare_set(
            &self.session,
            self.session.revision(),
            actor(),
            self.set,
            "Shop batch",
            definition,
        )
        .unwrap();
        self.session.commit(p).unwrap();
    }
}
fn near(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-5, "{a} != {b}");
}
#[test]
fn real_step_bundle_rebuilds_exact_geometry_and_reuses_identical_reopened_source() {
    let f = Fixture::new();
    let before = f.session.document().clone();
    let hash = ContentHash::document(&before).unwrap();
    let preview = f.preview();
    assert!(preview.can_run());
    assert_eq!(preview.artifacts.len(), 1);
    near(preview.artifacts[0].volume_mm3, 6000.);
    let root = tempfile::tempdir().unwrap();
    let receipt = f.run(&preview.key, root.path()).unwrap();
    assert!(!receipt.reused);
    assert_eq!(receipt.actor.channel, Channel::Mcp);
    assert_eq!(receipt.files.len(), 1);
    let file = Path::new(&receipt.directory).join(&receipt.files[0].artifact.output.filename);
    let bytes = std::fs::read(&file).unwrap();
    assert!(bytes.starts_with(b"ISO-10303-21;"));
    let fresh = Solid::read_step(&file, GeometryPolicy::default()).unwrap();
    near(fresh.volume_mm3().unwrap(), 6000.);
    assert_eq!(fresh.solid_count().unwrap(), 1);
    assert_eq!(f.session.document(), &before);
    assert_eq!(ContentHash::document(f.session.document()).unwrap(), hash);
    let mut bytes = vec![];
    project_file::write(
        &ProjectFile::new(before).unwrap(),
        &mut bytes,
        Default::default(),
    )
    .unwrap();
    let reopened = project_file::read(
        &mut bytes.as_slice(),
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap();
    let preview = Exporter::default()
        .preview(&reopened.project.document, 0, f.set, &actor(), None)
        .unwrap();
    let reused = Exporter::default()
        .run(
            &reopened.project.document,
            0,
            &preview.key,
            root.path(),
            &Actor::local(Channel::Ui),
            None,
        )
        .unwrap();
    assert!(reused.reused);
    assert_eq!(receipt.directory, reused.directory);
    assert_eq!(reused.actor.channel, Channel::Ui);
    assert_eq!(reused.key.source.revision, 0);
}
#[test]
fn explicit_assembly_scope_exports_each_exact_world_occurrence_without_collapsing_component_identity()
 {
    let mut f = Fixture::new();
    let q = (std::f64::consts::FRAC_PI_4).sin();
    let instances = [
        Entity::new(
            "Rotated copy",
            EntityData::Instance {
                component: f.part.body,
                placement: Placement {
                    translation_mm: [50., 5., 2.],
                    rotation_xyzw: [0., 0., q, q],
                },
                placement_source: None,
            },
        ),
        Entity::new(
            "Moved copy",
            EntityData::Instance {
                component: f.part.body,
                placement: Placement {
                    translation_mm: [-20., 40., 0.],
                    ..Default::default()
                },
                placement_source: None,
            },
        ),
    ];
    let assembly = Entity::new(
        "Bench assembly",
        EntityData::Assembly {
            instances: instances.iter().map(|i| i.id).collect(),
        },
    );
    let assembly_id = assembly.id;
    let mut edits: Vec<_> = instances.iter().cloned().map(Mutation::Insert).collect();
    edits.push(Mutation::Insert(assembly));
    apply(&mut f.session, edits);
    let mut d = read(f.session.document(), f.set).unwrap();
    d.scope.root = ScopeRoot::Assembly {
        assembly: assembly_id,
    };
    f.replace_set(&d);
    let preview = f.preview();
    assert_eq!(preview.artifacts.len(), 2);
    assert!(
        preview
            .artifacts
            .iter()
            .all(|a| a.producer == f.part.feature)
    );
    assert_ne!(preview.artifacts[0].solid, preview.artifacts[1].solid);
    let root = tempfile::tempdir().unwrap();
    let receipt = f.run(&preview.key, root.path()).unwrap();
    assert_eq!(receipt.files.len(), 2);
    for file in receipt.files {
        let shape = Solid::read_step(
            &Path::new(&receipt.directory).join(file.artifact.output.filename),
            GeometryPolicy::default(),
        )
        .unwrap();
        near(shape.volume_mm3().unwrap(), 6000.);
        let bounds = shape.bounds().unwrap();
        let instance = file.artifact.solid;
        if instance == instances[0].id {
            near(bounds.min[0], 30.);
            near(bounds.max[0], 50.);
            near(bounds.min[1], 5.);
            near(bounds.max[2], 32.);
        } else {
            assert_eq!(instance, instances[1].id);
            near(bounds.min[0], -20.);
            near(bounds.min[1], 40.);
        }
    }
}
#[test]
fn consumed_parts_are_explained_and_never_written_as_independent_solids() {
    let mut f = Fixture::new();
    let tool = PrimitiveRequest::new(
        "Cut tool",
        PrimitiveParameters::Box {
            size: [length(5.), length(20.), length(30.)],
            frame: Default::default(),
        },
    );
    let p = model::prepare_primitive(&f.session, f.session.revision(), actor(), &tool).unwrap();
    f.session.commit(p.prepared).unwrap();
    let p = materials::prepare_tags(
        &f.session,
        f.session.revision(),
        actor(),
        &[tool.body],
        &TagEdit::Add(["cnc".into()].into()),
    )
    .unwrap();
    f.session.commit(p).unwrap();
    let request = BooleanRequest::new(
        Operation::Cut,
        BodyInput {
            body: f.part.body,
            producer: f.part.feature,
        },
        vec![BodyInput {
            body: tool.body,
            producer: tool.feature,
        }],
    );
    let p = model::prepare_boolean(&f.session, f.session.revision(), actor(), &request).unwrap();
    f.session.commit(p.prepared).unwrap();
    let preview = f.preview();
    assert_eq!(preview.selection.parts.len(), 2);
    assert_eq!(preview.artifacts.len(), 1);
    near(preview.artifacts[0].volume_mm3, 3000.);
    assert!(preview.excluded.iter().any(|p|p.reference.part==tool.body&&p.reason==ExclusionReason::ConsumedProducer));
    let root = tempfile::tempdir().unwrap();
    assert_eq!(f.run(&preview.key, root.path()).unwrap().files.len(), 1);
}
#[test]
fn stale_unsupported_empty_and_cancelled_runs_do_not_publish_or_modify_source() {
    let mut f = Fixture::new();
    let preview = f.preview();
    let p = model::prepare_primitive_edit(
        &f.session,
        f.session.revision(),
        actor(),
        f.part.feature,
        &PrimitiveParameters::Box {
            size: [length(11.), length(20.), length(30.)],
            frame: Default::default(),
        },
    )
    .unwrap();
    f.session.commit(p.prepared).unwrap();
    let root = tempfile::tempdir().unwrap();
    assert_eq!(
        f.run(&preview.key, root.path()).unwrap_err().kind,
        ErrorKind::Stale
    );
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
    let mut d = read(f.session.document(), f.set).unwrap();
    if let RecipeOutput::Step { units, .. } = &mut d.recipes[0].output {
        *units = LengthUnit::Inch;
    }
    f.replace_set(&d);
    let preview = f.preview();
    assert!(matches!(
        preview.recipes[0].status,
        RecipeStatus::Unsupported(_)
    ));
    assert_eq!(
        f.run(&preview.key, root.path()).unwrap_err().kind,
        ErrorKind::Unsupported
    );
    d.recipes[0] = Recipe::step("STEP");
    d.scope.tags = Some(TagQuery {
        all: ["absent".into()].into(),
        ..Default::default()
    });
    f.replace_set(&d);
    let preview = f.preview();
    assert_eq!(preview.recipes[0].status, RecipeStatus::Empty);
    assert_eq!(
        f.run(&preview.key, root.path()).unwrap_err().kind,
        ErrorKind::Empty
    );
    let before = f.session.document().clone();
    assert_eq!(
        Exporter::default()
            .preview(
                &before,
                f.session.revision(),
                f.set,
                &actor(),
                Some(&|| true)
            )
            .unwrap_err()
            .kind,
        ErrorKind::Cancelled
    );
    assert_eq!(f.session.document(), &before);
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
}
#[test]
fn altered_or_unowned_output_bundles_are_never_overwritten() {
    let f = Fixture::new();
    let preview = f.preview();
    let root = tempfile::tempdir().unwrap();
    let receipt = f.run(&preview.key, root.path()).unwrap();
    let extra = Path::new(&receipt.directory).join("operator-note.txt");
    std::fs::write(&extra, "keep this").unwrap();
    assert!(
        f.run(&preview.key, root.path())
            .unwrap_err()
            .message
            .contains("additional")
    );
    assert_eq!(std::fs::read_to_string(&extra).unwrap(), "keep this");
    std::fs::remove_file(extra).unwrap();
    let path = Path::new(&receipt.directory).join(&receipt.files[0].artifact.output.filename);
    std::fs::write(&path, "operator changed this file").unwrap();
    assert!(
        f.run(&preview.key, root.path())
            .unwrap_err()
            .message
            .contains("modified")
    );
    assert_eq!(
        std::fs::read_to_string(path).unwrap(),
        "operator changed this file"
    );
}
fn reply(worker: &ExportWorker) -> ExportReply {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if let Some(reply) = worker.poll() {
            return reply;
        }
        assert!(Instant::now() < deadline, "export worker did not reply");
        std::thread::sleep(Duration::from_millis(2));
    }
}
#[test]
fn native_worker_preserves_repeated_run_intents_and_current_scope_provenance() {
    let f = Fixture::new();
    let worker = ExportWorker::new(Exporter::default()).unwrap();
    let id = worker
        .preview(f.session.document(), f.session.revision(), f.set, actor())
        .unwrap();
    let response = reply(&worker);
    assert_eq!(response.id, id);
    let ExportResult::Preview(Ok(preview)) = response.result else {
        panic!("preview failed")
    };
    let root = tempfile::tempdir().unwrap();
    let a = worker
        .run(
            f.session.document(),
            f.session.revision(),
            &preview.key,
            root.path(),
            actor(),
        )
        .unwrap();
    let b = worker
        .run(
            f.session.document(),
            f.session.revision(),
            &preview.key,
            root.path(),
            actor(),
        )
        .unwrap();
    let first = reply(&worker);
    let second = reply(&worker);
    assert_eq!((first.id, second.id), (a, b));
    assert_eq!(first.document, f.session.document().id());
    assert!(matches!(first.result,ExportResult::Run(Ok(ref r)) if !r.reused));
    assert!(matches!(second.result,ExportResult::Run(Ok(ref r)) if r.reused));
    assert!(!worker.pending());
}
#[test]
fn duplicate_part_names_get_short_suffixes_on_disk_with_identities_in_the_manifest() {
    let mut f = Fixture::new();
    // A second, larger Part with the same display name and tag.
    let twin = PrimitiveRequest::new(
        "Panel",
        PrimitiveParameters::Box {
            size: [length(10.), length(20.), length(60.)],
            frame: Default::default(),
        },
    );
    let p = model::prepare_primitive(&f.session, f.session.revision(), actor(), &twin).unwrap();
    f.session.commit(p.prepared).unwrap();
    let p = materials::prepare_tags(
        &f.session,
        f.session.revision(),
        actor(),
        &[twin.body],
        &TagEdit::Add(["cnc".into()].into()),
    )
    .unwrap();
    f.session.commit(p).unwrap();
    let preview = f.preview();
    assert!(preview.can_run());
    let names: Vec<_> = preview
        .artifacts
        .iter()
        .map(|a| a.output.filename.clone())
        .collect();
    assert_eq!(names.len(), 2);
    assert_ne!(names[0].to_ascii_lowercase(), names[1].to_ascii_lowercase());
    for name in &names {
        let stem = name
            .strip_prefix("Native_export_recipes_Panel_")
            .and_then(|s| s.strip_suffix(".step"))
            .unwrap_or_else(|| panic!("readable default name: {name}"));
        assert_eq!(stem.len(), 6, "{name}");
        assert!(stem.chars().all(|c| c.is_ascii_hexdigit()), "{name}");
    }
    // The real run writes both files under those names.
    let root = tempfile::tempdir().unwrap();
    let receipt = f.run(&preview.key, root.path()).unwrap();
    let mut on_disk: Vec<_> = std::fs::read_dir(&receipt.directory)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .filter(|n| n.ends_with(".step"))
        .collect();
    on_disk.sort();
    let mut expected = names.clone();
    expected.sort();
    assert_eq!(on_disk, expected);
    let mut volumes = vec![];
    for file in &receipt.files {
        let shape = Solid::read_step(
            &Path::new(&receipt.directory).join(&file.artifact.output.filename),
            GeometryPolicy::default(),
        )
        .unwrap();
        volumes.push(shape.volume_mm3().unwrap().round() as i64);
    }
    volumes.sort();
    assert_eq!(volumes, [6000, 12000], "each name holds its own Part");
    // The manifest binds each file name to its Part identity.
    let manifest: serde_json::Value = serde_json::from_slice(
        &std::fs::read(Path::new(&receipt.directory).join("multicad-export.json")).unwrap(),
    )
    .unwrap();
    let text = manifest.to_string();
    for id in [f.part.body, twin.body] {
        assert!(text.contains(&id.to_string()), "{id} missing from manifest");
    }
    for name in &names {
        assert!(text.contains(name.as_str()));
    }
}
