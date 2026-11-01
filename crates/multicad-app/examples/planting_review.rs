//! Reproducible native garden review: overlapping canopies, shrubs and context,
//! with two true-scale pages exported through the production drawing writers.
use multicad_core::{
    commands::{Actor, Channel, Session},
    document::{Document, EntityId, Placement},
    drawings::{self, *},
    model::{self, PrimitiveFrame, PrimitiveParameters, PrimitiveRequest, length},
    plants::{self, PlantKind},
    project_file::{self, ProjectFile},
    references::ReferenceFrame,
};
use multicad_model::{Compiler, Model};
use std::path::Path;
pub type Error = Box<dyn std::error::Error>;
pub struct Garden {
    pub session: Session,
    pub book: EntityId,
    pub pages: Vec<EntityId>,
    pub plants: Vec<plants::PlantPlaceRequest>,
}
fn actor() -> Actor {
    Actor::local(Channel::Mcp)
}
pub fn build() -> Result<Garden, Error> {
    let mut session = Session::new(Document::new("Planting review").unwrap())?;
    let mut sources = vec![];
    for (name, size, origin) in [
        ("Building context", [6000., 4000., 3000.], [0., 0., 0.]),
        ("Garden path", [12000., 1000., 100.], [0., 4500., 0.]),
    ] {
        let request = PrimitiveRequest::new(
            name,
            PrimitiveParameters::Box {
                size: size.map(length),
                frame: PrimitiveFrame {
                    origin: origin.map(length),
                    ..Default::default()
                },
            },
        );
        session.commit(
            model::prepare_primitive(&session, session.revision(), actor(), &request)?.prepared,
        )?;
        sources.push(request.body);
    }
    let mut plants = vec![];
    for (kind, name, position) in [
        (PlantKind::Tree, "T1", [4500., 5000., 0.]),
        (PlantKind::Tree, "T2", [7000., 6500., 200.]),
        (PlantKind::Shrub, "S1", [2500., 5000., 0.]),
        (PlantKind::Shrub, "S2", [9500., 4500., 0.]),
    ] {
        let mut request = plants::PlantPlaceRequest::new(
            &session,
            plants::generic_definition(kind)?,
            name.into(),
            if kind == PlantKind::Tree { 6000. } else { 800. },
            if kind == PlantKind::Tree {
                4000.
            } else {
                1000.
            },
            Placement {
                translation_mm: position,
                ..Default::default()
            },
        )?;
        request.settings.label = Some(name.into());
        session.commit(plants::prepare_place(
            &session,
            session.revision(),
            actor(),
            &request,
        )?)?;
        sources.push(request.placement.instance);
        plants.push(request);
    }
    let mut pages = vec![];
    let mut book = None;
    for denominator in [100, 50] {
        let mut page = Page::new(format!("P-{denominator}"));
        page.settings.paper = PaperSize::A3;
        page.settings.orientation = Orientation::Landscape;
        page.settings.scale = Scale {
            numerator: 1,
            denominator,
        };
        page.info.title = "Garden planting plan".into();
        page.views.push(DrawingView {
            id: EntityId::new(),
            name: "Planting plan".into(),
            source: ViewSource::Model {
                entities: sources.clone(),
            },
            frame: ReferenceFrame {
                origin_mm: [6000., 4000., 1100.],
                u: [1., 0., 0.],
                v: [0., 1., 0.],
            },
            kind: ViewKind::Projection,
            position_mm: [200., 130.],
            scale: Scale {
                numerator: 1,
                denominator,
            },
            hidden_lines: false,
            tangent_edges: false,
            paper_chord_tolerance_mm: 0.01,
            guide: None,
        });
        pages.push(page.id);
        if let Some(id) = book {
            session.commit(drawings::prepare(
                &session,
                session.revision(),
                actor(),
                id,
                PageAction::Add(page),
            )?)?;
        } else {
            let (id, p) = drawings::prepare_create(
                &session,
                session.revision(),
                actor(),
                "Planting sheets",
                Some(page),
            )?;
            session.commit(p)?;
            book = Some(id);
        }
    }
    Ok(Garden {
        session,
        book: book.expect("two pages"),
        pages,
        plants,
    })
}
pub fn compile(garden: &Garden) -> Result<Model, Error> {
    Ok(Compiler::default().compile(garden.session.document(), garden.session.revision())?)
}
pub fn write(garden: &Garden, root: &Path) -> Result<(), Error> {
    std::fs::create_dir_all(root)?;
    let model = compile(garden)?;
    let session = &garden.session;
    let evaluation = model.drawings(session.document(), session.revision(), &Default::default())?;
    for (&page, denominator) in garden.pages.iter().zip([100, 50]) {
        let render = drawings::render::render_page(
            session.document(),
            session.revision(),
            garden.book,
            page,
            &evaluation,
        )?;
        std::fs::write(
            root.join(format!("garden-1-{denominator}.dxf")),
            drawings::dxf::render_dxf(&render, 1., 4),
        )?;
        std::fs::write(
            root.join(format!("garden-1-{denominator}.pdf")),
            drawings::pdf::book_pdf(
                session.document(),
                session.revision(),
                &[(garden.book, page)],
                &evaluation,
            )?,
        )?;
    }
    let project = ProjectFile::new(session.document().clone())?;
    let mut bytes = vec![];
    project_file::write(&project, &mut bytes, Default::default())?;
    std::fs::write(root.join("garden.tcd"), bytes)?;
    let report = serde_json::json!({"revision":session.revision(),"book":garden.book,"pages":garden.pages,"plants":garden.plants.iter().map(|p|serde_json::json!({"name":p.placement.name,"record":p.placement.record,"instance":p.placement.instance,"pin":p.definition.pin().expect("checked pin")})).collect::<Vec<_>>(),"policy":"Authored canopy outlines; tree double ring, shrub single ring; transparent overlays after building/path vectors; 0.18 mm paper line weight; optional canonical labels; physical sections/elevations use the 3D envelope."});
    std::fs::write(
        root.join("garden-review.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    Ok(())
}
fn main() -> Result<(), Error> {
    let root = std::env::args_os()
        .nth(1)
        .ok_or("Usage: planting_review OUTPUT_FOLDER")?;
    let garden = build()?;
    write(&garden, Path::new(&root))
}
