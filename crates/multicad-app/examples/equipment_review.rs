//! Saved native equipment layout with all families and two true-scale plans.
use multicad_core::{
    commands::{Actor, Channel, Session},
    document::{Document, EntityId},
    drawings::{self, *},
    equipment::{self, CeilingDatum, HostPlacement, Kind, MountClass, WallSide},
    model::{
        self, BodyOutput, Justification, LevelDefinition, NewType, NewTypeKind, WallChange,
        WallDefinition, WallRequest, WallTypeDefinition, angle, length,
    },
    project_file::{self, ProjectFile},
    references::ReferenceFrame,
};
use multicad_model::{Compiler, Model};
use std::path::Path;
pub type Error = Box<dyn std::error::Error>;
pub struct Apartment {
    pub session: Session,
    pub floor: EntityId,
    pub wall: EntityId,
    pub book: EntityId,
    pub pages: Vec<EntityId>,
    pub equipment: Vec<equipment::EquipmentPlaceRequest>,
}
fn actor() -> Actor {
    Actor::local(Channel::Mcp)
}
pub fn build() -> Result<Apartment, Error> {
    let mut session = Session::new(Document::new("Apartment equipment review")?)?;
    let (floor, prepared) = model::prepare_level(
        &session,
        0,
        actor(),
        "Apartment finished floor",
        &LevelDefinition {
            elevation: length(100.),
            height: length(2800.),
            slab_thickness: length(200.),
        },
    )?;
    session.commit(prepared)?;
    let wall_type = NewType::new(
        "Generic apartment wall",
        NewTypeKind::Wall(WallTypeDefinition {
            thickness: length(200.),
            height: length(2700.),
            justification: Justification::Center,
            material: None,
        }),
    );
    let wall = WallRequest::new(
        "Apartment perimeter",
        WallDefinition {
            output: BodyOutput {
                body: EntityId::new(),
            },
            wall_type: wall_type.id,
            path: [[0., 0.], [12000., 0.], [12000., 8000.], [0., 8000.]]
                .map(|p| p.map(length))
                .to_vec(),
            closed: true,
            level: Some(floor),
            base_z: length(0.),
            thickness: None,
            height: None,
            justification: None,
            openings: vec![],
            cutouts: vec![],
        },
    );
    session.commit(
        model::prepare_wall_change(
            &session,
            session.revision(),
            actor(),
            std::slice::from_ref(&wall_type),
            &WallChange::Create(&wall),
        )?
        .prepared,
    )?;
    let mut sources = vec![wall.wall.output.body];
    for (name, path) in [
        (
            "Bathroom / kitchen divider",
            [[6500., 3500.], [6500., 8000.]],
        ),
        ("Laundry room divider", [[0., 3500.], [4500., 3500.]]),
    ] {
        let request = WallRequest::new(
            name,
            WallDefinition {
                output: BodyOutput {
                    body: EntityId::new(),
                },
                wall_type: wall_type.id,
                path: path.map(|p| p.map(length)).to_vec(),
                closed: false,
                level: Some(floor),
                base_z: length(0.),
                thickness: Some(length(120.)),
                height: None,
                justification: None,
                openings: vec![],
                cutouts: vec![],
            },
        );
        session.commit(
            model::prepare_wall_change(
                &session,
                session.revision(),
                actor(),
                &[],
                &WallChange::Create(&request),
            )?
            .prepared,
        )?;
        sources.push(request.wall.output.body);
    }
    let mut placed = vec![];
    for kind in Kind::ALL {
        let preset = kind.preset();
        let (xy, along) = match kind {
            Kind::PowerOutletTypeF => ([0., 0.], 1000.),
            Kind::DoublePowerOutletTypeF => ([0., 0.], 2000.),
            Kind::TriplePowerOutletTypeF => ([0., 0.], 3000.),
            Kind::DataOutletRj45 => ([0., 0.], 4000.),
            Kind::TvAntennaOutlet => ([0., 0.], 5000.),
            Kind::Switch => ([0., 0.], 6000.),
            Kind::WallLuminaire => ([0., 0.], 7000.),
            Kind::Radiator => ([0., 0.], 10000.),
            Kind::CeilingLuminaire => ([3000., 2000.], 0.),
            Kind::LampHolderE27 => ([8000., 2000.], 0.),
            Kind::KitchenSink => ([9500., 6500.], 0.),
            Kind::Washbasin => ([1000., 6500.], 0.),
            Kind::Toilet => ([2200., 6500.], 0.),
            Kind::Bathtub => ([3800., 6500.], 0.),
            Kind::Shower => ([5300., 5600.], 0.),
            Kind::CookerOven => ([7000., 6500.], 0.),
            Kind::Cooktop => ([8200., 6500.], 0.),
            Kind::WashingMachine => ([1000., 4500.], 0.),
            Kind::Dryer => ([2300., 4500.], 0.),
            Kind::Refrigerator => ([3600., 4500.], 0.),
            Kind::Dishwasher => ([4800., 4500.], 0.),
        };
        let host = match preset.mount {
            MountClass::WallCenter | MountClass::WallBottom => HostPlacement::Wall {
                wall: wall.feature,
                segment: 0,
                point_count: 4,
                closed: true,
                side: WallSide::Left,
                along: length(along),
                standoff: length(0.),
                height: length(preset.mounting_height_mm),
                rotation: angle(0.),
            },
            MountClass::CeilingTop => HostPlacement::Ceiling {
                xy: xy.map(length),
                datum: CeilingDatum::LevelUnderside,
                drop: length(0.),
                rotation: angle(0.),
            },
            _ => HostPlacement::Floor {
                xy: xy.map(length),
                height: length(preset.mounting_height_mm),
                rotation: angle(0.),
            },
        };
        let mut request = equipment::EquipmentPlaceRequest::new(
            &session,
            equipment::generic_definition(kind)?,
            preset.name.into(),
            preset.size_mm,
            floor,
            host,
        )?;
        request.settings.label = Some(
            match kind {
                Kind::PowerOutletTypeF => "P1",
                Kind::DoublePowerOutletTypeF => "P2",
                Kind::TriplePowerOutletTypeF => "P3",
                Kind::DataOutletRj45 => "D1",
                Kind::TvAntennaOutlet => "TV1",
                Kind::Switch => "S1",
                Kind::CeilingLuminaire => "L1",
                Kind::WallLuminaire => "L2",
                Kind::LampHolderE27 => "LH1",
                Kind::Radiator => "R1",
                Kind::KitchenSink => "KS",
                Kind::Washbasin => "B1",
                Kind::Toilet => "WC",
                Kind::Bathtub => "BT",
                Kind::Shower => "SH",
                Kind::CookerOven => "OV",
                Kind::Cooktop => "CT",
                Kind::WashingMachine => "WM",
                Kind::Dryer => "DR",
                Kind::Refrigerator => "FR",
                Kind::Dishwasher => "DW",
            }
            .into(),
        );
        session.commit(equipment::prepare_place(
            &session,
            session.revision(),
            actor(),
            &request,
        )?)?;
        sources.push(request.placement.instance);
        placed.push(request);
    }
    let mut book = None;
    let mut pages = vec![];
    for (denominator, electrical_only) in [(100, false), (50, false), (50, true)] {
        let mut page = Page::new(if electrical_only {
            "Electrical 1:50".into()
        } else {
            format!("Furnished 1:{denominator}")
        });
        page.settings.paper = PaperSize::A3;
        page.settings.orientation = Orientation::Landscape;
        page.settings.scale = Scale {
            numerator: 1,
            denominator,
        };
        page.info.title = if electrical_only {
            "Electrical layout — ToskCAD generic v1"
        } else {
            "Apartment equipment — generic editable planning"
        }
        .into();
        page.views.push(DrawingView {
            id: EntityId::new(),
            name: page.info.title.clone(),
            source: ViewSource::Model {
                entities: sources.clone(),
            },
            frame: ReferenceFrame {
                origin_mm: [6000., 4000., 1200.],
                u: [1., 0., 0.],
                v: [0., 1., 0.],
            },
            kind: ViewKind::Projection,
            position_mm: [200., 130.],
            scale: page.settings.scale,
            hidden_lines: false,
            tangent_edges: false,
            paper_chord_tolerance_mm: 0.01,
            guide: None,
            equipment_categories: electrical_only.then(|| {
                [equipment::Category::ElectricalLighting]
                    .into_iter()
                    .collect()
            }),
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
                "Equipment sheets",
                Some(page),
            )?;
            session.commit(p)?;
            book = Some(id);
        }
    }
    Ok(Apartment {
        session,
        floor,
        wall: wall.feature,
        book: book.expect("three pages"),
        pages,
        equipment: placed,
    })
}
pub fn compile(apartment: &Apartment) -> Result<Model, Error> {
    Ok(Compiler::default().compile(apartment.session.document(), apartment.session.revision())?)
}
pub fn write(apartment: &Apartment, root: &Path) -> Result<(), Error> {
    std::fs::create_dir_all(root)?;
    let model = compile(apartment)?;
    let session = &apartment.session;
    let evaluation = model.drawings(session.document(), session.revision(), &Default::default())?;
    for (page, name) in
        apartment
            .pages
            .iter()
            .zip(["furnished-1-100", "furnished-1-50", "electrical-1-50"])
    {
        let render = drawings::render::render_page(
            session.document(),
            session.revision(),
            apartment.book,
            *page,
            &evaluation,
        )?;
        std::fs::write(
            root.join(format!("{name}.dxf")),
            drawings::dxf::render_dxf(&render, 1., 4),
        )?;
        std::fs::write(
            root.join(format!("{name}.pdf")),
            drawings::pdf::book_pdf(
                session.document(),
                session.revision(),
                &[(apartment.book, *page)],
                &evaluation,
            )?,
        )?;
    }
    let project = ProjectFile::new(session.document().clone())?;
    let mut bytes = vec![];
    project_file::write(&project, &mut bytes, Default::default())?;
    std::fs::write(root.join("apartment-equipment.tcd"), bytes)?;
    std::fs::write(
        root.join("equipment-review.json"),
        serde_json::to_vec_pretty(
            &serde_json::json!({"revision":session.revision(),"floor":apartment.floor,"wall":apartment.wall,"book":apartment.book,"pages":apartment.pages,"equipment":apartment.equipment.iter().map(|r|serde_json::json!({"record":r.placement.record,"instance":r.placement.instance,"name":r.placement.name,"pin":r.definition.pin().expect("pin"),"settings":r.settings,"attachment":r.attachment})).collect::<Vec<_>>()}),
        )?,
    )?;
    Ok(())
}
fn main() -> Result<(), Error> {
    let root = std::env::args_os()
        .nth(1)
        .ok_or("Usage: equipment_review OUTPUT_FOLDER")?;
    write(&build()?, Path::new(&root))
}
