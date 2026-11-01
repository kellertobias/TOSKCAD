//! The shared CAD workflow checkpoint in one native editable project:
//! parameter-driven constrained sketches inside two independently configured
//! dynamic symbol instances, an imported DXF plate extruded by an exposed
//! expression, a two-page drawing book with an exterior projection,
//! section-only and finite-depth views, dimensions and annotations, and an
//! export set with PDF, drawing DXF, OBJ and 3MF recipes. The app test
//! `tests/shared_workflow.rs` builds the same project and checks it.
#[allow(dead_code)]
#[path = "symbol_review.rs"]
pub mod symbols_review;
use multicad_core::{
    commands::{Actor, Channel, Request, Session},
    constraint_sketch::Dimension,
    document::{Entity, EntityData, EntityId, Mutation},
    drawings::*,
    exchange::*,
    inspection::GeometryReference,
    materials::*,
    model::{self, Extent, ExtrudeParameters, ExtrudeRequest, Termination},
    parameters::BoundExpression,
    project_file::{self, ProjectFile},
    references::{AnalyticGeometry, GeometryIntent, ReferenceFrame, TopologyKind},
    sketch::Datum,
    sketch_import::{self, Format, ImportOptions, plan::plan},
};
use multicad_kernel::NativeRegionQueries;
use multicad_model::{Compiler, Model, export::Exporter};
use multicad_solver::import::{ImportSource, ImportTarget, prepare_import};
use std::{collections::BTreeSet, io::Write, path::Path};

pub type Error = Box<dyn std::error::Error>;
pub const PLATE_DXF: &[u8] =
    include_bytes!("../../multicad-core/tests/fixtures/sketch_import/plate.dxf");
/// Plate profile: 100 × 60 minus the R20 corner remainder, a Ø20 hole and a
/// 15 × 10 slot with round ends.
pub fn plate_area_mm2() -> f64 {
    use std::f64::consts::PI;
    6000. - (400. - PI * 100.) - PI * 100. - (150. + PI * 25.)
}

pub fn actor() -> Actor {
    Actor::local(Channel::CommandLine)
}

pub struct Scenario {
    pub symbols: symbols_review::Fixture,
    pub thickness: EntityId,
    pub plate_sketch: EntityId,
    pub plate: EntityId,
    pub plate_feature: EntityId,
    pub book: EntityId,
    pub pages: [EntityId; 2],
    pub plate_dimension: EntityId,
    pub panel_dimension: EntityId,
    pub set: EntityId,
}
impl Scenario {
    pub fn session(&self) -> &Session {
        &self.symbols.session
    }
    pub fn session_mut(&mut self) -> &mut Session {
        &mut self.symbols.session
    }
    pub fn instances(&self) -> [EntityId; 2] {
        [
            self.symbols.placed[0].instance,
            self.symbols.placed[1].instance,
        ]
    }
}

