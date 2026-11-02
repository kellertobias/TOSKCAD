use super::super::{self as model, BodyInput, PlanRectangleMm, common::number, source::invalid};
use super::{records::feature, *};
use crate::{
    document::{Document, DocumentError, EntityId},
    parameters::{EvaluatedParameters, Unit},
};
use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompoundReceiverKind {
    Wall,
    Slab,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompoundReceiverBase {
    pub feature: EntityId,
    pub kind: CompoundReceiverKind,
    /// Earlier compound sources in the frozen receiver history.
    pub previous: Vec<EntityId>,
}
pub fn receiver_base(
    document: &Document,
    input: &BodyInput,
) -> Result<CompoundReceiverBase, DocumentError> {
    input.resolve_producer(document)?;
    let mut producer = input.producer;
    let mut seen = BTreeSet::new();
    let mut previous = vec![];
    loop {
        if !seen.insert(producer) || seen.len() > 128 {
            return Err(invalid(
                "Compound receiver history is cyclic or exceeds 128 producers; repair it",
            ));
        }
        let (definition, _) = feature(document, producer)?;
        let kind = match definition.type_id.as_str() {
            model::WALL_TYPE => Some(CompoundReceiverKind::Wall),
            model::SLAB_TYPE => Some(CompoundReceiverKind::Slab),
            COMPOUND_CUTOUT_RECEIVER_TYPE => {
                let member = CompoundCutoutReceiverDefinition::from_definition(definition)?;
                if member.output.body != input.body || member.target.body != input.body {
                    return Err(invalid(
                        "Compound receiver history changes Part identity; repair it",
                    ));
                }
                previous.push(member.source);
                producer = member.target.producer;
                None
            }
            _ => {
                return Err(invalid(
                    "Compound cutouts require a wall/slab history or earlier compound cut; unrelated downstream operations require explicit repair",
                ));
            }
        };
        if let Some(kind) = kind {
            let body = match kind {
                CompoundReceiverKind::Wall => model::read_wall(document, producer)?.output.body,
                CompoundReceiverKind::Slab => model::read_slab(document, producer)?.output.body,
            };
            if body != input.body {
                return Err(invalid(
                    "Receiving Part does not own this wall/slab source; repair its target",
                ));
            }
            return Ok(CompoundReceiverBase {
                feature: producer,
                kind,
                previous,
            });
        }
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedCompoundBox {
    pub id: EntityId,
    pub rectangle: PlanRectangleMm,
    pub bottom_mm: f64,
    pub top_mm: f64,
}
impl ResolvedCompoundBox {
    pub fn corners(&self) -> [[f64; 3]; 8] {
        let xy = self.rectangle.corners();
        std::array::from_fn(|i| {
            [
                xy[i % 4][0],
                xy[i % 4][1],
                if i < 4 { self.bottom_mm } else { self.top_mm },
            ]
        })
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedCompoundReceiver {
    pub receiver: CompoundCutoutReceiver,
    pub base: CompoundReceiverBase,
    pub bottom_mm: f64,
    pub top_mm: f64,
}
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedCompoundCutout {
    pub boxes: Vec<ResolvedCompoundBox>,
    pub receivers: Vec<ResolvedCompoundReceiver>,
    pub shared_z_mm: [f64; 2],
}
fn range(parameters: &EvaluatedParameters, z: &CutoutZRange) -> Result<[f64; 2], DocumentError> {
    let value = [
        number(&z.bottom, Unit::Millimeter, parameters)?,
        number(&z.top, Unit::Millimeter, parameters)?,
    ];
    if value[1] <= value[0] {
        return Err(invalid("Cutout box top must be above its bottom"));
    }
    Ok(value)
}
fn receivers(
    document: &Document,
    parameters: &EvaluatedParameters,
    source: &CompoundCutoutDefinition,
) -> Result<Vec<ResolvedCompoundReceiver>, DocumentError> {
    source
        .receivers
        .iter()
        .map(|receiver| {
            let base = receiver_base(document, &receiver.target)?;
            let [bottom_mm, top_mm] = match base.kind {
                CompoundReceiverKind::Wall => {
                    let wall = model::resolve_wall(
                        document,
                        parameters,
                        &model::read_wall(document, base.feature)?,
                    )?;
                    [wall.base_z_mm, wall.base_z_mm + wall.height_mm]
                }
                CompoundReceiverKind::Slab => {
                    let slab = model::resolve_slab(
                        document,
                        parameters,
                        &model::read_slab(document, base.feature)?,
                    )?;
                    [slab.bottom_mm, slab.top_mm]
                }
            };
            Ok(ResolvedCompoundReceiver {
                receiver: receiver.clone(),
                base,
                bottom_mm,
                top_mm,
            })
        })
        .collect()
}
fn boxes(
    document: &Document,
    parameters: &EvaluatedParameters,
    source: &CompoundCutoutDefinition,
) -> Result<ResolvedCompoundCutout, DocumentError> {
    source.validate()?;
    let receivers = receivers(document, parameters, source)?;
    let shared = match &source.extent {
        CompoundCutoutExtent::ThroughReceivers => [
            receivers
                .iter()
                .map(|receiver| receiver.bottom_mm)
                .fold(f64::INFINITY, f64::min),
            receivers
                .iter()
                .map(|receiver| receiver.top_mm)
                .fold(f64::NEG_INFINITY, f64::max),
        ],
        CompoundCutoutExtent::Fixed(z) => range(parameters, z)?,
    };
    let boxes = source
        .boxes
        .iter()
        .map(|item| {
            let r = &item.rectangle;
            let rectangle = PlanRectangleMm {
                center: [
                    number(&r.center[0], Unit::Millimeter, parameters)?,
                    number(&r.center[1], Unit::Millimeter, parameters)?,
                ],
                width: number(&r.width, Unit::Millimeter, parameters)?,
                depth: number(&r.depth, Unit::Millimeter, parameters)?,
                rotation_deg: r.rotation_deg,
            };
            if rectangle.width <= 0. || rectangle.depth <= 0. {
                return Err(invalid("Cutout box width and depth must be positive"));
            }
            let [bottom_mm, top_mm] = item
                .vertical
                .as_ref()
                .map_or(Ok(shared), |z| range(parameters, z))?;
            Ok(ResolvedCompoundBox {
                id: item.id,
                rectangle,
                bottom_mm,
                top_mm,
            })
        })
        .collect::<Result<Vec<_>, DocumentError>>()?;
    Ok(ResolvedCompoundCutout {
        boxes,
        receivers,
        shared_z_mm: shared,
    })
}
pub fn resolve_compound_cutout(
    document: &Document,
    parameters: &EvaluatedParameters,
    source: &CompoundCutoutDefinition,
) -> Result<ResolvedCompoundCutout, DocumentError> {
    let ResolvedCompoundCutout {
        boxes,
        receivers,
        shared_z_mm,
    } = boxes(document, parameters, source)?;
    let levels = source
        .levels
        .as_ref()
        .map(|levels| {
            let a = model::resolve_level(document, parameters, levels.from)?.elevation_mm;
            let b = levels.to.map_or(Ok(a), |id| {
                model::resolve_level(document, parameters, id).map(|level| level.elevation_mm)
            })?;
            Ok::<_, DocumentError>([a.min(b), a.max(b)])
        })
        .transpose()?;
    for receiver in &receivers {
        if receiver.base.kind != CompoundReceiverKind::Slab {
            continue;
        }
        let definition = model::read_slab(document, receiver.base.feature)?;
        let slab = model::resolve_slab(document, parameters, &definition)?;
        if let Some([low, high]) = levels {
            let elevation =
                model::resolve_level(document, parameters, definition.level)?.elevation_mm;
            if definition.kind != model::SlabKind::Slab
                || elevation < low - 1e-6
                || elevation > high + 1e-6
            {
                return Err(invalid(
                    "Chosen slab is outside the explicit floor level range",
                ));
            }
        }
        let mut prior = vec![];
        for id in &receiver.base.previous {
            let previous = CompoundCutoutDefinition::from_definition(feature(document, *id)?.0)?;
            prior.extend(self::boxes(document, parameters, &previous)?.boxes);
        }
        for item in &boxes {
            if item.top_mm <= slab.bottom_mm + 1e-6 || item.bottom_mm >= slab.top_mm - 1e-6 {
                continue;
            }
            let corners = item.rectangle.corners();
            if !model::inside_outline(&slab.outline, &corners) {
                return Err(invalid(
                    "Compound cutout crosses or touches a slab edge; keep its cutting footprints strictly inside the slab",
                ));
            }
            if slab
                .holes
                .iter()
                .any(|hole| model::polygons_meet(&corners, &hole.corners))
                || prior.iter().any(|old| {
                    old.top_mm > slab.bottom_mm + 1e-6
                        && old.bottom_mm < slab.top_mm - 1e-6
                        && model::polygons_meet(&corners, &old.rectangle.corners())
                })
            {
                return Err(invalid(
                    "Compound cutout overlaps or touches an existing slab hole; edit the existing cutout instead",
                ));
            }
        }
    }
    Ok(ResolvedCompoundCutout {
        boxes,
        receivers,
        shared_z_mm,
    })
}

/// Source-derived box wireframes and receiver heights for editing an invalid
/// draft. This does not validate receiver edges, holes or native solids.
pub fn compound_cutout_layout(
    document: &Document,
    parameters: &EvaluatedParameters,
    source: &CompoundCutoutDefinition,
) -> Result<ResolvedCompoundCutout, DocumentError> {
    boxes(document, parameters, source)
}
