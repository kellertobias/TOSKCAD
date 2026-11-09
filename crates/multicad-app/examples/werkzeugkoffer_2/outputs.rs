use super::{CaseBuilder, Error, NAME, actor, printed::Printed};
use multicad_core::{
    commands::Session,
    document::{EntityId, Link},
    drawings::{
        Annotation, AnnotationKind, DrawingInfo, DrawingView, Page, PaperSize, Scale, ViewKind,
        ViewSource, prepare_create,
    },
    exchange::*,
    references::ReferenceFrame,
    schedules::{ScheduleKind, ScheduleSpec},
};
use multicad_model::{
    Compiler,
    export::{Exporter, mesh::read_three_mf},
};
use std::path::Path;

fn recipe(name: &str, output: RecipeOutput) -> Recipe {
    Recipe {
        id: EntityId::new(),
        name: name.into(),
        output,
    }
}

fn note(position_mm: [f64; 2], text: &str) -> Annotation {
    Annotation {
        id: EntityId::new(),
        kind: AnnotationKind::Note {
            position_mm,
            text: text.into(),
        },
    }
}

fn drawing(case: &mut CaseBuilder, sides: [EntityId; 2]) -> Result<(), Error> {
    let mut page = Page::new("WK-101");
    page.settings.paper = PaperSize::A3;
    page.info = DrawingInfo {
        title: "Mirrored plywood sides".into(),
        project: NAME.into(),
        author: "ToskCAD".into(),
        revision: "A".into(),
        checker: "Pending human review".into(),
    };
    for (i, part) in sides.into_iter().enumerate() {
        let u = if i == 0 { [0., -1., 0.] } else { [0., 1., 0.] };
        page.views.push(DrawingView {
            id: EntityId::new(),
            name: if i == 0 {
                "Left exterior"
            } else {
                "Right exterior"
            }
            .into(),
            source: ViewSource::Model {
                entities: vec![part],
            },
            frame: ReferenceFrame {
                origin_mm: [0., 117.5, 155.],
                u,
                v: [0., 0., 1.],
            },
            kind: ViewKind::Projection,
            position_mm: [110. + 200. * i as f64, 150.],
            scale: Scale {
                numerator: 1,
                denominator: 2,
            },
            hidden_lines: false,
            tangent_edges: false,
            paper_chord_tolerance_mm: 0.02,
            guide: None,
            equipment_categories: None,
        });
    }
    page.annotations.extend([
        note([22., 247.], "Two 6 mm birch plywood sides. Through-cut the same profile twice."),
        note([22., 254.], "Countersink the six slide holes and four rack holes on the outside face of each side. The outside faces mirror."),
        note([22., 261.], "Panel DXFs give nominal cut geometry. Apply measured stock thickness and 0.1 to 0.2 mm slot allowance at fabrication."),
        note([22., 268.], "Two 995 mm side angles. Notch stations from front end are 70, 380, 615 and 925 mm."),
        note([22., 275.], "Confirm installed slide travel, countersink head sizes and optional trim fasteners before fabrication."),
    ]);
    let (_, prepared) = prepare_create(
        &case.session,
        case.session.revision(),
        actor(),
        "Tool-case fabrication",
        Some(page),
    )?;
    case.session.commit(prepared)?;
    Ok(())
}

pub(super) fn configure_plywood(
    case: &mut CaseBuilder,
    panels: [EntityId; 7],
) -> Result<EntityId, Error> {
    drawing(case, [panels[0], panels[1]])?;
    let dxf = recipe(
        "Wood through-cuts",
        RecipeOutput::PlanarDxf {
            units: LengthUnit::Millimeter,
            layers: Layers::default(),
            aids: AidPolicy::ExcludeRemovable,
        },
    );
    let output = EntityId::new();
    let target_panels = [
        panels[0], panels[2], panels[3], panels[4], panels[5], panels[6],
    ];
    let scope = Scope {
        root: ScopeRoot::Explicit {
            entities: target_panels.to_vec(),
        },
        tags: None,
    };
    let catalog = geometry_exporter().planar_catalog(
        case.session.document(),
        case.session.revision(),
        &scope,
        None,
    )?;
    let mut definition = SetDefinition {
        scope,
        recipes: vec![dxf.clone()],
        ..Default::default()
    };
    let mut links = Vec::new();
    for part in catalog.parts {
        let index = target_panels
            .iter()
            .position(|id| *id == part.subject.part)
            .ok_or("Unexpected panel in the export scope")?;
        let normal = match index {
            0 => [1., 0., 0.],
            2 | 4 => [0., -1., 0.],
            _ => [0., 0., 1.],
        };
        let face = part
            .faces
            .into_iter()
            .filter(|face| {
                face.plane
                    .normal()
                    .into_iter()
                    .zip(normal)
                    .map(|(a, b)| a * b)
                    .sum::<f64>()
                    > 0.999
            })
            .max_by(|a, b| a.area_mm2.total_cmp(&b.area_mm2))
            .ok_or("A plywood panel has no unchamfered broad face")?;
        definition.planar_faces.push(PlanarFace {
            recipe: dxf.id,
            subject: part.subject,
            source: face.source,
            reference: face.intent.id,
            horizontal: if index == 0 {
                PlanarAxis::WorldY
            } else {
                PlanarAxis::WorldX
            },
        });
        links.push(Link {
            target: face.source,
            role: PLANAR_FACE_ROLE.into(),
            geometry: Some(face.intent),
        });
    }
    if links.len() != target_panels.len() {
        return Err("The wood export needs six selected native faces".into());
    }
    let prepared = prepare_set_with_geometry(
        &case.session,
        case.session.revision(),
        actor(),
        output,
        "Plywood profiles (side x2)",
        &definition,
        &links,
    )?;
    case.session.commit(prepared)?;
    Ok(output)
}

