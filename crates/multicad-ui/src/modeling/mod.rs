//! Reusable modeling forms. Native geometry and document commits belong to the
//! composition root; painting a form never changes the canonical document.
mod advanced;
mod alignment_panel;
mod architecture;
mod body_choices;
mod boolean;
mod boolean_edit;
mod cabinet;
pub mod cabinet_edit;
mod compound_cutout;
mod controller;
pub(crate) mod dimension;
mod edge_treatment;
mod equipment;
mod extrude;
mod extrude_operation;
mod fdm_aid;
mod fence;
pub(crate) use fence::fence_target;
mod form;
mod gear;
mod gizmo;
mod group_edit;
#[cfg(test)]
mod group_fixture;
mod guide_fields;
mod joinery_fields;
mod joint;
mod machining;
mod mechanical_part;
mod names;
#[cfg(test)]
pub(crate) mod native_fixture;
mod object_basis;
mod opening;
mod operation_guide;
mod outline;
mod panel_face;
#[cfg(test)]
pub(crate) mod panel_fixture;
mod placement;
mod plant;
mod primitive;
mod revolve;
mod scale_gizmo;
mod scaling;
mod sheet;
mod split;
mod termination;
mod thread;
pub(crate) mod value_entry;
mod wall;
mod wall_junctions;
mod wall_layers;
mod weld_connection;
mod weld_form;
mod weldment;

