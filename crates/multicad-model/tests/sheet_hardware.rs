//! Attached sheet hardware: mounting patterns cut receiving holes in the
//! sheet and produce separate purchased-hardware Parts placed on the
//! evaluated faces. Moving points, replacing presets and removing items
//! update holes, hardware and the developed pattern; stack and catalogue
//! thickness incompatibilities are reported. Measurements use exact solids.
use multicad_core::{
    commands::{Actor, Channel, Session},
    document::{Document, Entity, EntityData, EntityId},
    materials::*,
    model::{
        self, BendDirection, BodyOutput, CutoutFace, Flange, FlangeKind, HardwareItem,
        HardwarePreset, HardwareSide, MountingPattern, PanelPlane, SheetBase, SheetDefinition,
        SheetRequest, Thread, angle, length,
    },
    project_file::{self, ProjectFile},
};
use multicad_model::{Compiler, HardwareRecord, Model};
use std::f64::consts::PI;

fn actor() -> Actor {
    Actor::local(Channel::Ui)
}
fn near(a: f64, b: f64) {
    assert!((a - b).abs() <= 1e-6 * b.abs().max(1.), "{a} != {b}");
}
fn compile(s: &Session) -> Model {
    Compiler::default()
        .compile(s.document(), s.revision())
        .unwrap()
}
fn commit(s: &mut Session, prepared: multicad_core::commands::Prepared) {
    Compiler::default()
        .compile(prepared.document(), s.revision() + 1)
        .unwrap();
    s.commit(prepared).unwrap();
}
fn record(list: &[HardwareRecord], item: HardwareItem) -> &HardwareRecord {
    list.iter()
        .find(|r| r.item == item)
        .unwrap_or_else(|| panic!("no {item:?} record"))
}
fn screws(points: &[[f64; 2]], thread: Thread, len: f64, nut: bool) -> MountingPattern {
    MountingPattern {
        preset: HardwarePreset::Screw {
            thread,
            length: length(len),
            washers: true,
            nut,
        },
        face: CutoutFace::Base,
        side: HardwareSide::Material,
        points: points.iter().map(|p| p.map(length)).collect(),
    }
}
fn insert() -> MountingPattern {
    MountingPattern {
        preset: HardwarePreset::Insert {
            designation: "Self-clinching nut M3 (example entry)".into(),
            source: "User-entered catalogue values".into(),
            hole_diameter: length(4.2),
            body_diameter: length(6.),
            height: length(2.),
            min_sheet: length(0.8),
            max_sheet: length(1.2),
        },
        face: CutoutFace::Flange { index: 0 },
        side: HardwareSide::Material,
        points: vec![[length(50.), length(10.)]],
    }
}
fn bounds(m: &Model, part: EntityId) -> multicad_kernel::Bounds {
    m.solid(m.snapshot().bodies[&part])
        .unwrap()
        .bounds()
        .unwrap()
}

