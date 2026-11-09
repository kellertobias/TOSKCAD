//! Print packages written from exact native solids: a lifted block with a
//! removable support and a post on the plate. The 3MF and the per-Part STL
//! files are read back from disk for their objects, millimeter unit and
//! volumes, with and without the aid. Stale sources and failed evaluations
//! write nothing.
use multicad_core::{
    commands::{Actor, Channel, Request, Session},
    constraint_sketch::Dimension,
    document::{Document, Entity, EntityData, EntityId, Mutation},
    exchange::*,
    materials::{TagEdit, prepare_tags},
    model::{
        self, AidDefinition, AidRequest, AidShape, BodyOutput, PrimitiveFrame, PrimitiveParameters,
        PrimitiveRequest, ProcessDefinition, angle, length,
    },
    parameters::BoundExpression,
};
use multicad_model::export::{Exporter, mesh};
use std::{collections::BTreeSet, path::Path};

fn actor() -> Actor {
    Actor::local(Channel::Ui)
}
struct Fixture {
    s: Session,
    lift: EntityId,
    block: EntityId,
    block_feature: EntityId,
    post: EntityId,
    post_feature: EntityId,
    support: EntityId,
    process: EntityId,
}
fn primitive(s: &mut Session, request: &PrimitiveRequest) {
    let p = model::prepare_primitive(s, s.revision(), actor(), request).unwrap();
    s.commit(p.prepared).unwrap();
}
/// A 40 × 20 × 10 mm block lifted by `lift` (15 mm) over a support with a
/// 0.1 mm gap, and a 10 mm post resting on the plate.
fn fixture() -> Fixture {
    let mut s = Session::new(Document::new("Print packages").unwrap()).unwrap();
    let lift = Entity::new(
        "lift",
        EntityData::Parameter {
            expression: "15".into(),
            unit: "mm".into(),
            exposed: true,
            inputs: vec![],
        },
    );
    let lift_id = lift.id;
    s.apply(Request {
        expected_revision: 0,
        actor: actor(),
        label: "Parameter".into(),
        edits: vec![Mutation::Insert(lift)],
    })
    .unwrap();
    let block = PrimitiveRequest::new(
        "Block",
        PrimitiveParameters::Box {
            size: [length(40.), length(20.), length(10.)],
            frame: PrimitiveFrame {
                origin: [
                    length(0.),
                    length(0.),
                    Dimension::Expression {
                        expression: BoundExpression::bind(s.document(), "=$lift", "mm", None)
                            .unwrap(),
                    },
                ],
                ..Default::default()
            },
        },
    );
    primitive(&mut s, &block);
    let post = PrimitiveRequest::new(
        "Post",
        PrimitiveParameters::Box {
            size: [length(10.), length(10.), length(10.)],
            frame: PrimitiveFrame {
                origin: [length(60.), length(0.), length(0.)],
                ..Default::default()
            },
        },
    );
    primitive(&mut s, &post);
    let (process, prepared) = model::prepare_process(
        &s,
        s.revision(),
        actor(),
        None,
        "PLA 0.4 mm",
        &ProcessDefinition::default(),
    )
    .unwrap();
    s.commit(prepared).unwrap();
    let support = AidRequest::new(
        "Support",
        AidDefinition {
            output: BodyOutput {
                body: EntityId::new(),
            },
            target: block.body,
            shape: AidShape::Support {
                origin: [length(5.), length(5.)],
                angle: angle(0.),
                length: length(10.),
                width: length(10.),
                base: length(0.),
                gap: length(0.1),
            },
            process: Some(process),
        },
    );
    let p = model::prepare_aid(&s, s.revision(), actor(), &support).unwrap();
    s.commit(p.prepared).unwrap();
    Fixture {
        s,
        lift: lift_id,
        block: block.body,
        block_feature: block.feature,
        post: post.body,
        post_feature: post.feature,
        support: support.body,
        process,
    }
}
fn request(parts: Vec<EntityId>, aids: AidPolicy, stl: bool) -> PrintRequest {
    PrintRequest {
        source: PrintSource::Parts { parts },
        aids,
        quality: MeshQuality::default(),
        stl_per_part: stl,
    }
}
fn preview(s: &Session, request: &PrintRequest) -> PrintPreview {
    Exporter::default()
        .print_preview(s.document(), s.revision(), request, &actor(), None)
        .unwrap()
}
fn write(
    s: &Session,
    request: &PrintRequest,
    key: &PrintKey,
    path: &Path,
) -> Result<PrintReceipt, ExportError> {
    Exporter::default().write_print(
        s.document(),
        s.revision(),
        request,
        key,
        path,
        &actor(),
        None,
    )
}
fn near(a: f64, b: f64, tolerance: f64) {
    assert!((a - b).abs() <= tolerance, "{a} != {b} ± {tolerance}");
}
fn listing(dir: &Path) -> BTreeSet<String> {
    std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect()
}

