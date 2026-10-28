//! Reproducible native source and genuine DXF output for manual review.
#[path = "../src/files.rs"]
mod files;
use multicad_core::{
    commands::{Actor, Channel, Request, Session},
    document::{Document, Entity, EntityData, EntityId, Link, Mutation, Placement},
    exchange::*,
    model::{
        self, BodyInput, BooleanRequest, Operation, PrimitiveFrame, PrimitiveParameters,
        PrimitiveRequest, length,
    },
    project_file::ProjectFile,
};
use multicad_model::{Compiler, export::Exporter};
use multicad_ui::storage::{DocumentStorage, SaveMode};
use std::{fs::OpenOptions, io::Write, path::PathBuf};
fn actor() -> Actor {
    Actor::local(Channel::Ui)
}
fn primitive(
    s: &mut Session,
    name: &str,
    parameters: PrimitiveParameters,
) -> Result<PrimitiveRequest, Box<dyn std::error::Error>> {
    let r = PrimitiveRequest::new(name, parameters);
    let prepared = model::prepare_primitive(s, s.revision(), actor(), &r)?;
    s.commit(prepared.prepared)?;
    Ok(r)
}
fn save_set(
    s: &mut Session,
    name: &str,
    entities: Vec<EntityId>,
    units: LengthUnit,
    select: bool,
) -> Result<EntityId, Box<dyn std::error::Error>> {
    let scope = Scope {
        root: ScopeRoot::Explicit { entities },
        tags: None,
    };
    let recipe = Recipe {
        id: EntityId::new(),
        name: "Planar contour".into(),
        output: RecipeOutput::PlanarDxf {
            units,
            layers: Default::default(),
            aids: AidPolicy::IncludeAll,
        },
    };
    let catalog = Exporter::default().planar_catalog(s.document(), s.revision(), &scope, None)?;
    let mut definition = SetDefinition {
        scope,
        recipes: vec![recipe.clone()],
        ..Default::default()
    };
    let mut links = vec![];
    if select {
        for part in catalog.parts {
            let face = part
                .faces
                .into_iter()
                .find(|f| f.plane.normal()[2] > 0.999999)
                .ok_or("Fixture has no top face")?;
            definition.planar_faces.push(PlanarFace {
                recipe: recipe.id,
                subject: part.subject,
                source: face.source,
                reference: face.intent.id,
                horizontal: PlanarAxis::WorldX,
            });
            links.push(Link {
                target: face.source,
                role: PLANAR_FACE_ROLE.into(),
                geometry: Some(face.intent),
            });
        }
    }
    let id = EntityId::new();
    let p = prepare_set_with_geometry(s, s.revision(), actor(), id, name, &definition, &links)?;
    s.commit(p)?;
    Ok(id)
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let directory = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../artifacts/planar-dxf-review")
        });
    if directory.exists() {
        return Err("Review directory already exists; choose a new directory".into());
    }
    let mut s = Session::new(Document::new("Panel contour manufacturing review")?)?;
    let panel = primitive(
        &mut s,
        "Cabinet side",
        PrimitiveParameters::Box {
            size: [length(120.), length(80.), length(18.)],
            frame: Default::default(),
        },
    )?;
    let mut tools = vec![];
    for (x, y, r) in [(30., 20., 5.), (80., 50., 3.)] {
        let tool = primitive(
            &mut s,
            "Bore source",
            PrimitiveParameters::Cylinder {
                radius: length(r),
                height: length(20.),
                frame: PrimitiveFrame {
                    origin: [length(x), length(y), length(-1.)],
                    ..Default::default()
                },
            },
        )?;
        tools.push(BodyInput {
            body: tool.body,
            producer: tool.feature,
        });
    }
    let cut = BooleanRequest::new(
        Operation::Cut,
        BodyInput {
            body: panel.body,
            producer: panel.feature,
        },
        tools,
    );
    let p = model::prepare_boolean(&s, s.revision(), actor(), &cut)?;
    s.commit(p.prepared)?;
    let instance = Entity::new(
        "Rotated placed panel",
        EntityData::Instance {
            component: panel.body,
            placement: Placement {
                translation_mm: [200., 100., 10.],
                rotation_xyzw: [
                    0.,
                    0.,
                    std::f64::consts::FRAC_PI_4.sin(),
                    std::f64::consts::FRAC_PI_4.cos(),
                ],
            },
            placement_source: None,
        },
    );
    let placed = instance.id;
    s.apply(Request {
        expected_revision: s.revision(),
        actor: actor(),
        label: "Place review panel".into(),
        edits: vec![Mutation::Insert(instance)],
    })?;
    let panels = save_set(
        &mut s,
        "Planar panels",
        vec![panel.body, placed],
        LengthUnit::Millimeter,
        true,
    )?;
    let inch = save_set(
        &mut s,
        "Placed panel inches",
        vec![placed],
        LengthUnit::Inch,
        true,
    )?;
    let choose = save_set(
        &mut s,
        "Choose panel face",
        vec![panel.body],
        LengthUnit::Millimeter,
        false,
    )?;
    Compiler::default().compile(s.document(), s.revision())?;
    std::fs::create_dir(&directory)?;
    let directory = directory.canonicalize()?;
    let path = directory.join("panel-review.multicad");
    let mut store = files::ProjectStore::default();
    store
        .save_project(
            &path,
            &ProjectFile::new(s.document().clone())?,
            SaveMode::Create,
        )
        .map_err(|e| e.to_string())?;
    let reopened = store.open_project(&path).map_err(|e| e.to_string())?;
    if &reopened.file.document != s.document() {
        return Err("Saved review source changed on reopen".into());
    }
    Compiler::default().compile(&reopened.file.document, 0)?;
    let outputs = directory.join("outputs");
    std::fs::create_dir(&outputs)?;
    let exporter = Exporter::default();
    let mut reports = vec![];
    for set in [panels, inch] {
        let preview = exporter.preview(&reopened.file.document, 0, set, &actor(), None)?;
        if !preview.can_run() {
            return Err(format!("Review recipe not Ready: {:?}", preview.recipes).into());
        }
        let receipt = exporter.run(
            &reopened.file.document,
            0,
            &preview.key,
            &outputs,
            &actor(),
            None,
        )?;
        reports.push(serde_json::json!({"preview":preview,"receipt":receipt}));
    }
    let unselected = exporter.preview(&reopened.file.document, 0, choose, &actor(), None)?;
    if unselected.can_run() {
        return Err("An unselected planar face unexpectedly became exportable".into());
    }
    let mut report = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(directory.join("review.json"))?;
    report.write_all(&serde_json::to_vec_pretty(&serde_json::json!({"document":s.document().id(),"part":panel.body,"primitive":panel.feature,"placed":placed,"sets":{"panels":panels,"inches":inch,"choose_face":choose},"native_outputs":reports,"unselected_preview":unselected,"steps":["Open panel-review.multicad. Export > Planar panels previews two genuine 1:1 mm DXF files with outer rectangle and two circular holes.","Placed panel inches previews the rotated occurrence alone, with explicit inch units. World X/Y bounds before units conversion are [120,100] to [200,220] mm.","Choose panel face starts Unsupported. Load current planar faces; choose the face with normal [0,0,1], select WorldX, Save set, Preview and Run into a new existing absolute directory.","Edit the source primitive width 120 to 140. The chosen persistent face should regenerate, the original preview must become stale, and the rotated copy Y extent increases to240mm.","A WorldZ horizontal axis on a Z-normal face fails explicitly. Missing/ambiguous faces never adopt another face automatically."]}))?)?;
    println!(
        "Native source: {}\nActual DXF receipts: {}",
        path.display(),
        directory.join("review.json").display()
    );
    Ok(())
}
