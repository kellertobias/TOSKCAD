use super::{CaseBuilder, Error, actor, length};
use multicad_core::{
    commands::Request,
    constraint_sketch::{Curve, CurveKind, source::Source},
    document::{Entity, EntityData, EntityId, Mutation},
    materials::{self, Grain, LibraryDefinition, Stock, StockKind},
    model::{
        self, BodyInput, BodyOutput, EdgeTreatmentParameters, EdgeTreatmentProfile,
        EdgeTreatmentRequest, GrainAxis, NumericTransform, PanelDefinition, PanelRequest,
        PanelSource, ProfileSource, TransformRequest,
    },
    references::{AnalyticGeometry, GeometryIntent},
    sketch::Datum,
    sketch_regions::{BoundaryIntent, RegionIntent},
};
use multicad_model::Compiler;

#[path = "panel_contours.rs"]
mod contours;
use contours::*;

const PLYWOOD_THICKNESS: f64 = 6.;
const SIDE_LEFT_X: f64 = 2.;
const SIDE_RIGHT_X: f64 = 337.;
const SIDE_RIGHT_OUTSIDE_X: f64 = SIDE_RIGHT_X + PLYWOOD_THICKNESS;
const FRONT_Y: f64 = 2.;
const BOTTOM_Z: f64 = 2.;
const RAIL_HOLE_RADIUS: f64 = 2.25;
const RACK_HOLE_RADIUS: f64 = 1.7;

pub(super) fn build(case: &mut CaseBuilder) -> Result<[EntityId; 7], Error> {
    let stock = plywood_stock(case)?;
    let side_profile = profile(
        case,
        "Side panels · shared waterjet profile",
        Datum::Yz,
        [SIDE_LEFT_X, FRONT_Y, BOTTOM_Z],
        SIDE_OUTLINE,
        SIDE_SLOTS,
        &[
            (RAIL_CENTERS, RAIL_HOLE_RADIUS),
            (RACK_CENTERS, RACK_HOLE_RADIUS),
        ],
    )?;
    let left = panel(case, stock, "Left side", side_profile.clone())?;
    let right = panel(case, stock, "Right side", side_profile)?;
    let right = move_right_side(case, right)?;
    countersinks(case, left.clone(), SIDE_LEFT_X)?;
    countersinks(case, right.clone(), SIDE_RIGHT_OUTSIDE_X)?;

    let mut create = |name: &str, datum, origin, outline, slots| -> Result<EntityId, Error> {
        let profile = profile(
            case,
            &format!("{name} · waterjet profile"),
            datum,
            origin,
            outline,
            slots,
            &[],
        )?;
        Ok(panel(case, stock, name, profile)?.body)
    };
    // XZ profiles extrude in −Y. The back and partition therefore start at
    // their rear faces. The partition's lower tabs start at the mid-floor.
    let bottom = create("Bottom", Datum::Xy, [2., 2., 2.], BOTTOM_OUTLINE, &[])?;
    let back = create("Back", Datum::Xz, [2., 233., 2.], BACK_OUTLINE, BACK_SLOTS)?;
    let mid = create(
        "Mid-floor",
        Datum::Xy,
        [2., 2., 109.],
        MID_FLOOR_OUTLINE,
        MID_FLOOR_SLOTS,
    )?;
    let partition = create(
        "Vertical partition",
        Datum::Xz,
        [2., 125., 109.],
        PARTITION_OUTLINE,
        &[],
    )?;
    let top = create("Front top", Datum::Xy, [2., 2., 302.], TOP_OUTLINE, &[])?;
    Ok([left.body, right.body, bottom, back, mid, partition, top])
}

fn plywood_stock(case: &mut CaseBuilder) -> Result<EntityId, Error> {
    let material = Entity::new(
        "Birch plywood",
        EntityData::Material {
            density_kg_m3: None,
            color_rgba: [216, 185, 138, 255],
            section_hatch: None,
        },
    );
    let mut stock = LibraryDefinition::Stock(Stock {
        material: material.id,
        specification: StockKind::Panel {
            thickness_mm: PLYWOOD_THICKNESS,
            length_mm: 1_250.,
            width_mm: 625.,
            grain: Grain::None,
        },
    })
    .entity(EntityId::new(), "Birch plywood · nominal 6 mm")?;
    stock.description = "Nominal CUT geometry. Establish kerf from a test cut and allow 0.1–0.2 mm on slots for measured stock if needed. Sheet size is planning stock.".into();
    let id = stock.id;
    let prepared = materials::prepare_library(
        &case.session,
        case.session.revision(),
        actor(),
        &[material, stock],
    )?;
    case.session.commit(prepared)?;
    Ok(id)
}

fn boundary(source: &mut Source, points: &[[f64; 2]]) -> BoundaryIntent {
    let ids: Vec<_> = points
        .iter()
        .map(|point| source.add_point(*point))
        .collect();
    BoundaryIntent {
        curves: ids
            .iter()
            .zip(ids.iter().cycle().skip(1))
            .map(|(&a, &b)| source.add_line(a, b))
            .collect(),
    }
}

