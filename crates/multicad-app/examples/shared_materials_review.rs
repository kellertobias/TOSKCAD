//! CAD-224 shared stock geometry and material-hatch fixtures. Creates no window.
use multicad_core::{
    commands::{Actor, Channel, Prepared, Session},
    document::{Document, Entity, EntityData, EntityId},
    drawings::{
        self, DrawingView, Page, PaperSize, Scale, VectorRole, ViewEvaluation, ViewKind, ViewSource,
    },
    materials::*,
    model::*,
    project_file::{self, ProjectFile},
    references::ReferenceFrame,
};
use multicad_model::{Compiler, Model};
use std::path::PathBuf;
type Error = Box<dyn std::error::Error>;
pub struct Fixture {
    pub session: Session,
    pub stocks: [EntityId; 3],
    pub parts: [EntityId; 3],
    pub hatch: EntityId,
    pub views: [EntityId; 3],
}
fn actor() -> Actor {
    Actor::local(Channel::Ui)
}
fn checked(s: &mut Session, prepared: Prepared) -> Result<(), Error> {
    Compiler::default().compile(prepared.document(), s.revision() + 1)?;
    s.commit(prepared)?;
    Ok(())
}
pub fn fixture() -> Result<Fixture, Error> {
    let mut session = Session::new(Document::new("Project shared materials and stock")?)?;
    let material = |name, density, color| {
        Entity::new(
            name,
            EntityData::Material {
                density_kg_m3: Some(density),
                color_rgba: color,
                section_hatch: None,
            },
        )
    };
    let birch = material("Birch", 650., [210, 173, 120, 255]);
    let steel = material("Shared steel", 7850., [120, 162, 200, 255]);
    let hatch = LibraryDefinition::Hatch(HatchStyle {
        color_rgb: None,
        pattern: HatchPattern::Lines,
        angle_deg: 0.,
        spacing_mm: 2.,
        line_width_mm: 0.18,
    })
    .entity(EntityId::new(), "Shared section hatch")?;
    let prepared = prepare_library(
        &session,
        0,
        actor(),
        &[birch.clone(), steel.clone(), hatch.clone()],
    )?;
    checked(&mut session, prepared)?;
    let mut entries = vec![];
    for material in [birch.id, steel.id] {
        entries.push(
            LibraryDefinition::MaterialStyle(MaterialStyle {
                material,
                appearance: None,
                hatch: Some(LibraryRef::capture(session.document(), hatch.id)?),
            })
            .entity(EntityId::new(), "Shared material defaults")?,
        );
    }
    let specifications = [
        (
            birch.id,
            "Birch panel 18",
            StockKind::Panel {
                thickness_mm: 18.,
                length_mm: 2440.,
                width_mm: 1220.,
                grain: Grain::AlongLength,
            },
        ),
        (
            steel.id,
            "Steel sheet 2",
            StockKind::Sheet {
                thickness_mm: 2.,
                length_mm: 2000.,
                width_mm: 1000.,
                inside_bend_radius_mm: 2.,
                k_factor: 0.4,
            },
        ),
        (
            steel.id,
            "Steel tube 30x20x2",
            StockKind::Profile {
                designation: "RHS".into(),
                length_mm: 6000.,
                width_mm: 30.,
                height_mm: 20.,
                wall_mm: Some(2.),
                shape: ProfileShape::Rectangular,
            },
        ),
    ];
    let mut stocks = [EntityId::nil(); 3];
    for (index, (material, name, specification)) in specifications.into_iter().enumerate() {
        let stock = LibraryDefinition::Stock(Stock {
            material,
            specification,
        })
        .entity(EntityId::new(), name)?;
        stocks[index] = stock.id;
        entries.push(stock);
    }
    let prepared = prepare_library(&session, session.revision(), actor(), &entries)?;
    checked(&mut session, prepared)?;
    let panel = PanelRequest::new(
        "Wood shared-stock panel",
        PanelDefinition {
            output: BodyOutput {
                body: EntityId::new(),
            },
            source: PanelSource::Rectangle {
                stock: stocks[0],
                plane: PanelPlane::Xy,
                origin: [0., 0., 0.].map(length),
                length: length(100.),
                width: length(60.),
            },
            grain: GrainAxis::U,
            edges: PanelEdges::default(),
            oversize: None,
        },
    );
    let prepared = prepare_panel(&session, session.revision(), actor(), &panel)?;
    checked(&mut session, prepared.prepared)?;
    let sheet = SheetRequest::new(
        "Sheet-metal shared-stock plate",
        SheetDefinition {
            output: BodyOutput {
                body: EntityId::new(),
            },
            stock: stocks[1],
            base: SheetBase::Rectangle {
                plane: PanelPlane::Xy,
                origin: [140., 0., 0.].map(length),
                length: length(100.),
                width: length(60.),
            },
            reverse: false,
            inside_radius: None,
            k_factor: None,
            flanges: vec![],
            cutouts: vec![],
            fasteners: vec![],
        },
    );
    let prepared = prepare_sheet(&session, session.revision(), actor(), &sheet)?;
    checked(&mut session, prepared.prepared)?;
    let member = MemberRequest::new(
        "Weld shared-stock member",
        MemberDefinition {
            output: BodyOutput {
                body: EntityId::new(),
            },
            stock: stocks[2],
            path: MemberPath::Points {
                start: [0., 100., 40.].map(length),
                end: [200., 100., 40.].map(length),
            },
            anchor: [AxisAnchor::Center; 2],
            rotation: None,
            extend_start: None,
            extend_end: None,
        },
    );
    let prepared = prepare_members(
        &session,
        session.revision(),
        actor(),
        std::slice::from_ref(&member),
        None,
    )?;
    checked(&mut session, prepared.prepared)?;
    let parts = [panel.body, sheet.body, member.body];
    let mut page = Page::new("Shared material section checks");
    page.settings.paper = PaperSize::Custom {
        width_mm: 400.,
        height_mm: 240.,
    };
    let views = std::array::from_fn(|_| EntityId::new());
    for (index, part) in parts.into_iter().enumerate() {
        page.views.push(DrawingView {
            id: views[index],
            name: ["Panel", "Sheet", "Profile"][index].into(),
            source: ViewSource::Model {
                entities: vec![part],
            },
            frame: if index == 2 {
                ReferenceFrame {
                    origin_mm: [0., 100., 40.],
                    u: [0., 1., 0.],
                    v: [0., 0., 1.],
                }
            } else {
                ReferenceFrame {
                    origin_mm: [if index == 1 { 140. } else { 0. }, 0., 0.],
                    u: [1., 0., 0.],
                    v: [0., 1., 0.],
                }
            },
            kind: ViewKind::Section {
                offset_mm: [9., 1., 100.][index],
                range: Default::default(),
                window: Default::default(),
            },
            position_mm: [35. + 120. * index as f64, 110.],
            scale: Scale {
                numerator: 1,
                denominator: 1,
            },
            hidden_lines: false,
            tangent_edges: false,
            paper_chord_tolerance_mm: 0.01,
            guide: None,
            equipment_categories: None,
        });
    }
    let (_, prepared) = drawings::prepare_create(
        &session,
        session.revision(),
        actor(),
        "Shared material drawings",
        Some(page),
    )?;
    checked(&mut session, prepared)?;
    Ok(Fixture {
        session,
        stocks,
        parts,
        hatch: hatch.id,
        views,
    })
}
pub fn changed_definition(s: &Session, id: EntityId) -> Result<Entity, Error> {
    let old = s.document().get(id).ok_or("Missing shared definition")?;
    let mut definition = read(old)?.ok_or("Not a library definition")?;
    match &mut definition {
        LibraryDefinition::Stock(stock) => match &mut stock.specification {
            StockKind::Panel { thickness_mm, .. } => *thickness_mm = 24.,
            StockKind::Sheet { thickness_mm, .. } => *thickness_mm = 3.,
            StockKind::Profile {
                width_mm,
                height_mm,
                wall_mm,
                ..
            } => {
                *width_mm = 40.;
                *height_mm = 30.;
                *wall_mm = Some(3.);
            }
            _ => return Err("Unexpected stock kind".into()),
        },
        LibraryDefinition::Hatch(hatch) => {
            hatch.angle_deg = 45.;
            hatch.spacing_mm = 3.;
            hatch.line_width_mm = 0.28;
        }
        _ => return Err("Unexpected definition kind".into()),
    }
    Ok(definition.entity(id, &old.name)?)
}
pub fn check(s: &Session, fixture: &Fixture, changed: bool) -> Result<serde_json::Value, Error> {
    let native = Compiler::default().compile(s.document(), s.revision())?;
    let expected = if changed {
        [144000., 18000., 76800.]
    } else {
        [108000., 12000., 36800.]
    };
    let mut measurements = vec![];
    for (index, part) in fixture.parts.into_iter().enumerate() {
        let solid = native.solid(part).ok_or("Missing native Part")?;
        let volume = solid.volume_mm3()?;
        assert!(
            (volume - expected[index]).abs() < 1e-5,
            "part{index}: {volume}"
        );
        let properties = resolve(s.document(), part)?;
        assert_eq!(
            properties.stock.as_ref().map(|v| v.0),
            Some(fixture.stocks[index])
        );
        let bounds = solid.bounds()?;
        measurements.push(serde_json::json!({"part":part,"stock":fixture.stocks[index],"material":properties.material,"volume_mm3":volume,"bounds_mm":[bounds.min,bounds.max]}));
    }
    let evaluation = native.drawings(s.document(), s.revision(), &Default::default())?;
    let width = if changed { 0.28 } else { 0.18 };
    let mut hatches = vec![];
    for view in fixture.views {
        let Some(ViewEvaluation::Ready { vectors, .. }) = evaluation.views.get(&view) else {
            return Err(format!("Drawing unresolved: {:?}", evaluation.views.get(&view)).into());
        };
        let lines: Vec<_> = vectors
            .iter()
            .filter(|v| v.role == VectorRole::Hatch)
            .collect();
        assert!(!lines.is_empty());
        assert!(lines.iter().all(|v| (v.line_width_mm - width).abs() < 1e-9));
        hatches.push(
            serde_json::json!({"view":view,"hatch_lines":lines.len(),"paper_line_width_mm":width}),
        );
    }
    Ok(serde_json::json!({"parts":measurements,"drawing_hatches":hatches}))
}
fn main() -> Result<(), Error> {
    let output = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| "artifacts/shared-materials-review-20261102".into());
    std::fs::create_dir_all(&output)?;
    let mut fixture = fixture()?;
    let mut reports = vec![];
    for changed in [false, true] {
        if changed {
            let entities = fixture
                .stocks
                .into_iter()
                .chain([fixture.hatch])
                .map(|id| changed_definition(&fixture.session, id))
                .collect::<Result<Vec<_>, _>>()?;
            let prepared = prepare_library(
                &fixture.session,
                fixture.session.revision(),
                actor(),
                &entities,
            )?;
            checked(&mut fixture.session, prepared)?;
        }
        reports.push(check(&fixture.session, &fixture, changed)?);
        let stage = if changed { "after" } else { "before" };
        let project = output.join(format!("{stage}.tcd"));
        project_file::write(
            &ProjectFile::new(fixture.session.document().clone())?,
            &mut std::fs::File::create(&project)?,
            Default::default(),
        )?;
        let reopened = project_file::read(
            &mut std::fs::File::open(&project)?,
            Default::default(),
            &project_file::supported_capabilities(),
        )?
        .project;
        let reopened = Session::new(reopened.document)?;
        check(&reopened, &fixture, changed)?;
        let native: Model =
            Compiler::default().compile(fixture.session.document(), fixture.session.revision())?;
        for (index, part) in fixture.parts.into_iter().enumerate() {
            let solid = native.solid(part).ok_or("Missing export Part")?;
            let path = output.join(format!(
                "{stage}-{}.step",
                ["panel", "sheet", "profile"][index]
            ));
            solid.write_step(&path)?;
            let reread = multicad_kernel::Solid::read_step(&path, Default::default())?;
            assert!((reread.volume_mm3()? - solid.volume_mm3()?).abs() < 1e-4);
        }
    }
    std::fs::write(
        output.join("native-report.json"),
        serde_json::to_vec_pretty(&reports)?,
    )?;
    println!("{}", output.display());
    Ok(())
}
