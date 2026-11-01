//! An equipment pose derives from explicit host identities and a labeled floor
//! datum. Missing hosts never fall back to the last displayed coordinates.
use super::*;
use crate::{
    constraint_sketch::Dimension,
    model::{self, PlacementBase, RigidPose},
    parameters::EvaluatedParameters,
};
pub const ATTACHMENT_TYPE: &str = "architecture.equipment_attachment";
pub const ATTACHMENT_CAPABILITY: &str = "architecture.equipment_attachment.1";
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WallSide {
    Left,
    Right,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum CeilingDatum {
    /// Underside of the next slab: floor-to-floor height minus slab thickness.
    LevelUnderside,
    /// An explicitly named ceiling height above the referenced floor.
    AboveFloor { height: Dimension },
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum HostPlacement {
    Floor {
        xy: [Dimension; 2],
        height: Dimension,
        rotation: Dimension,
    },
    Wall {
        wall: EntityId,
        segment: usize,
        point_count: usize,
        closed: bool,
        side: WallSide,
        along: Dimension,
        standoff: Dimension,
        height: Dimension,
        rotation: Dimension,
    },
    Ceiling {
        xy: [Dimension; 2],
        datum: CeilingDatum,
        drop: Dimension,
        rotation: Dimension,
    },
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Attachment {
    pub owner: EntityId,
    pub floor: EntityId,
    pub placement: HostPlacement,
}
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedAttachment {
    pub pose: RigidPose,
    pub floor_elevation_mm: f64,
    pub anchor_above_floor_mm: f64,
    pub ceiling_above_floor_mm: Option<f64>,
}
impl HostPlacement {
    fn dimensions(&self) -> Vec<(&Dimension, Unit)> {
        let mut dimensions = vec![];
        match self {
            Self::Floor {
                xy,
                height,
                rotation,
            } => {
                dimensions.extend(xy.iter().map(|d| (d, Unit::Millimeter)));
                dimensions.push((height, Unit::Millimeter));
                dimensions.push((rotation, Unit::Degree));
            }
            Self::Wall {
                along,
                standoff,
                height,
                rotation,
                ..
            } => {
                dimensions.extend([along, standoff, height].map(|d| (d, Unit::Millimeter)));
                dimensions.push((rotation, Unit::Degree));
            }
            Self::Ceiling {
                xy,
                datum,
                drop,
                rotation,
            } => {
                dimensions.extend(xy.iter().map(|d| (d, Unit::Millimeter)));
                if let CeilingDatum::AboveFloor { height } = datum {
                    dimensions.push((height, Unit::Millimeter));
                }
                dimensions.push((drop, Unit::Millimeter));
                dimensions.push((rotation, Unit::Degree));
            }
        }
        dimensions
    }
}
impl Attachment {
    /// Convert a world-space rigid edit back into this host's local intent.
    /// Host changes later therefore rotate/translate the edited object with it.
    pub fn transformed(
        &self,
        document: &Document,
        parameters: &EvaluatedParameters,
        movement: &model::NumericTransform,
    ) -> Result<Self, DocumentError> {
        let old = self.resolve(document, parameters)?.pose;
        let new = old.then(movement.resolve(parameters)?)?;
        if new.rotation[2][2] < 1. - 1e-8 {
            return Err(invalid(
                "Hosted equipment must remain upright; edit its host or mounting orientation",
            ));
        }
        let delta =
            std::array::from_fn::<_, 3, _>(|i| new.translation_mm[i] - old.translation_mm[i]);
        let yaw = |p: model::RigidPose| p.rotation[1][0].atan2(p.rotation[0][0]).to_degrees();
        let mm = |d: &Dimension| {
            d.value_in(Unit::Millimeter, Some(parameters))
                .map_err(invalid)
        };
        let deg = |d: &Dimension| d.value_in(Unit::Degree, Some(parameters)).map_err(invalid);
        let mut moved = self.clone();
        match &mut moved.placement {
            HostPlacement::Floor {
                xy,
                height,
                rotation,
            } => {
                *xy = [
                    model::length(new.translation_mm[0]),
                    model::length(new.translation_mm[1]),
                ];
                *height = model::length(mm(height)? + delta[2]);
                *rotation = model::angle(yaw(new));
            }
            HostPlacement::Ceiling {
                xy, drop, rotation, ..
            } => {
                *xy = [
                    model::length(new.translation_mm[0]),
                    model::length(new.translation_mm[1]),
                ];
                *drop = model::length(mm(drop)? - delta[2]);
                *rotation = model::angle(yaw(new));
            }
            HostPlacement::Wall {
                wall,
                segment,
                side,
                along,
                standoff,
                height,
                rotation,
                ..
            } => {
                let source = model::read_wall(document, *wall)?;
                let resolved = model::resolve_wall(document, parameters, &source)?;
                let a = resolved.points[*segment];
                let b = resolved.points[(*segment + 1) % resolved.points.len()];
                let length = (b[0] - a[0]).hypot(b[1] - a[1]);
                let t = [(b[0] - a[0]) / length, (b[1] - a[1]) / length];
                let normal_delta = -t[1] * delta[0] + t[0] * delta[1];
                *along = model::length(mm(along)? + t[0] * delta[0] + t[1] * delta[1]);
                let offset =
                    mm(standoff)? + normal_delta * if *side == WallSide::Left { 1. } else { -1. };
                if offset < -1e-6 {
                    return Err(invalid(
                        "Move crosses the attached wall face; explicitly choose the other wall side",
                    ));
                }
                *standoff = model::length(offset.max(0.));
                *height = model::length(mm(height)? + delta[2]);
                *rotation = model::angle(deg(rotation)? + yaw(new) - yaw(old));
            }
        }
        moved.resolve(document, parameters)?;
        Ok(moved)
    }
    pub fn definition(&self) -> Result<Definition, DocumentError> {
        if self.owner.is_nil() || self.floor.is_nil() || self.owner == self.floor {
            return Err(invalid(
                "Equipment attachment requires distinct object and floor identities",
            ));
        }
        if let HostPlacement::Wall {
            wall,
            segment,
            point_count,
            closed,
            ..
        } = &self.placement
        {
            let segments = if *closed {
                *point_count
            } else {
                point_count.saturating_sub(1)
            };
            if wall.is_nil()
                || *wall == self.owner
                || !(2..=model::MAX_WALL_POINTS).contains(point_count)
                || *segment >= segments
            {
                return Err(invalid(
                    "Choose a valid wall path segment for the equipment attachment",
                ));
            }
        }
        for (dimension, unit) in self.placement.dimensions() {
            if let Dimension::Literal { value, .. } = dimension
                && (!value.is_finite() || value.abs() > 1e9)
            {
                return Err(invalid(
                    "Equipment attachment values must be finite and bounded",
                ));
            }
            if !Unit::parse(dimension.unit()).is_some_and(|u| u.dimension() == unit.dimension()) {
                return Err(invalid(
                    "Equipment attachment dimensions have the wrong units",
                ));
            }
            if let Some(expression) = dimension.expression() {
                expression.validate().map_err(|e| invalid(e.to_string()))?;
            }
        }
        encode(ATTACHMENT_TYPE, self)
    }
    pub fn from_definition(source: &Definition) -> Result<Self, DocumentError> {
        let value: Self = decode(source, ATTACHMENT_TYPE)?;
        value.definition()?;
        Ok(value)
    }
    pub fn links(&self) -> Vec<Link> {
        let mut parameters = std::collections::BTreeSet::new();
        for (d, _) in self.placement.dimensions() {
            if let Some(e) = d.expression() {
                parameters.extend(e.bindings.iter().map(|b| b.target));
            }
        }
        let mut links = parameters
            .into_iter()
            .map(|id| Link::new(id, format!("parameter/{id}")))
            .collect::<Vec<_>>();
        links.push(Link::new(self.floor, "floor"));
        if let HostPlacement::Wall { wall, .. } = self.placement {
            links.push(Link::new(wall, "wall"));
        }
        links
    }
    pub fn entity(&self, id: EntityId) -> Result<Entity, DocumentError> {
        Ok(super::entity(
            id,
            "Equipment attachment",
            self.definition()?,
            self.links(),
        ))
    }
    pub fn resolve(
        &self,
        document: &Document,
        parameters: &EvaluatedParameters,
    ) -> Result<ResolvedAttachment, DocumentError> {
        self.definition()?;
        if document.get(self.floor).is_none() {
            return Err(invalid(
                "Equipment floor datum is missing; choose a replacement floor before rebuilding",
            ));
        }
        let floor = model::resolve_level(document, parameters, self.floor)?;
        let mm = |d: &Dimension| {
            d.value_in(Unit::Millimeter, Some(parameters))
                .map_err(invalid)
        };
        let degrees = |d: &Dimension| d.value_in(Unit::Degree, Some(parameters)).map_err(invalid);
        let mut ceiling_above_floor_mm = None;
        let (xy, above, rotation) = match &self.placement {
            HostPlacement::Floor {
                xy,
                height,
                rotation,
            } => ([mm(&xy[0])?, mm(&xy[1])?], mm(height)?, degrees(rotation)?),
            HostPlacement::Ceiling {
                xy,
                datum,
                drop,
                rotation,
            } => {
                let ceiling = match datum {
                    CeilingDatum::LevelUnderside => floor.height_mm - floor.slab_mm,
                    CeilingDatum::AboveFloor { height } => mm(height)?,
                };
                let drop = mm(drop)?;
                if ceiling <= 0. || drop < 0. || drop > ceiling {
                    return Err(invalid(
                        "Ceiling datum must be above the floor, with a drop between zero and the ceiling height",
                    ));
                }
                ceiling_above_floor_mm = Some(ceiling);
                (
                    [mm(&xy[0])?, mm(&xy[1])?],
                    ceiling - drop,
                    degrees(rotation)?,
                )
            }
            HostPlacement::Wall {
                wall,
                segment,
                point_count,
                closed,
                side,
                along,
                standoff,
                height,
                rotation,
            } => {
                if document.get(*wall).is_none() {
                    return Err(invalid(
                        "Equipment wall host is missing; explicitly reattach it to a current wall",
                    ));
                }
                let source = model::read_wall(document, *wall)?;
                if source.path.len() != *point_count || source.closed != *closed {
                    return Err(invalid(
                        "Wall path topology changed; explicitly reattach equipment to a current segment",
                    ));
                }
                let resolved = model::resolve_wall(document, parameters, &source)?;
                let a = resolved.points[*segment];
                let b = resolved.points[(*segment + 1) % resolved.points.len()];
                let d = [b[0] - a[0], b[1] - a[1]];
                let length = d[0].hypot(d[1]);
                let along = mm(along)?;
                let standoff = mm(standoff)?;
                let above = mm(height)?;
                if length < 1e-6 || along < 0. || along > length || standoff < 0. {
                    return Err(invalid(
                        "Wall attachment offset must lie on its segment and standoff must be nonnegative",
                    ));
                }
                let world_z = floor.elevation_mm + above;
                if world_z < resolved.base_z_mm - 1e-6
                    || world_z > resolved.base_z_mm + resolved.height_mm + 1e-6
                {
                    return Err(invalid(
                        "Equipment anchor is outside the wall height; repair its mounting height or floor datum",
                    ));
                }
                let tangent = [d[0] / length, d[1] / length];
                let normal = [-tangent[1], tangent[0]];
                let half = resolved.thickness_mm / 2.;
                let (left, right) = match resolved.justification {
                    model::Justification::Center => (half, -half),
                    model::Justification::Left => (resolved.thickness_mm, 0.),
                    model::Justification::Right => (0., -resolved.thickness_mm),
                };
                let offset = match side {
                    WallSide::Left => left + standoff,
                    WallSide::Right => right - standoff,
                };
                let xy = std::array::from_fn(|i| a[i] + tangent[i] * along + normal[i] * offset);
                // Object fronts are local −Y. The left face reverses local X so the
                // upright frame stays right-handed while its front points out of the wall.
                let host_angle = tangent[1].atan2(tangent[0]).to_degrees()
                    + if *side == WallSide::Left { 180. } else { 0. };
                (xy, above, host_angle + degrees(rotation)?)
            }
        };
        let pose = model::NumericTransform {
            translation: [
                model::length(xy[0]),
                model::length(xy[1]),
                model::length(floor.elevation_mm + above),
            ],
            angle: model::angle(rotation),
            ..Default::default()
        }
        .resolve(parameters)?;
        Ok(ResolvedAttachment {
            pose,
            floor_elevation_mm: floor.elevation_mm,
            anchor_above_floor_mm: above,
            ceiling_above_floor_mm,
        })
    }
}
pub fn read_attachment(document: &Document, id: EntityId) -> Result<Attachment, DocumentError> {
    let Some(EntityData::Extension { definition, inputs }) = document.get(id).map(|e| &e.data)
    else {
        return Err(invalid(
            "Equipment attachment is missing or has the wrong entity kind",
        ));
    };
    let value = Attachment::from_definition(definition)?;
    links_match(inputs, &value.links())?;
    if document
        .get(value.owner)
        .is_some_and(|e| !matches!(e.data, EntityData::Instance { .. }))
    {
        return Err(invalid("Equipment attachment owner must be an Instance"));
    }
    if document.get(value.floor).is_some_and(|e|!matches!(&e.data,EntityData::Extension{definition,..} if definition.type_id==model::LEVEL_KIND)){return Err(invalid("Equipment floor reference must identify a level"));}
    if let HostPlacement::Wall{wall,..}=value.placement&&document.get(wall).is_some_and(|e|!matches!(&e.data,EntityData::Feature{definition,..} if definition.type_id==model::WALL_TYPE)){return Err(invalid("Equipment wall reference must identify a wall feature"));}
    Ok(value)
}
/// This is an absolute hosted source at the beginning of a rigid chain.
pub(crate) fn attachment_base() -> PlacementBase {
    PlacementBase::Literal {
        pose: RigidPose::default(),
    }
}