fn profile(
    case: &mut CaseBuilder,
    name: &str,
    datum: Datum,
    origin: [f64; 3],
    outline: &[[f64; 2]],
    slots: &[[[f64; 2]; 2]],
    circular_holes: &[(&[[f64; 2]], f64)],
) -> Result<ProfileSource, Error> {
    let frame = datum.frame();
    let plane = Entity::new(
        format!("{name} plane"),
        EntityData::Plane {
            origin_mm: origin,
            normal: frame.n,
            anchor: None,
        },
    );
    let mut source = Source {
        u: frame.u,
        sketch: Default::default(),
    };
    let mut region = RegionIntent {
        outer: boundary(&mut source, outline),
        holes: Default::default(),
    };
    for &[[u0, v0], [u1, v1]] in slots {
        region.holes.insert(boundary(
            &mut source,
            &[[u0, v0], [u1, v0], [u1, v1], [u0, v1]],
        ));
    }
    for &(centers, radius_mm) in circular_holes {
        for &point in centers {
            let id = EntityId::new();
            let center = source.add_point(point);
            source.sketch.curves.push(Curve {
                id,
                construction: false,
                kind: CurveKind::Circle { center, radius_mm },
            });
            region.holes.insert(BoundaryIntent {
                curves: [id].into(),
            });
        }
    }
    let sketch = Entity::new(
        name,
        EntityData::Sketch {
            plane: plane.id,
            definition: source.definition()?,
            inputs: vec![],
        },
    );
    let id = sketch.id;
    case.session.apply(Request {
        expected_revision: case.session.revision(),
        actor: actor(),
        label: "Create plywood cut profile".into(),
        edits: vec![Mutation::Insert(plane), Mutation::Insert(sketch)],
    })?;
    Ok(ProfileSource { sketch: id, region })
}

fn panel(
    case: &mut CaseBuilder,
    stock: EntityId,
    name: &str,
    profile: ProfileSource,
) -> Result<BodyInput, Error> {
    let request = PanelRequest::new(
        name,
        PanelDefinition {
            output: BodyOutput {
                body: EntityId::nil(),
            },
            source: PanelSource::Profile { stock, profile },
            grain: GrainAxis::Free,
            edges: Default::default(),
            oversize: None,
        },
    );
    let prepared = model::prepare_panel(&case.session, case.session.revision(), actor(), &request)?;
    case.checked(prepared.prepared)?;
    Ok(BodyInput {
        body: request.body,
        producer: request.feature,
    })
}

fn move_right_side(case: &mut CaseBuilder, input: BodyInput) -> Result<BodyInput, Error> {
    let body = input.body;
    let mut request = TransformRequest::new(
        input,
        NumericTransform {
            translation: [SIDE_RIGHT_X - SIDE_LEFT_X, 0., 0.].map(length),
            ..Default::default()
        },
    );
    request.name = "Place right side from shared cut profile".into();
    let prepared =
        model::prepare_transform(&case.session, case.session.revision(), actor(), &request)?;
    case.checked(prepared.prepared)?;
    Ok(BodyInput {
        body,
        producer: request.feature,
    })
}

fn countersinks(case: &mut CaseBuilder, mut input: BodyInput, outside_x: f64) -> Result<(), Error> {
    for (centers, through, head) in [
        (RAIL_CENTERS, RAIL_HOLE_RADIUS, 4.2),
        (RACK_CENTERS, RACK_HOLE_RADIUS, 3.2),
    ] {
        let compiled = Compiler {
            omit_display: true,
            ..Default::default()
        }
        .compile(case.session.document(), case.session.revision())?;
        let topology = &compiled.snapshot().shapes[&input.producer].topology;
        let mut edges = Vec::new();
        for &[u, v] in centers {
            let center = [outside_x, FRONT_Y + u, BOTTOM_Z + v];
            let mut matches = topology.elements.iter().filter(|element| {
                matches!(element.geometry, Some(AnalyticGeometry::CircularArc { frame, radius_mm, sweep_rad, .. })
                    if (radius_mm - through).abs() < 1e-6
                    && (sweep_rad.abs() - std::f64::consts::TAU).abs() < 1e-6
                    && frame.origin_mm.into_iter().zip(center).all(|(actual, nominal)| (actual - nominal).abs() < 1e-6))
            });
            let Some(edge) = matches.next().filter(|_| matches.next().is_none()) else {
                return Err(format!("Expected one exterior plywood hole rim at {center:?}").into());
            };
            edges.push(GeometryIntent::capture(topology, edge.key)?);
        }
        let request = EdgeTreatmentRequest::new(
            input.clone(),
            EdgeTreatmentParameters {
                profile: EdgeTreatmentProfile::Chamfer,
                size: length(head - through),
            },
            edges,
        );
        let prepared = model::prepare_edge_treatment(
            &case.session,
            case.session.revision(),
            actor(),
            &request,
        )?;
        case.checked(prepared.prepared)?;
        input.producer = request.feature;
    }
    Ok(())
}
