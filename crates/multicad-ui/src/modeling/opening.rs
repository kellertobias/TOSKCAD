//! Room tools: place, edit or remove one door, window or vertical cutout in
//! an existing wall. The wall segment and offset come from a click on the
//! wall in the viewport or from the Inspector; the placement is checked live
//! by the shared planner, previewed natively and applied as one transaction
//! on the wall feature (`model::prepare_wall_opening`).
//!
//! A vertical cutout chooses what it cuts: the wall, the floor slabs under
//! it on a range of levels (a shaft or floor penetration), or both. Those
//! go through `model::prepare_vertical_cutout`, one transaction on the wall
//! and every cut slab; a slab-only cutout is placed by a plan rectangle or a
//! click on a slab.
use super::{
    FormStamp, PreviewState, TerminationViewport,
    dimension::DimensionField,
    form::{self, FormAction, ModelEvent, ModelRequest},
};
use crate::{
    controls, egui,
    viewport::{PickFilter, PickOptions},
};
use multicad_core::{
    commands::{Actor, Session},
    constraint_sketch::Dimension,
    document::{Document, EntityData, EntityId},
    model::{
        self, CutoutChange, CutoutPlacement, Hinge, ModelPrepared, NewType, NewTypeKind, Opening,
        OpeningKind, OpeningTypeDefinition, PlanRectangle, Swing, VerticalCutout, WallFace,
        WallOpeningChange, length,
    },
};

/// Which Room tool the panel serves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RoomOpening {
    Door,
    Window,
    Cutout,
}
impl RoomOpening {
    fn noun(self) -> &'static str {
        match self {
            Self::Door => "door",
            Self::Window => "window",
            Self::Cutout => "vertical cutout",
        }
    }
    fn title(self) -> &'static str {
        match self {
            Self::Door => "Door",
            Self::Window => "Window",
            Self::Cutout => "Vertical cutout",
        }
    }
    fn matches(self, kind: &OpeningKind) -> bool {
        match self {
            Self::Door => matches!(kind, OpeningKind::Door { .. }),
            Self::Window => matches!(kind, OpeningKind::Window),
            Self::Cutout => false,
        }
    }
}

/// The typed opening change sent to native preview and Apply.
#[derive(Clone, Debug)]
pub struct OpeningCommand {
    /// The feature the edit is anchored to: the wall, or the first cut slab
    /// of a slab-only cutout.
    pub anchor: EntityId,
    pub new_types: Vec<NewType>,
    pub change: WallOpeningChange,
    /// A cutout that cuts slabs (or leaves them) uses the shared
    /// wall-and-slab operation instead of `change`.
    pub cutout: Option<CutoutChange>,
}
impl OpeningCommand {
    pub(super) fn prepare(&self, session: &Session, actor: Actor) -> Result<ModelPrepared, String> {
        if let Some(cutout) = &self.cutout {
            let mut prepared =
                model::prepare_vertical_cutout(session, session.revision(), actor, cutout)
                    .map_err(|e| e.to_string())?;
            // The window proves the anchor's rebuild; the other cut slabs are
            // rebuilt by the same native evaluation of the whole document.
            prepared.outputs.retain(|o| o.feature == self.anchor);
            return Ok(prepared);
        }
        model::prepare_wall_opening(
            session,
            session.revision(),
            actor,
            self.anchor,
            &self.new_types,
            &self.change,
        )
        .map_err(|e| e.to_string())
    }
}

/// The wall feature producing a Part, if any.
pub(super) fn wall_of_part(document: &Document, part: EntityId) -> Option<EntityId> {
    match document.get(part).map(|e| &e.data) {
        Some(EntityData::Part { features, .. }) => features.iter().copied().find(|f| {
            matches!(document.get(*f).map(|e| &e.data),
                Some(EntityData::Feature { definition, .. }) if definition.type_id == model::WALL_TYPE)
        }),
        Some(EntityData::Feature { definition, .. }) if definition.type_id == model::WALL_TYPE => {
            Some(part)
        }
        _ => None,
    }
}
fn literal(d: &Dimension) -> Option<f64> {
    match d {
        Dimension::Literal { value, .. } => Some(*value),
        Dimension::Expression { .. } => None,
    }
}
fn text(d: &Dimension) -> String {
    match d {
        Dimension::Literal { value, .. } => format!("{value}"),
        Dimension::Expression { expression } => expression.text.clone(),
    }
}

/// A hovered or picked wall location in world coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Hover {
    wall: EntityId,
    segment: usize,
    offset_mm: f64,
}

