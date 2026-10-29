//! Developed sheet DXF: an actual sheet part with up and down bends, a
//! relief and cutouts exports its blank at 1:1. The file is reopened with
//! the independent `dxf` reader and compared with the editable part's
//! unfolding: cut and inner contours, bend lines by direction, drill points
//! and the labels that name them. Edits regenerate the file; stale receipts
//! and failed unfolding export nothing.
use ::dxf::{Drawing, entities::EntityType, enums::Units};
use multicad_core::{
    commands::{Actor, Channel, Session},
    constraint_sketch::Dimension,
    document::{Document, Entity, EntityData, EntityId},
    exchange::{ErrorKind, LengthUnit},
    materials::*,
    model::{
        self, BendDirection, BodyOutput, Cutout, CutoutFace, CutoutShape, Flange, FlangeKind,
        PanelPlane, Relief, SheetBase, SheetDefinition, SheetRequest, angle, length,
    },
};
use multicad_model::{
    Compiler, DevelopedPattern,
    export::{
        Exporter,
        sheet_dxf::{SheetDxfLayers, SheetFlatProvider},
    },
};
use std::io::Cursor;

fn actor() -> Actor {
    Actor::local(Channel::Mcp)
}
fn near(a: f64, b: f64) {
    assert!((a - b).abs() <= 1e-6 * b.abs().max(1.), "{a} != {b}");
}
fn flange(edge: u32, direction: BendDirection, leg: f64) -> Flange {
    Flange {
        edge,
        kind: FlangeKind::Bend { angle: angle(90.) },
        direction,
        length: length(leg),
        inside_radius: None,
        start_inset: None,
        end_inset: None,
        relief: None,
    }
}
fn measured_area(poly: &::dxf::entities::LwPolyline) -> f64 {
    poly.vertices
        .iter()
        .enumerate()
        .map(|(i, a)| {
            let b = &poly.vertices[(i + 1) % poly.vertices.len()];
            let cross = (a.x * b.y - a.y * b.x) * 0.5;
            if a.bulge.abs() < 1e-12 {
                cross
            } else {
                let theta = 4. * a.bulge.atan();
                let chord = (a.x - b.x).hypot(a.y - b.y);
                let radius = chord / (2. * (theta.abs() * 0.5).sin());
                cross + radius * radius * (theta - theta.sin()) * 0.5
            }
        })
        .sum()
}
/// What the independent reader finds in a developed file.
#[derive(Default)]
struct Read {
    units: Option<Units>,
    outer: Vec<f64>,
    inner: Vec<f64>,
    bend_up: Vec<[[f64; 2]; 2]>,
    bend_down: Vec<[[f64; 2]; 2]>,
    drills: Vec<[f64; 2]>,
    labels: Vec<String>,
}
fn read(bytes: &[u8]) -> Read {
    let drawing = Drawing::load(&mut Cursor::new(bytes)).unwrap();
    let mut r = Read {
        units: Some(drawing.header.default_drawing_units),
        ..Default::default()
    };
    for e in drawing.entities() {
        let layer = e.common.layer.as_str();
        match &e.specific {
            EntityType::LwPolyline(poly) => {
                assert!(poly.is_closed());
                match layer {
                    "CUT" => r.outer.push(measured_area(poly)),
                    "CUT_INNER" => r.inner.push(measured_area(poly)),
                    other => panic!("polyline on {other}"),
                }
            }
            EntityType::Line(line) => {
                let l = [[line.p1.x, line.p1.y], [line.p2.x, line.p2.y]];
                match layer {
                    "BEND_UP" => r.bend_up.push(l),
                    "BEND_DOWN" => r.bend_down.push(l),
                    other => panic!("line on {other}"),
                }
            }
            EntityType::ModelPoint(p) => {
                assert_eq!(layer, "DRILL");
                r.drills.push([p.location.x, p.location.y]);
            }
            EntityType::Text(t) => {
                assert_eq!(layer, "LABEL");
                r.labels.push(t.value.clone());
            }
            other => panic!("unexpected entity {other:?}"),
        }
    }
    r
}
fn same_line(a: [[f64; 2]; 2], b: [[f64; 2]; 2], scale: f64) {
    for i in 0..2 {
        for j in 0..2 {
            near(a[i][j], b[i][j] * scale);
        }
    }
}
fn check(pattern: &DevelopedPattern, r: &Read, scale: f64) {
    let area: f64 = r.outer.iter().chain(&r.inner).sum();
    near(area, pattern.area_mm2 * scale * scale);
    assert_eq!(r.outer.len(), 1);
    assert_eq!(
        r.inner.len(),
        pattern.cut.loops.iter().filter(|l| !l.outer).count()
    );
    assert!(r.inner.iter().all(|a| *a < 0.));
    let (up, down): (Vec<_>, Vec<_>) = pattern
        .bends
        .iter()
        .partition(|b| b.record.direction == "up");
    assert_eq!((r.bend_up.len(), r.bend_down.len()), (up.len(), down.len()));
    for (line, b) in r.bend_up.iter().zip(&up) {
        same_line(*line, b.record.line, scale);
    }
    for (line, b) in r.bend_down.iter().zip(&down) {
        same_line(*line, b.record.line, scale);
    }
    assert_eq!(r.drills.len(), pattern.drills.len());
    for (p, d) in r.drills.iter().zip(&pattern.drills) {
        near(p[0], d.center[0] * scale);
        near(p[1], d.center[1] * scale);
    }
    for b in &pattern.bends {
        assert!(
            r.labels.iter().any(|l| l.starts_with(&format!(
                "{} {}",
                b.id,
                b.record.direction.to_uppercase()
            ))),
            "no label for {}: {:?}",
            b.id,
            r.labels
        );
    }
    for d in &pattern.drills {
        assert!(
            r.labels.iter().any(|l| l.starts_with(&d.id)),
            "{:?}",
            r.labels
        );
    }
}