#[test]
fn mounting_patterns_cut_holes_and_place_separate_hardware_parts() {
    let mut s = Session::new(Document::new("Hardware").unwrap()).unwrap();
    let steel = Entity::new(
        "DC01",
        EntityData::Material {
            density_kg_m3: Some(7850.),
            color_rgba: [180, 184, 190, 255],
            section_hatch: None,
        },
    );
    let stock = LibraryDefinition::Stock(Stock {
        material: steel.id,
        specification: StockKind::Sheet {
            thickness_mm: 1.5,
            length_mm: 2000.,
            width_mm: 1000.,
            inside_bend_radius_mm: 1.5,
            k_factor: 0.42,
        },
    })
    .entity(EntityId::new(), "DC01 1.5")
    .unwrap();
    let stock_id = stock.id;
    s.commit(prepare_library(&s, 0, actor(), &[steel, stock]).unwrap())
        .unwrap();
    let t = 1.5;
    let request = SheetRequest::new(
        "Panel",
        SheetDefinition {
            output: BodyOutput {
                body: EntityId::new(),
            },
            stock: stock_id,
            base: SheetBase::Rectangle {
                plane: PanelPlane::Xy,
                origin: [length(0.), length(0.), length(0.)],
                length: length(100.),
                width: length(60.),
            },
            reverse: false,
            inside_radius: None,
            k_factor: None,
            flanges: vec![Flange {
                edge: 0,
                kind: FlangeKind::Bend { angle: angle(90.) },
                direction: BendDirection::Up,
                length: length(20.),
                inside_radius: None,
                start_inset: None,
                end_inset: None,
                relief: None,
            }],
            cutouts: vec![],
            fasteners: vec![],
        },
    );
    let p = model::prepare_sheet(&s, s.revision(), actor(), &request).unwrap();
    commit(&mut s, p.prepared);
    let plain = compile(&s).sheet_metal(s.document()).unwrap()[0].volume_mm3;

    // Two M3 screws with washers and nuts on the base, one insert on the flange.
    let patterns = vec![
        screws(&[[20., 30.], [80., 30.]], Thread::M3, 8., true),
        insert(),
    ];
    let before = s.document().clone();
    let p = model::prepare_sheet_fasteners(&s, s.revision(), actor(), request.feature, &patterns)
        .unwrap();
    assert_eq!(p.hardware.len(), 4);
    let ids: Vec<_> = p.hardware.iter().map(|(_, _, o)| o.body).collect();
    commit(&mut s, p.model.prepared);
    let m = compile(&s);
    let sheet = &m.sheet_metal(s.document()).unwrap()[0];
    let holes = 2. * PI * 1.7f64.powi(2) * t + PI * 2.1f64.powi(2) * t;
    near(sheet.volume_mm3, plain - holes);
    let list = m.sheet_hardware(s.document()).unwrap();
    assert_eq!(list.len(), 4);
    let screw = record(&list, HardwareItem::Screw);
    assert_eq!(screw.designation, "ISO 4762 M3 × 8");
    assert_eq!(screw.source, "ISO 4762 nominal dimensions");
    assert_eq!(screw.quantity, 2);
    near(screw.hole_mm, 3.4);
    assert!(screw.findings.is_empty(), "{:?}", screw.findings);
    assert_eq!(screw.sheet_part, request.body);
    assert_eq!(record(&list, HardwareItem::Washer).quantity, 4);
    assert_eq!(record(&list, HardwareItem::Nut).quantity, 2);
    let nut = record(&list, HardwareItem::Nut);
    assert!(nut.findings.is_empty(), "{:?}", nut.findings);
    let ins = record(&list, HardwareItem::Insert);
    assert_eq!(ins.designation, "Self-clinching nut M3 (example entry)");
    assert_eq!(ins.source, "User-entered catalogue values");
    assert!(
        ins.findings
            .iter()
            .any(|f| f.contains("outside the insert's 0.8–1.2 mm")),
        "{:?}",
        ins.findings
    );
    // Hardware Parts are separate from the sheet and from each other.
    assert!(ids.iter().all(|id| *id != request.body));
    assert!(ids.iter().all(|id| {
        matches!(
            s.document().get(*id).map(|e| &e.data),
            Some(EntityData::Part { .. })
        )
    }));
    // The screws sit on the material side: head over a washer, shank down.
    let b = bounds(&m, screw.part);
    near(b.min[0], 20. - 2.75);
    near(b.max[0], 80. + 2.75);
    near(b.max[2], t + 0.5 + 3.);
    near(b.min[2], t + 0.5 - 8.);
    // The nuts hang under the base below their washers.
    let b = bounds(&m, nut.part);
    near(b.max[2], -0.5);
    near(b.min[2], -0.5 - 2.4);
    // The developed pattern drills every receiving hole by its pattern id.
    let flat = m.unfold(s.document(), request.body).unwrap();
    let drills: Vec<_> = flat.drills.iter().map(|d| d.id.as_str()).collect();
    assert_eq!(drills, ["fastener.0.0", "fastener.0.1", "fastener.1.0"]);
    near(flat.drills[0].center[0], 20.);
    near(flat.drills[0].diameter_mm, 3.4);

    // Moving the points moves the holes and the hardware together.
    let mut moved = patterns.clone();
    moved[0] = screws(&[[30., 30.], [70., 30.]], Thread::M3, 8., true);
    let p =
        model::prepare_sheet_fasteners(&s, s.revision(), actor(), request.feature, &moved).unwrap();
    assert_eq!(
        p.hardware
            .iter()
            .map(|(_, _, o)| o.body)
            .collect::<Vec<_>>(),
        ids
    );
    commit(&mut s, p.model.prepared);
    let m = compile(&s);
    near(bounds(&m, screw.part).min[0], 30. - 2.75);
    near(
        m.unfold(s.document(), request.body).unwrap().drills[0].center[0],
        30.,
    );

    // An M4 × 6 replacement widens the holes and is too short for its stack.
    let mut bigger = moved.clone();
    bigger[0] = screws(&[[30., 30.], [70., 30.]], Thread::M4, 6., true);
    let p = model::prepare_sheet_fasteners(&s, s.revision(), actor(), request.feature, &bigger)
        .unwrap();
    commit(&mut s, p.model.prepared);
    let m = compile(&s);
    near(
        m.sheet_metal(s.document()).unwrap()[0].volume_mm3,
        plain - 2. * PI * 2.25f64.powi(2) * t - PI * 2.1f64.powi(2) * t,
    );
    let list = m.sheet_hardware(s.document()).unwrap();
    let screw4 = record(&list, HardwareItem::Screw);
    assert_eq!(screw4.part, screw.part);
    assert_eq!(screw4.designation, "ISO 4762 M4 × 6");
    assert!(
        screw4
            .findings
            .iter()
            .any(|f| f.contains("shorter than the 6.300 mm stack")),
        "{:?}",
        screw4.findings
    );

    // Dropping the nut removes its Part; one washer per point remains.
    let mut no_nut = bigger.clone();
    no_nut[0] = screws(&[[30., 30.], [70., 30.]], Thread::M4, 6., false);
    let with_nut = s.document().clone();
    let p = model::prepare_sheet_fasteners(&s, s.revision(), actor(), request.feature, &no_nut)
        .unwrap();
    commit(&mut s, p.model.prepared);
    assert!(s.document().get(nut.part).is_none());
    let list = compile(&s).sheet_hardware(s.document()).unwrap();
    assert_eq!(list.len(), 3);
    assert_eq!(record(&list, HardwareItem::Washer).quantity, 2);
    s.undo(s.revision(), actor()).unwrap();
    assert_eq!(s.document(), &with_nut);

    // A pattern on a missing flange is refused before any change.
    let mut wrong = bigger.clone();
    wrong[1].face = CutoutFace::Flange { index: 4 };
    assert!(
        model::prepare_sheet_fasteners(&s, s.revision(), actor(), request.feature, &wrong).is_err()
    );

    // Save and reopen keep the patterns and identical hardware records.
    let records = compile(&s).sheet_hardware(s.document()).unwrap();
    let mut bytes = vec![];
    project_file::write(
        &ProjectFile::new(s.document().clone()).unwrap(),
        &mut bytes,
        Default::default(),
    )
    .unwrap();
    let opened = project_file::read(
        &mut bytes.as_slice(),
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap()
    .project;
    assert!(opened.capabilities().contains(model::HARDWARE_CAPABILITY));
    let reopened = Compiler::default().compile(&opened.document, 0).unwrap();
    assert_eq!(reopened.sheet_hardware(&opened.document).unwrap(), records);

    // Removing every pattern removes every hardware Part again.
    let p =
        model::prepare_sheet_fasteners(&s, s.revision(), actor(), request.feature, &[]).unwrap();
    commit(&mut s, p.model.prepared);
    assert!(compile(&s).sheet_hardware(s.document()).unwrap().is_empty());
    assert!(ids.iter().all(|id| s.document().get(*id).is_none()));
    // Four committed fastener changes since the plain sheet.
    for _ in 0..4 {
        s.undo(s.revision(), actor()).unwrap();
    }
    assert_eq!(s.document(), &before);
}

#[test]
fn rust_flat_sheet_builds_nominal_purchased_pieces_and_the_same_hardware_records() {
    use multicad_model::GeometryBackend;
    let mut session = Session::new(Document::new("Flat hardware").unwrap()).unwrap();
    let steel = Entity::new(
        "Steel",
        EntityData::Material {
            density_kg_m3: Some(7850.),
            color_rgba: [180, 184, 190, 255],
            section_hatch: None,
        },
    );
    let stock = LibraryDefinition::Stock(Stock {
        material: steel.id,
        specification: StockKind::Sheet {
            thickness_mm: 1.5,
            length_mm: 2000.,
            width_mm: 1000.,
            inside_bend_radius_mm: 1.5,
            k_factor: 0.42,
        },
    })
    .entity(EntityId::new(), "Steel 1.5")
    .unwrap();
    let stock_id = stock.id;
    session
        .commit(prepare_library(&session, 0, actor(), &[steel, stock]).unwrap())
        .unwrap();
    let sheet = SheetRequest::new(
        "Plate",
        SheetDefinition {
            output: BodyOutput {
                body: EntityId::new(),
            },
            stock: stock_id,
            base: SheetBase::Rectangle {
                plane: PanelPlane::Xz,
                origin: [1., 10., 5.].map(length),
                length: length(100.),
                width: length(60.),
            },
            reverse: true,
            inside_radius: None,
            k_factor: None,
            flanges: vec![],
            cutouts: vec![],
            fasteners: vec![],
        },
    );
    let prepared = model::prepare_sheet(&session, session.revision(), actor(), &sheet).unwrap();
    session.commit(prepared.prepared).unwrap();
    let patterns = [
        screws(&[[15., 12.], [85., 48.]], Thread::M3, 8., true),
        MountingPattern {
            preset: HardwarePreset::Standoff {
                thread: Thread::M4,
                length: length(6.),
                across_flats: length(7.),
            },
            face: CutoutFace::Base,
            side: HardwareSide::Base,
            points: vec![[20., 48.].map(length), [80., 12.].map(length)],
        },
        MountingPattern {
            face: CutoutFace::Base,
            side: HardwareSide::Base,
            points: vec![[50., 30.].map(length)],
            ..insert()
        },
    ];
    let prepared = model::prepare_sheet_fasteners(
        &session,
        session.revision(),
        actor(),
        sheet.feature,
        &patterns,
    )
    .unwrap();
    assert_eq!(prepared.hardware.len(), 5);
    session.commit(prepared.model.prepared).unwrap();
    let compiler = Compiler {
        omit_display: true,
        ..Default::default()
    };
    let rust = compiler
        .compile_with_backend(
            session.document(),
            session.revision(),
            GeometryBackend::Rust,
            None,
        )
        .unwrap();
    let native = compiler
        .compile(session.document(), session.revision())
        .unwrap();
    let rust_records = rust.sheet_hardware(session.document()).unwrap();
    assert_eq!(
        rust_records,
        native.sheet_hardware(session.document()).unwrap()
    );
    let scratch = tempfile::tempdir().unwrap();
    for (item, count) in [
        (HardwareItem::Screw, 2),
        (HardwareItem::Washer, 4),
        (HardwareItem::Nut, 2),
        (HardwareItem::Standoff, 2),
        (HardwareItem::Insert, 1),
    ] {
        let hardware = record(&rust_records, item);
        assert_eq!(hardware.quantity, count);
        let [r, n] = [&rust, &native].map(|model| {
            model
                .solid(model.snapshot().bodies[&hardware.part])
                .unwrap()
        });
        assert!(r.uses_rust_geometry());
        near(r.volume_mm3().unwrap(), n.volume_mm3().unwrap());
        assert_eq!(r.faces().unwrap().len(), n.faces().unwrap().len());
        assert!(!r.mesh(Default::default()).unwrap().indices.is_empty());
        let manufacturing = r.manufacturing_mesh_with_receipt(Default::default());
        if matches!(item, HardwareItem::Nut | HardwareItem::Standoff) {
            assert_eq!(
                manufacturing.unwrap_err().kind,
                multicad_kernel::GeometryErrorKind::Unsupported
            );
        } else {
            let receipt = manufacturing.unwrap_or_else(|e| panic!("{item:?}: {e}"));
            assert!(!receipt.mesh.indices.is_empty());
        }
        assert_eq!(
            r.supports_step_export(),
            matches!(
                item,
                HardwareItem::Screw | HardwareItem::Washer | HardwareItem::Insert
            )
        );
        if item == HardwareItem::Screw {
            let path = scratch.path().join("sheet-screws.step");
            r.write_step(&path).unwrap();
            let readback = multicad_kernel::Solid::read_step(&path, compiler.policy).unwrap();
            assert_eq!(readback.solid_count().unwrap() as usize, count);
            near(readback.volume_mm3().unwrap(), n.volume_mm3().unwrap());
        }
        let [a, b] = [r.bounds().unwrap(), n.bounds().unwrap()];
        for (x, y) in a
            .min
            .into_iter()
            .chain(a.max)
            .zip(b.min.into_iter().chain(b.max))
        {
            near(x, y);
        }
    }
}