pub(super) struct OpeningPanel {
    stamp: FormStamp,
    kind: RoomOpening,
    wall: Option<EntityId>,
    /// The existing opening or cutout being edited; `None` places a new one.
    editing: Option<EntityId>,
    remove: bool,
    new_id: EntityId,
    /// One-based segment number shown to the user.
    segment: usize,
    offset: DimensionField,
    opening_type: Option<EntityId>,
    new_type: EntityId,
    hinge: Hinge,
    swing: Swing,
    sized: bool,
    width: DimensionField,
    height: DimensionField,
    sill: DimensionField,
    through: bool,
    depth: DimensionField,
    face: WallFace,
    /// What a vertical cutout cuts.
    cut_wall: bool,
    cut_slabs: bool,
    /// Slabs are cut on levels from `from_level` up to `to_level` (or that
    /// level only).
    from_level: Option<EntityId>,
    to_level: Option<EntityId>,
    /// The plan rectangle of a slab-only cutout.
    plan_center: [DimensionField; 2],
    plan_depth: DimensionField,
    plan_rotation_deg: f64,
    picking: bool,
    hover: Option<Hover>,
    slab_hover: Option<[f64; 3]>,
    diagnostic: Option<(FormStamp, Result<String, String>)>,
    error: Option<String>,
}
impl OpeningPanel {
    pub fn new(
        document: &Document,
        kind: RoomOpening,
        wall: Option<EntityId>,
    ) -> Result<Self, String> {
        let (w, h, s) = match kind {
            RoomOpening::Door => (900., 2100., 0.),
            RoomOpening::Window => (1200., 1200., 900.),
            RoomOpening::Cutout => (300., 2500., 0.),
        };
        let opening_type = model::opening_types(document)
            .find(|e| model::read_opening_type(document, e.id).is_ok_and(|t| kind.matches(&t.kind)))
            .map(|e| e.id);
        let wall = wall.or_else(|| model::walls(document).next().map(|e| e.id));
        let from_level = wall
            .and_then(|w| model::read_wall(document, w).ok())
            .and_then(|w| w.level)
            .or_else(|| model::levels(document).next().map(|e| e.id));
        Ok(Self {
            stamp: FormStamp::new(),
            kind,
            wall,
            editing: None,
            remove: false,
            new_id: EntityId::new(),
            segment: 1,
            offset: DimensionField::from_dimension(&length(500.), "mm")?,
            opening_type,
            new_type: EntityId::new(),
            hinge: Hinge::Start,
            swing: Swing::Left,
            sized: false,
            width: DimensionField::from_dimension(&length(w), "mm")?,
            height: DimensionField::from_dimension(&length(h), "mm")?,
            sill: DimensionField::from_dimension(&length(s), "mm")?,
            through: true,
            depth: DimensionField::from_dimension(&length(100.), "mm")?,
            face: WallFace::Left,
            cut_wall: wall.is_some(),
            cut_slabs: false,
            from_level,
            to_level: None,
            plan_center: [
                DimensionField::from_dimension(&length(1000.), "mm")?,
                DimensionField::from_dimension(&length(1000.), "mm")?,
            ],
            plan_depth: DimensionField::from_dimension(&length(600.), "mm")?,
            plan_rotation_deg: 0.,
            picking: true,
            hover: None,
            slab_hover: None,
            diagnostic: None,
            error: None,
        })
    }
    /// Reopen an existing door, window or cutout of `wall` for editing.
    pub fn existing(document: &Document, wall: EntityId, id: EntityId) -> Result<Self, String> {
        let source = model::read_wall(document, wall).map_err(|e| e.to_string())?;
        if let Some(o) = source.openings.iter().find(|o| o.id == id) {
            let t =
                model::read_opening_type(document, o.opening_type).map_err(|e| e.to_string())?;
            let kind = if matches!(t.kind, OpeningKind::Window) {
                RoomOpening::Window
            } else {
                RoomOpening::Door
            };
            let mut panel = Self::new(document, kind, Some(wall))?;
            panel.load_opening(document, o)?;
            return Ok(panel);
        }
        if !source.cutouts.iter().any(|c| c.id == id) {
            return Err("This wall has no opening with that identity".into());
        }
        Self::existing_cutout(document, id)
    }
    /// Reopen a vertical cutout, wherever it cuts: its wall and/or slabs.
    pub fn existing_cutout(document: &Document, id: EntityId) -> Result<Self, String> {
        let members = model::cutout_members(document, id);
        if members.is_empty() {
            return Err("No wall or slab has a vertical cutout with that identity".into());
        }
        let wall = members.wall.as_ref().map(|(w, _)| *w);
        let mut panel = Self::new(document, RoomOpening::Cutout, wall)?;
        panel.editing = Some(id);
        panel.picking = false;
        panel.cut_wall = members.wall.is_some();
        if let Some((_, c)) = &members.wall {
            panel.load_cutout(c)?;
        }
        panel.cut_slabs = !members.slabs.is_empty();
        if panel.cut_slabs {
            let parameters =
                multicad_core::parameters::evaluate(document).map_err(|e| e.to_string())?;
            let mut levels: Vec<(f64, EntityId)> = members
                .slabs
                .iter()
                .filter_map(|(s, _)| model::read_slab(document, *s).ok())
                .filter_map(|s| {
                    model::resolve_level(document, &parameters, s.level)
                        .ok()
                        .map(|l| (l.elevation_mm, s.level))
                })
                .collect();
            levels.sort_by(|a, b| a.0.total_cmp(&b.0));
            panel.from_level = levels.first().map(|l| l.1);
            panel.to_level = levels
                .last()
                .map(|l| l.1)
                .filter(|l| Some(*l) != panel.from_level);
        }
        if let (None, Some((_, hole))) = (&members.wall, members.slabs.first()) {
            panel.plan_center = [
                DimensionField::from_dimension(&hole.center[0], "mm")?,
                DimensionField::from_dimension(&hole.center[1], "mm")?,
            ];
            panel.width = DimensionField::from_dimension(&hole.width, "mm")?;
            panel.plan_depth = DimensionField::from_dimension(&hole.depth, "mm")?;
            panel.plan_rotation_deg = hole.rotation_deg;
        }
        Ok(panel)
    }
    fn load_opening(&mut self, document: &Document, o: &Opening) -> Result<(), String> {
        let t = model::read_opening_type(document, o.opening_type).map_err(|e| e.to_string())?;
        self.editing = Some(o.id);
        self.picking = false;
        self.segment = o.segment + 1;
        self.offset = DimensionField::from_dimension(&o.offset, "mm")?;
        self.opening_type = Some(o.opening_type);
        self.sized = o.width.is_some() || o.height.is_some() || o.sill.is_some();
        self.width = DimensionField::from_dimension(o.width.as_ref().unwrap_or(&t.width), "mm")?;
        self.height = DimensionField::from_dimension(o.height.as_ref().unwrap_or(&t.height), "mm")?;
        self.sill = DimensionField::from_dimension(o.sill.as_ref().unwrap_or(&t.sill), "mm")?;
        Ok(())
    }
    fn load_cutout(&mut self, c: &VerticalCutout) -> Result<(), String> {
        self.editing = Some(c.id);
        self.picking = false;
        self.cut_wall = true;
        self.segment = c.segment + 1;
        self.offset = DimensionField::from_dimension(&c.offset, "mm")?;
        self.width = DimensionField::from_dimension(&c.width, "mm")?;
        self.through = c.depth.is_none();
        if let Some(d) = &c.depth {
            self.depth = DimensionField::from_dimension(d, "mm")?;
        }
        self.face = c.face;
        Ok(())
    }
    pub fn stamp(&self) -> FormStamp {
        self.stamp
    }
    pub fn label(&self) -> &'static str {
        self.kind.title()
    }
    pub fn picking(&self) -> bool {
        self.picking && self.editing.is_none()
    }
    fn changed(&mut self) {
        self.stamp.changed();
        self.error = None;
        self.diagnostic = None;
    }
    /// Width used to center a click: the literal override or type width.
    fn width_mm(&self, document: &Document) -> f64 {
        let type_width = || {
            self.opening_type
                .and_then(|t| model::read_opening_type(document, t).ok())
                .and_then(|t| literal(&t.width))
        };
        let own = || self.width.resolve(document).ok().and_then(|d| literal(&d));
        let value = match self.kind {
            RoomOpening::Cutout => own(),
            _ if self.opening_type.is_some() && !self.sized => type_width(),
            _ => own(),
        };
        value.unwrap_or(0.)
    }
    /// Whether this cutout goes through the shared wall-and-slab operation:
    /// it cuts slabs, cuts no wall, or is an existing cutout that cut slabs.
    fn slab_mode(&self, document: &Document) -> bool {
        self.kind == RoomOpening::Cutout
            && (self.cut_slabs
                || !self.cut_wall
                || self.editing.is_some_and(|id| {
                    !model::cutout_members(document, id).slabs.is_empty()
                        || model::cutout_members(document, id).wall.is_none()
                }))
    }
    fn cutout_request(&self, document: &Document) -> Result<OpeningCommand, String> {
        let id = self.editing.unwrap_or(self.new_id);
        let current = model::cutout_members(document, id);
        if self.remove {
            let anchor = current
                .wall
                .as_ref()
                .map(|(w, _)| *w)
                .or_else(|| current.slabs.first().map(|(s, _)| *s))
                .ok_or("Only an existing cutout can be removed")?;
            return Ok(OpeningCommand {
                anchor,
                new_types: vec![],
                change: WallOpeningChange::Remove(id),
                cutout: Some(CutoutChange::Remove(id)),
            });
        }
        if !self.cut_wall && !self.cut_slabs {
            return Err("Choose what the cutout cuts: the wall, floor slabs or both".into());
        }
        let wall = if self.cut_wall {
            let wall = self
                .wall
                .ok_or("Draw a wall first, then pick it in the viewport or choose it here")?;
            let segment = self
                .segment
                .checked_sub(1)
                .ok_or("Segments are numbered from 1")?;
            Some((
                wall,
                VerticalCutout {
                    id,
                    segment,
                    offset: self.offset.resolve(document)?,
                    width: self.width.resolve(document)?,
                    depth: (!self.through)
                        .then(|| self.depth.resolve(document))
                        .transpose()?,
                    face: self.face,
                },
            ))
        } else {
            None
        };
        let plan = if self.cut_wall {
            None
        } else {
            Some(PlanRectangle {
                center: [
                    self.plan_center[0].resolve(document)?,
                    self.plan_center[1].resolve(document)?,
                ],
                width: self.width.resolve(document)?,
                depth: self.plan_depth.resolve(document)?,
                rotation_deg: self.plan_rotation_deg,
            })
        };
        let mut placement = CutoutPlacement {
            id,
            wall,
            plan,
            slabs: vec![],
        };
        if self.cut_slabs {
            let from = self
                .from_level
                .ok_or("Choose the level whose floor slab the cutout cuts")?;
            let parameters =
                multicad_core::parameters::evaluate(document).map_err(|e| e.to_string())?;
            let footprint = model::cutout_footprint(document, &parameters, &placement)
                .map_err(|e| e.to_string())?;
            placement.slabs = model::slabs_in_levels(
                document,
                &parameters,
                from,
                self.to_level,
                &footprint.corners(),
            )
            .map_err(|e| e.to_string())?;
            if placement.slabs.is_empty() {
                return Err("No floor slab lies under the cutout on the chosen levels".into());
            }
        }
        let anchor = placement
            .wall
            .as_ref()
            .map(|(w, _)| *w)
            .or_else(|| placement.slabs.first().copied())
            .ok_or("Choose what the cutout cuts: the wall, floor slabs or both")?;
        Ok(OpeningCommand {
            anchor,
            new_types: vec![],
            change: WallOpeningChange::Remove(id),
            cutout: Some(CutoutChange::place(placement)),
        })
    }
    pub fn request(&self, document: &Document, enabled: bool) -> Result<OpeningCommand, String> {
        if !enabled {
            return Err("The native modeling preview provider is unavailable".into());
        }
        if self.slab_mode(document) {
            return self.cutout_request(document);
        }
        let wall = self
            .wall
            .ok_or("Draw a wall first, then pick it in the viewport or choose it here")?;
        if self.remove {
            let id = self
                .editing
                .ok_or("Only an existing opening can be removed")?;
            return Ok(OpeningCommand {
                anchor: wall,
                new_types: vec![],
                change: WallOpeningChange::Remove(id),
                cutout: None,
            });
        }
        let segment = self
            .segment
            .checked_sub(1)
            .ok_or("Segments are numbered from 1")?;
        let offset = self.offset.resolve(document)?;
        let id = self.editing.unwrap_or(self.new_id);
        let mut new_types = vec![];
        let change = match self.kind {
            RoomOpening::Cutout => {
                let cutout = VerticalCutout {
                    id,
                    segment,
                    offset,
                    width: self.width.resolve(document)?,
                    depth: (!self.through)
                        .then(|| self.depth.resolve(document))
                        .transpose()?,
                    face: self.face,
                };
                if self.editing.is_some() {
                    WallOpeningChange::ReplaceCutout(cutout)
                } else {
                    WallOpeningChange::AddCutout(cutout)
                }
            }
            RoomOpening::Door | RoomOpening::Window => {
                let opening_type = match self.opening_type {
                    Some(t) => t,
                    None => {
                        let width = self.width.resolve(document)?;
                        new_types.push(NewType {
                            id: self.new_type,
                            name: format!("{} {}", self.kind.title(), text(&width)),
                            kind: NewTypeKind::Opening(OpeningTypeDefinition {
                                kind: if self.kind == RoomOpening::Window {
                                    OpeningKind::Window
                                } else {
                                    OpeningKind::Door {
                                        hinge: self.hinge,
                                        swing: self.swing,
                                    }
                                },
                                width,
                                height: self.height.resolve(document)?,
                                sill: self.sill.resolve(document)?,
                            }),
                        });
                        self.new_type
                    }
                };
                let sized = self.opening_type.is_some() && self.sized;
                let opening = Opening {
                    id,
                    opening_type,
                    segment,
                    offset,
                    width: sized.then(|| self.width.resolve(document)).transpose()?,
                    height: sized.then(|| self.height.resolve(document)).transpose()?,
                    sill: sized.then(|| self.sill.resolve(document)).transpose()?,
                };
                if self.editing.is_some() {
                    WallOpeningChange::ReplaceOpening(opening)
                } else {
                    WallOpeningChange::AddOpening(opening)
                }
            }
        };
        Ok(OpeningCommand {
            anchor: wall,
            new_types,
            change,
            cutout: None,
        })
    }
    /// The live placement check: the shared planner on the current inputs.
    pub fn diagnose(&self, document: &Document) -> Result<String, String> {
        let command = self.request(document, true)?;
        if let Some(cutout) = &command.cutout {
            let planned =
                model::plan_vertical_cutout(document, cutout).map_err(|e| e.to_string())?;
            if matches!(cutout, CutoutChange::Remove(_)) {
                return Ok("Apply removes it from its wall and every slab".into());
            }
            let name = |id: EntityId| document.get(id).map_or("?".into(), |e| e.name.clone());
            let mut cuts = vec![];
            if let Some(w) = planned.wall {
                cuts.push(format!("wall {}", name(w)));
            }
            if !planned.slabs.is_empty() {
                let names: Vec<String> = planned.slabs.iter().map(|s| name(*s)).collect();
                cuts.push(format!(
                    "{} slab{} ({})",
                    names.len(),
                    if names.len() == 1 { "" } else { "s" },
                    names.join(", ")
                ));
            }
            let size = planned
                .footprint
                .map(|f| format!("; {:.0} × {:.0} mm hole", f.width, f.depth))
                .unwrap_or_default();
            return Ok(format!("Fits: cuts {}{size}", cuts.join(" and ")));
        }
        let planned = model::plan_wall_opening(
            document,
            command.anchor,
            &command.new_types,
            &command.change,
        )
        .map_err(|e| e.to_string())?;
        if matches!(command.change, WallOpeningChange::Remove(_)) {
            return Ok("Apply removes it from the wall".into());
        }
        let target = command.change.target();
        let (segment, a, b) =
            if let Some(o) = planned.resolved.openings.iter().find(|o| o.id == target) {
                (o.segment, o.offset_mm, o.offset_mm + o.width_mm)
            } else {
                let c = planned
                    .resolved
                    .cutouts
                    .iter()
                    .find(|c| c.id == target)
                    .ok_or("The planned opening is missing")?;
                (c.segment, c.offset_mm, c.offset_mm + c.width_mm)
            };
        let length = planned.plan.segments[segment].length_mm;
        Ok(format!(
            "Fits segment {} ({length:.0} mm): {a:.0}–{b:.0} mm along it, {:.0} mm to its end",
            segment + 1,
            length - b
        ))
    }
    /// Hover or pick a wall location from a world point on a wall Part.
    fn locate(&self, document: &Document, part: EntityId, point: [f64; 3]) -> Option<Hover> {
        let wall = wall_of_part(document, part)?;
        let parameters = multicad_core::parameters::evaluate(document).ok()?;
        let source = model::read_wall(document, wall).ok()?;
        let resolved = model::resolve_wall(document, &parameters, &source).ok()?;
        let plan = resolved.plan().ok()?;
        let at = model::locate_on_wall(&plan, [point[0], point[1]], 50.)?;
        let width = self.width_mm(document);
        let length = plan.segments[at.segment].length_mm;
        let centered = (at.along_mm - width / 2.).clamp(0., (length - width).max(0.));
        Some(Hover {
            wall,
            segment: at.segment,
            offset_mm: (centered / 10.).round() * 10.,
        })
    }
    /// The slab feature producing a Part, if any.
    fn slab_of_part(document: &Document, part: EntityId) -> Option<EntityId> {
        match document.get(part).map(|e| &e.data) {
            Some(EntityData::Part { features, .. }) => features
                .iter()
                .copied()
                .find(|f| model::read_slab(document, *f).is_ok()),
            _ => None,
        }
    }
    /// A slab-only cutout centers on a click on a slab and cuts from its level.
    fn pick_slab(&mut self, document: &Document, part: EntityId, point: [f64; 3]) -> bool {
        let Some(slab) =
            Self::slab_of_part(document, part).and_then(|f| model::read_slab(document, f).ok())
        else {
            self.error = Some("Pick a floor slab to place the cutout".into());
            return false;
        };
        let snap = |v: f64| (v / 10.).round() * 10.;
        self.plan_center[0].set_literal(snap(point[0]), "mm");
        self.plan_center[1].set_literal(snap(point[1]), "mm");
        self.from_level = Some(slab.level);
        self.cut_slabs = true;
        self.changed();
        true
    }
    /// Apply a viewport pick on a wall Part; returns whether inputs changed.
    pub(super) fn pick(&mut self, document: &Document, part: EntityId, point: [f64; 3]) -> bool {
        if self.kind == RoomOpening::Cutout && !self.cut_wall {
            return self.pick_slab(document, part, point);
        }
        let Some(hover) = self.locate(document, part, point) else {
            self.error = Some(format!(
                "Pick a wall face to place the {}",
                self.kind.noun()
            ));
            return false;
        };
        self.wall = Some(hover.wall);
        self.segment = hover.segment + 1;
        self.offset.set_literal(hover.offset_mm, "mm");
        self.changed();
        true
    }
    pub fn viewport(
        &mut self,
        document: &Document,
        revision: u64,
        view: TerminationViewport<'_>,
    ) -> bool {
        if !self.picking() {
            return false;
        }
        let Some(scene) = view.scene else {
            return false;
        };
        if scene.stamp().document != document.id() || scene.stamp().revision != revision {
            return false;
        }
        let Some(pointer) = view.pointer.filter(|p| view.rect.contains(*p)) else {
            self.hover = None;
            return false;
        };
        let hit = scene.pick_screen_with(
            view.camera,
            view.rect,
            pointer,
            PickOptions {
                filter: PickFilter::Faces,
                include_hidden: false,
                section: view.section,
            },
        );
        let hit = hit.map(|hit| (hit.instance, hit.point_mm));
        if self.kind == RoomOpening::Cutout && !self.cut_wall {
            self.slab_hover = hit
                .filter(|(part, _)| Self::slab_of_part(document, *part).is_some())
                .map(|(_, point)| point);
            if let Some(point) = self.slab_hover {
                self.paint_slab_hover(document, point, &view);
            }
            return match hit {
                Some((part, point)) if view.clicked => self.pick(document, part, point),
                _ => false,
            };
        }
        self.hover = hit.and_then(|(part, point)| self.locate(document, part, point));
        if let Some(hover) = self.hover {
            self.paint_hover(document, hover, &view);
        }
        match hit {
            Some((part, point)) if view.clicked => self.pick(document, part, point),
            _ => false,
        }
    }
    /// Live outline of a slab-only cutout's rectangle at the hovered point.
    fn paint_slab_hover(
        &self,
        document: &Document,
        point: [f64; 3],
        view: &TerminationViewport<'_>,
    ) {
        let size = |f: &DimensionField| {
            f.resolve(document)
                .ok()
                .and_then(|d| literal(&d))
                .unwrap_or(0.)
                .max(10.)
        };
        let corners = model::rectangle_corners(
            [point[0], point[1]],
            size(&self.width),
            size(&self.plan_depth),
            self.plan_rotation_deg,
        );
        let stroke = egui::Stroke::new(2., egui::Color32::from_rgb(255, 176, 64));
        let points: Vec<egui::Pos2> = corners
            .iter()
            .map(|c| {
                let s = view
                    .camera
                    .project([c[0] as f32, c[1] as f32, point[2] as f32], view.rect);
                egui::pos2(s[0], s[1])
            })
            .collect();
        for i in 0..4 {
            view.painter
                .line_segment([points[i], points[(i + 1) % 4]], stroke);
        }
    }
    /// Live outline of the footprint at the hovered location, drawn on the
    /// wall's left face (cutouts: their full-height plan rectangle).
    fn paint_hover(&self, document: &Document, hover: Hover, view: &TerminationViewport<'_>) {
        let Ok(parameters) = multicad_core::parameters::evaluate(document) else {
            return;
        };
        let Some((resolved, plan)) = model::read_wall(document, hover.wall)
            .ok()
            .and_then(|s| model::resolve_wall(document, &parameters, &s).ok())
            .and_then(|r| r.plan().ok().map(|p| (r, p)))
        else {
            return;
        };
        let seg = plan.segments[hover.segment];
        let width = self.width_mm(document).max(10.);
        let (a, b) = (hover.offset_mm, hover.offset_mm + width);
        let base = resolved.base_z_mm;
        let at = |along: f64, across: f64, z: f64| -> [f32; 3] {
            [
                (seg.start[0] + seg.direction[0] * along + seg.normal[0] * across) as f32,
                (seg.start[1] + seg.direction[1] * along + seg.normal[1] * across) as f32,
                z as f32,
            ]
        };
        let (z0, z1) = match self.kind {
            RoomOpening::Cutout => (base, base + resolved.height_mm),
            _ => {
                let sill = self
                    .opening_type
                    .and_then(|t| model::read_opening_type(document, t).ok())
                    .and_then(|t| literal(&t.sill))
                    .unwrap_or(0.);
                let height = self
                    .opening_type
                    .and_then(|t| model::read_opening_type(document, t).ok())
                    .and_then(|t| literal(&t.height))
                    .unwrap_or(2000.);
                (base + sill, base + sill + height)
            }
        };
        let face = plan.left_mm;
        let corners = [
            at(a, face, z0),
            at(b, face, z0),
            at(b, face, z1),
            at(a, face, z1),
        ];
        let stroke = egui::Stroke::new(2., egui::Color32::from_rgb(255, 176, 64));
        let points: Vec<egui::Pos2> = corners
            .iter()
            .map(|p| {
                let s = view.camera.project(*p, view.rect);
                egui::pos2(s[0], s[1])
            })
            .collect();
        for i in 0..4 {
            view.painter
                .line_segment([points[i], points[(i + 1) % 4]], stroke);
        }
    }
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        document: &Document,
        enabled: bool,
        preview: PreviewState<'_>,
    ) -> Option<ModelEvent> {
        controls::inspector_layout(ui);
        ui.strong(self.kind.title());
        ui.small(match self.kind {
            RoomOpening::Cutout => {
                "Choose what the cutout cuts: a full-height cutout in the wall (through it, or a chase from one face), the floor slabs under it on a range of levels (a shaft or floor penetration), or both."
            }
            _ => "Place an opening of a reusable type in one wall segment. Click a wall in the viewport or enter the segment and offset.",
        });
        let mut changed = false;
        let walls: Vec<_> = model::walls(document)
            .map(|e| (e.id, e.name.clone()))
            .collect();
        if walls.is_empty() {
            ui.colored_label(
                egui::Color32::LIGHT_RED,
                "No wall yet. Draw one with Room → Wall first.",
            );
        }
        ui.label("Wall");
        let selected = self
            .wall
            .and_then(|id| walls.iter().find(|(w, _)| *w == id).map(|(_, n)| n.clone()))
            .unwrap_or_else(|| "Choose a wall".into());
        controls::inspector_combo(ui, "opening-wall")
            .selected_text(selected)
            .show_ui(ui, |ui| {
                for (id, name) in &walls {
                    if ui
                        .selectable_value(&mut self.wall, Some(*id), name)
                        .changed()
                    {
                        self.editing = None;
                        self.remove = false;
                        changed = true;
                    }
                }
            });
        // Existing openings of this kind in the chosen wall can be reopened.
        let existing: Vec<(EntityId, String)> = self
            .wall
            .and_then(|w| model::read_wall(document, w).ok())
            .map(|wall| match self.kind {
                RoomOpening::Cutout => wall
                    .cutouts
                    .iter()
                    .map(|c| {
                        (
                            c.id,
                            format!(
                                "Cutout · segment {} · {} mm",
                                c.segment + 1,
                                text(&c.offset)
                            ),
                        )
                    })
                    .chain(Self::slab_only_cutouts(document))
                    .collect(),
                kind => wall
                    .openings
                    .iter()
                    .filter(|o| {
                        model::read_opening_type(document, o.opening_type)
                            .is_ok_and(|t| kind.matches(&t.kind))
                    })
                    .map(|o| {
                        (
                            o.id,
                            format!(
                                "{} · segment {} · {} mm",
                                kind.title(),
                                o.segment + 1,
                                text(&o.offset)
                            ),
                        )
                    })
                    .collect(),
            })
            .unwrap_or_else(|| {
                if self.kind == RoomOpening::Cutout {
                    Self::slab_only_cutouts(document)
                } else {
                    vec![]
                }
            });
        if !existing.is_empty() {
            let current = self
                .editing
                .and_then(|id| {
                    existing
                        .iter()
                        .find(|(e, _)| *e == id)
                        .map(|(_, l)| l.clone())
                })
                .unwrap_or_else(|| format!("New {}", self.kind.noun()));
            let mut pick = self.editing;
            controls::inspector_combo(ui, "opening-existing")
                .selected_text(current)
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut pick, None, format!("New {}", self.kind.noun()));
                    for (id, label) in &existing {
                        ui.selectable_value(&mut pick, Some(*id), label);
                    }
                });
            if pick != self.editing {
                let reopened = match (pick, self.wall) {
                    (Some(id), _) if self.kind == RoomOpening::Cutout => {
                        Some(Self::existing_cutout(document, id))
                    }
                    (Some(id), Some(wall)) => Some(Self::existing(document, wall, id)),
                    _ => None,
                };
                match reopened {
                    Some(reopened) => {
                        if let Ok(mut panel) = reopened {
                            panel.stamp = self.stamp;
                            *self = panel;
                        }
                    }
                    _ => {
                        self.editing = None;
                        self.remove = false;
                        self.picking = true;
                    }
                }
                changed = true;
            }
        }
        if self.editing.is_none() {
            let label = if self.picking {
                "Picking: click a wall face in the viewport"
            } else {
                "Pick location in viewport"
            };
            if ui.selectable_label(self.picking, label).clicked() {
                self.picking = !self.picking;
            }
        } else {
            changed |= ui
                .checkbox(
                    &mut self.remove,
                    format!("Remove this {} on Apply", self.kind.noun()),
                )
                .changed();
        }
        ui.separator();
        ui.add_enabled_ui(!self.remove, |ui| {
            if self.kind == RoomOpening::Cutout {
                changed |= self.show_cut_targets(ui, document);
            }
            if self.kind == RoomOpening::Cutout && !self.cut_wall {
                changed |= self.show_plan_rectangle(ui);
                return;
            }
            let segments = self
                .wall
                .and_then(|w| model::read_wall(document, w).ok())
                .map_or(1, |w| w.segment_count().max(1));
            ui.horizontal(|ui| {
                ui.label("Segment");
                changed |= ui
                    .add(egui::DragValue::new(&mut self.segment).range(1..=segments))
                    .changed();
                ui.small(format!("of {segments}"));
            });
            changed |= self.offset.show(
                ui,
                "Offset along segment",
                "From the segment start point to the opening's start side",
            );
            match self.kind {
                RoomOpening::Cutout => {
                    changed |= self.width.show(ui, "Width", "Cutout width along the wall");
                    changed |= ui
                        .checkbox(&mut self.through, "Through the full wall thickness")
                        .changed();
                    if !self.through {
                        changed |= self.depth.show(
                            ui,
                            "Chase depth",
                            "Depth into the wall from the chosen face",
                        );
                        ui.horizontal(|ui| {
                            changed |= ui
                                .radio_value(&mut self.face, WallFace::Left, "Left face")
                                .changed();
                            changed |= ui
                                .radio_value(&mut self.face, WallFace::Right, "Right face")
                                .changed();
                        });
                    }
                }
                kind => changed |= self.show_type(ui, document, kind),
            }
        });
        ui.separator();
        if self
            .diagnostic
            .as_ref()
            .is_none_or(|(stamp, _)| *stamp != self.stamp)
        {
            self.diagnostic = Some((self.stamp, self.diagnose(document)));
        }
        match self.diagnostic.as_ref().map(|(_, d)| d) {
            Some(Ok(note)) => {
                ui.colored_label(egui::Color32::from_rgb(120, 200, 140), note);
            }
            Some(Err(problem)) => {
                ui.colored_label(egui::Color32::LIGHT_RED, problem);
            }
            None => {}
        }
        form::preview_status(ui, self.stamp, preview);
        let valid = matches!(self.diagnostic, Some((_, Ok(_))));
        let action = form::actions(ui, valid && enabled, changed, self.stamp, preview);
        if let Some(error) = &self.error {
            ui.colored_label(egui::Color32::LIGHT_RED, error);
        }
        if action == Some(FormAction::Cancel) {
            return Some(ModelEvent::Cancel);
        }
        if changed {
            self.changed();
            if action != Some(FormAction::Preview) {
                return Some(ModelEvent::Changed);
            }
        }
        match action {
            Some(FormAction::Preview) => match self.request(document, enabled) {
                Ok(request) => Some(ModelEvent::Preview {
                    stamp: self.stamp,
                    request: ModelRequest::Opening(Box::new(request)),
                }),
                Err(error) => {
                    self.error = Some(error);
                    None
                }
            },
            Some(FormAction::Apply)
                if preview.ready(self.stamp) && self.request(document, enabled).is_ok() =>
            {
                Some(ModelEvent::Apply)
            }
            _ => None,
        }
    }
    /// Cutouts that cut slabs but no wall, for reopening.
    fn slab_only_cutouts(document: &Document) -> Vec<(EntityId, String)> {
        model::slab_cutouts(document)
            .into_iter()
            .filter(|id| model::cutout_members(document, *id).wall.is_none())
            .map(|id| {
                let n = model::cutout_members(document, id).slabs.len();
                (
                    id,
                    format!("Shaft · {n} slab{}", if n == 1 { "" } else { "s" }),
                )
            })
            .collect()
    }
    /// What the cutout cuts, and the level range for slabs.
    fn show_cut_targets(&mut self, ui: &mut egui::Ui, document: &Document) -> bool {
        let mut changed = false;
        ui.label("Cuts");
        ui.horizontal(|ui| {
            changed |= ui.checkbox(&mut self.cut_wall, "Wall").changed();
            changed |= ui.checkbox(&mut self.cut_slabs, "Floor slabs").changed();
        });
        if !self.cut_slabs {
            return changed;
        }
        let levels: Vec<(EntityId, String)> = model::levels(document)
            .map(|e| (e.id, e.name.clone()))
            .collect();
        if levels.is_empty() {
            ui.colored_label(
                egui::Color32::LIGHT_RED,
                "No level yet. Add levels and floor slabs with Levels & stairs first.",
            );
            return changed;
        }
        let name = |v: Option<EntityId>, none: &str| {
            v.and_then(|v| levels.iter().find(|(l, _)| *l == v).map(|(_, n)| n.clone()))
                .unwrap_or_else(|| none.into())
        };
        ui.horizontal(|ui| {
            ui.label("From level");
            controls::inspector_combo(ui, "cutout-from-level")
                .selected_text(name(self.from_level, "Choose a level"))
                .show_ui(ui, |ui| {
                    for (id, n) in &levels {
                        changed |= ui
                            .selectable_value(&mut self.from_level, Some(*id), n)
                            .changed();
                    }
                });
        });
        ui.horizontal(|ui| {
            ui.label("To level");
            controls::inspector_combo(ui, "cutout-to-level")
                .selected_text(name(self.to_level, "Same level"))
                .show_ui(ui, |ui| {
                    changed |= ui
                        .selectable_value(&mut self.to_level, None, "Same level")
                        .changed();
                    for (id, n) in &levels {
                        changed |= ui
                            .selectable_value(&mut self.to_level, Some(*id), n)
                            .changed();
                    }
                });
        });
        ui.small(if self.cut_wall {
            "Every floor slab on these levels under the wall cutout's footprint gets a matching hole."
        } else {
            "Every floor slab on these levels under the rectangle gets a hole (a shaft through several levels)."
        });
        changed
    }
    /// The plan rectangle of a slab-only cutout.
    fn show_plan_rectangle(&mut self, ui: &mut egui::Ui) -> bool {
        let mut changed = false;
        ui.small("Click a floor slab in the viewport to center the cutout there, or enter it.");
        let [x, y] = &mut self.plan_center;
        changed |= x.show(ui, "Center X", "World X of the cutout center");
        changed |= y.show(ui, "Center Y", "World Y of the cutout center");
        changed |= self
            .width
            .show(ui, "Width", "Size along the rotated X direction");
        changed |= self
            .plan_depth
            .show(ui, "Depth", "Size along the rotated Y direction");
        ui.horizontal(|ui| {
            ui.label("Rotation");
            changed |= ui
                .add(
                    egui::DragValue::new(&mut self.plan_rotation_deg)
                        .range(-360.0..=360.0)
                        .suffix(" °"),
                )
                .changed();
        });
        changed
    }
    fn show_type(&mut self, ui: &mut egui::Ui, document: &Document, kind: RoomOpening) -> bool {
        let mut changed = false;
        ui.label(format!("{} type", kind.title()));
        let types: Vec<_> = model::opening_types(document)
            .filter(|e| {
                model::read_opening_type(document, e.id).is_ok_and(|t| kind.matches(&t.kind))
            })
            .map(|e| (e.id, e.name.clone()))
            .collect();
        let selected = self
            .opening_type
            .and_then(|id| types.iter().find(|(t, _)| *t == id).map(|(_, n)| n.clone()))
            .unwrap_or_else(|| "New type…".into());
        controls::inspector_combo(ui, "opening-type")
            .selected_text(selected)
            .show_ui(ui, |ui| {
                for (id, name) in &types {
                    changed |= ui
                        .selectable_value(&mut self.opening_type, Some(*id), name)
                        .changed();
                }
                changed |= ui
                    .selectable_value(&mut self.opening_type, None, "New type…")
                    .changed();
            });
        if self.opening_type.is_none() && kind == RoomOpening::Door {
            ui.horizontal(|ui| {
                changed |= ui
                    .radio_value(&mut self.hinge, Hinge::Start, "Hinge at start")
                    .changed();
                changed |= ui
                    .radio_value(&mut self.hinge, Hinge::End, "Hinge at end")
                    .changed();
            });
            ui.horizontal(|ui| {
                changed |= ui
                    .radio_value(&mut self.swing, Swing::Left, "Opens left")
                    .changed();
                changed |= ui
                    .radio_value(&mut self.swing, Swing::Right, "Opens right")
                    .changed();
            });
        }
        if let Some(t) = self
            .opening_type
            .and_then(|t| model::read_opening_type(document, t).ok())
        {
            ui.small(format!(
                "Type: {} wide · {} high · sill {}. A type edit updates every opening that does not override it.",
                text(&t.width),
                text(&t.height),
                text(&t.sill)
            ));
            changed |= ui
                .checkbox(&mut self.sized, "Override size for this opening")
                .changed();
        }
        if self.opening_type.is_none() || self.sized {
            changed |= self.width.show(ui, "Width", "Opening width along the wall");
            changed |= self
                .height
                .show(ui, "Height", "Opening height above its sill");
            changed |= self.sill.show(
                ui,
                "Sill",
                "Height of the opening bottom above the wall base",
            );
        }
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use multicad_core::{
        commands::{Channel, Session},
        model::{
            BodyOutput, Justification, WallChange, WallDefinition, WallRequest, WallTypeDefinition,
        },
    };
    use multicad_model::Compiler;

    /// A committed 6 m straight wall, 200 thick and 2500 high, without types
    /// for openings.
    fn wall_session() -> (Session, EntityId, EntityId) {
        let mut session = Session::new(Document::new("Room").unwrap()).unwrap();
        let ty = NewType::new(
            "Block 200",
            NewTypeKind::Wall(WallTypeDefinition {
                thickness: length(200.),
                height: length(2500.),
                justification: Justification::Center,
                material: None,
            }),
        );
        let request = WallRequest::new(
            "Wall 1",
            WallDefinition {
                output: BodyOutput {
                    body: EntityId::new(),
                },
                wall_type: ty.id,
                path: vec![[length(0.), length(0.)], [length(6000.), length(0.)]],
                closed: false,
                level: None,
                base_z: length(0.),
                thickness: None,
                height: None,
                justification: None,
                openings: vec![],
                cutouts: vec![],
            },
        );
        let (feature, body) = (request.feature, request.body);
        let prepared = model::prepare_wall_change(
            &session,
            0,
            Actor::local(Channel::Ui),
            &[ty],
            &WallChange::Create(&request),
        )
        .unwrap();
        session.commit(prepared.prepared).unwrap();
        (session, feature, body)
    }
    fn volume(document: &Document, revision: u64, feature: EntityId) -> f64 {
        Compiler::default()
            .compile(document, revision)
            .unwrap()
            .snapshot()
            .shapes[&feature]
            .volume_mm3
    }

    #[test]
    fn a_picked_door_with_a_new_type_applies_in_one_transaction_and_reopens() {
        let (mut session, wall, body) = wall_session();
        let mut panel = OpeningPanel::new(session.document(), RoomOpening::Door, None).unwrap();
        assert_eq!(panel.wall, Some(wall));
        assert!(panel.picking() && panel.opening_type.is_none());
        // A click on the wall Part at x = 2000 centers the 900 mm door there.
        assert!(panel.pick(session.document(), body, [2000., 100., 1000.]));
        assert_eq!(panel.segment, 1);
        assert_eq!(
            panel.offset.resolve(session.document()).unwrap(),
            length(1550.)
        );
        assert!(
            panel
                .diagnose(session.document())
                .unwrap()
                .contains("1550–2450")
        );
        let command = panel.request(session.document(), true).unwrap();
        assert_eq!(command.new_types.len(), 1);
        let prepared = command
            .prepare(&session, Actor::local(Channel::Ui))
            .unwrap();
        assert_eq!(prepared.outputs[0].feature, wall);
        let full = 6000. * 200. * 2500.;
        let door = 900. * 200. * 2100.;
        let v = volume(prepared.prepared.document(), 1, wall);
        assert!((v - (full - door)).abs() < 1e-3, "{v}");
        session.commit(prepared.prepared).unwrap();
        assert_eq!(session.revision(), 2);
        assert_eq!(
            session.activity().last().map(|a| a.label.as_str()),
            Some("Place door")
        );

        // Reopened for editing, the door keeps its identity and type.
        let id = model::read_wall(session.document(), wall).unwrap().openings[0].id;
        let mut edit = OpeningPanel::existing(session.document(), wall, id).unwrap();
        assert_eq!(edit.kind, RoomOpening::Door);
        assert!(!edit.picking() && edit.opening_type.is_some());
        edit.offset.set_literal(4000., "mm");
        let command = edit.request(session.document(), true).unwrap();
        assert!(command.new_types.is_empty());
        assert!(matches!(&command.change, WallOpeningChange::ReplaceOpening(o) if o.id == id));
        let prepared = command
            .prepare(&session, Actor::local(Channel::Ui))
            .unwrap();
        session.commit(prepared.prepared).unwrap();
        let wall_source = model::read_wall(session.document(), wall).unwrap();
        assert_eq!(wall_source.openings[0].offset, length(4000.));

        // Removing it is one more transaction; Undo restores it.
        let mut remove = OpeningPanel::existing(session.document(), wall, id).unwrap();
        remove.remove = true;
        let prepared = remove
            .request(session.document(), true)
            .unwrap()
            .prepare(&session, Actor::local(Channel::Ui))
            .unwrap();
        session.commit(prepared.prepared).unwrap();
        assert!(
            model::read_wall(session.document(), wall)
                .unwrap()
                .openings
                .is_empty()
        );
        session
            .undo(session.revision(), Actor::local(Channel::Ui))
            .unwrap();
        assert_eq!(
            model::read_wall(session.document(), wall).unwrap().openings[0].id,
            id
        );
    }

    #[test]
    fn misfits_are_diagnosed_live_and_never_reach_the_document() {
        let (session, _, body) = wall_session();
        let before = session.document().clone();
        let mut panel = OpeningPanel::new(session.document(), RoomOpening::Window, None).unwrap();
        panel.offset.set_literal(5500., "mm");
        let problem = panel.diagnose(session.document()).unwrap_err();
        assert!(problem.contains("does not fit"), "{problem}");
        assert!(
            panel
                .request(session.document(), true)
                .unwrap()
                .prepare(&session, Actor::local(Channel::Ui))
                .is_err()
        );
        // A click away from any wall is reported, not guessed.
        assert!(!panel.pick(session.document(), EntityId::new(), [0.; 3]));
        assert!(!panel.pick(session.document(), body, [3000., 900., 0.]));
        assert_eq!(session.document(), &before);
    }

    #[test]
    fn a_vertical_cutout_chase_and_a_splitting_gap_are_checked() {
        let (session, wall, body) = wall_session();
        let mut panel = OpeningPanel::new(session.document(), RoomOpening::Cutout, None).unwrap();
        assert!(panel.pick(session.document(), body, [3000., 0., 0.]));
        // A through cut inside an open wall would split it.
        assert!(
            panel
                .diagnose(session.document())
                .unwrap_err()
                .contains("splits it")
        );
        panel.through = false;
        panel.depth.set_literal(80., "mm");
        panel.face = WallFace::Right;
        assert!(panel.diagnose(session.document()).is_ok());
        let prepared = panel
            .request(session.document(), true)
            .unwrap()
            .prepare(&session, Actor::local(Channel::Ui))
            .unwrap();
        let v = volume(prepared.prepared.document(), 1, wall);
        let expected = 6000. * 200. * 2500. - 300. * 80. * 2500.;
        assert!((v - expected).abs() < 1e-3, "{v}");
        let cut = &model::read_wall(prepared.prepared.document(), wall)
            .unwrap()
            .cutouts[0];
        assert_eq!(
            (cut.face, cut.depth.clone()),
            (WallFace::Right, Some(length(80.)))
        );
    }

    /// Two levels 3 m apart with a 6 x 4 m floor slab each, under the 6 m
    /// wall of `wall_session` (which stands on no level).
    fn building() -> (
        Session,
        EntityId,
        [EntityId; 2],
        [EntityId; 2],
        [EntityId; 2],
    ) {
        let (mut session, wall, body) = wall_session();
        let mut levels = [EntityId::nil(); 2];
        let mut slabs = [EntityId::nil(); 2];
        let mut parts = [EntityId::nil(); 2];
        for (k, z) in [0., 3000.].into_iter().enumerate() {
            let (level, p) = model::prepare_level(
                &session,
                session.revision(),
                Actor::local(Channel::Ui),
                if k == 0 { "Ground" } else { "Upper" },
                &model::LevelDefinition {
                    elevation: length(z),
                    height: length(3000.),
                    slab_thickness: length(200.),
                },
            )
            .unwrap();
            session.commit(p).unwrap();
            let pt = |x: f64, y: f64| [length(x), length(y)];
            let p = model::prepare_slab(
                &session,
                session.revision(),
                Actor::local(Channel::Ui),
                "Slab",
                &model::SlabDefinition {
                    output: BodyOutput {
                        body: EntityId::new(),
                    },
                    level,
                    kind: model::SlabKind::Slab,
                    outline: vec![
                        pt(0., -2000.),
                        pt(6000., -2000.),
                        pt(6000., 2000.),
                        pt(0., 2000.),
                    ],
                    thickness: None,
                    holes: vec![],
                },
                None,
            )
            .unwrap();
            (levels[k], slabs[k], parts[k]) = (level, p.outputs[0].feature, p.outputs[0].body);
            session.commit(p.prepared).unwrap();
        }
        (session, wall, levels, slabs, [body, parts[0]])
    }

    #[test]
    fn a_slab_only_shaft_is_picked_on_a_slab_cut_over_two_levels_and_reopened() {
        let (mut session, _, levels, slabs, [_, ground_part]) = building();
        let before = session.document().clone();
        let mut panel = OpeningPanel::new(session.document(), RoomOpening::Cutout, None).unwrap();
        // Neither target chosen: refused before anything is staged.
        panel.cut_wall = false;
        assert!(
            panel
                .diagnose(session.document())
                .unwrap_err()
                .contains("Choose what the cutout cuts")
        );
        // A click on the ground slab centers the shaft there and cuts its level.
        assert!(panel.pick(session.document(), ground_part, [4003., 1497., 0.]));
        assert!(panel.cut_slabs);
        assert_eq!(panel.from_level, Some(levels[0]));
        panel.width.set_literal(600., "mm");
        panel.plan_depth.set_literal(800., "mm");
        panel.to_level = Some(levels[1]);
        let note = panel.diagnose(session.document()).unwrap();
        assert!(
            note.contains("2 slabs") && note.contains("600 × 800"),
            "{note}"
        );
        let command = panel.request(session.document(), true).unwrap();
        assert_eq!(command.anchor, slabs[0]);
        let prepared = command
            .prepare(&session, Actor::local(Channel::Ui))
            .unwrap();
        assert_eq!(prepared.outputs.len(), 1);
        let full = 6000. * 4000. * 200.;
        let hole = 600. * 800. * 200.;
        for slab in slabs {
            let v = volume(prepared.prepared.document(), 1, slab);
            assert!((v - (full - hole)).abs() < 1e-3, "{v}");
        }
        let hole_source = &model::read_slab(prepared.prepared.document(), slabs[1])
            .unwrap()
            .holes[0];
        assert_eq!(hole_source.center, [length(4000.), length(1500.)]);
        session.commit(prepared.prepared).unwrap();
        assert_eq!(
            session.activity().last().map(|a| a.label.as_str()),
            Some("Place vertical cutout")
        );

        // Reopened, it is a slab-only cutout over both levels; removal is one
        // transaction and Undo restores the shaft, then the original model.
        let id = hole_source.id;
        let mut reopened = OpeningPanel::existing_cutout(session.document(), id).unwrap();
        assert!(!reopened.cut_wall && reopened.cut_slabs);
        assert_eq!(
            (reopened.from_level, reopened.to_level),
            (Some(levels[0]), Some(levels[1]))
        );
        assert!(reopened.diagnose(session.document()).is_ok());
        reopened.remove = true;
        let prepared = reopened
            .request(session.document(), true)
            .unwrap()
            .prepare(&session, Actor::local(Channel::Ui))
            .unwrap();
        session.commit(prepared.prepared).unwrap();
        assert!(model::cutout_members(session.document(), id).is_empty());
        session
            .undo(session.revision(), Actor::local(Channel::Ui))
            .unwrap();
        assert_eq!(model::cutout_members(session.document(), id).slabs.len(), 2);
        session
            .undo(session.revision(), Actor::local(Channel::Ui))
            .unwrap();
        assert_eq!(session.document(), &before);
    }

    #[test]
    fn a_wall_chase_can_also_cut_the_slab_below_and_edge_crossings_are_refused() {
        let (session, wall, levels, slabs, [wall_part, _]) = building();
        let mut panel = OpeningPanel::new(session.document(), RoomOpening::Cutout, None).unwrap();
        assert!(panel.cut_wall && !panel.cut_slabs);
        assert!(panel.pick(session.document(), wall_part, [3000., 0., 0.]));
        panel.through = false;
        panel.depth.set_literal(80., "mm");
        panel.cut_slabs = true;
        panel.from_level = Some(levels[0]);
        let note = panel.diagnose(session.document()).unwrap();
        assert!(
            note.contains("wall Wall 1") && note.contains("1 slab"),
            "{note}"
        );
        let command = panel.request(session.document(), true).unwrap();
        assert_eq!(command.anchor, wall);
        let prepared = command
            .prepare(&session, Actor::local(Channel::Ui))
            .unwrap();
        let document = prepared.prepared.document();
        let w = volume(document, 1, wall);
        assert!(
            (w - (6000. * 200. * 2500. - 300. * 80. * 2500.)).abs() < 1e-3,
            "{w}"
        );
        let s = volume(document, 1, slabs[0]);
        assert!(
            (s - (6000. * 4000. * 200. - 300. * 80. * 200.)).abs() < 1e-3,
            "{s}"
        );

        // A slab-only rectangle reaching past the slab edge is diagnosed live.
        let mut edge = OpeningPanel::new(session.document(), RoomOpening::Cutout, None).unwrap();
        edge.cut_wall = false;
        edge.cut_slabs = true;
        edge.plan_center[0].set_literal(5900., "mm");
        let problem = edge.diagnose(session.document()).unwrap_err();
        assert!(problem.contains("not inside the slab outline"), "{problem}");
    }
}