pub(super) fn configure(
    case: &mut CaseBuilder,
    printed: &Printed,
    auxiliary: &[EntityId],
) -> Result<(EntityId, Vec<EntityId>), Error> {
    let paperwork = EntityId::new();
    let definition = SetDefinition {
        pages: Pages::Project,
        recipes: vec![
            recipe("Plywood drawing", RecipeOutput::Pdf),
            recipe(
                "Bill of materials",
                RecipeOutput::ScheduleCsv {
                    schedule: ScheduleSpec::new(ScheduleKind::Bom),
                },
            ),
            recipe(
                "Plywood cut list",
                RecipeOutput::ScheduleCsv {
                    schedule: ScheduleSpec::new(ScheduleKind::PanelCutList),
                },
            ),
        ],
        ..Default::default()
    };
    let prepared = prepare_set(
        &case.session,
        case.session.revision(),
        actor(),
        paperwork,
        "Fabrication paperwork",
        &definition,
    )?;
    case.session.commit(prepared)?;

    let mut plates = Vec::new();
    for (i, drawer) in printed.large.into_iter().enumerate() {
        plates.push(plate(
            case,
            &format!("Large drawer {}", i + 1),
            [350., 350.],
            &[(drawer, [4., 4.], [0.; 3])],
        )?);
    }
    for (i, row) in printed.small.as_chunks::<2>().0.iter().enumerate() {
        plates.push(plate(
            case,
            &format!("Small drawers row {}", i + 1),
            [350., 350.],
            &[(row[0], [4., 4.], [0.; 3]), (row[1], [222., 4.], [0.; 3])],
        )?);
    }
    plates.push(plate(
        case,
        "Rack on its back",
        [350., 350.],
        &[(printed.rack, [4., 4.], [-90., 0., 0.])],
    )?);
    let strips: Vec<_> = printed
        .strips
        .iter()
        .enumerate()
        .map(|(i, &id)| {
            (
                id,
                [4. + 16. * (i % 8) as f64, 4. + 120. * (i / 8) as f64],
                [0., if i % 2 == 0 { -90. } else { 90. }, 0.],
            )
        })
        .collect();
    plates.push(plate(case, "Sixteen rack strips", [256., 256.], &strips)?);
    plates.push(plate(
        case,
        "Single rack strip",
        [256., 256.],
        &[strips[0]],
    )?);
    let extras: Vec<_> = auxiliary
        .iter()
        .enumerate()
        .map(|(i, &id)| {
            if i < 4 {
                (id, [4. + 52. * i as f64, 4.], [0., 90., 0.])
            } else {
                (id, [4. + 26. * (i - 4) as f64, 220.], [90., 0., 0.])
            }
        })
        .collect();
    plates.push(plate(
        case,
        "Slide spacers and magnet stops",
        [256., 256.],
        &extras,
    )?);
    let (first, rest) = printed.coupons.split_at(2);
    for ((name, angle), id) in [("Rack fit coupon", -90.), ("Drawer fit coupon", 0.)]
        .into_iter()
        .zip(first)
    {
        plates.push(plate(
            case,
            name,
            [350., 350.],
            &[(*id, [4., 4.], [angle, 0., 0.])],
        )?);
    }
    let strips: Vec<_> = rest
        .iter()
        .enumerate()
        .map(|(i, &id)| (id, [4. + 22. * i as f64, 4.], [0., -90., 0.]))
        .collect();
    plates.push(plate(
        case,
        "Dovetail strip fit coupons",
        [256., 256.],
        &strips,
    )?);
    Ok((paperwork, plates))
}

fn geometry_exporter() -> Exporter {
    Exporter {
        compiler: Compiler {
            omit_display: true,
            ..Default::default()
        },
    }
}

type PlacementSpec = (EntityId, [f64; 2], [f64; 3]);