fn checked(
    session: &mut Session,
    prepared: multicad_core::commands::Prepared,
) -> Result<(), Error> {
    symbols_review::checked(session, prepared)
}
/// A planar face of a placed Part or instance whose normal matches `normal`
/// and whose plane passes through coordinate `at` along that normal.
pub fn face(
    model: &Model,
    entity: EntityId,
    normal: [f64; 3],
    at: f64,
) -> Result<GeometryReference, Error> {
    let mut topology = if let Some(occurrence) = model.snapshot().occurrences.get(&entity) {
        occurrence.topology.clone()
    } else {
        let feature = model.snapshot().bodies[&entity];
        model.snapshot().shapes[&feature].topology.clone()
    };
    topology.owner = entity;
    let element = topology
        .elements
        .iter()
        .find(|e| {
            e.key.kind == TopologyKind::Face
                && matches!(e.geometry, Some(AnalyticGeometry::Plane { frame })
                    if frame.normal().iter().zip(normal).all(|(a, b)| (a - b).abs() < 1e-9)
                        && (frame.origin_mm.iter().zip(normal).map(|(o, n)| o * n).sum::<f64>() - at).abs() < 1e-6)
        })
        .ok_or("Expected planar face is missing")?;
    Ok(GeometryReference {
        source: entity,
        intent: GeometryIntent::capture(&topology, element.key)?,
    })
}
fn view(
    name: &str,
    parts: Vec<EntityId>,
    at: [f64; 2],
    denominator: u32,
    kind: ViewKind,
) -> DrawingView {
    DrawingView {
        id: EntityId::new(),
        name: name.into(),
        source: ViewSource::Model { entities: parts },
        frame: ReferenceFrame {
            origin_mm: [0.; 3],
            u: [1., 0., 0.],
            v: [0., 0., 1.],
        },
        kind,
        position_mm: at,
        scale: Scale {
            numerator: 1,
            denominator,
        },
        hidden_lines: true,
        tangent_edges: false,
        paper_chord_tolerance_mm: 0.01,
        guide: None,
        equipment_categories: None,
    }
}
fn dimension(
    view: EntityId,
    a: GeometryReference,
    b: GeometryReference,
    offset_mm: f64,
) -> Annotation {
    Annotation {
        id: EntityId::new(),
        kind: AnnotationKind::Dimension {
            view,
            measure: Box::new(DimensionMeasure::Linear {
                a,
                b,
                direction: LinearDirection::Horizontal,
            }),
            offset_mm,
            precision: 1,
            reference: false,
        },
    }
}

