//! Flange guides and pre-checks for choosing flanges visually: the base
//! outline, its numbered edges and each flange's folded cross-section come
//! from the resolved sheet and agree with the native folded solid; the
//! flange pre-check reports every invalid bend and touching corner the
//! native evaluator refuses.
use multicad_core::{
    commands::{Actor, Channel, Session},
    document::{Document, Entity, EntityData, EntityId},
    materials::*,
    model::{
        self, BendDirection, BodyOutput, Flange, FlangeKind, PanelPlane, SheetBase,
        SheetDefinition, SheetRequest, angle, length,
    },
    parameters,
};
use multicad_model::{Compiler, Model};

const T: f64 = 2.;
const R: f64 = 3.;

fn actor() -> Actor {
    Actor::local(Channel::Ui)
}
fn near(a: f64, b: f64) {
    assert!((a - b).abs() <= 1e-6 * b.abs().max(1.), "{a} != {b}");
}
fn session() -> (Session, EntityId) {
    let mut s = Session::new(Document::new("Guides").unwrap()).unwrap();
    let steel = Entity::new(
        "DC01",
        EntityData::Material {
            density_kg_m3: None,
            color_rgba: [180, 184, 190, 255],
            section_hatch: None,
        },
    );
    let stock = LibraryDefinition::Stock(Stock {
        material: steel.id,
        specification: StockKind::Sheet {
            thickness_mm: T,
            length_mm: 2000.,
            width_mm: 1000.,
            inside_bend_radius_mm: R,
            k_factor: 0.4,
        },
    })
    .entity(EntityId::new(), "DC01 2.0")
    .unwrap();
    let id = stock.id;
    s.commit(prepare_library(&s, 0, actor(), &[steel, stock]).unwrap())
        .unwrap();
    (s, id)
}
fn flange(edge: u32, kind: FlangeKind, direction: BendDirection, leg: f64) -> Flange {
    Flange {
        edge,
        kind,
        direction,
        length: length(leg),
        inside_radius: None,
        start_inset: None,
        end_inset: None,
        relief: None,
    }
}
fn bend(edge: u32, deg: f64, leg: f64) -> Flange {
    flange(
        edge,
        FlangeKind::Bend { angle: angle(deg) },
        BendDirection::Up,
        leg,
    )
}
fn sheet(
    stock: EntityId,
    plane: PanelPlane,
    reverse: bool,
    flanges: Vec<Flange>,
) -> SheetDefinition {
    SheetDefinition {
        output: BodyOutput {
            body: EntityId::new(),
        },
        stock,
        base: SheetBase::Rectangle {
            plane,
            origin: [length(10.), length(20.), length(30.)],
            length: length(200.),
            width: length(100.),
        },
        reverse,
        inside_radius: None,
        k_factor: None,
        flanges,
        cutouts: vec![],
        fasteners: vec![],
    }
}
fn compile(s: &Session, definition: &SheetDefinition) -> Result<Model, String> {
    let p = model::prepare_sheet(
        s,
        s.revision(),
        actor(),
        &SheetRequest {
            feature: EntityId::new(),
            body: definition.output.body,
            name: "Panel".into(),
            sheet: definition.clone(),
        },
    )
    .map_err(|e| e.to_string())?;
    Compiler::default()
        .compile(p.prepared.document(), s.revision() + 1)
        .map_err(|e| e.message)
}
fn issues(s: &Session, definition: &SheetDefinition) -> Vec<model::FlangeIssue> {
    let parameters = parameters::evaluate(s.document()).unwrap();
    model::check_flanges(s.document(), &parameters, definition).unwrap()
}
/// World bounds of the base box and every guide's cross-section swept over
/// its span, with the mid-surface offset by half the thickness.
fn guide_bounds(s: &Session, definition: &SheetDefinition) -> ([f64; 3], [f64; 3]) {
    let parameters = parameters::evaluate(s.document()).unwrap();
    let resolved = model::resolve_sheet(s.document(), &parameters, definition).unwrap();
    let outline = model::sheet_outline(&resolved).unwrap();
    let mut min = [f64::INFINITY; 3];
    let mut max = [f64::NEG_INFINITY; 3];
    let mut grow = |p: [f64; 3]| {
        for i in 0..3 {
            min[i] = min[i].min(p[i]);
            max[i] = max[i].max(p[i]);
        }
    };
    for e in &outline.edges {
        for p in [e.start_mm, e.end_mm] {
            grow(p);
            grow(std::array::from_fn(|i| p[i] + outline.material[i] * T));
        }
    }
    for g in model::flange_guides(&resolved, &outline) {
        for w in g.section.windows(2) {
            let d = [w[1][0] - w[0][0], w[1][1] - w[0][1]];
            let l = d[0].hypot(d[1]);
            if l < 1e-12 {
                continue;
            }
            let n = [-d[1] / l * T / 2., d[0] / l * T / 2.];
            for p in w {
                for side in [-1., 1.] {
                    let q = [p[0] + side * n[0], p[1] + side * n[1]];
                    grow(g.world(&outline, q, 0.));
                    grow(g.world(&outline, q, g.width_mm()));
                }
            }
        }
    }
    (min, max)
}
fn native_bounds(m: &Model, definition: &SheetDefinition) -> multicad_kernel::Bounds {
    m.solid(m.snapshot().bodies[&definition.output.body])
        .unwrap()
        .bounds()
        .unwrap()
}