fn plate(
    case: &mut CaseBuilder,
    name: &str,
    bed_mm: [f64; 2],
    items: &[PlacementSpec],
) -> Result<EntityId, Error> {
    let all = resolve_parts(case.session.document(), &Scope::default())?;
    let placements = items
        .iter()
        .map(|&(part, position_mm, rotation_deg)| {
            let source = all
                .iter()
                .find(|candidate| candidate.reference.part == part)
                .ok_or_else(|| format!("No finished occurrence for {part} on {name}"))?
                .reference
                .clone();
            Ok::<_, Error>(PrintPlatePlacement {
                source,
                position_mm,
                rotation_deg,
            })
        })
        .collect::<Result<_, _>>()?;
    let plate = PrintPlateDefinition {
        bed_mm,
        placements,
        ..Default::default()
    };
    let id = EntityId::new();
    let prepared = prepare_print_plate(
        &case.session,
        case.session.revision(),
        actor(),
        id,
        name,
        &plate,
    )?;
    case.session.commit(prepared)?;
    Ok(id)
}

pub(super) fn verify_and_export(
    session: &Session,
    export_sets: &[EntityId],
    print_plates: &[EntityId],
    directory: &Path,
) -> Result<(), Error> {
    if export_sets.is_empty() || print_plates.is_empty() {
        return Err("The case needs saved fabrication sets and print plates".into());
    }
    std::fs::create_dir_all(directory)?;
    let document = session.document();
    let revision = session.revision();
    let exporter = geometry_exporter();
    let mut evidence = Vec::new();
    for &id in export_sets {
        eprintln!("Exporting {}", document.get(id).unwrap().name);
        let preview = exporter.preview(document, revision, id, &actor(), None)?;
        if !preview.can_run() {
            return Err(format!(
                "{} could not export: {:?}",
                document.get(id).unwrap().name,
                preview.recipes
            )
            .into());
        }
        let written = exporter.run(document, revision, &preview.key, directory, &actor(), None)?;
        evidence.push(serde_json::to_value(written)?);
    }
    for (i, &id) in print_plates.iter().enumerate() {
        eprintln!("Exporting {}", document.get(id).unwrap().name);
        let plate = read_print_plate(document, id)?;
        let preview = exporter.print_plate_preview(document, revision, &plate, &actor(), None)?;
        if !preview.can_write() {
            return Err(format!(
                "{} could not print: {:?}",
                document.get(id).unwrap().name,
                preview.blocked
            )
            .into());
        }
        let name = &document.get(id).unwrap().name;
        let path = directory.join(format!("{:02}-{name}.3mf", i + 1));
        let written = exporter.write_print_plate(
            document,
            revision,
            &plate,
            &preview.key,
            &path,
            &actor(),
            None,
        )?;
        let checked = read_three_mf(&std::fs::read(&path)?)?;
        if checked.unit_scale != 1.
            || checked.items != plate.placements.len()
            || checked.volume_mm3() <= 0.
        {
            return Err(format!("3MF readback failed for {name}").into());
        }
        evidence.push(serde_json::json!({"name": name, "items": checked.items, "triangles": checked.triangles, "receipt": written}));
    }
    std::fs::write(
        directory.join("aluminium-cut-list.csv"),
        "Part,Quantity,Stock,Blank length mm,Note\nSide frame,2,L 20x20x2 aluminium,995,Notch stations 70 / 380 / 615 / 925 from front end\nHandle cross-angle,1,L 20x20x2 aluminium,328,128 mm handle hole spacing\nRear cross-angle,2,L 20x20x2 aluminium,305,Top and bottom\nBottom-front angle,1,L 20x10x2 aluminium,305,Front trim\n",
    )?;
    std::fs::write(
        directory.join("native-readback.json"),
        serde_json::to_vec_pretty(&evidence)?,
    )?;
    Ok(())
}

#[cfg(not(test))]
pub(super) fn verify_motion(session: &Session, path: &Path) -> Result<(), Error> {
    use multicad_core::{
        document::EntityData,
        model::{MOTION_STUDY_TYPE, read_saved_motion_study},
    };
    use multicad_model::assembly_inspection::sample_assembly_range;

    let document = session.document();
    let compiler = Compiler {
        omit_display: true,
        ..Default::default()
    };
    let mut reports = Vec::new();
    for entity in document.entities() {
        if !matches!(&entity.data, EntityData::Extension { definition, .. }
            if definition.type_id == MOTION_STUDY_TYPE)
        {
            continue;
        }
        eprintln!("Checking {}", entity.name);
        let query = read_saved_motion_study(document, entity.id)?.query(session)?;
        let report = sample_assembly_range(&compiler, document, &query, None)?;
        reports.push(serde_json::json!({"study": entity.name, "report": report}));
        std::fs::write(path, serde_json::to_vec_pretty(&reports)?)?;
    }
    if reports.len() != 3 {
        return Err("The example needs three saved travel checks".into());
    }
    Ok(())
}
