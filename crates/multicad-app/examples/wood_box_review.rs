//! CAD-249 native review fixture; no application window is created.
use multicad_core::{
    commands::{Actor, Channel, Session},
    document::{Document, Entity, EntityData, EntityId, Mutation},
    materials::{Grain, LibraryDefinition, Stock, StockKind},
    model::*,
    project_file::{self, ProjectFile},
};
use multicad_model::Compiler;
use std::path::PathBuf;
fn fixture() -> (Session, [EntityId; 2]) {
    let mut edits = vec![];
    let mut stocks = vec![];
    for (name, thickness) in [("Birch 18", 18.), ("Birch 24", 24.)] {
        let material = Entity::new(
            name,
            EntityData::Material {
                density_kg_m3: Some(650.),
                color_rgba: [210, 180, 140, 255],
                section_hatch: None,
            },
        );
        let stock = LibraryDefinition::Stock(Stock {
            material: material.id,
            specification: StockKind::Panel {
                thickness_mm: thickness,
                length_mm: 2500.,
                width_mm: 1250.,
                grain: Grain::None,
            },
        })
        .entity(EntityId::new(), name)
        .unwrap();
        stocks.push(stock.id);
        edits.extend([Mutation::Insert(material), Mutation::Insert(stock)]);
    }
    (
        Session::new(
            Document::new("Wood Box source")
                .unwrap()
                .edited(&edits)
                .unwrap(),
        )
        .unwrap(),
        stocks.try_into().unwrap(),
    )
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let output = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| "artifacts/wood-box-review-20261102".into());
    std::fs::create_dir_all(&output)?;
    let (mut session, stocks) = fixture();
    let mut source = WoodBoxRequest::new(
        "Open-top drawer box",
        WoodBoxDefinition::new(stocks[0], length(600.), length(400.), length(300.)),
    );
    source.definition.faces.remove(&WoodBoxFace::Top);
    let prepared = prepare_wood_box(
        &session,
        session.revision(),
        Actor::local(Channel::Ui),
        &WoodBoxChange::Set(Box::new(source.clone())),
    )?;
    Compiler::default().compile(prepared.model.prepared.document(), session.revision() + 1)?;
    session.commit(prepared.model.prepared)?;
    let bottom = source.definition.panels[&WoodBoxFace::Bottom].body;
    let pocket = MachiningDefinition {
        generated_by: None,
        kind: MachiningKind::Pocket {
            center: [length(100.), length(100.)],
            size: [length(40.), length(30.)],
            depth: length(5.),
        },
        face: MachiningFace::Inside,
        targets: vec![bottom],
        suppressed: false,
    };
    let prepared = prepare_machining(
        &session,
        session.revision(),
        Actor::local(Channel::Ui),
        "Bottom pocket",
        &pocket,
    )?;
    Compiler::default().compile(prepared.model.prepared.document(), session.revision() + 1)?;
    session.commit(prepared.model.prepared)?;
    let native = Compiler::default().compile(session.document(), session.revision())?;
    let layout = resolve_wood_box(session.document(), session.parameters(), &source.definition)?;
    let mut proofs = vec![];
    for panel in &layout.panels {
        let ids = &source.definition.panels[&panel.face];
        let solid = native.solid(ids.body).ok_or("Missing native panel")?;
        let expected = panel.panel.volume_mm3()
            - if ids.body == bottom {
                40. * 30. * 5.
            } else {
                0.
            };
        assert!((solid.volume_mm3()? - expected).abs() < 1e-3);
        let path = output.join(format!("{}.step", panel.face.label().to_ascii_lowercase()));
        solid.write_step(&path)?;
        let readback = multicad_kernel::Solid::read_step(&path, Default::default())?;
        assert!((readback.volume_mm3()? - expected).abs() < 1e-3);
        proofs.push(serde_json::json!({"face":panel.face,"part":ids.body,"feature":ids.feature,"volume_mm3":expected,"step":path}));
    }
    let path = output.join("wood-box.tcd");
    project_file::write(
        &ProjectFile::new(session.document().clone())?,
        &mut std::fs::File::create(&path)?,
        Default::default(),
    )?;
    let reopened = project_file::read(
        &mut std::fs::File::open(&path)?,
        Default::default(),
        &project_file::supported_capabilities(),
    )?
    .project;
    let reopened = Compiler::default().compile(&reopened.document, 0)?;
    assert_eq!(reopened.snapshot().bodies.len(), 5);
    let report = serde_json::json!({"project":path,"recipe":source.id,"definition":source.definition,"outside_mm":layout.outside_mm,"usable_inner_mm":layout.clear_mm,"actual_world_bounds_mm":layout.actual_bounds,"native_panels":proofs,"evidence":"Exact native source, downstream pocket, STEP readback and TCD reopen. No OS-window interaction."});
    std::fs::write(
        output.join("native-report.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    println!("{}", path.display());
    Ok(())
}