#[test]
fn outline_numbers_rectangle_edges_on_every_plane() {
    let (s, stock) = session();
    let parameters = parameters::evaluate(s.document()).unwrap();
    for (plane, reverse, material) in [
        (PanelPlane::Xy, false, [0., 0., 1.]),
        (PanelPlane::Xz, false, [0., -1., 0.]),
        (PanelPlane::Yz, true, [-1., 0., 0.]),
    ] {
        let d = sheet(stock, plane, reverse, vec![]);
        let r = model::resolve_sheet(s.document(), &parameters, &d).unwrap();
        let o = model::sheet_outline(&r).unwrap();
        assert_eq!(o.edges.len(), 4);
        assert_eq!(o.material, material);
        let (u, v) = plane.axes();
        let at = |a: f64, b: f64| -> [f64; 3] {
            std::array::from_fn(|i| [10., 20., 30.][i] + u[i] * a + v[i] * b)
        };
        // Edge 1 runs along u at v = 0 and faces −v; edge 2 at u = length faces +u.
        assert_eq!(o.edges[0].start_mm, at(0., 0.));
        assert_eq!(o.edges[0].end_mm, at(200., 0.));
        assert_eq!(o.edges[0].outward, v.map(|x| -x));
        assert_eq!(o.edges[1].start_mm, at(200., 0.));
        assert_eq!(o.edges[1].outward, u);
        assert_eq!(o.edges[2].outward, v);
        assert_eq!(o.edges[3].end_mm, at(0., 0.));
        assert_eq!(o.edges[3].outward, u.map(|x| -x));
        near(o.edges[1].length_mm, 100.);
        assert!(o.edges.iter().all(|e| e.straight));
    }
}