pub fn build() -> Result<Scenario, Error> {
    let mut symbols = symbols_review::seed(false)?;
    let session = &mut symbols.session;

    // An exposed project expression drives the imported plate's thickness.
    let thickness = Entity::new(
        "plate_thickness",
        EntityData::Parameter {
            expression: "10".into(),
            unit: "mm".into(),
            exposed: true,
            inputs: vec![],
        },
    );
    session.apply(Request {
        expected_revision: session.revision(),
        actor: actor(),
        label: "Add plate thickness".into(),
        edits: vec![Mutation::Insert(thickness.clone())],
    })?;

    // DXF artwork becomes editable sketch geometry on XY.
    let options = ImportOptions {
        layers: Some(BTreeSet::from(["CUT".into()])),
        ..Default::default()
    };
    let art = sketch_import::parse(Format::Dxf, PLATE_DXF, options.units)?;
    let import = prepare_import(
        session,
        session.revision(),
        actor(),
        &ImportTarget::New {
            datum: Datum::Xy,
            name: "Imported plate".into(),
        },
        None,
        &plan(&art, &options)?,
        &ImportSource {
            file_name: "plate.dxf",
            bytes: PLATE_DXF,
        },
    )?;
    let plate_sketch = import.sketch;
    session.commit(import.edit.preview.ok_or("Import has no checked preview")?)?;
    let evaluated = multicad_solver::evaluate_document_sketch(
        session.document(),
        session.revision(),
        plate_sketch,
        None,
    )?;
    let regions = evaluated.regions_with_queries(&NativeRegionQueries::default())?;
    let main = regions
        .regions
        .iter()
        .find(|r| r.holes.len() == 2)
        .ok_or("Imported plate region is missing")?;
    let request = ExtrudeRequest::new(
        plate_sketch,
        vec![main.intent()],
        ExtrudeParameters {
            extent: Extent::OneSided {
                end: Termination::Distance {
                    distance: Dimension::Expression {
                        expression: BoundExpression::bind(
                            session.document(),
                            "=$plate_thickness",
                            "mm",
                            None,
                        )?,
                    },
                },
            },
            ..Default::default()
        },
    );
    let extruded = model::prepare_extrude(session, session.revision(), actor(), &request)?;
    let (plate, plate_feature) = (extruded.outputs[0].body, extruded.outputs[0].feature);
    checked(session, extruded.prepared)?;

    // Hatched steel on the plate for section views.
    let material = Entity::new(
        "Steel",
        EntityData::Material {
            density_kg_m3: Some(7850.),
            color_rgba: [150, 150, 160, 255],
            section_hatch: None,
        },
    );
    let hatch = LibraryDefinition::Hatch(HatchStyle {
        pattern: HatchPattern::Lines,
        angle_deg: 45.,
        spacing_mm: 1.5,
        line_width_mm: 0.18,
    })
    .entity(EntityId::new(), "Steel section")?;
    let library = prepare_library(
        session,
        session.revision(),
        actor(),
        &[material.clone(), hatch.clone()],
    )?;
    session.commit(library)?;
    let style = LibraryDefinition::MaterialStyle(MaterialStyle {
        material: material.id,
        appearance: None,
        hatch: Some(LibraryRef::capture(session.document(), hatch.id)?),
    })
    .entity(EntityId::new(), "Defaults")?;
    let style = prepare_library(session, session.revision(), actor(), &[style])?;
    session.commit(style)?;
    let assigned = prepare_assignment(
        session,
        session.revision(),
        actor(),
        &[plate],
        &Assignment {
            material: Some(material.id),
            stock: None,
            appearance: None,
            hatch: None,
        },
    )?;
    session.commit(assigned)?;

    let model = Compiler::default().compile(session.document(), session.revision())?;
    let instances = [symbols.placed[0].instance, symbols.placed[1].instance];
    let session = &mut symbols.session;

    // Page 1, A3 landscape: an exterior front projection of every subject at
    // 1:5 with the plate and panel A widths dimensioned, a note and a table.
    let mut first = Page::new("W-101");
    first.settings.paper = PaperSize::A3;
    first.info = DrawingInfo {
        title: "Shared workflow".into(),
        author: "MultiCAD".into(),
        project: "Checkpoint".into(),
        revision: "A".into(),
        checker: "QA".into(),
    };
    let front = view(
        "Front",
        vec![plate, instances[0], instances[1]],
        [40., 120.],
        5,
        ViewKind::Projection,
    );
    let plate_dimension = dimension(
        front.id,
        face(&model, plate, [-1., 0., 0.], 0.)?,
        face(&model, plate, [1., 0., 0.], 100.)?,
        8.,
    );
    let panel_dimension = dimension(
        front.id,
        face(&model, instances[0], [-1., 0., 0.], -250.)?,
        face(&model, instances[0], [1., 0., 0.], 400.)?,
        16.,
    );
    let ids = (plate_dimension.id, panel_dimension.id);
    first.annotations.push(plate_dimension);
    first.annotations.push(panel_dimension);
    first.annotations.push(Annotation {
        id: EntityId::new(),
        kind: AnnotationKind::Note {
            position_mm: [30., 220.],
            text: "Plate from plate.dxf".into(),
        },
    });
    first.annotations.push(Annotation {
        id: EntityId::new(),
        kind: AnnotationKind::Table {
            position_mm: [250., 40.],
            title: "Parts".into(),
            columns: vec!["Part".into(), "Qty".into()],
            rows: vec![
                vec!["Plate".into(), "1".into()],
                vec!["Panel".into(), "2".into()],
            ],
            widths_mm: vec![30., 12.],
            row_height_mm: 5.,
            schedule: None,
        },
    });
    first.views.push(front);
    // Page 2, A4 portrait: plate sections through the Ø20 hole at y = 30,
    // section-only and 20 mm deep, at 1:2.
    let mut second = Page::new("W-102");
    second.settings.orientation = Orientation::Portrait;
    let cut = |depth: Option<f64>| ViewKind::Section {
        offset_mm: -30.,
        range: depth.map_or(SectionRange::SectionOnly, |d| SectionRange::Depth {
            depth_mm: d,
        }),
        window: SectionWindow::default(),
    };
    second
        .views
        .push(view("Section only", vec![plate], [50., 80.], 2, cut(None)));
    second.views.push(view(
        "Section depth 20",
        vec![plate],
        [50., 170.],
        2,
        cut(Some(20.)),
    ));
    let pages = [first.id, second.id];
    let (book, created) = prepare_create(
        session,
        session.revision(),
        actor(),
        "Workshop set",
        Some(first),
    )?;
    session.commit(created)?;
    let added = prepare(
        session,
        session.revision(),
        actor(),
        book,
        PageAction::Add(second),
    )?;
    session.commit(added)?;

    let set = EntityId::new();
    let mesh = |name: &str, format: MeshFormat| Recipe {
        id: EntityId::new(),
        name: name.into(),
        output: RecipeOutput::Mesh {
            format,
            units: LengthUnit::Millimeter,
            hierarchy: HierarchyMode::SeparateParts,
            names: NameHandling::ProductNames,
            aids: AidPolicy::IncludeAll,
            quality: MeshQuality {
                chord_deviation_mm: 0.05,
                angular_deviation_deg: 10.,
            },
        },
    };
    let definition = SetDefinition {
        scope: Scope {
            root: ScopeRoot::Explicit {
                entities: vec![plate, instances[0], instances[1]],
            },
            tags: None,
        },
        pages: Pages::Project,
        recipes: vec![
            Recipe {
                id: EntityId::new(),
                name: "PDF".into(),
                output: RecipeOutput::Pdf,
            },
            Recipe {
                id: EntityId::new(),
                name: "DXF".into(),
                output: RecipeOutput::DrawingDxf {
                    units: LengthUnit::Millimeter,
                    layers: Layers::default(),
                },
            },
            mesh("OBJ", MeshFormat::Obj),
            mesh("3MF", MeshFormat::ThreeMf),
        ],
        ..Default::default()
    };
    let created = prepare_set(
        session,
        session.revision(),
        actor(),
        set,
        "Workshop outputs",
        &definition,
    )?;
    session.commit(created)?;
    Ok(Scenario {
        symbols,
        thickness: thickness.id,
        plate_sketch,
        plate,
        plate_feature,
        book,
        pages,
        plate_dimension: ids.0,
        panel_dimension: ids.1,
        set,
    })
}