pub use advanced::{AdvancedCommand, AdvancedKind, AdvancedSelection};
pub use alignment_panel::AlignmentCommand;
pub use controller::{ModelToolController, ModelToolOutcome};
pub use controller::{OperationViewport, PlacementViewportResult};
pub use edge_treatment::EdgeTreatmentCommand;
pub use extrude::{ExtrudeCapabilities, ExtrudeEvent, ExtrudePanel};
pub use form::ModelRequest;
pub use gear::GearPairCommand;
pub use gizmo::{GizmoBasis, PlacementViewport};
use multicad_core::document::EntityId;
pub use object_basis::{ObjectGizmoBasis, object_gizmo_basis};
pub use operation_guide::{GuideLine, ProfileGuide, anchor as profile_anchor};
pub(crate) use panel_face::{seeded_message, selected_panels, with_opposite_sides};
pub use placement::{PlacementCommand, PlacementMode};
pub use scale_gizmo::ScaleViewport;
pub use scaling::ScaleCommand;
pub(crate) use split::SplitInputField;
pub use termination::TerminationViewport;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModelingTool {
    Extrude,
    Box,
    Cylinder,
    Thread,
    Gear,
    MechanicalPart,
    Revolve,
    Boolean,
    Split,
    Move,
    Copy,
    Align,
    Scale,
    EdgeTreatment,
    Wall,
    Fence,
    Door,
    Window,
    Cutout,
    Level,
    Stair,
    Plant,
    Equipment,
    Cabinet,
    Panel,
    Joint,
    Machining,
    ShelfPins,
    LooseTenon,
    FdmAid,
    Member,
    Connection,
    Weld,
    Sheet,
    Advanced(AdvancedKind),
}
impl ModelingTool {
    pub const ALL: [Self; 41] = [
        Self::Box,
        Self::Cylinder,
        Self::Thread,
        Self::Gear,
        Self::MechanicalPart,
        Self::Wall,
        Self::Fence,
        Self::Door,
        Self::Window,
        Self::Cutout,
        Self::Level,
        Self::Stair,
        Self::Plant,
        Self::Equipment,
        Self::Cabinet,
        Self::Panel,
        Self::Joint,
        Self::Machining,
        Self::ShelfPins,
        Self::LooseTenon,
        Self::FdmAid,
        Self::Member,
        Self::Connection,
        Self::Weld,
        Self::Sheet,
        Self::Extrude,
        Self::Revolve,
        Self::Boolean,
        Self::Split,
        Self::Move,
        Self::Copy,
        Self::Align,
        Self::Scale,
        Self::EdgeTreatment,
        Self::Advanced(AdvancedKind::Shell),
        Self::Advanced(AdvancedKind::Sweep),
        Self::Advanced(AdvancedKind::Loft),
        Self::Advanced(AdvancedKind::Draft),
        Self::Advanced(AdvancedKind::Mirror),
        Self::Advanced(AdvancedKind::LinearPattern),
        Self::Advanced(AdvancedKind::CircularPattern),
    ];
    pub fn advanced_kind(self) -> Option<AdvancedKind> {
        match self {
            Self::Advanced(kind) => Some(kind),
            _ => None,
        }
    }
    pub fn id(self) -> &'static str {
        match self {
            Self::Extrude => "extrude",
            Self::Box => "box",
            Self::Cylinder => "cylinder",
            Self::Thread => "thread",
            Self::Gear => "gear",
            Self::MechanicalPart => "mechanical-part",
            Self::Revolve => "revolve",
            Self::Boolean => "boolean",
            Self::Split => "split",
            Self::Move => "move",
            Self::Copy => "placed-copy",
            Self::Align => "align",
            Self::Scale => "scale",
            Self::EdgeTreatment => "edge-treatment",
            Self::Wall => "wall",
            Self::Fence => "fence",
            Self::Door => "door",
            Self::Window => "window-tool",
            Self::Cutout => "vertical-cutout",
            Self::Level => "level",
            Self::Stair => "stairs",
            Self::Plant => "plant",
            Self::Equipment => "equipment",
            Self::Cabinet => "cabinet",
            Self::Panel => "panel",
            Self::Joint => "joint",
            Self::Machining => "drill",
            Self::ShelfPins => "system32",
            Self::LooseTenon => "domino",
            Self::FdmAid => "fdm-aid",
            Self::Member => "tube",
            Self::Connection => "cope",
            Self::Weld => "weld-joint",
            Self::Sheet => "sheet",
            Self::Advanced(kind) => match kind {
                AdvancedKind::Shell | AdvancedKind::Thickness => "shell",
                AdvancedKind::Sweep => "sweep",
                AdvancedKind::Loft => "loft",
                AdvancedKind::Draft => "draft",
                AdvancedKind::Mirror => "model-mirror",
                AdvancedKind::LinearPattern => "model-linear-pattern",
                AdvancedKind::CircularPattern => "model-circular-pattern",
            },
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Extrude => "Extrude",
            Self::Box => "Box",
            Self::Cylinder => "Cylinder",
            Self::Thread => "Thread",
            Self::Gear => "Gear",
            Self::MechanicalPart => "Hardware",
            Self::Revolve => "Revolve",
            Self::Boolean => "Boolean",
            Self::Split => "Split",
            Self::Move => "Move / Copy",
            Self::Copy => "Placed copy",
            Self::Align => "Align",
            Self::Scale => "Scale",
            Self::EdgeTreatment => "Fillet / Chamfer",
            Self::Wall => "Wall",
            Self::Fence => "Fence",
            Self::Door => "Door",
            Self::Window => "Window",
            Self::Cutout => "Cutout",
            Self::Level => "Levels",
            Self::Stair => "Stairs",
            Self::Plant => "Planting",
            Self::Equipment => "Equipment",
            Self::Cabinet => "Add Cabinet",
            Self::Panel => "Panel",
            Self::Joint => "Panel joint",
            Self::Machining => "Drill / pocket",
            Self::ShelfPins => "System 32 shelf pins",
            Self::LooseTenon => "Domino joint",
            Self::FdmAid => "Tab / bridge / support",
            Self::Member => "Tube / profile",
            Self::Connection => "Miter / cope",
            Self::Weld => "Weld joint",
            Self::Sheet => "Sheet part / flanges",
            Self::Advanced(kind) => match kind {
                AdvancedKind::Shell | AdvancedKind::Thickness => "Shell / Thickness",
                AdvancedKind::Sweep => "Sweep",
                AdvancedKind::Loft => "Loft",
                AdvancedKind::Draft => "Draft",
                AdvancedKind::Mirror => "Mirror",
                AdvancedKind::LinearPattern => "Linear pattern",
                AdvancedKind::CircularPattern => "Circular pattern",
            },
        }
    }
    pub fn icon(self) -> crate::icons::Icon {
        match self {
            Self::Extrude => crate::icons::Icon::Extrude,
            Self::Box => crate::icons::Icon::Box,
            Self::Cylinder => crate::icons::Icon::Cylinder,
            Self::Thread => crate::icons::Icon::Revolve,
            Self::Gear => crate::icons::Icon::Revolve,
            Self::MechanicalPart => crate::icons::Icon::Revolve,
            Self::Revolve => crate::icons::Icon::Revolve,
            Self::Boolean => crate::icons::Icon::Boolean,
            Self::Split => crate::icons::Icon::Split,
            Self::Move => crate::icons::Icon::Transform,
            Self::Copy => crate::icons::Icon::LinearPattern,
            Self::Align => crate::icons::Icon::Link,
            Self::Scale => crate::icons::Icon::Scale,
            Self::EdgeTreatment => crate::icons::Icon::Edge,
            Self::Wall => crate::icons::Icon::Box,
            Self::Fence => crate::icons::Icon::Grid,
            Self::Door | Self::Window => crate::icons::Icon::Box,
            Self::Cutout => crate::icons::Icon::Cut,
            Self::Level => crate::icons::Icon::Edge,
            Self::Stair => crate::icons::Icon::Stairs,
            Self::Plant => crate::icons::Icon::Plant,
            Self::Equipment => crate::icons::Icon::Box,
            Self::Panel | Self::Cabinet => crate::icons::Icon::Box,
            Self::Joint | Self::LooseTenon => crate::icons::Icon::Link,
            Self::Machining | Self::ShelfPins => crate::icons::Icon::Cut,
            Self::FdmAid => crate::icons::Icon::Edge,
            Self::Member => crate::icons::Icon::Box,
            Self::Connection => crate::icons::Icon::Cut,
            Self::Weld => crate::icons::Icon::Link,
            Self::Sheet => crate::icons::Icon::Page,
            Self::Advanced(kind) => match kind {
                AdvancedKind::Shell | AdvancedKind::Thickness => crate::icons::Icon::Shell,
                AdvancedKind::Sweep => crate::icons::Icon::Sweep,
                AdvancedKind::Loft => crate::icons::Icon::Loft,
                AdvancedKind::Draft => crate::icons::Icon::Draft,
                AdvancedKind::Mirror => crate::icons::Icon::Mirror,
                AdvancedKind::LinearPattern => crate::icons::Icon::LinearPattern,
                AdvancedKind::CircularPattern => crate::icons::Icon::CircularPattern,
            },
        }
    }
    pub fn aliases(self) -> &'static [&'static str] {
        match self {
            Self::Extrude => &["ext", "extrusion"],
            Self::Box => &["cube", "add-box"],
            Self::Cylinder => &["cyl", "add-cylinder"],
            Self::Thread => &["screw-thread", "tap-thread", "helical-thread"],
            Self::Gear => &["spur-gear", "helical-gear", "herringbone-gear"],
            Self::MechanicalPart => &["washer", "hex-nut", "bearing-envelope"],
            Self::Revolve => &["rev", "lathe"],
            Self::Boolean => &["combine", "body-operation"],
            Self::Split => &["split-body", "inside-outside"],
            Self::Move => &["place", "move-body", "rotate-body"],
            Self::Copy => &["copy-body", "instance-copy"],
            Self::Align => &["align-body", "mate", "align-part"],
            Self::Scale => &["scale-body", "resize-solid"],
            Self::EdgeTreatment => &["fillet", "chamfer", "edge"],
            Self::Wall => &["walls", "architectural-wall", "doors-windows"],
            Self::Fence => &["draw-fence", "fence-posts", "pickets", "railing"],
            Self::Door => &["place-door", "wall-door"],
            Self::Window => &["place-window", "wall-window"],
            Self::Cutout => &["wall-cutout", "chase", "wall-chase"],
            Self::Level => &["levels", "slab", "pedestal", "service-envelope"],
            Self::Stair => &["stair", "staircase"],
            Self::Plant => &["tree", "shrub", "landscape", "garden"],
            Self::Equipment => &["sanitary", "appliance"],
            Self::Cabinet => &["carcass", "shelf", "add cabinet", "cabinet generator"],
            Self::Panel => &["board", "edge-banding"],
            Self::Joint => &["finger-joint", "box-joint", "miter-joint", "butt-joint"],
            Self::Machining => &["groove", "dado", "rabbet", "pocket", "hardware-holes"],
            Self::ShelfPins => &["shelf-pins", "shelf-pin-holes", "system-32", "line-boring"],
            Self::LooseTenon => &["loose-tenon", "domino-joint", "floating-tenon", "mortise"],
            Self::FdmAid => &[
                "support",
                "tab",
                "breakaway-tab",
                "sacrificial-bridge",
                "fdm",
            ],
            Self::Member => &["member", "profile", "frame-member", "structural-member"],
            Self::Connection => &["miter", "member-butt", "notch", "end-cut"],
            Self::Weld => &["weld-definition", "weld-mark", "weld-order"],
            Self::Sheet => &[
                "sheet-metal",
                "base-sheet",
                "sheet-part",
                "flange",
                "bend",
                "hem",
                "jog",
            ],
            Self::Advanced(kind) => match kind {
                AdvancedKind::Shell | AdvancedKind::Thickness => &["thickness", "hollow-body"],
                AdvancedKind::Sweep => &["sweep-body", "pipe-body"],
                AdvancedKind::Loft => &["loft-body"],
                AdvancedKind::Draft => &["draft-faces"],
                AdvancedKind::Mirror => &["mirror-body", "solid-mirror"],
                AdvancedKind::LinearPattern => &["array-body", "pattern-linear"],
                AdvancedKind::CircularPattern => &["polar-array-body", "pattern-circular"],
            },
        }
    }
    pub fn usage(self) -> &'static str {
        match self {
            Self::MechanicalPart => {
                "Usage: mechanical-part (configure a washer, hex nut or bearing mounting envelope; Preview and Apply)"
            }
            Self::Gear => {
                "Usage: gear (choose family, module system, teeth, bore and clearance in the Inspector; Preview and Apply)"
            }
            Self::Thread => {
                "Usage: thread (choose a sourced or explicit custom profile, pitch, starts, representation and receiving Part in the Inspector; Preview and Apply)"
            }
            Self::Advanced(_) => {
                "Usage: choose explicit source geometry and controls in the Inspector, then Preview and Apply"
            }
            Self::Equipment => {
                "Usage: equipment (choose a family and floor/wall host, place in the apartment view, Preview and Apply)"
            }
            Self::Plant => {
                "Usage: plant (choose tree or shrub, click the garden plane, configure height/spread, Preview and Apply)"
            }
            Self::Stair => {
                "Usage: stairs (choose a level, rise, run, width and risers; Preview and Apply)"
            }
            Self::Level => {
                "Usage: level (define a level, then place floor slabs, pedestals, stairs or service envelopes in the Inspector; Preview and Apply)"
            }
            Self::Machining => {
                "Usage: drill (choose a groove, rabbet, pocket, drilling or hardware pattern, click target panel faces in the viewport or tick them in the Inspector, then Preview and Apply)"
            }
            Self::ShelfPins => {
                "Usage: system32 (select cabinet sides or click their inside faces, set rows, pitch, limits, diameter and depth in the Inspector, then Preview and Apply)"
            }
            Self::LooseTenon => {
                "Usage: domino (select or click the two mating panels, set the tenon, spacing, mortise depths and fits in the Inspector, then Preview and Apply)"
            }
            Self::Joint => {
                "Usage: joint (choose panel pairs that overlap at a corner and a butt, miter or finger joint in the Inspector, then Preview and Apply)"
            }
            Self::FdmAid => {
                "Usage: fdm-aid (choose tab, bridge or support, pick its contact and direction on the finished Part in the viewer or enter them in the Inspector, then Preview and Apply)"
            }
            Self::Weld => {
                "Usage: weld (attach a weld to a member connection with mark, symbol, size, process and notes, or order the weld sequence, in the Inspector; Preview and Apply)"
            }
            Self::Connection => {
                "Usage: cope (choose miter, butt or cope, the two members and a gap in the Inspector; Preview and Apply)"
            }
            Self::Member => {
                "Usage: tube (choose profile stock, then a sketch whose lines become members or two points, and the section placement in the Inspector; Preview and Apply)"
            }
            Self::Panel => {
                "Usage: panel (build a carcass from an envelope or place a rectangular panel from panel stock in the Inspector, then Preview and Apply)"
            }
            Self::Cabinet => {
                "Usage: cabinet (set outside width, floor-to-top height, depth and toe clearance, then add individual shelves and choose fixed joinery or System 32 support; Preview and Apply)"
            }
            Self::Sheet => {
                "Usage: sheet (choose sheet stock, base size and edge flanges, hems or jogs, optionally override the inside radius and K-factor in the Inspector, then Preview and Apply)"
            }
            Self::Fence => {
                "Usage: fence (choose or define a post and infill construction, draw the run in the viewport, then Preview and Apply)"
            }
            Self::Wall => {
                "Usage: wall (choose or define a wall type, draw the path in the viewport or enter its corners, add doors/windows in the Inspector, then Preview and Apply)"
            }
            Self::Door | Self::Window => {
                "Usage: door / window-tool (click a wall face in the viewport or choose the wall, segment and offset, pick or define a type in the Inspector, then Preview and Apply)"
            }
            Self::Cutout => {
                "Usage: vertical-cutout (click a wall face or choose the wall, segment and offset, set the width and through or chase depth, then Preview and Apply)"
            }
            Self::EdgeTreatment => {
                "Usage: edge-treatment (choose Part edges, a Fillet radius or Chamfer distances/angle and explicit adjacent faces, then Preview and Apply)"
            }
            Self::Scale => {
                "Usage: scale (select Parts or Instances, set positive factors and a pivot, then drag or Preview and Apply)"
            }
            Self::Align => {
                "Usage: align-body (select one moving object, choose source and target anchors, then Preview and Apply)"
            }
            Self::Move | Self::Copy => {
                "Usage: place / placed-copy (select Parts or Instances, set world translation and pivot/axis, then Preview and Apply)"
            }
            Self::Extrude => {
                "Usage: extrude (select closed sketch areas, open the controls, then Preview and Apply)"
            }
            Self::Box => "Usage: box (enter dimensions in the Inspector, then Preview and Apply)",
            Self::Cylinder => {
                "Usage: cylinder (enter radius and height in the Inspector, then Preview and Apply)"
            }
            Self::Revolve => {
                "Usage: revolve (select closed sketch areas, choose an axis in the Inspector, then Preview and Apply)"
            }
            Self::Boolean => {
                "Usage: boolean (choose separate targets and tools in the Inspector, then Preview and Apply)"
            }
            Self::Split => {
                "Usage: split (select Targets and a separate model or plane in Splitting tool; Enter changes field, then executes after validation)"
            }
        }
    }
    pub fn requirements(self) -> &'static str {
        match self {
            Self::MechanicalPart => {
                "Requires native modeling and no pending edits. Generic washers, hex nuts and bearing mounting envelopes; no verified stocked catalog designation."
            }
            Self::Gear => {
                "Requires native modeling and no pending edits. Supports unshifted external spur, helical, continuous herringbone and gapped double-helical gears."
            }
            Self::Thread => {
                "Requires native modeling and no pending edits. Receiving cuts need a complete matching cylinder/cone or sufficient internal wall; a new shaft does not require a sketch."
            }
            Self::Advanced(AdvancedKind::Sweep | AdvancedKind::Loft) => {
                "Choose current closed sketch profiles and an explicitly ordered path or section list. Native Preview validates the resulting solid."
            }
            Self::Advanced(_) => {
                "Choose one current Part definition. Linked Instances follow its editable source; face-based tools capture native geometry explicitly. Patterns create a compound Part."
            }
            Self::Equipment => {
                "Requires native modeling and no pending edits. Generic apartment equipment uses explicit floor, wall and ceiling references."
            }
            Self::Plant => {
                "Requires native modeling and no pending edits. Place a generic tree or shrub graphically, using one retained source for 3D context and authored plan footprints."
            }
            Self::Level | Self::Stair => {
                "Requires native modeling and no pending edits. Slabs and stairs are placed on an existing level; editing a level rebuilds everything on it. Stair figures are geometric notes, not code approval."
            }
            Self::Machining => {
                "Requires rectangular panels and no pending edits. Cuts are modeled geometry, not CNC toolpaths."
            }
            Self::ShelfPins => {
                "Requires rectangular panels and no pending edits. Rows are blind holes on each panel's inside face; settings are project choices, not a hardware compatibility claim."
            }
            Self::LooseTenon => {
                "Requires two rectangular panels where one panel's edge lies flat on the other's face, and no pending edits. Modeled mortises and tenons, not toolpaths or a machine compatibility claim."
            }
            Self::Joint => {
                "Requires rectangular panels that overlap at a right-angle corner, one full thickness of each, and no pending edits."
            }
            Self::FdmAid => {
                "Requires native modeling, a finished Part and no pending edits. Each aid is its own Part; the finished Part is not changed. No slicer, support generation or G-code."
            }
            Self::Weld => {
                "Requires native modeling, a member connection and no pending edits. Welds and sequences are documentation, not welding procedures or calculations."
            }
            Self::Connection => {
                "Requires native modeling, two weldment members and no pending edits. Cuts are derived from both members; unsupported joints are refused with a reason."
            }
            Self::Member => {
                "Requires native modeling, profile stock defined in Model → Materials and no pending edits. Members follow straight sketch lines or point pairs."
            }
            Self::Panel | Self::Cabinet => {
                "Requires native modeling, panel stock defined in Model → Materials and no pending edits. Carcass panels are edited through their carcass."
            }
            Self::Sheet => {
                "Requires native modeling, sheet stock defined in Model → Materials and no pending edits. Sketch-region bases and solid conversion are available through MCP."
            }
            Self::Fence => {
                "Requires native modeling, project materials and no pending edits. Draw a flat open or closed run; Preview checks each post and infill member."
            }
            Self::Wall => {
                "Requires native modeling and no pending edits. Wall and opening types are created with the wall or reused from the project."
            }
            Self::Door | Self::Window => {
                "Requires an existing wall, native modeling and no pending edits. Places one opening of a reusable type at a segment offset; misfitting, top-reaching or overlapping openings are refused. Reopen an existing opening from the same tool to edit or remove it."
            }
            Self::Cutout => {
                "Requires an existing wall or floor slab, native modeling and no pending edits. Choose what it cuts: the full wall height, through the wall or as a chase from one face, and/or the floor slabs under it on a range of levels (shafts, floor penetrations). Cutouts that would split the wall or cross a slab edge are refused."
            }
            Self::EdgeTreatment => {
                "Select native edges of one Part definition. Sided Chamfer distances/angles use explicit adjacent face captures. Linked Instances follow the definition."
            }
            Self::Scale => {
                "Select current native Parts or Instances and finish pending edits. Positive scale factors change exact geometry; an Instance selection affects only that occurrence."
            }
            Self::Align => {
                "Select one current native Part or Instance. Anchors need exact named face frames, line/circle/plane axes, vertices or explicit datums."
            }
            Self::Move | Self::Copy => {
                "Select 1–128 current native Parts or Instances and finish pending edits. Placement changes use one native-validated transaction."
            }
            Self::Extrude | Self::Revolve => {
                "Select current closed areas in a sketch and finish pending edits. Native modeling must be available."
            }
            Self::Box | Self::Cylinder => {
                "Requires native modeling and no pending edits. A sketch is not required."
            }
            Self::Boolean => {
                "Requires at least two current native bodies and no pending edits. Targets and tools are chosen explicitly."
            }
            Self::Split => {
                "Choose one or more body targets and one separate plane or closed model. Finish pending edits first. Both resulting sides must be nonempty and connected."
            }
        }
    }
}

/// A tool instance and its current local inputs. A successful native preview
/// must carry this exact stamp before the form offers Apply. The host also
/// checks the source/session identity and native candidate receipt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FormStamp {
    pub tool: EntityId,
    pub revision: u64,
}
impl FormStamp {
    fn new() -> Self {
        Self {
            tool: EntityId::new(),
            revision: 0,
        }
    }
    fn changed(&mut self) {
        if let Some(next) = self.revision.checked_add(1) {
            self.revision = next;
        } else {
            *self = Self::new();
        }
    }
}

/// Presentation of an independently owned native preview. This is not proof of
/// geometry validity; only the host holding the matching checked transaction
/// may publish Ready or handle Apply.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PreviewState<'a> {
    #[default]
    Idle,
    Pending {
        stamp: FormStamp,
    },
    Ready {
        stamp: FormStamp,
    },
    Failed {
        stamp: FormStamp,
        message: &'a str,
    },
}
impl PreviewState<'_> {
    fn ready(self, current: FormStamp) -> bool {
        matches!(self, Self::Ready { stamp } if stamp == current)
    }
}