#[test]
fn guides_match_the_native_folded_solid() {
    let (s, stock) = session();
    let hem = flange(
        1,
        FlangeKind::Hem { gap: length(4.) },
        BendDirection::Up,
        15.,
    );
    let mut down = bend(0, 90., 20.);
    down.direction = BendDirection::Down;
    down.start_inset = Some(length(5.));
    down.end_inset = Some(length(5.));
    let jog = flange(
        2,
        FlangeKind::Jog {
            angle: angle(60.),
            offset: length(15.),
        },
        BendDirection::Up,
        10.,
    );
    let mut wide = bend(3, 120., 12.);
    wide.start_inset = Some(length(6.));
    wide.end_inset = Some(length(6.));
    for (plane, reverse, flanges) in [
        (PanelPlane::Xy, false, vec![bend(0, 90., 20.)]),
        (PanelPlane::Xz, true, vec![down, hem]),
        (PanelPlane::Yz, false, vec![jog, wide]),
    ] {
        let d = sheet(stock, plane, reverse, flanges);
        assert_eq!(issues(&s, &d), vec![], "{plane:?}");
        let m = compile(&s, &d).unwrap();
        let native = native_bounds(&m, &d);
        let (min, max) = guide_bounds(&s, &d);
        // The guide samples each bend every 7.5°, so its chords lie at most
        // r · (1 − cos 3.75°) inside the exact arc.
        for i in 0..3 {
            assert!(
                (min[i] - native.min[i]).abs() < 0.01,
                "{plane:?} {min:?} {native:?}"
            );
            assert!(
                (max[i] - native.max[i]).abs() < 0.01,
                "{plane:?} {max:?} {native:?}"
            );
        }
    }
    // The 90° bend on XY: the leg ends t + r + 20 above the base face and its
    // outer face lies r + t beyond the edge.
    let d = sheet(stock, PanelPlane::Xy, false, vec![bend(0, 90., 20.)]);
    let parameters = parameters::evaluate(s.document()).unwrap();
    let r = model::resolve_sheet(s.document(), &parameters, &d).unwrap();
    let o = model::sheet_outline(&r).unwrap();
    let g = &model::flange_guides(&r, &o)[0];
    near(g.leg_mm, 20.);
    near(g.inside_radius_mm, R);
    assert_eq!(g.bends.len(), 1);
    near(g.bends[0].1, 90.);
    let end = g.world(&o, g.leg[1], 0.);
    near(end[1], 20. - (R + T / 2.));
    near(end[2], 30. + R + T + 20.);
}

#[test]
fn pre_check_reports_what_native_evaluation_refuses() {
    let (s, stock) = session();
    // Neighboring flanges without a corner inset touch at the shared corner.
    let touching = sheet(
        stock,
        PanelPlane::Xy,
        false,
        vec![bend(0, 90., 20.), bend(1, 90., 20.)],
    );
    let found = issues(&s, &touching);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].edges, vec![0, 1]);
    assert_eq!(found[0].flanges, vec![0, 1]);
    assert!(
        found[0].message.contains("edges 1 and 2"),
        "{}",
        found[0].message
    );
    let Err(native) = compile(&s, &touching) else {
        panic!("touching flanges evaluated");
    };
    assert!(native.contains("Flanges 1 and 2"), "{native}");
    // The wrap-around corner between the last and the first edge counts too.
    let wrap = sheet(
        stock,
        PanelPlane::Xy,
        false,
        vec![bend(3, 90., 20.), bend(0, 90., 20.)],
    );
    assert_eq!(issues(&s, &wrap)[0].edges, vec![3, 0]);
    assert!(compile(&s, &wrap).is_err());
    // An inset on either side of the corner clears it, natively as well.
    let mut inset = touching.clone();
    inset.flanges[1].start_inset = Some(length(6.));
    assert_eq!(issues(&s, &inset), vec![]);
    compile(&s, &inset).unwrap();

    // Every invalid bend is reported at once, not only the first one.
    let mut bad = sheet(
        stock,
        PanelPlane::Xy,
        false,
        vec![
            bend(0, 200., 20.),
            flange(
                2,
                FlangeKind::Jog {
                    angle: angle(90.),
                    offset: length(3.),
                },
                BendDirection::Up,
                10.,
            ),
        ],
    );
    let found = issues(&s, &bad);
    assert_eq!(found.len(), 2, "{found:?}");
    assert!(
        found[0]
            .message
            .starts_with("Flange on edge 1: the bend angle")
    );
    assert!(
        found[1].message.contains("needs at least"),
        "{}",
        found[1].message
    );
    assert!(compile(&s, &bad).is_err());
    // Insets that leave no flange and reliefs wider than their inset.
    bad.flanges = vec![bend(1, 90., 20.)];
    bad.flanges[0].start_inset = Some(length(60.));
    bad.flanges[0].end_inset = Some(length(40.));
    assert!(issues(&s, &bad)[0].message.contains("leave no flange"));
    assert!(compile(&s, &bad).is_err());
    bad.flanges[0].end_inset = Some(length(2.));
    bad.flanges[0].relief = Some(model::Relief {
        width: length(3.),
        depth: length(4.),
    });
    assert!(issues(&s, &bad)[0].message.contains("wider than"));
    assert!(compile(&s, &bad).is_err());
}