/// Preview and run the export set into `root`; returns the receipt.
pub fn export(session: &Session, set: EntityId, root: &Path) -> Result<RunReceipt, Error> {
    let exporter = Exporter::default();
    let preview = exporter.preview(session.document(), session.revision(), set, &actor(), None)?;
    if !preview.can_run() {
        return Err(format!("Export set cannot run: {:?}", preview.recipes).into());
    }
    Ok(exporter.run(
        session.document(),
        session.revision(),
        &preview.key,
        root,
        &actor(),
        None,
    )?)
}

pub fn archive(session: &Session) -> Result<Vec<u8>, Error> {
    let mut bytes = vec![];
    project_file::write(
        &ProjectFile::new(session.document().clone())?,
        &mut bytes,
        Default::default(),
    )?;
    Ok(bytes)
}

pub fn generate(directory: &Path) -> Result<serde_json::Value, Error> {
    std::fs::create_dir(directory)?;
    let scenario = build()?;
    let project = directory.join("shared-workflow.multicad");
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&project)?;
    file.write_all(&archive(scenario.session())?)?;
    file.sync_all()?;
    let exports = directory.join("exports");
    std::fs::create_dir(&exports)?;
    let exports = exports.canonicalize()?;
    let receipt = export(scenario.session(), scenario.set, &exports)?;
    let manifest = serde_json::json!({
        "generator": "shared_workflow_review",
        "project": project,
        "revision": scenario.session().revision(),
        "exports": receipt.files.iter().map(|f| &f.artifact.output.filename).collect::<Vec<_>>(),
        "export_directory": receipt.directory,
        "verification": {"native_geometry": true, "interactive_macos": false, "windows_linux_runtime": false},
    });
    std::fs::write(
        directory.join("manifest.json"),
        serde_json::to_vec_pretty(&manifest)?,
    )?;
    Ok(manifest)
}

#[cfg(not(test))]
fn main() -> Result<(), Error> {
    let path = std::env::args_os().nth(1).ok_or(
        "Usage: cargo run -p multicad-app --example shared_workflow_review -- NEW_DIRECTORY",
    )?;
    println!(
        "{}",
        serde_json::to_string_pretty(&generate(Path::new(&path))?)?
    );
    Ok(())
}