#[test]
fn package_reads_back_with_and_without_the_removable_support() {
    let f = fixture();
    let dir = tempfile::tempdir().unwrap();
    let with = request(vec![f.block, f.post], AidPolicy::IncludeAll, false);
    let p = preview(&f.s, &with);
    assert!(p.can_write(), "{:?}", p.blocked);
    assert_eq!(
        p.objects.iter().map(|o| o.part).collect::<Vec<_>>(),
        [f.block, f.post, f.support]
    );
    assert!(matches!(p.objects[2].role, PrintRole::Aid { target, .. } if target == f.block));
    // The support rises from the plate to 0.1 mm under the block at z = 15.
    near(p.objects[2].bounds_mm[1][2], 14.9, 1e-6);
    near(p.objects[2].volume_mm3, 10. * 10. * 14.9, 1e-6);
    let process = &p.processes[0];
    assert_eq!(process.id, f.process);
    assert_eq!((process.layer_height_mm, process.line_width_mm), (0.2, 0.4));
    assert!(
        p.findings
            .iter()
            .any(|x| x.part == Some(f.support) && x.message.contains("gap")),
        "the 0.1 mm gap is below the 0.2 mm process gap: {:?}",
        p.findings
    );
    assert_eq!(p.files.len(), 1);
    assert_eq!(p.files[0].filename("plate"), "plate.3mf");
    let path = dir.path().join("plate.3mf");
    let receipt = write(&f.s, &with, &p.key, &path).unwrap();
    assert_eq!(receipt.files.len(), 1);
    let read = mesh::read_three_mf(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!((read.objects, read.items, read.unit_scale), (3, 3, 1.));
    near(
        read.volume_mm3(),
        8000. + 1000. + 1490.,
        1e-6 * 10490. + 1e-3,
    );
    let bounds = read.bounds_mm().unwrap();
    near(bounds[0][2], 0., 1e-6);
    near(bounds[1][2], 25., 1e-6);
    near(bounds[1][0], 70., 1e-6);
    // The model part names the objects; the unit is declared millimeter.
    let entries = mesh::read_stored_zip(&std::fs::read(&path).unwrap()).unwrap();
    let model = String::from_utf8(
        entries
            .iter()
            .find(|(n, _)| n == "3D/3dmodel.model")
            .unwrap()
            .1
            .clone(),
    )
    .unwrap();
    assert!(model.contains("unit=\"millimeter\""));
    for name in ["Block", "Post", "Support"] {
        assert!(model.contains(&format!("name=\"{name}\"")), "{name}");
    }

    // Without the aid: the support is left out and listed, the block's
    // findings still say what the left-out support would face.
    let without = request(vec![f.block, f.post], AidPolicy::ExcludeRemovable, false);
    let p = preview(&f.s, &without);
    assert_eq!(
        p.objects.iter().map(|o| o.part).collect::<Vec<_>>(),
        [f.block, f.post]
    );
    assert!(p.selection.skipped.iter().any(|s| s.entity == f.support));
    assert!(p.findings.iter().any(|x| x.subject.contains("(left out)")));
    let path = dir.path().join("finished.3mf");
    write(&f.s, &without, &p.key, &path).unwrap();
    let read = mesh::read_three_mf(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!((read.objects, read.items), (2, 2));
    near(read.volume_mm3(), 9000., 1e-2);
    assert_eq!(
        listing(dir.path()),
        BTreeSet::from(["plate.3mf".into(), "finished.3mf".into()])
    );
}

#[test]
fn per_part_stl_files_sit_beside_the_3mf_and_replace_atomically() {
    let f = fixture();
    let dir = tempfile::tempdir().unwrap();
    let r = request(vec![f.block], AidPolicy::IncludeAll, true);
    let p = preview(&f.s, &r);
    assert_eq!(
        p.files
            .iter()
            .map(|x| x.filename("kit"))
            .collect::<Vec<_>>(),
        ["kit.3mf", "kit-Block.stl", "kit-Support.stl"]
    );
    let path = dir.path().join("kit.3mf");
    std::fs::write(&path, b"old bytes").unwrap();
    let receipt = write(&f.s, &r, &p.key, &path).unwrap();
    assert_eq!(receipt.files.len(), 3);
    let block = mesh::read_stl(&std::fs::read(dir.path().join("kit-Block.stl")).unwrap()).unwrap();
    near(block.volume_mm3(), 8000., 0.1);
    near(block.bounds_mm().unwrap()[0][2], 15., 1e-4);
    let support =
        mesh::read_stl(&std::fs::read(dir.path().join("kit-Support.stl")).unwrap()).unwrap();
    near(support.volume_mm3(), 1490., 0.1);
    let package = mesh::read_three_mf(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(package.objects, 2);
    for written in &receipt.files {
        let bytes = std::fs::read(&written.path).unwrap();
        assert_eq!(
            multicad_core::project_file::ContentHash::of(&bytes),
            written.content_hash
        );
    }
    // No staging files remain.
    assert_eq!(listing(dir.path()).len(), 3);
}

#[test]
fn tags_choose_parts_and_empty_packages_cannot_be_written() {
    let mut f = fixture();
    let tag = PrintRequest {
        source: PrintSource::Tag {
            tag: DEFAULT_PRINT_TAG.into(),
        },
        ..request(vec![], AidPolicy::IncludeAll, false)
    };
    let empty = preview(&f.s, &tag);
    assert!(!empty.can_write());
    assert!(empty.blocked.as_deref().unwrap().contains("printed"));
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("empty.3mf");
    let refused = write(&f.s, &tag, &empty.key, &path).unwrap_err();
    assert_eq!(refused.kind, ErrorKind::Empty);
    assert!(!path.exists());
    let p = prepare_tags(
        &f.s,
        f.s.revision(),
        actor(),
        &[f.post],
        &TagEdit::Add(BTreeSet::from([DEFAULT_PRINT_TAG.to_string()])),
    )
    .unwrap();
    f.s.commit(p).unwrap();
    let tagged = preview(&f.s, &tag);
    assert_eq!(
        tagged.objects.iter().map(|o| o.part).collect::<Vec<_>>(),
        [f.post]
    );
    assert!(tagged.processes.is_empty(), "the post has no aids");
    write(&f.s, &tag, &tagged.key, &path).unwrap();
    let read = mesh::read_three_mf(&std::fs::read(&path).unwrap()).unwrap();
    near(read.volume_mm3(), 1000., 1e-6);
}

#[test]
fn stale_sources_changed_plans_and_failed_evaluations_write_nothing() {
    let mut f = fixture();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("plate.3mf");
    let r = request(vec![f.block], AidPolicy::IncludeAll, true);
    let old = preview(&f.s, &r);
    // A current source with a different plan (no STL files) is refused.
    let mut changed = r.clone();
    changed.stl_per_part = false;
    let refused = write(&f.s, &changed, &old.key, &path).unwrap_err();
    assert_eq!(refused.kind, ErrorKind::Stale);
    let mut lift = f.s.document().get(f.lift).unwrap().clone();
    let EntityData::Parameter { expression, .. } = &mut lift.data else {
        unreachable!()
    };
    *expression = "0".into();
    f.s.apply(Request {
        expected_revision: f.s.revision(),
        actor: actor(),
        label: "Lower the block onto the plate".into(),
        edits: vec![Mutation::Replace(lift)],
    })
    .unwrap();
    // The old preview's source no longer matches the document.
    let stale = write(&f.s, &r, &old.key, &path).unwrap_err();
    assert_eq!(stale.kind, ErrorKind::Stale);
    // The block now reaches the plate over the support footprint, so the
    // support cannot be evaluated: preview and write both report why.
    let failed = Exporter::default()
        .print_preview(f.s.document(), f.s.revision(), &r, &actor(), None)
        .unwrap_err();
    assert!(
        failed.message.to_lowercase().contains("support"),
        "{failed}"
    );
    let key = PrintKey {
        source: ExportStamp::capture(f.s.document(), f.s.revision()).unwrap(),
        plan_hash: old.key.plan_hash,
    };
    assert!(write(&f.s, &r, &key, &path).is_err());
    assert!(listing(dir.path()).is_empty(), "nothing was written");
}

/// Place a copy of a Part by Move/Copy and return the new Instance.
fn copy(s: &mut Session, body: EntityId, producer: EntityId, name: &str, dy: f64) -> EntityId {
    let mut request = model::InstanceCopyRequest::new(
        model::InstanceCopySource::Part {
            source: model::BodyInput { body, producer },
        },
        model::NumericTransform {
            translation: [length(0.), length(dy), length(0.)],
            ..Default::default()
        },
    );
    request.name = name.into();
    let p = model::prepare_instance_copies(s, s.revision(), actor(), &[request.clone()]).unwrap();
    s.commit(p.prepared).unwrap();
    request.instance
}
/// Volumes of the placed objects in build-item order, given each object's
/// triangle count, from a read-back 3MF.
fn placed_volumes(read: &mesh::ReadMesh, triangles: &[usize]) -> Vec<f64> {
    let mut rest = read.world.as_slice();
    triangles
        .iter()
        .map(|n| {
            let (own, tail) = rest.split_at(*n);
            rest = tail;
            mesh::ReadMesh {
                unit_scale: 1.,
                objects: 1,
                items: 1,
                triangles: own.len(),
                world: own.to_vec(),
            }
            .volume_mm3()
        })
        .collect()
}

#[test]
fn selected_placed_copies_are_packaged_per_occurrence_and_share_one_mesh() {
    let mut f = fixture();
    let a = copy(&mut f.s, f.post, f.post_feature, "Copy A", 30.);
    let b = copy(&mut f.s, f.post, f.post_feature, "Copy B", 60.);
    let dir = tempfile::tempdir().unwrap();

    // Only the selected copies are packaged, each at its own placement.
    let r = request(vec![a, b], AidPolicy::IncludeAll, true);
    let p = preview(&f.s, &r);
    assert!(p.can_write(), "{:?}", p.blocked);
    assert_eq!(
        p.objects
            .iter()
            .map(|o| (o.part, o.occurrence_path.clone(), o.name.as_str()))
            .collect::<Vec<_>>(),
        [
            (f.post, vec![a], "Post (Copy A)"),
            (f.post, vec![b], "Post (Copy B)"),
        ]
    );
    for (object, y) in p.objects.iter().zip([30., 60.]) {
        near(object.bounds_mm[0][1], y, 1e-6);
        near(object.bounds_mm[1][0], 70., 1e-6);
        near(object.volume_mm3, 1000., 1e-6);
    }
    assert_eq!((p.files[0].objects, p.files[0].items), (1, 2));
    assert_eq!(
        p.files
            .iter()
            .map(|x| x.filename("copies"))
            .collect::<Vec<_>>(),
        [
            "copies.3mf",
            "copies-Post (Copy A).stl",
            "copies-Post (Copy B).stl"
        ]
    );
    let path = dir.path().join("copies.3mf");
    let receipt = write(&f.s, &r, &p.key, &path).unwrap();
    assert_eq!(receipt.files.len(), 3);
    let read = mesh::read_three_mf(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(
        (read.objects, read.items),
        (1, 2),
        "one shared mesh, two items"
    );
    let triangles: Vec<_> = p.objects.iter().map(|o| o.triangles).collect();
    for volume in placed_volumes(&read, &triangles) {
        near(volume, 1000., 1e-6);
    }
    let bounds = read.bounds_mm().unwrap();
    near(bounds[0][1], 30., 1e-6);
    near(bounds[1][1], 70., 1e-6);
    let stl = mesh::read_stl(&std::fs::read(dir.path().join("copies-Post (Copy B).stl")).unwrap())
        .unwrap();
    near(stl.volume_mm3(), 1000., 0.1);
    near(stl.bounds_mm().unwrap()[0][1], 60., 1e-4);

    // The world Part and a copy share the mesh; the copy keeps its name.
    let r = request(vec![f.post, a], AidPolicy::IncludeAll, false);
    let p = preview(&f.s, &r);
    assert_eq!(
        p.objects
            .iter()
            .map(|o| o.name.as_str())
            .collect::<Vec<_>>(),
        ["Post", "Post (Copy A)"]
    );
    let path = dir.path().join("mixed.3mf");
    write(&f.s, &r, &p.key, &path).unwrap();
    let read = mesh::read_three_mf(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!((read.objects, read.items), (1, 2));
    near(read.volume_mm3(), 2000., 1e-3);

    // Copies with the same name are numbered so each one is told apart.
    let c = copy(&mut f.s, f.post, f.post_feature, "Copy A", 90.);
    let p = preview(&f.s, &request(vec![a, c], AidPolicy::IncludeAll, false));
    assert_eq!(
        p.objects
            .iter()
            .map(|o| o.name.as_str())
            .collect::<Vec<_>>(),
        ["Post (Copy A) #1", "Post (Copy A) #2"]
    );
}

#[test]
fn a_placed_copy_brings_its_aids_at_the_copy_placement() {
    let mut f = fixture();
    let copied = copy(&mut f.s, f.block, f.block_feature, "Lifted copy", 50.);
    let dir = tempfile::tempdir().unwrap();
    let r = request(vec![copied], AidPolicy::IncludeAll, false);
    let p = preview(&f.s, &r);
    assert_eq!(
        p.objects
            .iter()
            .map(|o| (o.part, o.occurrence_path.clone()))
            .collect::<Vec<_>>(),
        [(f.block, vec![copied]), (f.support, vec![copied])]
    );
    assert!(matches!(p.objects[1].role, PrintRole::Aid { target, .. } if target == f.block));
    assert_eq!(p.objects[1].name, "Support (Lifted copy)");
    // The support moves with the copy: 5 mm into the block, 50 mm along y.
    near(p.objects[1].bounds_mm[0][1], 55., 1e-6);
    near(p.objects[1].bounds_mm[1][2], 14.9, 1e-6);
    near(p.objects[1].volume_mm3, 1490., 1e-6);
    near(p.objects[0].bounds_mm[0][2], 15., 1e-6);
    assert_eq!(p.selection.processes, [f.process]);
    let path = dir.path().join("copy.3mf");
    write(&f.s, &r, &p.key, &path).unwrap();
    let read = mesh::read_three_mf(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!((read.objects, read.items), (2, 2));
    let triangles: Vec<_> = p.objects.iter().map(|o| o.triangles).collect();
    let volumes = placed_volumes(&read, &triangles);
    near(volumes[0], 8000., 1e-3);
    near(volumes[1], 1490., 1e-3);

    // Leaving aids out lists the support of this copy; the copy stays.
    let without = preview(
        &f.s,
        &request(vec![copied], AidPolicy::ExcludeRemovable, false),
    );
    assert_eq!(without.objects.len(), 1);
    assert!(
        without
            .selection
            .skipped
            .iter()
            .any(|s| s.entity == f.support && s.name == "Support (Lifted copy)")
    );

    // A source edit makes the copy's preview stale; nothing is written.
    let mut lift = f.s.document().get(f.lift).unwrap().clone();
    let EntityData::Parameter { expression, .. } = &mut lift.data else {
        unreachable!()
    };
    *expression = "20".into();
    f.s.apply(Request {
        expected_revision: f.s.revision(),
        actor: actor(),
        label: "Raise the block".into(),
        edits: vec![Mutation::Replace(lift)],
    })
    .unwrap();
    let stale_path = dir.path().join("stale.3mf");
    let stale = write(&f.s, &r, &p.key, &stale_path).unwrap_err();
    assert_eq!(stale.kind, ErrorKind::Stale);
    assert!(!stale_path.exists());
}

#[test]
fn a_tag_packages_the_world_part_and_every_placed_copy() {
    let mut f = fixture();
    let a = copy(&mut f.s, f.post, f.post_feature, "Copy A", 30.);
    let b = copy(&mut f.s, f.post, f.post_feature, "Copy B", 60.);
    let p = prepare_tags(
        &f.s,
        f.s.revision(),
        actor(),
        &[f.post],
        &TagEdit::Add(BTreeSet::from([DEFAULT_PRINT_TAG.to_string()])),
    )
    .unwrap();
    f.s.commit(p).unwrap();
    let tag = PrintRequest {
        source: PrintSource::Tag {
            tag: DEFAULT_PRINT_TAG.into(),
        },
        ..request(vec![], AidPolicy::IncludeAll, false)
    };
    let p = preview(&f.s, &tag);
    let mut paths: Vec<_> = p
        .objects
        .iter()
        .map(|o| (o.part, o.occurrence_path.clone()))
        .collect();
    paths.sort();
    let mut expected = vec![(f.post, vec![]), (f.post, vec![a]), (f.post, vec![b])];
    expected.sort();
    assert_eq!(paths, expected);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("tagged.3mf");
    write(&f.s, &tag, &p.key, &path).unwrap();
    let read = mesh::read_three_mf(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!((read.objects, read.items), (1, 3));
    near(read.volume_mm3(), 3000., 1e-3);
}

#[test]
fn placed_assembly_occurrences_are_packaged_at_their_nested_world_placements() {
    let mut f = fixture();
    let commit = |s: &mut Session, p: model::AssemblyPrepared| s.commit(p.prepared).unwrap();
    // A reusable "Rack" holds the post 30 mm along y; it is placed twice.
    let mut member = model::ComponentPlacementRequest::new(
        f.post,
        model::NumericTransform {
            translation: [length(0.), length(30.), length(0.)],
            ..Default::default()
        },
    );
    member.name = "Slot".into();
    let p = model::prepare_place_component(&f.s, f.s.revision(), actor(), &member).unwrap();
    commit(&mut f.s, p);
    let rack = model::AssemblyRequest::new("Rack", vec![member.instance], vec![f.post]);
    let p = model::prepare_assembly(&f.s, f.s.revision(), actor(), &rack).unwrap();
    commit(&mut f.s, p);
    let mut roots = vec![];
    for (name, dx) in [("Left rack", 100.), ("Right rack", 200.)] {
        let mut root = model::ComponentPlacementRequest::new(
            rack.assembly,
            model::NumericTransform {
                translation: [length(dx), length(0.), length(0.)],
                ..Default::default()
            },
        );
        root.name = name.into();
        let p = model::prepare_place_component(&f.s, f.s.revision(), actor(), &root).unwrap();
        commit(&mut f.s, p);
        roots.push(root.instance);
    }
    // The member and the definition Part are not world occurrences: they are
    // listed as not packaged, never substituted.
    let r = request(
        vec![roots[0], roots[1], member.instance, f.post],
        AidPolicy::IncludeAll,
        false,
    );
    let p = preview(&f.s, &r);
    assert!(p.can_write(), "{:?}", p.blocked);
    assert_eq!(
        p.objects
            .iter()
            .map(|o| (o.part, o.occurrence_path.clone(), o.name.as_str()))
            .collect::<Vec<_>>(),
        [
            (
                f.post,
                vec![roots[0], member.instance],
                "Post (Left rack / Slot)"
            ),
            (
                f.post,
                vec![roots[1], member.instance],
                "Post (Right rack / Slot)"
            ),
        ]
    );
    for (object, x) in p.objects.iter().zip([160., 260.]) {
        near(object.bounds_mm[0][0], x, 1e-6);
        near(object.bounds_mm[0][1], 30., 1e-6);
        near(object.volume_mm3, 1000., 1e-6);
    }
    let skipped: BTreeSet<_> = p.selection.skipped.iter().map(|s| s.entity).collect();
    assert_eq!(skipped, BTreeSet::from([member.instance, f.post]));
    // The Print source picker offers exactly the placed racks and world
    // Parts, never the reusable definition, its member or its Part.
    let candidates = print_candidates(f.s.document()).unwrap();
    let offered: BTreeSet<_> = candidates.iter().map(|c| c.entity).collect();
    assert!(offered.contains(&roots[0]) && offered.contains(&roots[1]));
    for hidden in [member.instance, f.post, rack.assembly] {
        assert!(!offered.contains(&hidden));
    }
    let left = candidates.iter().find(|c| c.entity == roots[0]).unwrap();
    assert_eq!(left.kind, PrintCandidateKind::PlacedCopy);
    assert_eq!(left.source.as_deref(), Some("Rack"));
    let all = preview(
        &f.s,
        &request(offered.into_iter().collect(), AidPolicy::IncludeAll, false),
    );
    assert!(
        all.selection.skipped.is_empty(),
        "{:?}",
        all.selection.skipped
    );
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("racks.3mf");
    write(&f.s, &r, &p.key, &path).unwrap();
    let read = mesh::read_three_mf(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!((read.objects, read.items), (1, 2));
    let bounds = read.bounds_mm().unwrap();
    near(bounds[0][0], 160., 1e-6);
    near(bounds[1][0], 270., 1e-6);
    near(read.volume_mm3(), 2000., 1e-3);
}

#[test]
fn arranged_plate_reopens_as_shared_3mf_items_and_stl_in_bed_coordinates() {
    let mut f = fixture();
    let placed = copy(&mut f.s, f.block, f.block_feature, "World copy", 123.);
    let selection = |root: EntityId| {
        resolve_occurrences(f.s.document(), root).unwrap()[0]
            .reference
            .clone()
    };
    let plate = PrintPlateDefinition {
        bed_mm: [90., 75.],
        placements: vec![
            PrintPlatePlacement {
                source: selection(f.block),
                position_mm: [8., 10.],
                rotation_deg: [90., 0., 90.],
            },
            PrintPlatePlacement {
                source: selection(placed),
                position_mm: [42., 12.],
                rotation_deg: [0., 0., 90.],
            },
        ],
        stl_per_part: true,
        ..Default::default()
    };
    let plate_id = EntityId::new();
    let saved = prepare_print_plate(
        &f.s,
        f.s.revision(),
        actor(),
        plate_id,
        "Assembly parts",
        &plate,
    )
    .unwrap();
    f.s.commit(saved).unwrap();
    let document = f.s.document();
    let revision = f.s.revision();
    let plate = read_print_plate(document, plate_id).unwrap();
    let exporter = Exporter::default();
    let preview_plate = |plate: &PrintPlateDefinition| {
        exporter
            .print_plate_preview(document, revision, plate, &actor(), None)
            .unwrap()
    };
    let before = ExportStamp::capture(document, revision).unwrap();
    let ready = preview_plate(&plate);
    assert!(ready.can_write(), "{:?}", ready.blocked);
    assert_eq!(ready.request, plate.summary_request(document));
    assert_eq!((ready.files[0].objects, ready.files[0].items), (1, 2));
    assert_eq!(ready.objects[1].occurrence_path, [placed]);
    let expected = [
        [[8., 10., 0.], [18., 50., 20.]],
        [[42., 12., 0.], [62., 52., 10.]],
    ];
    let verify_bounds = |actual: [[f64; 3]; 2], expected: [[f64; 3]; 2]| {
        for (actual, expected) in actual
            .into_iter()
            .flatten()
            .zip(expected.into_iter().flatten())
        {
            near(actual, expected, 1e-6);
        }
    };
    for (index, object) in ready.objects.iter().enumerate() {
        verify_bounds(object.bounds_mm, expected[index]);
        assert_eq!(
            object.plate_pose.unwrap().rotation,
            plate.placements[index].rotation()
        );
        near(object.volume_mm3, 8000., 1e-6);
    }

    let directory = tempfile::tempdir().unwrap();
    let target = directory.path().join("plate.3mf");
    let mut resized = plate.clone();
    resized.bed_mm[0] += 10.;
    let larger_bed = preview_plate(&resized);
    assert!(larger_bed.can_write());
    assert_ne!(ready.key, larger_bed.key, "bed dimensions bind the plan");
    let write = |plate: &PrintPlateDefinition, key: &PrintKey| {
        exporter.write_print_plate(document, revision, plate, key, &target, &actor(), None)
    };
    assert_eq!(
        write(&resized, &ready.key).unwrap_err().kind,
        ErrorKind::Stale
    );
    assert!(listing(directory.path()).is_empty());

    let mut outside = plate.clone();
    outside.placements[1].position_mm[0] = 80.;
    let blocked = preview_plate(&outside);
    let message = blocked.blocked.as_ref().unwrap();
    assert!(
        message.contains("World copy") && message.contains("outside"),
        "{message}"
    );
    assert!(blocked.files.is_empty());
    assert!(write(&outside, &blocked.key).is_err());
    assert!(listing(directory.path()).is_empty());

    let mut overlapping = plate.clone();
    overlapping.placements[1].position_mm = [9., 15.];
    let blocked = preview_plate(&overlapping);
    assert!(blocked.blocked.as_ref().unwrap().contains("overlaps"));
    assert!(blocked.files.is_empty());

    let receipt = write(&plate, &ready.key).unwrap();
    assert_eq!(receipt.files.len(), 3);
    let reopened = mesh::read_three_mf(&std::fs::read(&target).unwrap()).unwrap();
    assert_eq!((reopened.objects, reopened.items), (1, 2));
    assert_eq!(reopened.unit_scale, 1.);
    let mut faces = reopened.world.as_slice();
    for (index, object) in ready.objects.iter().enumerate() {
        let (world, tail) = faces.split_at(object.triangles);
        faces = tail;
        let actual = mesh::ReadMesh {
            world: world.to_vec(),
            unit_scale: 1.,
            objects: 1,
            items: 1,
            triangles: world.len(),
        };
        verify_bounds(actual.bounds_mm().unwrap(), expected[index]);
        near(actual.volume_mm3(), 8000., 1e-6);
        let stl = &receipt.files[index + 1];
        assert_eq!(stl.kind, PrintFileKind::Stl);
        let read = mesh::read_stl(&std::fs::read(&stl.path).unwrap()).unwrap();
        verify_bounds(read.bounds_mm().unwrap(), expected[index]);
        near(read.volume_mm3(), 8000., 1e-6);
    }
    assert!(faces.is_empty());
    assert_eq!(ExportStamp::capture(document, revision).unwrap(), before);

    // A direct-Part-only preflight can evaluate a subset of native geometry.
    // Its plan must still match the independently rebuilt full write.
    let mut direct = plate.clone();
    direct.placements.truncate(1);
    let direct_preview = preview_plate(&direct);
    assert!(direct_preview.can_write(), "{:?}", direct_preview.blocked);
    let direct_target = directory.path().join("direct.3mf");
    let receipt = exporter
        .write_print_plate(
            document,
            revision,
            &direct,
            &direct_preview.key,
            &direct_target,
            &actor(),
            None,
        )
        .unwrap();
    assert_eq!(receipt.key, direct_preview.key);
    let reopened = mesh::read_three_mf(&std::fs::read(&direct_target).unwrap()).unwrap();
    assert_eq!((reopened.objects, reopened.items), (1, 1));
    verify_bounds(reopened.bounds_mm().unwrap(), expected[0]);
}

#[test]
fn rotated_cylinders_use_exact_bounds_and_overlap_instead_of_bounding_boxes() {
    let mut s = Session::new(Document::new("Cylinder plate").unwrap()).unwrap();
    let cylinder = PrimitiveRequest::new(
        "Cylinder",
        PrimitiveParameters::Cylinder {
            radius: length(5.),
            height: length(10.),
            frame: Default::default(),
        },
    );
    primitive(&mut s, &cylinder);
    let placed = copy(&mut s, cylinder.body, cylinder.feature, "World copy", 50.);
    let placement = |root, position_mm| PrintPlatePlacement {
        source: resolve_occurrences(s.document(), root).unwrap()[0]
            .reference
            .clone(),
        position_mm,
        rotation_deg: [0., 0., 45.],
    };
    let mut plate = PrintPlateDefinition {
        bed_mm: [18., 18.],
        placements: vec![
            placement(cylinder.body, [0., 0.]),
            placement(placed, [8., 8.]),
        ],
        ..Default::default()
    };
    let preview = |plate: &PrintPlateDefinition| {
        Exporter::default()
            .print_plate_preview(s.document(), s.revision(), plate, &actor(), None)
            .unwrap()
    };
    let separated = preview(&plate);
    assert!(separated.can_write(), "{:?}", separated.blocked);
    near(separated.objects[0].bounds_mm[1][0], 10., 1e-6);
    near(separated.objects[0].bounds_mm[1][1], 10., 1e-6);
    near(separated.objects[1].bounds_mm[1][0], 18., 1e-6);
    plate.placements[1].position_mm = [7., 7.];
    assert!(preview(&plate).blocked.unwrap().contains("overlaps"));
}