#[test]
fn developed_sheet_dxf_reopens_with_cut_bend_and_drill_layers() {
    let mut s = Session::new(Document::new("Sheet DXF").unwrap()).unwrap();
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
    let mut side = flange(1, BendDirection::Up, 12.);
    side.start_inset = Some(length(4.));
    side.end_inset = Some(length(4.));
    side.relief = Some(Relief {
        width: length(3.),
        depth: length(4.),
    });
    let request = SheetRequest::new(
        "Bracket",
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
            flanges: vec![
                flange(0, BendDirection::Up, 20.),
                side,
                flange(2, BendDirection::Down, 15.),
            ],
            cutouts: vec![
                Cutout {
                    face: CutoutFace::Base,
                    shape: CutoutShape::Circle {
                        center: [length(50.), length(30.)],
                        diameter: length(8.),
                    },
                },
                Cutout {
                    face: CutoutFace::Flange { index: 0 },
                    shape: CutoutShape::Circle {
                        center: [length(50.), length(10.)],
                        diameter: length(5.),
                    },
                },
                Cutout {
                    face: CutoutFace::Base,
                    shape: CutoutShape::Rectangle {
                        center: [length(20.), length(30.)],
                        size: [length(10.), length(6.)],
                    },
                },
            ],
        },
    );
    let p = model::prepare_sheet(&s, s.revision(), actor(), &request).unwrap();
    Compiler::default()
        .compile(p.prepared.document(), s.revision() + 1)
        .unwrap();
    s.commit(p.prepared).unwrap();
    let exporter = Exporter::default();
    let layers = SheetDxfLayers::default();
    let file = exporter
        .developed_sheet_dxf(
            s.document(),
            s.revision(),
            request.body,
            LengthUnit::Millimeter,
            &layers,
            None,
        )
        .unwrap();
    file.check_current(s.document(), s.revision()).unwrap();
    assert_eq!(file.pattern.feature, request.feature);
    let r = read(&file.bytes);
    assert_eq!(r.units, Some(Units::Millimeters));
    check(&file.pattern, &r, 1.);
    // Up: flange 0 and the inset side flange; down: flange 2. Three openings,
    // two circular drill points.
    assert_eq!((r.bend_up.len(), r.bend_down.len()), (2, 1));
    assert_eq!(r.inner.len(), 3);
    assert_eq!(r.drills.len(), 2);
    // The relief notches are in the outer cut contour: it holds the base,
    // three developed flanges and the reliefs removed.
    let (t, rad, k) = (1.5, 1.5, 0.42);
    let ba = std::f64::consts::FRAC_PI_2 * (rad + k * t);
    near(
        r.outer[0],
        100. * 60. + 100. * (20. + ba) + 52. * (12. + ba) + 100. * (15. + ba) - 2. * 12.,
    );

    // The same blank in inches is scaled, not re-developed.
    let inches = exporter
        .developed_sheet_dxf(
            s.document(),
            s.revision(),
            request.body,
            LengthUnit::Inch,
            &layers,
            None,
        )
        .unwrap();
    let ri = read(&inches.bytes);
    assert_eq!(ri.units, Some(Units::Inches));
    check(&inches.pattern, &ri, 1. / 25.4);

    // A K-factor and a direction edit regenerate dimensions and layers.
    let mut sheet = model::read_sheet(s.document(), request.feature).unwrap();
    sheet.k_factor = Some(Dimension::Literal {
        value: 0.33,
        unit: "ratio".into(),
    });
    sheet.flanges[2].direction = BendDirection::Up;
    let p = model::prepare_sheet_edit(&s, s.revision(), actor(), request.feature, &sheet).unwrap();
    s.commit(p.prepared).unwrap();
    // The earlier file is stale now.
    let stale = file.check_current(s.document(), s.revision()).unwrap_err();
    assert_eq!(stale.kind, ErrorKind::Stale);
    let edited = exporter
        .developed_sheet_dxf(
            s.document(),
            s.revision(),
            request.body,
            LengthUnit::Millimeter,
            &layers,
            None,
        )
        .unwrap();
    let re = read(&edited.bytes);
    check(&edited.pattern, &re, 1.);
    assert_eq!((re.bend_up.len(), re.bend_down.len()), (3, 0));
    let ba2 = std::f64::consts::FRAC_PI_2 * (rad + 0.33 * t);
    near(re.bend_up[0][0][1], -ba2 / 2.);
    assert!(edited.pattern.size_mm[1] < file.pattern.size_mm[1]);

    // The developed-contour provider returns the same stamped cut loops.
    let stamped = exporter
        .developed_contours(
            s.document(),
            s.revision(),
            request.feature,
            &SheetFlatProvider,
            None,
        )
        .unwrap();
    assert_eq!(stamped.contours, edited.pattern.cut);

    // Failed unfolding exports nothing: touching flanges in a candidate,
    // a non-sheet Part, or a foreign layer set.
    let mut touching = model::read_sheet(s.document(), request.feature).unwrap();
    touching.flanges[1].start_inset = None;
    touching.flanges[1].end_inset = None;
    touching.flanges[1].relief = None;
    let p =
        model::prepare_sheet_edit(&s, s.revision(), actor(), request.feature, &touching).unwrap();
    let e = exporter
        .developed_sheet_dxf(
            p.prepared.document(),
            s.revision() + 1,
            request.body,
            LengthUnit::Millimeter,
            &layers,
            None,
        )
        .unwrap_err();
    assert!(e.message.contains("touch"), "{}", e.message);
    let e = exporter
        .developed_sheet_dxf(
            s.document(),
            s.revision(),
            stock_id,
            LengthUnit::Millimeter,
            &layers,
            None,
        )
        .unwrap_err();
    assert!(e.message.contains("No developed export"), "{}", e.message);
    let clash = SheetDxfLayers {
        drill: "cut".into(),
        ..Default::default()
    };
    assert!(
        exporter
            .developed_sheet_dxf(
                s.document(),
                s.revision(),
                request.body,
                LengthUnit::Millimeter,
                &clash,
                None
            )
            .is_err()
    );
}
