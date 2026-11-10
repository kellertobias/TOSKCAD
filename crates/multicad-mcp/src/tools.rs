//! Typed MCP tools over the shared Session, solver and native model. Every
//! mutation names the revision it was based on and commits one attributable
//! transaction; failures and cancellation leave the document unchanged.
use crate::{
    error::ToolError,
    host::{Host, Target, with},
};
use multicad_core::{
    commands::{Actor, Prepared, Request as SessionRequest},
    constraint_sketch::{
        Axis, Constraint, ConstraintKind, Curve, CurveKind, Dimension, Point,
        binding::{DimensionInput, DimensionRequest},
        source::Source,
    },
    control::{AuditOutcome, AuditRecord, PendingAction, PendingChange, ReviewPolicy},
    document::{Entity, EntityData, EntityId, Mutation},
    model::{
        self, BodyInput, Direction, Extent, ExtrudeBooleanRequest, ExtrudeParameters,
        ExtrudeRequest, Operation, Termination,
    },
    parameters::{self, BoundExpression, Unit},
    sketch::Datum,
    sketch_import::{self, Format, ImportOptions, SourceUnit, UnitChoice},
    sketch_regions::RegionQueries,
};
use multicad_kernel::NativeRegionQueries;
use multicad_model::{Compiler, GeometryBackend};
use schemars::JsonSchema;
use serde::{Deserialize, de::DeserializeOwned};
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

mod architecture;
mod cabinetry;
mod wood_boxes;
use wood_boxes::{
    WoodBoxArgs, WoodBoxInspectArgs, WoodBoxRemoveArgs, inspect_wood_box, preview_wood_box,
    remove_wood_box, set_wood_box,
};
mod compound_cutouts;
use compound_cutouts::{
    CompoundCutoutArgs, CompoundCutoutInspectArgs, CompoundCutoutRemoveArgs,
    inspect_compound_cutout, preview_compound_cutout, remove_compound_cutout, set_compound_cutout,
};
mod engraving;
mod fdm;
mod profile_connections;
use profile_connections::{
    ProfileConnectionArgs, ProfileConnectionInspectArgs, ProfileConnectionRemoveArgs,
    inspect_profile_connection, remove_profile_connection, set_profile_connection,
};
mod screw_connections;
use screw_connections::{
    ScrewConnectionArgs, ScrewConnectionInspectArgs, ScrewConnectionRemoveArgs,
    inspect_screw_connection, remove_screw_connection, set_screw_connection,
};
mod gear_pairs;
mod gears;
mod guides;
mod imports;
mod levels;
mod mechanical_parts;
mod plants;
mod symbols;
use plants::{PlantArgs, PlantInspectArgs, inspect_plant, set_plant};
mod equipment;
mod fences;
use equipment::{
    EquipmentArgs, EquipmentInspectArgs, equipment_profiles, inspect_equipment, set_equipment,
};
use fences::{
    FenceArgs, FenceInspectArgs, FenceTypeArgs, FenceTypeInspectArgs, inspect_fence,
    inspect_fence_type, preview_fence, set_fence, set_fence_type,
};
mod reference_repair;
mod render;
mod schedules;
mod sheet_metal;
mod surface_structure;
use surface_structure::{
    SurfaceStructureArgs, SurfaceStructureCaptureArgs, SurfaceStructureInspectArgs,
    capture_surface_structure_faces, inspect_surface_structure, preview_surface_structure,
    set_surface_structure,
};
mod text;
mod threads;
mod wall_layers;
mod walls;
pub use cabinetry::{
    CarcassArgs, DeleteJointArgs, DeleteMachiningArgs, HardwareArgs, MachiningArgs, PanelArgs,
    PanelJointArgs, PanelStockArgs, StackArgs,
};
use cabinetry::{
    create_panel, create_panel_stack, create_panel_stock, cut_list, delete_machining,
    delete_panel_joint, set_carcass, set_hardware_pattern, set_machining, set_panel_joint,
};
pub use engraving::EngravingArgs;
use engraving::create_engraving_sketch;
pub use fdm::{AidArgs, AidKindArg, ProcessArgs, RemoveAidArgs, TabContactArgs};
use fdm::{fdm_aid_report, fdm_tab_contact, remove_fdm_aid, set_fdm_aid, set_fdm_process};
pub use gear_pairs::{GearPairArgs, InspectGearPairArgs};
use gear_pairs::{inspect_gear_pair, set_gear_pair};
pub use gears::{GearArgs, InspectGearArgs};
use gears::{gear_profiles, inspect_gear, set_gear};
pub use mechanical_parts::{InspectMechanicalPartArgs, MechanicalPartArgs};
use mechanical_parts::{inspect_mechanical_part, mechanical_part_profiles, set_mechanical_part};
pub use symbols::{
    EditSymbolInstanceArgs, ImportSymbolArgs, PlaceLibrarySymbolArgs, PlaceSymbolArgs,
};
use symbols::{edit_symbol_instance, import_symbol, place_library_symbol, place_symbol};
pub use text::{
    AddTextArgs, EditTextArgs, RemoveTextArgs, TextAlignmentArg, TextAnchorArg, TextFontArg,
    TextStyleArgs,
};
use text::{sketch_add_text, sketch_edit_text, sketch_remove_text};
pub use threads::{InspectThreadArgs, ThreadArgs};
use threads::{inspect_thread, set_thread, thread_profiles};
mod weldment;
use guides::set_drawing_guide;
pub use guides::{GuideArgs, GuideKindArg};
pub use imports::{KicadImportArgs, StepImportArgs};
use imports::{import_kicad, import_step};
pub use levels::{EnvelopeArgs, LevelArgs, SlabArgs, StairArgs};
use levels::{check_clearances, create_envelope, create_slab, set_level, set_stair};
pub use reference_repair::{
    CandidateArgs, ElementArg, ElementKind, PreviewArgs, RepairArgs, StepArg,
};
use reference_repair::{apply_reference_repairs, preview_reference_repairs, reference_candidates};
pub use render::{AssignAppearanceArgs, CreateAppearanceArgs, HandoffArgs, RenderSceneArgs};
use render::{assign_appearance, blender_handoff, create_appearance, set_render_scene};
use schedules::schedule;
pub use schedules::{ScheduleArgs, ScheduleKindArg};
pub use sheet_metal::{
    ConvertSheetArgs, SheetDxfArgs, SheetEdgesArgs, SheetFastenersArgs, SheetFormingArgs,
    SheetPartArgs, SheetStockArgs, UnfoldSheetArgs,
};
use sheet_metal::{
    convert_to_sheet, create_sheet_stock, export_sheet_dxf, set_sheet_fasteners, set_sheet_forming,
    set_sheet_part, sheet_edges, sheet_hardware, sheet_metal, unfold_sheet,
};
pub use wall_layers::{
    CreateLayeredWallTypeArgs, EditLayeredWallTypeArgs, InspectWallConstructionArgs,
};
use wall_layers::{create_layered_wall_type, edit_layered_wall_type, inspect_wall_construction};
pub use walls::{
    CreateWallArgs, EditWallArgs, EditWallTypeArgs, FaceArg, InspectVerticalCutoutArgs,
    InspectWallArgs, NewOpeningTypeArg, OpeningTypeArgs, PlanRectangleArg,
    RemoveVerticalCutoutArgs, RemoveWallOpeningArgs, VerticalCutoutArgs, WallArgs, WallOpeningArgs,
    WallTypeArgs,
};
use walls::{
    create_opening_type, create_wall, create_wall_type, edit_wall, edit_wall_type,
    inspect_vertical_cutout, inspect_wall, remove_vertical_cutout, remove_wall_opening,
    set_vertical_cutout, set_wall_opening,
};
pub use weldment::{
    ConnectArgs, EditMemberArgs, MembersArgs, ProfileStockArgs, SequenceArgs, WeldArgs,
    WeldPlanArgs,
};
use weldment::{
    connect_members, create_members, create_profile_stock, edit_member, place_weld_plan, set_weld,
    set_weld_sequence, weld_schedule, weldment_cut_list,
};

pub type ToolResult = Result<Value, ToolError>;

/// Per-call context: who acts and whether the client has cancelled.
#[derive(Clone)]
pub struct Call {
    pub actor: Actor,
    pub cancel: Arc<AtomicBool>,
}
/// One dispatched request: its call, tool and arguments, and the review it
/// was queued as, if the owner required review.
#[derive(Clone)]
struct Request {
    call: Call,
    backend: GeometryBackend,
    tool: &'static str,
    arguments: Value,
    review: Arc<std::sync::Mutex<Option<u64>>>,
}
impl Request {
    fn region_queries(&self) -> &'static dyn RegionQueries {
        static NATIVE: std::sync::LazyLock<NativeRegionQueries> =
            std::sync::LazyLock::new(NativeRegionQueries::default);
        static RUST: std::sync::LazyLock<NativeRegionQueries> = std::sync::LazyLock::new(|| {
            NativeRegionQueries::analytic(Default::default())
                .expect("valid default geometry policy")
        });
        match self.backend {
            GeometryBackend::OpenCascade => &*NATIVE,
            GeometryBackend::Rust => &*RUST,
        }
    }

    fn require_open_cascade(&self, workflow: &str) -> Result<(), ToolError> {
        self.check()?;
        if self.backend == GeometryBackend::Rust {
            return Err(ToolError::new(
                "unsupported",
                format!("{workflow} is not supported by the Rust geometry backend"),
            ));
        }
        Ok(())
    }
}
impl std::ops::Deref for Request {
    type Target = Call;
    fn deref(&self) -> &Call {
        &self.call
    }
}
impl Call {
    fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }
    /// Interrupted native work reports cancellation, not its own failure.
    fn finish<T, E: Into<ToolError>>(&self, result: Result<T, E>) -> Result<T, ToolError> {
        self.check()?;
        result.map_err(Into::into)
    }
    fn check(&self) -> Result<(), ToolError> {
        if self.cancelled() {
            Err(ToolError::cancelled())
        } else {
            Ok(())
        }
    }
}

pub struct ToolInfo {
    pub name: &'static str,
    pub title: &'static str,
    pub description: &'static str,
    pub read_only: bool,
    pub schema: Value,
}

macro_rules! tools {
    ($( $name:literal => $args:ty, $title:literal, $read_only:expr, $description:literal, $run:path; )*) => {
        /// Stable tool catalog. Names and argument schemas are a compatibility
        /// contract documented in docs/architecture/mcp-server.md.
        pub fn catalog() -> Vec<ToolInfo> {
            vec![$(ToolInfo {
                name: $name,
                title: $title,
                description: $description,
                read_only: $read_only,
                schema: serde_json::to_value(schemars::schema_for!($args)).expect("schema serializes"),
            }),*]
        }
        /// Run a tool synchronously and record its outcome in the owner's
        /// audit. Callers execute this off the async runtime.
        pub fn dispatch(host: &dyn Host, call: &Call, name: &str, arguments: Value) -> ToolResult {
            let (tool, read_only) = match name {
                $($name => ($name, $read_only),)*
                _ => return Err(ToolError::new("unknown_tool", format!("Unknown tool '{name}'"))),
            };
            let request = Request {
                call: call.clone(),
                backend: host.geometry_backend(),
                tool,
                arguments: arguments.clone(),
                review: Arc::default(),
            };
            let result = if !read_only && host.scope().read_only {
                Err(ToolError::new("read_only", "This server was started read-only; mutating tools are refused"))
            } else {
                match tool {
                    $($name => parse::<$args>(arguments).and_then(|args| $run(host, &request, args)),)*
                    _ => unreachable!("matched above"),
                }
            };
            finish(host, &request, read_only, result)
        }
        /// Record a known tool rejected before native dispatch, preserving the
        /// same actor, arguments and append-only failure audit as normal calls.
        pub(crate) fn reject(host: &dyn Host, call: &Call, name: &str, arguments: Value, error: ToolError) -> ToolResult {
            let (tool, read_only) = match name {
                $($name => ($name, $read_only),)*
                _ => return Err(ToolError::new("unknown_tool", format!("Unknown tool '{name}'"))),
            };
            let request = Request {
                call: call.clone(),
                backend: host.geometry_backend(),
                tool,
                arguments,
                review: Arc::default(),
            };
            finish(host, &request, read_only, Err(error))
        }
    };
}

tools! {
    "document_info" => NoArgs, "Document information", true,
        "Name, identity and current revision of the controlled document, undo/redo availability, entity counts, geometry backend and access scope (project file, export folder, read-only, transport).",
        document_info;
    "list_entities" => ListEntities, "List entities", true,
        "Canonical entities with persistent IDs, names, kinds and references. Optionally filter by kind (parameter, plane, sketch, feature, part, assembly, instance, material, drawing, export_set, extension, group).",
        list_entities;
    "get_entity" => EntityArgs, "Get entity", true,
        "The complete canonical source record of one entity, exactly as stored in the project.",
        get_entity;
    "list_parameters" => NoArgs, "List parameters", true,
        "Every project parameter with its expression, declared unit, evaluated value in that unit, role (driving or derived) and exposure. Parameters private to placed symbols are omitted; change them through the instance's exposed overrides.",
        list_parameters;
    "set_parameter" => SetParameter, "Create or edit parameter", false,
        "Create a parameter, or edit the one with this name. Literal values are plain numbers ('40'); formulas start with '=' and reference parameters with '$' ('=$width/2'). The unit is required. Commits one transaction.",
        set_parameter;
    "create_sketch" => CreateSketch, "Create sketch", false,
        "Create an empty constrained sketch on an origin datum plane (xy, xz or yz). Commits one transaction and returns the sketch ID.",
        create_sketch;
    "create_engraving_sketch" => EngravingArgs, "Create engraving sketch", false,
        "Create an empty sketch attached to a current planar face of a source feature (face ordinal from reference_candidates at expected_revision), named \"Engraving N\" unless given. Curved, missing or ambiguous faces are refused and nothing changes. Defines only the sketch: no engraving depth, material removal or toolpath. Commits one transaction.",
        create_engraving_sketch;
    "sketch_add_rectangle" => AddRectangle, "Add rectangle", false,
        "Add a fully constrained axis-aligned rectangle: the first corner is fixed at origin_mm (sketch coordinates, mm), width along sketch U and height along sketch V are driving dimensions (literal or parameter formula). Commits one transaction.",
        sketch_add_rectangle;
    "sketch_add_circle" => AddCircle, "Add circle", false,
        "Add a circle whose center is fixed at center_mm (sketch coordinates, mm) with a driving diameter dimension (literal or parameter formula). Commits one transaction.",
        sketch_add_circle;
    "sketch_add_text" => AddTextArgs, "Add sketch text", false,
        "Add an editable text element to a sketch: printable ASCII (or ÄÖÜäöüßéèàç°±µ) with \\n line breaks, size (cap height) in mm, the anchor position in sketch mm, the anchor (top_left, top_center, top_right, left_center, center, right_center, bottom_left, bottom_center, bottom_right of the text block box; default bottom_left), line alignment (left, center, right), rotation about the anchor in degrees and font (cantarell_regular proportional, default; hack_regular monospaced). Its exact glyph outlines (closed quadratic B-splines) become closed regions with counters as holes, listed by inspect_sketch for extrude. Empty text, invalid size, unsupported characters or more than 64 glyph contours per sketch are refused and nothing changes. Commits one transaction.",
        sketch_add_text;
    "sketch_edit_text" => EditTextArgs, "Edit sketch text", false,
        "Replace the string, font, size, anchor position, anchor, rotation or line alignment of a sketch text element (text_id from sketch_add_text or inspect_sketch). Contour identities follow character and contour position, so extrusions of unchanged glyph structure rebuild. Validation matches sketch_add_text; commits one transaction.",
        sketch_edit_text;
    "sketch_remove_text" => RemoveTextArgs, "Remove sketch text", false,
        "Remove a sketch text element. Features that used its regions report that their profile changed. Commits one transaction.",
        sketch_remove_text;
    "set_sketch_dimension" => SetDimension, "Set sketch dimension", false,
        "Change the value of an existing driving dimension constraint to a literal or parameter formula. The sketch is re-solved; a failed solve changes nothing.",
        set_sketch_dimension;
    "inspect_sketch" => SketchArgs, "Inspect sketch", true,
        "Solve a sketch and report its state, degrees of freedom, conflicts, solved points, curves, constraints, text elements (with their derived glyph contour curve IDs) and the exact closed regions (with index and area in mm²) usable by extrude.",
        inspect_sketch;
    "extrude" => Extrude, "Extrude regions", false,
        "Extrude regions of a sketch (indices from inspect_sketch at the same revision), or planar model faces (source feature and face ordinal from reference_candidates at the same revision; exact outer boundary and holes, persistent face reference), by a signed nonzero distance. Give either sketch with regions or faces. A negative distance extrudes the same magnitude opposite to the profile normal (for faces: into the body) without changing the operation. operation new_body creates parts; join, cut and intersect modify target_part. Curved, missing or ambiguous faces are refused and nothing changes. The candidate is evaluated natively before it commits.",
        extrude;
    "evaluate_model" => NoArgs, "Evaluate model", true,
        "Evaluate the complete model with the native kernel and report every final part (producing feature, solid count, volume in mm³, bounds in mm) and every placed instance (component, solid count, volume, bounds), plus reference diagnostics. Evaluation failures are returned as structured errors.",
        evaluate_model;
    "thread_profiles" => NoArgs, "Thread profile coverage", true,
        "Offline supported thread-family matrix, metric and explicit UNC/UNF inch/TPI presets, historical primary sources, external 2A/internal 2B applicable diameter limits and bounded coverage. Unspecified limit endpoints are null. Unavailable roadmap families and manufacturing-representation semantics are explicit.",
        thread_profiles;
    "inspect_thread" => InspectThreadArgs, "Inspect thread source", true,
        "Read the canonical thread definition, separate basic nominal and fitted construction pitch diameters, sourced fit class/limits, pitch/lead and representation at the current revision. This source-only inspection does not certify native geometry, gage compliance or physical fit; use evaluate_model for native geometry.",
        inspect_thread;
    "set_thread" => ThreadArgs, "Create or edit thread", false,
        "Create a new threaded shaft or cut an external/internal thread on target_part; edit an existing thread by feature ID while preserving omitted fields, Part and original target predecessor. All dimensions carry units; custom profile angles and radius taper accept typed values/formulas. UNC/UNF require a listed diameter/pitch and explicit unified_2a externally or unified_2b internally; omitted edit fields retain the saved fit. These historical uncoated single-start UN flat-root fits are bounded to length <=1.5 nominal diameters; metric/Custom nominal-basic fits and fabrication clearance remain separate. Native Preview checks complete receiving geometry before one attributable transaction or the owner's review queue. Unsupported families, invalid walls and failed geometry change nothing. Cosmetic envelopes are display-only and cannot be exported as STEP/mesh/print manufacturing geometry.",
        set_thread;
    "equipment_profiles" => NoArgs, "Apartment equipment families", true,
        "List owned generic equipment families, dimensions, categories, connector convention and floor/wall/ceiling mount classes. No manufacturer or installation certification.",
        equipment_profiles;
    "inspect_equipment" => EquipmentInspectArgs, "Inspect hosted equipment", true,
        "Read the canonical equipment ownership record, pinned family, dimensions, floor/wall/ceiling attachment, optional labels and clearance visibility. Unresolved hosts are reported while source remains available for repair.",
        inspect_equipment;
    "set_equipment" => EquipmentArgs, "Place, edit or repair equipment", false,
        "Create a generic apartment equipment family or edit its retained ownership record. Explicit length units and floor/host references share the UI source validation and native preflight. Replacing host intent repairs lost hosts; omitted edits retain fields. Service and door access boxes are separate from physical geometry. Existing instances retain their pinned family.",
        set_equipment;
    "set_plant" => PlantArgs, "Place or edit a plant", false,
        "Place a generic editable tree or shrub, or edit a retained plant by canonical ownership record. Lengths require explicit units. Spread is the true outer canopy footprint. UI and MCP share one native-preflighted transaction, pinned source, linked plan sketch and occurrence placement. Optional plan visibility does not remove physical sections/elevations. Generic envelopes do not predict species or growth.",
        set_plant;
    "inspect_plant" => PlantInspectArgs, "Inspect linked plant source", true,
        "Inspect resolved plant height/spread, planned-mature versus current size meaning, optional label, plan visibility, current pose, retained pin and linked plan sketch.",
        inspect_plant;
    "mechanical_part_profiles" => NoArgs, "Mechanical part geometry coverage", true,
        "Read supported generic hardware geometry and declared catalog/representation limits. This is not a stocked standard-parts catalog.",
        mechanical_part_profiles;
    "inspect_mechanical_part" => InspectMechanicalPartArgs, "Inspect mechanical part source", true,
        "Read retained mechanical Part dimensions, optional reference attribution and representation without native evaluation. A reference match does not certify stock, fit or standards compliance.",
        inspect_mechanical_part;
    "set_mechanical_part" => MechanicalPartArgs, "Create or edit mechanical part", false,
        "Create a parametric washer, hex nut, headed machine screw or bearing mounting envelope, or edit by generator feature ID retaining Part identity and omitted fields. Machine-screw underhead_length retains neck-to-tip semantics; alternatively length_input selects kind underhead or overall with a typed value. Exactly one authored length is retained; supplying both refuses, omission preserves its expression and convention. Overall includes actual head height. thread.length is its threaded extent and thread.frame locates the neck datum. Head and drive kinds are explicit; switching a kind requires all its dimensions. Supported heads include socket-cap, external hex, pan, button and countersunk with compatible external-hex, hex-socket, hexalobular or Phillips H drives. Hexalobular recesses use tangent circular arcs; Phillips H0 uses a rounded inner form and H1–H4 use a shallow V. Profile angles require angular units deg or rad; length quantities require length units. Omitted fields preserve their authored expressions. Socket recesses are real hexagons. Nut thread.length is its height. Cosmetic threads are refused for manufacturing export until Modeled. Symbol-owned features must be changed through edit_symbol_instance. Native evaluation precedes one transaction or owner review; stale, invalid, cancelled and failed requests change nothing. No sourced catalog membership, fitted metric tolerance or stock availability is implied.",
        set_mechanical_part;
    "import_symbol" => ImportSymbolArgs, "Import pinned symbol", false,
        "Import one canonical reusable definition from a bare .tcd file inside the exchange folder. source_definition is the source extension ID and is required for ambiguous files. Returns the embedded definition ID, immutable pin and exposed template parameter IDs. Source import does not place or certify native geometry; private template source remains owned.",
        import_symbol;
    "place_library_symbol" => PlaceLibrarySymbolArgs, "Place component from library", false,
        "Atomically embed or reuse one exact pinned reusable definition from an authorized exchange-folder .tcd file and place one instance. source_definition is required for files with multiple definitions. Required overrides use exposed template IDs; empty inherits defaults. Optional typed placement and canonical parent container choose the receiving context; omitted parent means project root. A conflicting embedded revision requires an explicit update. Native preflight precedes one attributable transaction or owner review; missing, incompatible, cancelled and rejected candidates change nothing. Architectural constructions use dedicated draw tools.",
        place_library_symbol;
    "place_symbol" => PlaceSymbolArgs, "Place pinned symbol", false,
        "Instantiate an embedded definition with a required overrides map keyed by exposed TEMPLATE parameter IDs, each with a literal value and unit. Empty inherits pinned defaults. Optional placement has three typed translation lengths and an x/y/z/w unit quaternion. The complete candidate passes cancellation-aware native evaluation before commit or owner review. Architectural paths use their dedicated draw tools.",
        place_symbol;
    "edit_symbol_instance" => EditSymbolInstanceArgs, "Configure symbol instance", false,
        "Configure a placed instance or ownership record through canonical exposed overrides, preserving its pin, identities and placement. overrides is a required REPLACEMENT map, not a partial patch: omitted keys inherit pinned defaults and an empty map restores all defaults. Keys are exposed template IDs, never private materialized IDs. Native preflight, revision, cancellation and owner review guards apply; direct private-source edits remain refused.",
        edit_symbol_instance;
    "gear_profiles" => NoArgs, "Gear profile coverage", true,
        "Read supported gear families, primary dimension/profile sources and limits. Unsupported families and pair placement are explicit.",
        gear_profiles;
    "inspect_gear" => InspectGearArgs, "Inspect gear source", true,
        "Read canonical gear source and actual rotary/rack/bevel/worm dimensions. Worm uses axial module with normal20-degree pressure and independent worm length / wheel face width. Bevel reports cone angles and distances, not cylindrical spur radii; nominal/design/physical tooth thickness remains separate. Use evaluate_model for native geometry.",
        inspect_gear;
    "set_gear" => GearArgs, "Create or edit gear", false,
        "Create a parametric involute gear or finite straight rack, or edit by feature ID, preserving omitted fields and Part identity. The one module scalar uses pitch_input module (mm, legacy default) or diametral_pitch (teeth_per_in), independently of normal/transverse convention; no inactive numeric copy. Switching pitch_input requires a correctly typed authored scalar. Optional intrinsic hub diameter/axial_start/axial_end is along localZ, not another Part. hub patches retain omitted dimensions; clear_hub explicitly removes it. Nonzero profile_shift is bounded to external Spur/Helical/Herringbone/Double-helical x[-0.5,1] in the selected normal/transverse module plane. Selected pressure14.5–35deg and selected-plane source undercut, crest/root/active-line/contact-ratio guards remain. Optional shift_mate teeth/profile_shift retains a KHK clearance-matched tip context in the same selected plane; absent is standalone unshortened. clear_shift_mate explicitly removes standalone context. Custom roots are not generated rack trochoids. Helix magnitude and hand are separate. Backlash is per-gear reference-circle tooth thinning; fabrication_allowance is additional thinning. Supports spur, helical, continuous herringbone, gapped double-helical, bounded internal ring, straight rack, spherical-involute straight bevel, and bounded ZA worm/wheel. Standalone worm requires worm_role and mate_teeth. Worm module_system is axial with NORMAL pressure_angle20deg (derived axial angle), q10, one right-hand start; module0.5–2mm, wheel32–64 teeth. face_width independently means worm threaded length16–32 times module or wheel axial width0.5–2 times module. No ZI/ZN/ZK, efficiency, load or physical contact certification. Standalone bevel creation requires mate_teeth and shaft_angle; only 20-degree pressure, 90-degree shafts and 24–64 teeth are supported. Bevel origin is the pitch apex and face_width runs along the pitch cone; KG/KHK backcones and explicitly custom spherical-circle roots apply. Gleason/octoid/spiral/Zerol remain unavailable. Rack teeth means whole centered pitch cells, length is teeth*pi*module, and backing_thickness defines its connected base below the root line. Rack helix, bore and profile shift are unsupported. Native evaluation precedes one attributable transaction or owner review. Unsupported family/shift combinations, source undercut policy, insufficient walls and failed geometry change nothing. Pair placement is not provided by this tool.",
        set_gear;
    "set_screw_connection" => ScrewConnectionArgs, "Create or edit screw connection", false,
        "Atomically create or edit a connection using the complete canonical ScrewConnectionRequest. Stable connection IDs edit in place. Native candidate validation precedes commit; no partial hardware or receiver changes on failure.", set_screw_connection;
    "inspect_screw_connection" => ScrewConnectionInspectArgs, "Inspect screw connection", true,
        "Read the complete canonical editable request and normalized owned references for a screw connection.", inspect_screw_connection;
    "remove_screw_connection" => ScrewConnectionRemoveArgs, "Remove screw connection", false,
        "Atomically remove owned connection hardware and receiving features. Refuse foreign/downstream ownership conflicts; preserve unrelated geometry.", remove_screw_connection;
    "set_profile_connection" => ProfileConnectionArgs, "Create or edit profile connection", false,
        "Create or edit one item Line 8 40 × 40 profile connection using the complete canonical request. Supported kinds are angle_bracket and joining_plate (two groove anchors), and universal_end (first member end and second member groove, with access bores in a unique receiving Part). The optional nut defaults to zinc_plated; angle_bracket and joining_plate also accept stainless. Groove stations are typed lengths measured from the current extended start. Supply exact member body/producer IDs and occurrence paths. Keep the connection UUID when editing to retain owned Part IDs. Native candidate validation precedes one transaction or owner review.", set_profile_connection;
    "inspect_profile_connection" => ProfileConnectionInspectArgs, "Inspect profile connection", true,
        "Read the retained connector and nut articles and complete editable canonical request for a profile connection. This is source inspection.", inspect_profile_connection;
    "remove_profile_connection" => ProfileConnectionRemoveArgs, "Remove profile connection", false,
        "Atomically remove a profile connection, its owned hardware and its access bores where applicable. Downstream ownership conflicts and failed native validation preserve the current source.", remove_profile_connection;
    "capture_surface_structure_faces" => SurfaceStructureCaptureArgs, "Capture Surface Structure faces", true,
        "Capture one to four distinct current planar faces of a single Part using ordinals from reference_candidates. Returns canonical input and faces for preview_surface_structure or set_surface_structure.", capture_surface_structure_faces;
    "inspect_surface_structure" => SurfaceStructureInspectArgs, "Inspect Surface Structure", true,
        "Read the exact editable Surface Structure request, including embedded grayscale samples and retained planar face references.", inspect_surface_structure;
    "preview_surface_structure" => SurfaceStructureArgs, "Preview Surface Structure", true,
        "Check a complete Surface Structure request against native geometry without committing. Parameters select ribs, waves, stipple or height_map, raised or recessed relief, dimensions for amplitude, pitch, size, offset, rotation and border. Height Map embeds row-major grayscale samples with row zero at positive V.", preview_surface_structure;
    "set_surface_structure" => SurfaceStructureArgs, "Apply Surface Structure", false,
        "Create or edit a Surface Structure on its checked Part and planar face captures. Reuse the feature UUID from inspect_surface_structure to edit. Commits one transaction or stages it for owner review after a native geometry check.", set_surface_structure;
    "inspect_gear_pair" => InspectGearPairArgs, "Inspect gear pair", true,
        "Read canonical member recipes, tooth phase and mating frames, plus rotary center distance, finite rack travel, explicit bevel shared-apex/cone context or explicit worm/wheel crossed-axis roles without evaluating geometry. evaluate_model reports the actual native current-pose and sampled interference checks.",
        inspect_gear_pair;
    "set_gear_pair" => GearPairArgs, "Create or edit gear pair", false,
        "Create two named parametric gears, a reusable assembly, pair-controlled Fixed relationships and one ordinary placed occurrence in one native-checked transaction. Default first member has 20 teeth (24 for bevel); creation inherits its settings for a 30 tooth opposite-hand second member before applying explicit overrides. A first rack instead defaults the second member to a 24 tooth spur pinion. Pair origin/axes control layout; member frames describe local definitions. Edits retain omitted member fields independently and can update both recipes atomically; drive-only edits preserve them. Intrinsic hubs and own profile_shift values are independent, and a new second member defaults to no hub/zero shift. Shifted external cylindrical pairs clone opposite ORIGINAL typed count/shift into both matched contexts, derive KHK operating center/tip shortening, and refuse explicit typed conflicts even for equal evaluated values. Omitted contexts follow edited opposite sources. Hub collisions participate in native whole-body checks. Coverage includes matching external gears, bounded internal ring/spur pairs, finite rack/spur pairs, 90-degree spherical-involute straight bevel pairs, and bounded ZA worm/wheel pairs. A first Worm family member requires explicit worm_role; creation defaults the opposite role to wheel32 teeth / width1mm or worm1 start / length24mm. Axial module uses NORMAL pressure20deg; q10 and right hand only. Independent member face_width means threaded worm length versus wheel face width. Worm drive_angle is the worm angle for either member order; wheel follows one-start tooth ratio. Opposite mate_teeth and wheel quotient derive from original typed sources; explicit conflicts refuse atomically. Bevel mate counts clone opposite original typed teeth; omitted shaft_angle shares the existing source or creation default90deg. Shifted helical/chevron pairs require matching selected normal/transverse conventions; unsupported mixed shifted conventions refuse. Explicit conflicting mate/shaft sources are refused even when their evaluated values match. No Gleason/octoid/spiral/Zerol claim. Rack drive couples pinion angle to positive local X travel and refuses beyond the conservative finite tip-circle envelope. Native current pose and nine phases over one pinion tooth cycle are checked; rack pairs also check both finite travel endpoints and native proximity. Rotary cycle reuse assumes unmodified periodic generators; this does not certify continuous collision-free motion, strength or fit.",
        set_gear_pair;
    "undo" => RevisionArgs, "Undo", false,
        "Undo the most recent committed transaction, from any channel, if expected_revision is current.",
        undo;
    "redo" => RevisionArgs, "Redo", false,
        "Redo the most recently undone transaction if expected_revision is current.",
        redo;
    "history" => HistoryArgs, "Activity history", true,
        "Recent committed activity: actor, channel, label, kind (apply, undo, redo) and changed entities.",
        history;
    "review_status" => ReviewArgs, "Review status", true,
        "State of a change queued for review: pending (with whether it is stale), applied with its revision, rejected, stale (the document moved; prepare it again) or failed.",
        review_status;
    "activity" => HistoryArgs, "Control activity", true,
        "The append-only audit of this document: every external request (inspected, previewed, applied, rejected, cancelled, failed, undone, redone, stale) and imported window actions, with actor, tool, arguments, base and resulting revision and affected IDs. Undo never removes entries.",
        activity;
    "save_project" => RevisionArgs, "Save project", false,
        "Save the document to the project file in scope, if expected_revision is current. Never writes elsewhere.",
        save_project;
    "export_step" => ExportStep, "Export STEP", true,
        "Write one current Part definition as a STEP B-rep into the export folder in scope (bare file name ending .step or .stp), including definition-only Parts of an Assembly. Uses definition coordinates; export sets with occurrence paths select posed members. Consumed Parts remain unavailable. The file is read back and its volume reported. Does not change the document.",
        export_step;
    "set_level" => LevelArgs, "Create or edit level", false,
        "Create a building level (elevation, floor-to-floor height, slab thickness) or edit one by ID. Every hosted slab, pedestal, stair and wall rebuilds and is evaluated natively before commit.",
        set_level;
    "create_slab" => SlabArgs, "Create slab or pedestal", false,
        "Create a slab (top at the level, down by the slab thickness) or a pedestal (standing on the level, thickness = height) from a closed outline. Evaluated natively before commit.",
        create_slab;
    "set_stair" => StairArgs, "Create or edit stair", false,
        "Create a straight, quarter-turn or half-turn stair from a level to the next (or by its floor-to-floor height), or edit one by ID. Returns geometric riser/going/2R+G notes that are not a code approval. Evaluated natively before commit.",
        set_stair;
    "create_envelope" => EnvelopeArgs, "Add service envelope", false,
        "Add a service/clearance envelope around a placed Part or Instance: its own bounds plus front/back/left/right/top clearances in its local frame, moving with it.",
        create_envelope;
    "check_clearances" => NoArgs, "Check service clearances", true,
        "Evaluate every service envelope against the final model solids and report each box and the entities it overlaps, with overlap volume.",
        check_clearances;
    "set_drawing_guide" => GuideArgs, "Create or edit drawing guide", false,
        "Create or edit a floor plan (cut height and view depth above each listed level), section (plan line, optional depth, limited to the levels' extent) or elevation (plan direction) guide, and optionally generate or update its linked drawing views in the same transaction. Rerunning updates the views it made before; views whose level left the guide are reported as stale. Generated views are evaluated natively before commit.",
        set_drawing_guide;
    "inspect_architectural_symbol" => architecture::InspectArgs, "Inspect architectural symbols", true,
        "List embedded wall/fence definitions or inspect a definition/output, public defaults, overrides and receiving context. Source inspection does not certify native geometry.",
        architecture::inspect;
    "author_architectural_symbol" => architecture::AuthorArgs, "Author architectural symbol interface", false,
        "Publish selected typed dimensions, material choices, options and numeric parameters from a live canonical layered-wall or fence construction. Sample paths are excluded. Retain symbol, blueprint and slot IDs across explicit authored revisions.",
        architecture::author;
    "import_architectural_symbol" => architecture::ImportArgs, "Import pinned architectural symbol", false,
        "Embed an architectural definition from a .tcd library in the configured exchange folder. No geometry is placed; the embedded pin works after the library is unavailable.",
        architecture::import;
    "draw_architectural_symbol" => architecture::PlaceArgs, "Draw architectural symbol", false,
        "Draw a real wall or fence from an embedded compatible pin, public inputs and receiving path/context. Native preflight precedes one attributable transaction; private construction values cannot be overridden.",
        architecture::place;
    "edit_architectural_symbol" => architecture::EditArgs, "Edit architectural symbol inputs", false,
        "Replace public overrides and optionally receiving context in one native-preflighted transaction. Omitted context preserves the path, wall openings and joins. Omitted override entries inherit the pin; preserve returned fence vertex IDs on path edits.",
        architecture::edit;
    "compare_architectural_symbol_update" => architecture::CompareArgs, "Compare architectural symbol revision", true,
        "Compare an explicit .tcd library revision and optional per-record input/context repairs. Reports affected instances, removed layers/members and conflicts without changing pins or certifying native geometry.",
        architecture::compare;
    "update_architectural_symbol" => architecture::UpdateArgs, "Apply architectural symbol revision", false,
        "Explicitly replace one embedded pin and all its instances, composing selected public-input/context repairs. Compatible identities remain stable. Full native preflight precedes one undoable commit; failure leaves the previous pin and instances intact.",
        architecture::update;
    "set_fence_type" => FenceTypeArgs, "Create or edit fence construction", false,
        "Create a canonical named fence construction, or explicitly edit every run of fence_type. Retain recipe IDs on edits. Unused definitions are source-only; affected runs receive exact native preflight before one attributable commit. No cross-project Library update is implied.",
        set_fence_type;
    "inspect_fence_type" => FenceTypeInspectArgs, "Inspect fence construction", true,
        "Read the canonical construction as unit-bearing values/formulas, retained recipe IDs and affected run IDs. The construction field can be supplied to set_fence_type; unchanged formulas retain their original parameter bindings.",
        inspect_fence_type;
    "preview_fence" => FenceArgs, "Preview fence run", true,
        "Prepare a complete fence create/edit and optional explicit shared construction edit, then check exact native geometry without committing. Returns candidate member IDs, post positions and actual bay widths at expected_revision. New candidate IDs are temporary. Flat runs only; slopes and gates are unsupported.",
        preview_fence;
    "set_fence" => FenceArgs, "Create or edit fence run", false,
        "Create a run of a saved construction or edit its canonical fence ID. Supply a complete path/context and retain vertex IDs for retained corners. Omitted post_positions retain explicit offsets; [] restores automatic positions. Optional edit_shared_construction repairs this run atomically with every shared user. Native preflight precedes one commit or owner review; invalid spans/collisions/cancellation change nothing.",
        set_fence;
    "inspect_fence" => FenceInspectArgs, "Inspect fence run", true,
        "Read the canonical run/context, stable vertex and member identities, source-resolved post positions and actual bay widths. Source inspection does not certify native geometry; use preview_fence or evaluate_model for that.",
        inspect_fence;
    "create_layered_wall_type" => CreateLayeredWallTypeArgs, "Create layered construction", false,
        "Create a named reusable Outside-to-Inside wall layer stack. Finite solids carry materials, air cavities carry real width, and coatings carry zero-volume material metadata. Returns stable layer IDs; no solid is made until create_wall uses the type. Keep layer IDs when editing.",
        create_layered_wall_type;
    "edit_layered_wall_type" => EditLayeredWallTypeArgs, "Edit shared construction", false,
        "Explicitly change a shared layered construction, retaining layer IDs. Every active affected wall is checked natively before commit; missing boundary references require repair. Use edit_wall with edit_shared_construction to repair that wall's reference atomically. Other walls must remain valid. No cross-project Library definition is silently updated.",
        edit_layered_wall_type;
    "inspect_wall_construction" => InspectWallConstructionArgs, "Inspect wall construction", true,
        "Inspect the uniform or layered canonical construction, derived total width, source-bound expressions, stable layer IDs and affected wall IDs. The layered construction field is suitable for a subsequent edit request.",
        inspect_wall_construction;
    "create_wall_type" => WallTypeArgs, "Create wall type", false,
        "Create a reusable wall type: thickness, height, justification (center, left or right of the drawn path) and an optional material for new walls. Commits one transaction.",
        create_wall_type;
    "edit_wall_type" => EditWallTypeArgs, "Edit wall type", false,
        "Change a wall type. Every wall of the type rebuilds unless it overrides the changed value; walls still using the previous type material follow a material change. All affected walls are evaluated natively before the edit commits.",
        edit_wall_type;
    "create_opening_type" => OpeningTypeArgs, "Create door or window type", false,
        "Create a reusable door (hinge start/end, swing left/right) or window type with width, height and sill. Commits one transaction.",
        create_opening_type;
    "create_wall" => CreateWallArgs, "Create wall", false,
        "Draw a wall of a wall type along a path (world X/Y mm, optionally closed into a room) with optional thickness/height/justification overrides and door/window openings placed by segment and offset. The mitered wall and its cut openings are evaluated natively before the transaction commits; unsupported joints or openings are refused with a reason.",
        create_wall;
    "edit_wall" => EditWallArgs, "Edit wall", false,
        "Replace a wall's path, overrides and openings, keeping its Part, the identities of openings at the same positions and its vertical cutouts. Evaluated natively before commit.",
        edit_wall;
    "inspect_wall" => InspectWallArgs, "Inspect wall", true,
        "Report a wall's resolved segments (index, start, direction, length), thickness, height, base, and every door, window and vertical cutout with its ID, segment, offset and size, plus any current placement diagnostic. Use the segment indices and IDs with set_wall_opening, set_vertical_cutout and remove_wall_opening.",
        inspect_wall;
    "set_wall_opening" => WallOpeningArgs, "Place or edit door/window", false,
        "Add a door or window to one wall segment at an offset (from the segment start to the start jamb), or edit an existing one by ID, with an existing opening type or a new type committed in the same transaction and optional width/height/sill overrides. Misfitting, top-reaching or overlapping openings are refused with a reason; the wall is evaluated natively before the one transaction commits.",
        set_wall_opening;
    "set_wood_box" => WoodBoxArgs, "Create or edit Wood Box", false,
        "Create or edit one canonical six-face wood box. Shared panel stock supplies thickness/material; any nonempty face subset is supported. Preserve recipe and face identities. Every panel and downstream operation passes native preflight before atomic commit; removing referenced faces requires repair.", set_wood_box;
    "preview_wood_box" => WoodBoxArgs, "Preview Wood Box", true,
        "Validate the complete proposed wood box without committing. Returns proposed recipe/face identities, exact native panel volumes, outside and usable inner dimensions, and actual world bounds. Reuse returned identities on Apply.", preview_wood_box;
    "inspect_wood_box" => WoodBoxInspectArgs, "Inspect Wood Box", true,
        "Read canonical wood box recipes, stable panel identities, stock thickness, outside/inner dimensions and actual world bounds. Omit recipe to list all.", inspect_wood_box;
    "remove_wood_box" => WoodBoxRemoveArgs, "Remove Wood Box", false,
        "Remove the complete wood box atomically after native preflight. Referenced panels and downstream operations require explicit repair.", remove_wood_box;
    "set_compound_cutout" => CompoundCutoutArgs, "Create or edit compound Cutout", false,
        "Union editable boxes and cut explicit wall/slab receiver histories. All boxes and all receivers pass exact native preflight before one atomic commit. Retain cutout, box and frozen producer identities on edits. Missing, disconnected, off-target, overlapping existing holes, partial slab cuts and receiving-solid splits are refused.",
        set_compound_cutout;
    "preview_compound_cutout" => CompoundCutoutArgs, "Preview compound Cutout", true,
        "Evaluate the complete proposed compound Cutout and every receiver without committing. Returns the proposed source, stable box/member identities and exact volumes.",
        preview_compound_cutout;
    "inspect_compound_cutout" => CompoundCutoutInspectArgs, "Inspect compound Cutout", true,
        "Read complete canonical compound Cutout definitions and generated receiver identities, or list all when cutout is omitted.",
        inspect_compound_cutout;
    "remove_compound_cutout" => CompoundCutoutRemoveArgs, "Remove compound Cutout", false,
        "Remove the source and all generated receiver cuts atomically after exact native preflight. Downstream dependencies require explicit repair.",
        remove_compound_cutout;
    "set_vertical_cutout" => VerticalCutoutArgs, "Place or edit vertical cutout", false,
        "Add or edit (by ID) a vertical cutout and choose what it cuts: a wall, floor slabs, or both. With `wall` (plus segment, offset, width) it is a full-height rectangular cutout in one wall segment, through the thickness when depth is omitted, otherwise a chase of that depth from the left or right face. With `slabs` (IDs) or `from_level`/`to_level` (every floor slab under the cutout on those levels: a shaft) it also cuts a hole through each slab, using `plan` (center, width, depth, rotation) or else the wall cutout's footprint; a slab-only cutout needs `plan`. Cutouts that do not fit, overlap an opening or another hole, cross or touch a slab edge, exceed the wall thickness or would split the wall are refused with no change. One transaction for the wall and every slab, evaluated natively before commit.",
        set_vertical_cutout;
    "remove_wall_opening" => RemoveWallOpeningArgs, "Remove door, window or cutout", false,
        "Remove one door, window or vertical cutout from a wall by ID as one transaction, evaluated natively before commit. A cutout that also cuts slabs is removed from them in the same transaction.",
        remove_wall_opening;
    "remove_vertical_cutout" => RemoveVerticalCutoutArgs, "Remove vertical cutout", false,
        "Remove a vertical cutout by ID from its wall and every slab it cuts as one transaction, evaluated natively before commit.",
        remove_vertical_cutout;
    "inspect_vertical_cutout" => InspectVerticalCutoutArgs, "Inspect vertical cutouts", true,
        "Report what a vertical cutout cuts (its wall and each slab with level, vertical span and hole corners), or list every cutout that cuts a slab when no ID is given.",
        inspect_vertical_cutout;
    "create_panel_stock" => PanelStockArgs, "Create panel stock", false,
        "Create a panel stock definition (thickness, sheet size, grain relative to the sheet length) on an existing Material or a new one. Commits one transaction.",
        create_panel_stock;
    "set_carcass" => CarcassArgs, "Create or edit carcass", false,
        "Create or edit a cabinet with separate named panel Parts. The cabinet configuration adds floor-to-bottom toe clearance, default shelf stock, stable individual shelf keys, finished depths, stock overrides and Fixed or System32 level supports, plus generated Dowels or Domino joinery and pin drilling. It replaces all explicit settings/rows when supplied; omission on edit preserves an existing configuration. Legacy count/offset shelves remain supported without it. Native fit, receiving-cut collision and solid checks precede one undoable commit.",
        set_carcass;
    "create_panel" => PanelArgs, "Create panel", false,
        "Create a rectangular panel Part from panel stock on a principal plane, with grain axis, per-edge banding (thickness and allowance) and cut oversize. Evaluated natively before commit.",
        create_panel;
    "create_panel_stack" => StackArgs, "Create panel stack", false,
        "Record panel Parts as face-to-face stack layers in order, keeping each layer's Part, material and stock; optionally group them as one assembly. Contact is checked by cut_list.",
        create_panel_stack;
    "set_panel_joint" => PanelJointArgs, "Create or edit panel joint", false,
        "Join overlapping right-angle panel corners with butt, miter or finger joints. For panels meeting face to edge, use kind loose_tenon with tenon settings for paired Domino-style mortises and tenon Parts, or kind dowels with dowels settings for paired circular blind holes (diameter, count, inset, offset, receiving depths and diametral fit). Each panel gets its own exact native cut and machining records. Loose-tenon joints require separate Parts and cannot be changed into another joint kind. Native validation precedes commit.",
        set_panel_joint;
    "delete_panel_joint" => DeleteJointArgs, "Delete panel joint", false,
        "Delete a panel joint and its cuts, restoring the panels. Later joint cuts on the same Parts are re-attached.",
        delete_panel_joint;
    "set_hardware_pattern" => HardwareArgs, "Create or edit hardware pattern", false,
        "Create or edit a reusable hardware hole pattern (hole positions, diameters, blind depths or through) with per-material diameter and depth allowances. Editing it rebuilds every placement, evaluated natively before commit.",
        set_hardware_pattern;
    "set_machining" => MachiningArgs, "Create or edit machining", false,
        "Cut a groove, rabbet, pocket, drilling, System 32 shelf-pin rows (kind shelf_pins: rows, pitch default 32 mm, start, end limit, optional count, diameter, blind depth) or placed hardware pattern into one or more panels, in panel face coordinates on the top, bottom, inside (each panel's face toward the carcass interior) or (drilling only) an edge face. Holes that overlap each other or another operation's holes are refused. Shelf-pin results list the carcass shelf offsets that rest on pins both sides share. Edits keep the targets. Cuts that leave no material, leave the face or would cut through a blind feature are refused. Modeled features only, not CNC toolpaths.",
        set_machining;
    "delete_machining" => DeleteMachiningArgs, "Delete machining", false,
        "Delete a machining operation and its cuts, restoring the panels.",
        delete_machining;
    "import_step" => StepImportArgs, "Import or update STEP", false,
        "Import every solid of a STEP file from the exchange folder in scope as its own Part (exact B-rep, file placement, millimeters), or update an earlier import: solids matching by shape or name keep their Parts; new ones are added and vanished ones are kept and reported missing.",
        import_step;
    "import_kicad" => KicadImportArgs, "Import or update KiCad board", false,
        "Import a .kicad_pcb from the exchange folder in scope: the Edge.Cuts outline with cutouts and drilled holes (mounting holes by default) as a board Part, and selected footprints as component Parts from their STEP models in that folder or courtyard envelopes when a model is missing. Updating keeps the board and matching components. Missing models and unsupported items are reported.",
        import_kicad;
    "schedule" => ScheduleArgs, "Schedule", true,
        "A schedule from the evaluated model as columns and rows with its revision and source hash: the BOM (one row per Part, quantity from physical occurrences, supply class, explicit aid/purchased inclusion, stack layers listed individually), the panel cut list, machining, the grouped member cut list, the weld schedule or the bend schedule. With file_name, also writes it as CSV into the export folder in scope.",
        schedule;
    "cut_list" => NoArgs, "Panel cut list", true,
        "Every panel Part measured from its evaluated solid: finished and core sizes along and across the grain, thickness, banded edges, stock and whether the core fits one stock sheet, plus each stack's layers and whether they touch face to face.",
        cut_list;
    "set_fdm_process" => ProcessArgs, "Set FDM process allowances", false,
        "Create or edit an FDM process allowance record: layer height, line width, minimum aid thickness in layers, minimum aid width in lines, support Z gap and fit clearance to Parts an aid does not serve. Used only to inspect aids; not a slicer profile.",
        set_fdm_process;
    "set_fdm_aid" => AidArgs, "Create or edit FDM aid", false,
        "Create or edit a removable FDM aid as its own Part serving a finished target Part: a breakaway tab (origin, angle, length, width, thickness, overlap), a thin bridge (start, end, width, thickness, optional second Part) or a flat-topped support (footprint corner, angle, length, width, base, gap) whose height is derived from the target's solid. The target is never changed. Evaluated natively before commit.",
        set_fdm_aid;
    "remove_fdm_aid" => RemoveAidArgs, "Remove FDM aid", false,
        "Remove an FDM aid feature and its Part; the target Part is untouched.",
        remove_fdm_aid;
    "fdm_aid_report" => NoArgs, "FDM aid report", true,
        "Every FDM aid measured from the evaluated solids: bounds, volume, contact with its target and bridge partner (separated, contact or overlapping, distance and common volume), neighbouring Parts inside the fit clearance, findings against its process allowances, the placement findings (contact_issues) the window refuses to apply, and a tab's exact contact near its own.",
        fdm_aid_report;
    "fdm_tab_contact" => TabContactArgs, "FDM tab contact", true,
        "Find a breakaway tab contact on a finished Part from its exact solid: give a point on or near its side (and optionally an outward build-plane direction); returns the outline point at the Part's lowest height and the outward angle about +Z for set_fdm_aid. A point inside the outline is refused. Changes nothing.",
        fdm_tab_contact;
    "set_render_scene" => RenderSceneArgs, "Create or edit render scene", false,
        "Create or replace a render scene stored in the document: camera eye and target in millimeters (+Z up) with a vertical field of view or orthographic height, background color, sun (W/m²) and point (W) lights, display style (shaded, shaded_edges, xray, wireframe), an optional Part scope and whether removable FDM aids are left out. Holds no geometry.",
        set_render_scene;
    "create_appearance" => CreateAppearanceArgs, "Create appearance", false,
        "Create a render appearance definition (base color, roughness, metallic). Appearances are separate from manufacturing Materials, stock and section hatches.",
        create_appearance;
    "assign_appearance" => AssignAppearanceArgs, "Assign appearance", false,
        "Give one Part an appearance, or clear it, keeping the Part's Material, stock and hatch.",
        assign_appearance;
    "blender_handoff" => HandoffArgs, "Blender handoff", true,
        "Write a render scene's Parts as NAME.glb (glTF 2.0 in meters, assembly groups, shared instance meshes, appearances, the scene camera and KHR_lights_punctual lights) and NAME.render.json (scope, units, revision, lights in Blender units, excluded aids, notes) into the export folder, built from the current native model. MultiCAD does not render; Blender does.",
        blender_handoff;
    "create_profile_stock" => ProfileStockArgs, "Create profile stock", false,
        "Create tube, bar or angle profile stock (rectangular SHS/RHS or flat bar, round CHS or round bar, angle) with section size, optional wall and bar length, on an existing Material or a new one. Commits one transaction.",
        create_profile_stock;
    "create_members" => MembersArgs, "Create weldment members", false,
        "Create straight tube/profile members from profile stock, one Part each: from every (or selected) solid line of a sketch, following those lines through later sketch edits, or from explicit point pairs. Section anchor, rotation about the axis and end extensions apply to all; optionally grouped. Every member is evaluated natively before commit.",
        create_members;
    "edit_member" => EditMemberArgs, "Edit weldment member", false,
        "Change a member's stock, point-pair path, section anchor, rotation or end extensions, keeping its Part. Evaluated natively before commit.",
        edit_member;
    "connect_members" => ConnectArgs, "Connect weldment members", false,
        "Create or edit a connection between two members: a paired miter on the bisector plane (sections must mirror), a square butt joint (first member terminates against a flat face of the continuing second member, which is cut square to cover a corner), or a cope fitting the first member to the actual outside of the second, with an optional gap. Cuts follow later path edits. Both members are evaluated natively before commit; unsupported geometry is refused with a reason.",
        connect_members;
    "weldment_cut_list" => NoArgs, "Weldment cut list", true,
        "Every weldment member measured from its evaluated solid: section, material and stock, overall length along the axis, rotation, anchor and world orientation, volume and whether it fits one stock bar.",
        weldment_cut_list;
    "set_weld" => WeldArgs, "Add or edit weld", false,
        "Attach a weld to a member connection, or edit one by ID: unique mark (its ID on drawings), symbol and side, size, length, all-around and field flags, process and access/orientation notes. Documentation only, not a welding procedure.",
        set_weld;
    "set_weld_sequence" => SequenceArgs, "Create or reorder weld sequence", false,
        "Create a weld sequence listing welds in the order they are to be made, with optional step notes, or replace an existing sequence's steps to reorder them. Welds and their connections keep their identities. Documentation only; no distortion or heat-input calculation.",
        set_weld_sequence;
    "weld_schedule" => NoArgs, "Weld schedule", true,
        "Welds in authored sequence order with marks, connections, members, joint positions and plane-cut joint perimeters resolved from the current members, plus identical member cuts grouped with quantities, lengths and end treatments.",
        weld_schedule;
    "place_weld_plan" => WeldPlanArgs, "Place weld plan", false,
        "Place weld symbols pointing at each weld's joint in a drawing view, tailed with the weld mark and process, and a weld sequence table on the page. Replaces earlier symbols with the same marks and earlier weld sequence tables by default. Placed annotations are page content and are refreshed by placing the plan again.",
        place_weld_plan;
    "create_sheet_stock" => SheetStockArgs, "Create sheet stock", false,
        "Create a sheet-metal stock definition (thickness, sheet size, default inside bend radius and K-factor) on an existing Material or a new one. Commits one transaction.",
        create_sheet_stock;
    "set_sheet_part" => SheetPartArgs, "Create or edit sheet part", false,
        "Create a sheet-metal Part from sheet stock with a rectangle base on a principal plane or a sketch-region base, or edit one by feature ID (omitting the base keeps it). Thickness comes from the stock; the inside radius and K-factor come from the stock unless given here. Evaluated natively before commit.",
        set_sheet_part;
    "convert_to_sheet" => ConvertSheetArgs, "Convert solid to sheet part", false,
        "Analyze a Part's current solid as a constant-thickness plate and, if supported, create a sheet Part whose base is the captured stationary face. Inclined faces, steps, thickness that differs from the stock and curved bend regions are refused with their measurements. The source Part is unchanged.",
        convert_to_sheet;
    "set_sheet_forming" => SheetFormingArgs, "Set sheet flanges and cutouts", false,
        "Replace a sheet part's flanges, hems and jogs (on straight base edges, with angle, direction, leg length, inside radius, corner insets and reliefs) and its through-cutouts (circle or rectangle in the base or a flange leg). Overlapping or touching flanges, cutouts outside their face and flanges running into the base are refused with the failing feature; the document is unchanged. Evaluated natively before commit.",
        set_sheet_forming;
    "sheet_metal" => NoArgs, "Sheet-metal records", true,
        "Every sheet Part measured from its evaluated solid: thickness and measured thickness, inside radius and K-factor with their source, volume, and the developed flat pattern of the same feature with its size and area.",
        sheet_metal;
    "sheet_edges" => SheetEdgesArgs, "Sheet edges and flange guides", true,
        "Base edges of one sheet feature (index, label, world ends, length, outward direction, straightness and the flange on it), a guide per flange (kind, direction, bend angles, inside radius, leg, flange span, world leg start and end, mid-thickness cross-section) from the stock thickness and resolved radius with their sources, and pre-check findings (invalid bends, insets that leave no flange, reliefs wider than their inset, neighboring flanges touching at a corner). Pass candidate flanges to review them without changing the document.",
        sheet_edges;
    "unfold_sheet" => UnfoldSheetArgs, "Unfold sheet part", true,
        "Develop one sheet Part without changing it: closed cut loops, bend lines (id, order, up/down, angle, inside radius, allowance, flat line), drill points for circular cutouts, size and area in explicit millimeters, and scoped diagnostics (hole-to-bend proximity, short legs, tight radii, missing or shallow reliefs as rules of thumb; stock-sheet fit against the model; tooling data that is not defined). Parts that are not sheet parts, or that later features change, are refused.",
        unfold_sheet;
    "export_sheet_dxf" => SheetDxfArgs, "Export developed sheet DXF", true,
        "Unfold one sheet Part from a fresh evaluation and write its developed blank as a 1:1 DXF in the export folder: CUT and CUT_INNER closed contours (reliefs and openings included), BEND_UP and BEND_DOWN center lines, DRILL points for circular cutouts and LABEL texts naming each bend and cutout id. Failed unfolding or a source that changed during the export writes nothing.",
        export_sheet_dxf;
    "set_sheet_fasteners" => SheetFastenersArgs, "Set sheet mounting hardware", false,
        "Replace a sheet part's mounting patterns: ISO 4762 screws (optional ISO 7089 washers and ISO 4032 nut, ISO 273 clearance holes), user-sized hex standoffs, or catalogue inserts with their designation, source, dimensions and sheet thickness range, on the base or a flange leg at points. Each pattern cuts receiving holes in the sheet; each purchased item is its own hardware Part. Evaluated natively before commit.",
        set_sheet_fasteners;
    "sheet_hardware" => NoArgs, "Sheet hardware list", true,
        "Every attached hardware Part: designation, dimension source, quantity measured from its solids, receiving hole and compatibility findings (stack longer than the screw, sheet outside an insert's range, hardware intersecting the sheet). No torque, strength or installation data.",
        sheet_hardware;
    "reference_candidates" => CandidateArgs, "Reference repair candidates", true,
        "The current exact topology elements (face, edge, vertex) of a source feature with their analytic geometry, for choosing a replacement in a reference repair. Element keys are valid only at the reported revision.",
        reference_candidates;
    "preview_reference_repairs" => PreviewArgs, "Preview reference repairs", true,
        "Stage several sketch-reference repairs (rebind, rebind_occurrence, remove, remove_sketch_items, source_edit rows, applied in order) on the current source without changing it. Reports each row's old binding, replacement, affected entities and error, plus blockers. can_apply is true only when every row staged, the full model rebuilds, every driving reference resolves and every affected sketch solves.",
        preview_reference_repairs;
    "apply_reference_repairs" => RepairArgs, "Apply reference repairs", false,
        "Stage and check the same rows as preview_reference_repairs and, only if the complete candidate passes, commit every row as one transaction. A blocked, stale or changed-resource request changes nothing.",
        apply_reference_repairs;
    "import_sketch_artwork" => ImportArtwork, "Import DXF/SVG artwork", false,
        "Import DXF or SVG text as exact sketch lines, arcs, circles and splines into an existing sketch or a new sketch on a datum plane. Unsupported content is reported. Commits one transaction with a provenance record.",
        import_sketch_artwork;
}

fn parse<T: DeserializeOwned>(arguments: Value) -> Result<T, ToolError> {
    let arguments = if arguments.is_null() {
        json!({})
    } else {
        arguments
    };
    serde_json::from_value(arguments).map_err(|e| ToolError::invalid(e.to_string()))
}
fn id(value: &str) -> Result<EntityId, ToolError> {
    EntityId::parse(value).map_err(|_| ToolError::invalid(format!("'{value}' is not an entity ID")))
}

// ---- Argument types -------------------------------------------------------

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NoArgs {}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListEntities {
    /// Optional entity kind filter, e.g. "sketch" or "part".
    #[serde(default)]
    pub kind: Option<String>,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EntityArgs {
    /// Persistent entity ID.
    pub id: String,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RevisionArgs {
    /// The document revision this request is based on. Stale requests fail.
    pub expected_revision: u64,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReviewArgs {
    /// Review ID returned with status "pending_review".
    pub review: u64,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct HistoryArgs {
    /// Most recent entries to return (default 20, at most 200).
    #[serde(default)]
    pub limit: Option<usize>,
}

/// A unit-bearing value. Give exactly one of `value` (a number) or
/// `expression` (a formula such as "=$width/2"). `unit` is always required.
#[derive(Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Quantity {
    #[serde(default)]
    pub value: Option<f64>,
    #[serde(default)]
    pub expression: Option<String>,
    /// Length fields accept mm, cm, m, in or ft; angular profile fields accept deg or rad.
    pub unit: String,
}
impl Quantity {
    fn length_unit(&self) -> Result<Unit, ToolError> {
        match Unit::parse(&self.unit) {
            Some(unit) if unit.dimension() == parameters::Dimension::LENGTH => Ok(unit),
            _ => Err(ToolError::invalid(format!(
                "'{}' is not a length unit; use mm, cm, m, in or ft",
                self.unit
            ))),
        }
    }
    fn input(&self) -> Result<DimensionInput, ToolError> {
        self.length_unit()?;
        match (self.value, &self.expression) {
            (Some(value), None) if value.is_finite() => Ok(DimensionInput::Literal {
                value,
                unit: self.unit.clone(),
            }),
            (None, Some(text)) => Ok(DimensionInput::Expression {
                text: text.clone(),
                unit: self.unit.clone(),
            }),
            _ => Err(ToolError::invalid(
                "Give exactly one finite 'value' or one 'expression'",
            )),
        }
    }
    /// The source dimension and its current value in millimeters.
    fn dimension(
        &self,
        document: &multicad_core::document::Document,
        evaluated: &parameters::EvaluatedParameters,
    ) -> Result<(Dimension, f64), ToolError> {
        let unit = self.length_unit()?;
        match self.input()? {
            DimensionInput::Literal { value, unit: u } => Ok((
                Dimension::Literal { value, unit: u },
                value * unit.scale() / Unit::Millimeter.scale(),
            )),
            DimensionInput::Expression { text, unit: u } => {
                let expression = BoundExpression::bind(document, &text, &u, None)?;
                let mm = expression
                    .evaluate(evaluated)?
                    .in_unit(Unit::Millimeter)
                    .ok_or_else(|| ToolError::invalid("Expression is not a length"))?;
                Ok((Dimension::Expression { expression }, mm))
            }
        }
    }
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SetParameter {
    pub expected_revision: u64,
    /// Identifier: letters, digits and underscore, not starting with a digit.
    pub name: String,
    /// A number ("40") or a formula starting with '=' ("=$width*2").
    pub expression: String,
    /// mm, cm, m, in, ft, deg, rad, count, ratio or %.
    pub unit: String,
    #[serde(default)]
    pub exposed: Option<bool>,
    #[serde(default)]
    pub description: Option<String>,
}
#[derive(Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Plane {
    Xy,
    Xz,
    Yz,
}
impl From<Plane> for Datum {
    fn from(plane: Plane) -> Self {
        match plane {
            Plane::Xy => Datum::Xy,
            Plane::Xz => Datum::Xz,
            Plane::Yz => Datum::Yz,
        }
    }
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateSketch {
    pub expected_revision: u64,
    pub plane: Plane,
    pub name: String,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AddRectangle {
    pub expected_revision: u64,
    pub sketch: String,
    /// First corner in sketch coordinates, millimeters.
    pub origin_mm: [f64; 2],
    pub width: Quantity,
    pub height: Quantity,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AddCircle {
    pub expected_revision: u64,
    pub sketch: String,
    /// Center in sketch coordinates, millimeters.
    pub center_mm: [f64; 2],
    pub diameter: Quantity,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SetDimension {
    pub expected_revision: u64,
    pub sketch: String,
    /// ID of a driving dimension constraint (from inspect_sketch).
    pub constraint: String,
    pub value: Quantity,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SketchArgs {
    pub sketch: String,
}
#[derive(Clone, Copy, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ExtrudeOperation {
    NewBody,
    Join,
    Cut,
    Intersect,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Extrude {
    pub expected_revision: u64,
    /// Sketch whose regions are extruded; omit when extruding faces.
    #[serde(default)]
    pub sketch: Option<String>,
    /// Region indices reported by inspect_sketch at expected_revision.
    #[serde(default)]
    pub regions: Vec<usize>,
    /// Planar model faces to extrude instead of sketch regions: each is a
    /// source feature and a face ordinal from reference_candidates at
    /// expected_revision. Each face becomes one profile with its exact outer
    /// boundary and holes; curved, missing or ambiguous faces are refused.
    #[serde(default)]
    pub faces: Vec<ExtrudeFace>,
    pub distance: Quantity,
    pub operation: ExtrudeOperation,
    /// Required for join, cut and intersect: the part to modify.
    #[serde(default)]
    pub target_part: Option<String>,
    /// Extrude against the sketch normal.
    #[serde(default)]
    pub reverse: bool,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExtrudeFace {
    /// Source feature that produces the face.
    pub source: String,
    /// Face ordinal from reference_candidates at expected_revision.
    pub face: u32,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExportStep {
    pub part: String,
    /// Bare file name ending in .step or .stp; written inside the export folder.
    pub file_name: String,
}
#[derive(Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ArtworkFormat {
    Dxf,
    Svg,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NewSketch {
    pub plane: Plane,
    pub name: String,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ImportArtwork {
    pub expected_revision: u64,
    pub format: ArtworkFormat,
    /// Source name recorded in provenance; any directories are dropped.
    pub file_name: String,
    /// The complete ASCII DXF or SVG text.
    pub content: String,
    /// Append to this existing sketch...
    #[serde(default)]
    pub sketch: Option<String>,
    /// ...or create a new sketch on a datum plane.
    #[serde(default)]
    pub new_sketch: Option<NewSketch>,
    /// mm, cm, m, in, ft, pt or px. Omit to use the file's declared units.
    #[serde(default)]
    pub source_unit: Option<String>,
    #[serde(default)]
    pub scale: Option<f64>,
    #[serde(default)]
    pub insertion_mm: Option<[f64; 2]>,
    #[serde(default)]
    pub rotation_deg: Option<f64>,
    /// Layers (DXF) or top-level groups (SVG) to import; omit for all.
    #[serde(default)]
    pub layers: Option<Vec<String>>,
}

// ---- Shared helpers -------------------------------------------------------

fn revision_guard(target: &dyn Target, expected: u64) -> Result<(), ToolError> {
    let current = target.session().revision();
    if current == expected {
        Ok(())
    } else {
        Err(ToolError::new(
            "stale_revision",
            format!("Request is based on revision {expected}, but the document is at {current}"),
        )
        .at_revision(current))
    }
}
fn editable(target: &dyn Target) -> Result<(), ToolError> {
    match target.edit_blocked() {
        Some(reason) => {
            Err(ToolError::new("busy", reason).at_revision(target.session().revision()))
        }
        None => Ok(()),
    }
}
/// Commit a checked candidate unless the call was cancelled meanwhile.
/// Commit a checked candidate, or queue it for review when the owner reviews
/// this client. A queued change returns the unchanged base revision.
fn commit(target: &mut dyn Target, call: &Request, prepared: Prepared) -> Result<u64, ToolError> {
    call.check()?;
    editable(target)?;
    // Native work may run outside the owner. Check the prepared session and
    // revision before either trusted commit or review queueing.
    target
        .session()
        .validate_preview(&prepared)
        .map_err(|error| ToolError::from(error).at_revision(target.session().revision()))?;
    if target.review_policy(&call.actor) == ReviewPolicy::Reviewed {
        let affected = affected(target.session().document(), prepared.document());
        let label = prepared.label().to_owned();
        return queue(
            target,
            call,
            label,
            PendingAction::Commit(Box::new(prepared)),
            affected,
        );
    }
    let change = target
        .session_mut()
        .commit(prepared)
        .map_err(|e| ToolError::from(e).at_revision(target.session().revision()))?;
    Ok(change.revision)
}
fn queue(
    target: &mut dyn Target,
    call: &Request,
    label: String,
    action: PendingAction,
    affected: Vec<EntityId>,
) -> Result<u64, ToolError> {
    let base = target.session().revision();
    let change = PendingChange {
        id: 0,
        actor: call.actor.clone(),
        tool: call.tool.into(),
        arguments: call.arguments.clone(),
        label,
        base_revision: base,
        action,
        affected,
    };
    let (session, ledger) = target.control();
    let id = ledger
        .queue(change, session)
        .map_err(|e| ToolError::new("busy", e).at_revision(base))?;
    *call.review.lock().unwrap_or_else(|e| e.into_inner()) = Some(id);
    Ok(base)
}
fn affected(
    before: &multicad_core::document::Document,
    after: &multicad_core::document::Document,
) -> Vec<EntityId> {
    let ids: BTreeSet<_> = before
        .entities()
        .chain(after.entities())
        .map(|e| e.id)
        .collect();
    ids.into_iter()
        .filter(|id| before.get(*id) != after.get(*id))
        .collect()
}
/// Mark queued results, then record the outcome in the owner's audit.
fn finish(host: &dyn Host, request: &Request, read_only: bool, result: ToolResult) -> ToolResult {
    let review = *request.review.lock().unwrap_or_else(|e| e.into_inner());
    let result = result.map(|mut value| {
        if let Some(object) = value.as_object_mut() {
            match review {
                Some(id) => {
                    object.insert("status".into(), json!("pending_review"));
                    object.insert("review".into(), json!(id));
                }
                None if !read_only => {
                    object.insert("status".into(), json!("applied"));
                }
                None => {}
            }
        }
        value
    });
    if review.is_some() {
        // The ledger already recorded the preview when it queued the change.
        return result;
    }
    let outcome = match (&result, request.tool) {
        (Ok(_), "preview_fence" | "preview_compound_cutout" | "preview_wood_box") => {
            AuditOutcome::Previewed
        }
        (Ok(_), _) if read_only => AuditOutcome::Inspected,
        (Ok(_), "undo") => AuditOutcome::Undone,
        (Ok(_), "redo") => AuditOutcome::Redone,
        (Ok(_), _) => AuditOutcome::Applied,
        (Err(e), _) if e.code == "cancelled" => AuditOutcome::Cancelled,
        (Err(_), _) => AuditOutcome::Failed,
    };
    let revision = match &result {
        Ok(value) if !read_only => value.get("revision").and_then(Value::as_u64),
        _ => None,
    };
    let message = result
        .as_ref()
        .err()
        .map(|e| e.to_string())
        .unwrap_or_default();
    let (actor, tool, arguments) = (
        request.actor.clone(),
        request.tool.to_string(),
        request.arguments.clone(),
    );
    let _ = with(host, move |target| {
        let session_revision = target.session().revision();
        let changed = revision
            .and_then(|r| target.session().activity().find(|a| a.change.revision == r))
            .map(|a| (a.label.clone(), a.change.changed.clone()));
        let base = arguments
            .get("expected_revision")
            .and_then(Value::as_u64)
            .unwrap_or(session_revision);
        let (label, affected) = changed.unwrap_or_default();
        let (session, ledger) = target.control();
        ledger.sync_below(session, revision);
        ledger.record(AuditRecord {
            actor,
            tool,
            arguments,
            label,
            base_revision: base,
            revision: revision.filter(|r| *r > base),
            outcome,
            review: None,
            affected,
            message,
        });
    });
    result
}
fn kind_name(entity: &Entity) -> Value {
    serde_json::to_value(entity.data.kind()).unwrap_or(Value::Null)
}
fn sketch_entity(target: &dyn Target, sketch: EntityId) -> Result<(), ToolError> {
    match target.session().document().get(sketch).map(|e| &e.data) {
        Some(EntityData::Sketch { .. }) => Ok(()),
        Some(_) => Err(ToolError::invalid(format!("{sketch} is not a sketch")).involving(sketch)),
        None => Err(ToolError::not_found(sketch, "Sketch")),
    }
}
fn edit_sketch(
    target: &mut dyn Target,
    call: &Request,
    expected: u64,
    sketch: EntityId,
    source: Source,
) -> Result<u64, ToolError> {
    let edit = multicad_solver::prepare_edit(
        target.session(),
        expected,
        call.actor.clone(),
        sketch,
        source,
    )?;
    let Some(preview) = edit.preview else {
        return Err(solve_failure(sketch, &edit.evaluation.diagnostics));
    };
    commit(target, call, preview)
}
fn solve_failure(sketch: EntityId, diagnostics: &multicad_solver::Diagnostics) -> ToolError {
    let mut error = ToolError::new(
        "solve_failed",
        format!(
            "The sketch does not solve ({:?}); the document is unchanged",
            diagnostics.state
        ),
    )
    .involving(sketch);
    error.involved.extend(
        diagnostics
            .conflicts
            .iter()
            .chain(&diagnostics.unmet)
            .copied(),
    );
    error
}
fn diagnostics_json(d: &multicad_solver::Diagnostics) -> Value {
    json!({
        "state": format!("{:?}", d.state),
        "degrees_of_freedom": d.degrees_of_freedom,
        "free_entities": d.free_entities,
        "conflicts": d.conflicts,
        "redundant": d.redundant,
        "unmet": d.unmet,
        "residual_mm": d.residual_mm,
    })
}

// ---- Tools ----------------------------------------------------------------

fn document_info(host: &dyn Host, call: &Request, _: NoArgs) -> ToolResult {
    let scope = host.scope();
    let geometry_backend = match call.backend {
        GeometryBackend::OpenCascade => "opencascade",
        GeometryBackend::Rust => "rust",
    };
    with(host, move |target| {
        let session = target.session();
        let document = session.document();
        let mut counts = std::collections::BTreeMap::<String, usize>::new();
        for entity in document.entities() {
            *counts
                .entry(kind_name(entity).as_str().unwrap_or("unknown").into())
                .or_default() += 1;
        }
        json!({
            "document": document.id(),
            "name": document.name(),
            "revision": session.revision(),
            "dirty": session.dirty(),
            "can_undo": session.can_undo(),
            "can_redo": session.can_redo(),
            "entity_counts": counts,
            "units": { "length": "mm", "angle": "deg", "area": "mm2", "volume": "mm3" },
            "geometry_backend": geometry_backend,
            "project_path": target.project_path(),
            "scope": scope,
        })
    })
}

fn list_entities(host: &dyn Host, _: &Request, args: ListEntities) -> ToolResult {
    with(host, move |target| {
        let session = target.session();
        let entities: Vec<_> = session
            .document()
            .entities()
            .filter(|e| {
                args.kind
                    .as_deref()
                    .is_none_or(|k| kind_name(e).as_str() == Some(k))
            })
            .map(|e| {
                json!({
                    "id": e.id,
                    "name": e.name,
                    "kind": kind_name(e),
                    "references": e.references().iter().map(|l| json!({"target": l.target, "role": l.role})).collect::<Vec<_>>(),
                })
            })
            .collect();
        json!({ "revision": session.revision(), "entities": entities })
    })
}

fn get_entity(host: &dyn Host, _: &Request, args: EntityArgs) -> ToolResult {
    let entity_id = id(&args.id)?;
    with(host, move |target| {
        let session = target.session();
        let entity = session
            .document()
            .get(entity_id)
            .ok_or_else(|| ToolError::not_found(entity_id, "Entity"))?;
        Ok(json!({
            "revision": session.revision(),
            "kind": kind_name(entity),
            "entity": entity,
        }))
    })?
}

fn list_parameters(host: &dyn Host, _: &Request, _: NoArgs) -> ToolResult {
    with(host, move |target| {
        let session = target.session();
        let evaluated = session.parameters();
        // Parameters owned by placed symbols are private, as in the window;
        // instances change them only through their exposed overrides.
        let owned = multicad_core::symbols::owned_entities(session.document()).unwrap_or_default();
        let rows: Vec<_> = session
            .document()
            .entities()
            .filter(|e| !owned.contains(&e.id))
            .filter_map(|e| match &e.data {
                EntityData::Parameter {
                    expression,
                    unit,
                    exposed,
                    ..
                } => {
                    let value = evaluated.values.get(&e.id);
                    Some(json!({
                        "id": e.id,
                        "name": e.name,
                        "description": e.description,
                        "expression": expression,
                        "unit": unit,
                        "value": value.and_then(|v| v.quantity.in_unit(v.unit)),
                        "role": value.map(|v| format!("{:?}", v.role).to_lowercase()),
                        "exposed": exposed,
                    }))
                }
                _ => None,
            })
            .collect();
        json!({ "revision": session.revision(), "parameters": rows })
    })
}

fn set_parameter(host: &dyn Host, call: &Request, args: SetParameter) -> ToolResult {
    let call = call.clone();
    with(host, move |target| {
        revision_guard(target, args.expected_revision)?;
        let document = target.session().document();
        let existing = document
            .entities()
            .find(|e| e.name == args.name && matches!(e.data, EntityData::Parameter { .. }))
            .cloned();
        let (mutation, parameter, created) = match existing {
            Some(mut entity) => {
                let EntityData::Parameter {
                    expression,
                    unit,
                    exposed,
                    ..
                } = &mut entity.data
                else {
                    unreachable!("filtered to parameters");
                };
                *expression = args.expression.clone();
                *unit = args.unit.clone();
                if let Some(value) = args.exposed {
                    *exposed = value;
                }
                if let Some(description) = &args.description {
                    entity.description = description.clone();
                }
                let id = entity.id;
                (Mutation::Replace(entity), id, false)
            }
            None => {
                let mut entity = Entity::new(
                    args.name.clone(),
                    EntityData::Parameter {
                        expression: args.expression.clone(),
                        unit: args.unit.clone(),
                        exposed: args.exposed.unwrap_or(false),
                        inputs: vec![],
                    },
                );
                entity.description = args.description.clone().unwrap_or_default();
                let id = entity.id;
                (Mutation::Insert(entity), id, true)
            }
        };
        let prepared = target.session().prepare(SessionRequest {
            expected_revision: args.expected_revision,
            actor: call.actor.clone(),
            label: format!("Set parameter {}", args.name),
            edits: vec![mutation],
        })?;
        let value = prepared.parameters().values.get(&parameter).cloned();
        let revision = commit(target, &call, prepared)?;
        Ok(json!({
            "revision": revision,
            "parameter": parameter,
            "created": created,
            "value": value.as_ref().and_then(|v| v.quantity.in_unit(v.unit)),
            "unit": args.unit,
        }))
    })?
}

fn create_sketch(host: &dyn Host, call: &Request, args: CreateSketch) -> ToolResult {
    let call = call.clone();
    with(host, move |target| {
        revision_guard(target, args.expected_revision)?;
        let (sketch, edits) = Source::create(args.plane.into(), &args.name)?;
        let prepared = target.session().prepare(SessionRequest {
            expected_revision: args.expected_revision,
            actor: call.actor.clone(),
            label: format!("Create sketch {}", args.name),
            edits,
        })?;
        let revision = commit(target, &call, prepared)?;
        Ok(json!({ "revision": revision, "sketch": sketch }))
    })?
}

fn point(source: &mut Source, position_mm: [f64; 2]) -> EntityId {
    let id = EntityId::new();
    source.sketch.points.push(Point { id, position_mm });
    id
}
fn curve(source: &mut Source, kind: CurveKind) -> EntityId {
    let id = EntityId::new();
    source.sketch.curves.push(Curve {
        id,
        construction: false,
        kind,
    });
    id
}
fn constrain(source: &mut Source, kind: ConstraintKind) -> EntityId {
    let id = EntityId::new();
    source.sketch.constraints.push(Constraint {
        id,
        reference: false,
        kind,
    });
    id
}
fn read_source(target: &dyn Target, sketch: EntityId, revision: u64) -> Result<Source, ToolError> {
    sketch_entity(target, sketch)?;
    Ok(Source::read_for_evaluation(target.session().document(), sketch, revision, None)?.0)
}

fn sketch_add_rectangle(host: &dyn Host, call: &Request, args: AddRectangle) -> ToolResult {
    let call = call.clone();
    let sketch = id(&args.sketch)?;
    with(host, move |target| {
        revision_guard(target, args.expected_revision)?;
        let session = target.session();
        let (width, w) = args
            .width
            .dimension(session.document(), session.parameters())?;
        let (height, h) = args
            .height
            .dimension(session.document(), session.parameters())?;
        if !(w > 0. && h > 0.) {
            return Err(ToolError::invalid("Width and height must be positive"));
        }
        let mut source = read_source(target, sketch, args.expected_revision)?;
        let [x, y] = args.origin_mm;
        let corners = [[x, y], [x + w, y], [x + w, y + h], [x, y + h]]
            .map(|position| point(&mut source, position));
        let lines = [0, 1, 2, 3].map(|i| {
            curve(
                &mut source,
                CurveKind::Line {
                    start: corners[i],
                    end: corners[(i + 1) % 4],
                },
            )
        });
        let mut constraints = vec![
            constrain(&mut source, ConstraintKind::Fix { point: corners[0] }),
            constrain(&mut source, ConstraintKind::Horizontal { line: lines[0] }),
            constrain(&mut source, ConstraintKind::Vertical { line: lines[1] }),
            constrain(&mut source, ConstraintKind::Horizontal { line: lines[2] }),
            constrain(&mut source, ConstraintKind::Vertical { line: lines[3] }),
        ];
        let width_constraint = constrain(
            &mut source,
            ConstraintKind::Distance {
                a: corners[0],
                b: corners[1],
                axis: Axis::X,
                dimension: width,
            },
        );
        let height_constraint = constrain(
            &mut source,
            ConstraintKind::Distance {
                a: corners[0],
                b: corners[3],
                axis: Axis::Y,
                dimension: height,
            },
        );
        constraints.extend([width_constraint, height_constraint]);
        let revision = edit_sketch(target, &call, args.expected_revision, sketch, source)?;
        Ok(json!({
            "revision": revision,
            "sketch": sketch,
            "points": corners,
            "lines": lines,
            "constraints": constraints,
            "width_constraint": width_constraint,
            "height_constraint": height_constraint,
        }))
    })?
}

fn sketch_add_circle(host: &dyn Host, call: &Request, args: AddCircle) -> ToolResult {
    let call = call.clone();
    let sketch = id(&args.sketch)?;
    with(host, move |target| {
        revision_guard(target, args.expected_revision)?;
        let session = target.session();
        let (diameter, d) = args
            .diameter
            .dimension(session.document(), session.parameters())?;
        if d <= 0. {
            return Err(ToolError::invalid("Diameter must be positive"));
        }
        let mut source = read_source(target, sketch, args.expected_revision)?;
        let center = point(&mut source, args.center_mm);
        let circle = curve(
            &mut source,
            CurveKind::Circle {
                center,
                radius_mm: d / 2.,
            },
        );
        let fix = constrain(&mut source, ConstraintKind::Fix { point: center });
        let diameter_constraint = constrain(
            &mut source,
            ConstraintKind::Diameter {
                curve: circle,
                dimension: diameter,
            },
        );
        let revision = edit_sketch(target, &call, args.expected_revision, sketch, source)?;
        Ok(json!({
            "revision": revision,
            "sketch": sketch,
            "center": center,
            "circle": circle,
            "constraints": [fix, diameter_constraint],
            "diameter_constraint": diameter_constraint,
        }))
    })?
}

fn set_sketch_dimension(host: &dyn Host, call: &Request, args: SetDimension) -> ToolResult {
    let call = call.clone();
    let sketch = id(&args.sketch)?;
    let constraint = id(&args.constraint)?;
    let input = args.value.input()?;
    with(host, move |target| {
        revision_guard(target, args.expected_revision)?;
        sketch_entity(target, sketch)?;
        let edit = multicad_solver::prepare_dimension(
            target.session(),
            args.expected_revision,
            call.actor.clone(),
            sketch,
            None,
            &DimensionRequest {
                constraint,
                input,
                rebind: false,
            },
        )?;
        let Some(preview) = edit.preview else {
            return Err(solve_failure(sketch, &edit.evaluation.diagnostics));
        };
        let revision = commit(target, &call, preview)?;
        Ok(json!({ "revision": revision, "sketch": sketch, "constraint": constraint }))
    })?
}

/// Snapshot what native work needs, without holding the document owner.
fn snapshot(
    host: &dyn Host,
) -> Result<
    (
        multicad_core::document::Document,
        u64,
        parameters::EvaluatedParameters,
    ),
    ToolError,
> {
    with(host, |target| {
        let session = target.session();
        (
            session.document().clone(),
            session.revision(),
            session.parameters().clone(),
        )
    })
}

fn inspect_sketch(host: &dyn Host, call: &Request, args: SketchArgs) -> ToolResult {
    let sketch = id(&args.sketch)?;
    let (document, revision, evaluated) = snapshot(host)?;
    match document.get(sketch).map(|e| &e.data) {
        Some(EntityData::Sketch { .. }) => {}
        Some(_) => return Err(ToolError::invalid(format!("{sketch} is not a sketch"))),
        None => return Err(ToolError::not_found(sketch, "Sketch")),
    }
    let report = sketch_support_report(&document, revision, sketch, call)?;
    let source = Source::read_for_evaluation(&document, sketch, revision, report.as_ref())?.0;
    let result = multicad_solver::evaluate_document_sketch_with_parameters(
        &document,
        revision,
        sketch,
        report.as_ref(),
        &evaluated,
    )?;
    let interrupt = || call.cancelled();
    let regions = match result.solved() {
        Some(_) => {
            let extraction = call.finish(
                result.regions_with_queries_and_interrupt(call.region_queries(), Some(&interrupt)),
            )?;
            json!({
                "regions": extraction.regions.iter().enumerate().map(|(index, r)| json!({
                    "index": index,
                    "area_mm2": r.area_mm2,
                    "holes": r.holes.len(),
                    "outer_curves": r.outer.intent.curves,
                })).collect::<Vec<_>>(),
                "region_diagnostics": extraction.diagnostics.iter().map(|d| format!("{d:?}")).collect::<Vec<_>>(),
            })
        }
        None => json!({ "regions": [], "region_diagnostics": ["Sketch does not solve"] }),
    };
    let solved = result.solved().unwrap_or(&source.sketch);
    let measurements: serde_json::Map<String, Value> = result
        .evaluation
        .measurements
        .iter()
        .map(|(id, m)| (id.to_string(), json!({ "value": m.value, "unit": m.unit })))
        .collect();
    Ok(json!({
        "revision": revision,
        "sketch": sketch,
        "solve": diagnostics_json(&result.evaluation.diagnostics),
        "solved": result.solved().is_some(),
        "points": solved.points,
        "curves": solved.curves,
        "constraints": source.sketch.constraints,
        "texts": source.sketch.texts.iter().map(|t| json!({
            "id": t.id,
            "style": t.style,
            "contour_curves": t.contours().map(|c| c.into_iter().map(|c| c.id).collect::<Vec<_>>()).unwrap_or_default(),
            "block_corners_mm": t.style.block_corners().ok(),
        })).collect::<Vec<_>>(),
        "measurements": measurements,
        "regions": regions["regions"],
        "region_diagnostics": regions["region_diagnostics"],
    }))
}

/// The Part's final unsuppressed feature, as the desktop body choices use it.
fn current_producer(
    document: &multicad_core::document::Document,
    part: EntityId,
) -> Result<EntityId, ToolError> {
    let Some(EntityData::Part { features, .. }) = document.get(part).map(|e| &e.data) else {
        return Err(ToolError::not_found(part, "Part"));
    };
    let producer = features
        .iter()
        .rev()
        .copied()
        .find(|id| {
            matches!(
                document.get(*id).map(|e| &e.data),
                Some(EntityData::Feature {
                    suppressed: false,
                    ..
                })
            )
        })
        .ok_or_else(|| ToolError::invalid("The part has no active feature").involving(part))?;
    Ok(BodyInput {
        body: part,
        producer,
    }
    .resolve_producer(document)?)
}

fn compile(
    document: &multicad_core::document::Document,
    revision: u64,
    call: &Request,
) -> Result<multicad_model::Model, ToolError> {
    let interrupt = || call.cancelled();
    call.finish(Compiler::default().compile_with_backend(
        document,
        revision,
        call.backend,
        Some(&interrupt),
    ))
}

fn compile_without_display(
    document: &multicad_core::document::Document,
    revision: u64,
    call: &Request,
) -> Result<multicad_model::Model, ToolError> {
    let interrupt = || call.cancelled();
    call.finish(
        Compiler {
            omit_display: true,
            ..Default::default()
        }
        .compile_with_backend(document, revision, call.backend, Some(&interrupt)),
    )
}

fn compile_sources(
    document: &multicad_core::document::Document,
    revision: u64,
    roots: &[EntityId],
    call: &Request,
) -> Result<multicad_model::Model, ToolError> {
    let interrupt = || call.cancelled();
    call.finish(Compiler::default().compile_sources_with_backend(
        document,
        revision,
        roots,
        call.backend,
        Some(&interrupt),
    ))
}

fn sketch_support_report(
    document: &multicad_core::document::Document,
    revision: u64,
    sketch: EntityId,
    call: &Request,
) -> Result<Option<multicad_core::references::ReferenceReport>, ToolError> {
    let has_external_inputs = matches!(
        document.get(sketch).map(|entity| &entity.data),
        Some(EntityData::Sketch { inputs, .. })
            if inputs.iter().any(|link| link.role == multicad_core::sketch_external::ROLE)
    );
    if multicad_core::sketch_regions::supporting_face(document, sketch).is_none()
        && !has_external_inputs
    {
        return Ok(None);
    }
    let interrupt = || call.cancelled();
    call.finish(Compiler::default().sketch_reference_report_with_backend(
        document,
        revision,
        sketch,
        call.backend,
        Some(&interrupt),
    ))
    .map(Some)
}

fn extrude(host: &dyn Host, call: &Request, args: Extrude) -> ToolResult {
    let target_part = args.target_part.as_deref().map(id).transpose()?;
    if !args.faces.is_empty() && (args.sketch.is_some() || !args.regions.is_empty()) {
        return Err(ToolError::invalid(
            "Give either sketch with regions or faces, not both",
        ));
    }
    let sketch = match &args.sketch {
        Some(sketch) => Some(id(sketch)?),
        None if args.faces.is_empty() => {
            return Err(ToolError::invalid(
                "Choose a sketch with regions, or planar faces",
            ));
        }
        None => None,
    };
    if sketch.is_some() && args.regions.is_empty() {
        return Err(ToolError::invalid("Choose at least one region"));
    }
    let (document, revision, evaluated) = snapshot(host)?;
    if revision != args.expected_revision {
        return Err(ToolError::new(
            "stale_revision",
            format!(
                "Request is based on revision {}, but the document is at {revision}",
                args.expected_revision
            ),
        )
        .at_revision(revision));
    }
    let mut intents = vec![];
    if let Some(sketch) = sketch {
        let report = sketch_support_report(&document, revision, sketch, call)?;
        let evaluated_sketch = multicad_solver::evaluate_document_sketch_with_parameters(
            &document,
            revision,
            sketch,
            report.as_ref(),
            &evaluated,
        )?;
        let interrupt = || call.cancelled();
        let extraction = call.finish(
            evaluated_sketch
                .regions_with_queries_and_interrupt(call.region_queries(), Some(&interrupt)),
        )?;
        for index in &args.regions {
            let region = extraction.regions.get(*index).ok_or_else(|| {
                ToolError::invalid(format!(
                    "Region {index} does not exist; the sketch has {} regions",
                    extraction.regions.len()
                ))
                .involving(sketch)
            })?;
            intents.push(region.intent());
        }
    }
    let faces = extrude_faces(&document, revision, call, &args.faces)?;
    let (distance, _) = args.distance.dimension(&document, &evaluated)?;
    let parameters = ExtrudeParameters {
        direction: Direction {
            reverse: args.reverse,
            ..Default::default()
        },
        extent: Extent::OneSided {
            end: Termination::Distance { distance },
        },
        ..Default::default()
    };
    let request = match sketch {
        Some(sketch) => ExtrudeRequest::new(sketch, intents, parameters),
        None => ExtrudeRequest::from_faces(faces, parameters),
    };
    let call_owned = call.clone();
    let operation = args.operation;
    let (prepared, created, results) = with(host, move |target| {
        revision_guard(target, revision)?;
        let session = target.session();
        if operation == ExtrudeOperation::NewBody {
            let prepared =
                model::prepare_extrude(session, revision, call_owned.actor.clone(), &request)?;
            let created: Vec<_> = prepared
                .outputs
                .iter()
                .map(|o| json!({ "part": o.body, "feature": o.feature }))
                .collect();
            let results = prepared
                .outputs
                .iter()
                .map(|o| (o.body, o.feature))
                .collect();
            Ok::<_, ToolError>((prepared.prepared, created, results))
        } else {
            let part = target_part
                .ok_or_else(|| ToolError::invalid("join, cut and intersect require target_part"))?;
            let producer = current_producer(session.document(), part)?;
            let op = match operation {
                ExtrudeOperation::Join => Operation::Join,
                ExtrudeOperation::Cut => Operation::Cut,
                _ => Operation::Intersect,
            };
            let boolean = ExtrudeBooleanRequest::new(
                request,
                op,
                vec![BodyInput {
                    body: part,
                    producer,
                }],
                false,
            );
            let prepared = model::prepare_extrude_boolean(
                session,
                revision,
                call_owned.actor.clone(),
                &boolean,
            )?;
            let feature = boolean.booleans[0].feature;
            Ok((
                prepared.prepared,
                vec![json!({ "part": part, "feature": feature })],
                vec![(part, feature)],
            ))
        }
    })??;
    // The source-only candidate must evaluate natively before it may commit.
    let model = compile(prepared.document(), revision, call)?;
    let mut bodies = vec![];
    for (part, feature) in &results {
        let shape = model
            .snapshot()
            .shapes
            .get(feature)
            .ok_or_else(|| ToolError::new("model_failed", "Extrusion produced no solid"))?;
        bodies.push(json!({
            "part": part,
            "feature": feature,
            "volume_mm3": shape.volume_mm3,
            "solid_count": shape.solid_count,
        }));
    }
    let call_owned = call.clone();
    let new_revision = with(host, move |target| commit(target, &call_owned, prepared))??;
    Ok(json!({ "revision": new_revision, "created": created, "bodies": bodies }))
}

/// Capture each requested face of its source feature's current native
/// topology as a persistent planar profile reference (shared validation with
/// the window: curved, missing, unnamed or multi-frame faces are refused).
fn extrude_faces(
    document: &multicad_core::document::Document,
    revision: u64,
    call: &Request,
    faces: &[ExtrudeFace],
) -> Result<Vec<model::FaceInput>, ToolError> {
    let mut captured = vec![];
    for face in faces {
        let source = id(&face.source)?;
        if !matches!(
            document.get(source).map(|e| &e.data),
            Some(EntityData::Feature {
                suppressed: false,
                ..
            })
        ) {
            return Err(
                ToolError::invalid(format!("{source} is not an active source feature"))
                    .involving(source),
            );
        }
        let model = compile_sources(document, revision, &[source], call)?;
        let topology = model
            .snapshot()
            .shapes
            .get(&source)
            .map(|shape| &shape.topology)
            .ok_or_else(|| {
                ToolError::new(
                    "model_failed",
                    "Source feature has no current native topology",
                )
                .involving(source)
            })?;
        let key = multicad_core::references::TopologyKey {
            kind: multicad_core::references::TopologyKind::Face,
            ordinal: face.face,
        };
        if !topology.elements.iter().any(|e| e.key == key) {
            return Err(ToolError::invalid(format!(
                "Face {} does not exist on the current source; list faces with reference_candidates",
                face.face
            ))
            .involving(source));
        }
        let solid = model.solid(source).ok_or_else(|| {
            ToolError::new("model_failed", "Source feature has no current solid").involving(source)
        })?;
        let planar = solid
            .faces()
            .map_err(|e| ToolError::new("model_failed", e.to_string()))?
            .into_iter()
            .any(|f| f.index == face.face && f.plane.is_some());
        if !planar {
            return Err(ToolError::invalid(format!(
                "Face {} is curved; extrude needs a planar face",
                face.face
            ))
            .involving(source));
        }
        captured.push(
            model::capture_profile_face(topology, key, None)
                .map_err(|message| ToolError::invalid(message).involving(source))?,
        );
    }
    Ok(captured)
}

fn evaluate_model(host: &dyn Host, call: &Request, _: NoArgs) -> ToolResult {
    let (document, revision, _) = snapshot(host)?;
    let model = compile_without_display(&document, revision, call)?;
    let snapshot = model.snapshot();
    let parts: Vec<_> = snapshot
        .bodies
        .iter()
        .filter(|(part, _)| !snapshot.consumed_bodies.contains(part))
        .map(|(part, feature)| {
            let shape = snapshot.shapes.get(feature);
            json!({
                "part": part,
                "name": document.get(*part).map(|e| e.name.as_str()),
                "feature": feature,
                "solid_count": shape.map(|s| s.solid_count),
                "volume_mm3": shape.map(|s| s.volume_mm3),
                "bounds_mm": shape.map(|s| json!({ "min": s.bounds.min, "max": s.bounds.max })),
            })
        })
        .collect();
    let instances: Vec<_> = snapshot
        .occurrences
        .iter()
        .map(|(instance, occurrence)| {
            let solid = model.solid(*instance);
            json!({
                "instance": instance,
                "name": document.get(*instance).map(|e| e.name.as_str()),
                "component": occurrence.component,
                "solid_count": solid.and_then(|s| s.solid_count().ok()),
                "volume_mm3": solid.and_then(|s| s.volume_mm3().ok()),
                "bounds_mm": json!({ "min": occurrence.bounds.min, "max": occurrence.bounds.max }),
            })
        })
        .collect();
    let references: Vec<_> = snapshot
        .references
        .entries
        .iter()
        .map(|d| {
            json!({
                "source": d.source,
                "role": d.role,
                "resolution": d.resolution,
                "affected": d.affected,
            })
        })
        .collect();
    Ok(json!({
        "revision": revision,
        "scope": "full",
        "parts": parts,
        "instances": instances,
        "consumed_parts": snapshot.consumed_bodies,
        "references": references,
        "gear_pairs":model.gear_pair_checks().values().collect::<Vec<_>>(),
        "screw_connections":model.snapshot().screw_connection_checks,
        "native_timings":model.timings(),
        "physical_leaves":model.assembly_evaluation().leaves,
        "source_hash":snapshot.stamp.source_hash,
    }))
}

fn undo(host: &dyn Host, call: &Request, args: RevisionArgs) -> ToolResult {
    let call = call.clone();
    with(host, move |target| {
        revision_guard(target, args.expected_revision)?;
        call.check()?;
        editable(target)?;
        if target.review_policy(&call.actor) == ReviewPolicy::Reviewed {
            if !target.session().can_undo() {
                return Err(ToolError::new("no_history", "Nothing to undo"));
            }
            let revision = queue(target, &call, "Undo".into(), PendingAction::Undo, vec![])?;
            return Ok(json!({ "revision": revision }));
        }
        let change = target
            .session_mut()
            .undo(args.expected_revision, call.actor.clone())?;
        Ok(json!({ "revision": change.revision, "changed": change.changed }))
    })?
}

fn redo(host: &dyn Host, call: &Request, args: RevisionArgs) -> ToolResult {
    let call = call.clone();
    with(host, move |target| {
        revision_guard(target, args.expected_revision)?;
        call.check()?;
        editable(target)?;
        if target.review_policy(&call.actor) == ReviewPolicy::Reviewed {
            if !target.session().can_redo() {
                return Err(ToolError::new("no_history", "Nothing to redo"));
            }
            let revision = queue(target, &call, "Redo".into(), PendingAction::Redo, vec![])?;
            return Ok(json!({ "revision": revision }));
        }
        let change = target
            .session_mut()
            .redo(args.expected_revision, call.actor.clone())?;
        Ok(json!({ "revision": change.revision, "changed": change.changed }))
    })?
}

fn history(host: &dyn Host, _: &Request, args: HistoryArgs) -> ToolResult {
    let limit = args.limit.unwrap_or(20).min(200);
    with(host, move |target| {
        let session = target.session();
        let all: Vec<_> = session.activity().collect();
        let entries: Vec<_> = all
            .iter()
            .rev()
            .take(limit)
            .map(|a| {
                json!({
                    "revision": a.change.revision,
                    "actor": a.actor.id,
                    "channel": a.actor.channel,
                    "label": a.label,
                    "kind": format!("{:?}", a.kind).to_lowercase(),
                    "changed": a.change.changed,
                })
            })
            .collect();
        json!({ "revision": session.revision(), "entries": entries })
    })
}

fn save_project(host: &dyn Host, call: &Request, args: RevisionArgs) -> ToolResult {
    let call = call.clone();
    with(host, move |target| {
        revision_guard(target, args.expected_revision)?;
        call.check()?;
        let path = target
            .save()
            .map_err(|e| ToolError::new("save_failed", e))?;
        Ok(json!({ "revision": args.expected_revision, "path": path }))
    })?
}

fn export_step(host: &dyn Host, call: &Request, args: ExportStep) -> ToolResult {
    let directory = host.scope().export_directory.ok_or_else(|| {
        ToolError::new(
            "out_of_scope",
            "No export folder is in scope; start the server with --export-dir",
        )
    })?;
    let name = &args.file_name;
    let valid = !name.is_empty()
        && name.len() <= 200
        && !name.starts_with('.')
        && !name.contains(['/', '\\', ':', '\0'])
        && [".step", ".stp"]
            .iter()
            .any(|e| name.to_ascii_lowercase().ends_with(e));
    if !valid {
        return Err(ToolError::new(
            "out_of_scope",
            "file_name must be a bare name ending in .step or .stp",
        ));
    }
    let part = id(&args.part)?;
    let (document, revision, _) = snapshot(host)?;
    if !matches!(
        document.get(part).map(|e| &e.data),
        Some(EntityData::Part { .. })
    ) {
        return Err(ToolError::not_found(part, "Part"));
    }
    let model = compile_without_display(&document, revision, call)?;
    let definition_parts = multicad_core::model::assembly_definition_parts(&document)?;
    let feature = *model
        .snapshot()
        .bodies
        .get(&part)
        .or_else(|| {
            (definition_parts.contains(&part) && !model.snapshot().consumed_bodies.contains(&part))
                .then(|| model.snapshot().body_sources.get(&part))
                .flatten()
        })
        .ok_or_else(|| {
            ToolError::new("model_failed", "The part has no final solid").involving(part)
        })?;
    if let Some(limitation) =
        multicad_core::model::manufacturing_limitation_in_history(&document, feature)?
    {
        return Err(ToolError::new(
            "unsupported",
            format!("{}; exporting STEP is unavailable", limitation.message()),
        )
        .involving(part));
    }
    let solid = model.solid(feature).ok_or_else(|| {
        ToolError::new("model_failed", "The part has no final solid").involving(part)
    })?;
    let path: PathBuf = directory.join(name);
    let partial = directory.join(format!(".{name}.partial.step"));
    let verify = || -> Result<f64, ToolError> {
        let policy = solid.geometry_policy();
        if call.backend == GeometryBackend::Rust {
            let shape = model.snapshot().shapes.get(&feature).ok_or_else(|| {
                ToolError::new("export_failed", "The Rust solid has no evaluated topology")
                    .involving(feature)
            })?;
            multicad_model::export::verify_rust_step_shape(
                &partial,
                solid,
                shape,
                policy.linear_tolerance_mm,
            )
            .map_err(|e| ToolError::new("export_failed", format!("STEP readback failed: {e}")))
        } else {
            multicad_kernel::Solid::read_step(&partial, policy)
                .and_then(|s| s.volume_mm3())
                .map_err(|e| ToolError::new("export_failed", format!("STEP readback failed: {e}")))
        }
    };
    let written = solid
        .write_step(&partial)
        .map_err(|e| ToolError::new("export_failed", e.to_string()))
        .and_then(|()| verify())
        .and_then(|volume| {
            call.check()?;
            std::fs::rename(&partial, &path)
                .map_err(|e| ToolError::new("export_failed", e.to_string()))?;
            Ok(volume)
        });
    let volume = match written {
        Ok(volume) => volume,
        Err(error) => {
            let _ = std::fs::remove_file(&partial);
            return Err(error);
        }
    };
    let bytes = std::fs::read(&path).map_err(|e| ToolError::new("export_failed", e.to_string()))?;
    Ok(json!({
        "revision": revision,
        "part": part,
        "feature": feature,
        "path": path,
        "bytes": bytes.len(),
        "sha256": sketch_import::provenance::sha256_hex(&bytes),
        "units": "mm",
        "model_volume_mm3": model.snapshot().shapes.get(&feature).map(|s| s.volume_mm3),
        "readback_volume_mm3": volume,
    }))
}

fn import_sketch_artwork(host: &dyn Host, call: &Request, args: ImportArtwork) -> ToolResult {
    let format = match args.format {
        ArtworkFormat::Dxf => Format::Dxf,
        ArtworkFormat::Svg => Format::Svg,
    };
    let units = match args.source_unit.as_deref() {
        None => UnitChoice::FromFile,
        Some(label) => UnitChoice::Explicit(
            SourceUnit::ALL
                .into_iter()
                .find(|u| u.label() == label)
                .ok_or_else(|| ToolError::invalid(format!("Unknown source unit '{label}'")))?,
        ),
    };
    let target = match (&args.sketch, args.new_sketch) {
        (Some(sketch), None) => multicad_solver::import::ImportTarget::Existing(id(sketch)?),
        (None, Some(new)) => multicad_solver::import::ImportTarget::New {
            datum: new.plane.into(),
            name: new.name,
        },
        _ => {
            return Err(ToolError::invalid(
                "Give exactly one of 'sketch' or 'new_sketch'",
            ));
        }
    };
    let defaults = ImportOptions::default();
    let options = ImportOptions {
        units,
        scale: args.scale.unwrap_or(defaults.scale),
        insertion_mm: args.insertion_mm.unwrap_or(defaults.insertion_mm),
        rotation_deg: args.rotation_deg.unwrap_or(defaults.rotation_deg),
        layers: args.layers.map(|l| l.into_iter().collect::<BTreeSet<_>>()),
        ..defaults
    };
    let bytes = args.content.into_bytes();
    let artwork = sketch_import::parse(format, &bytes, units)
        .map_err(|e| ToolError::new("import_failed", e.to_string()))?;
    let plan = sketch_import::plan::plan(&artwork, &options)
        .map_err(|e| ToolError::new("import_failed", e.to_string()))?;
    let unsupported: Vec<_> = plan.unsupported.iter().map(|d| format!("{d:?}")).collect();
    let call = call.clone();
    let file_name = args.file_name;
    let expected = args.expected_revision;
    with(host, move |target_owner| {
        revision_guard(target_owner, expected)?;
        let edit = multicad_solver::import::prepare_import(
            target_owner.session(),
            expected,
            call.actor.clone(),
            &target,
            None,
            &plan,
            &multicad_solver::import::ImportSource {
                file_name: &file_name,
                bytes: &bytes,
            },
        )?;
        let Some(preview) = edit.edit.preview else {
            return Err(solve_failure(
                edit.sketch,
                &edit.edit.evaluation.diagnostics,
            ));
        };
        let revision = commit(target_owner, &call, preview)?;
        Ok(json!({
            "revision": revision,
            "sketch": edit.sketch,
            "curves": edit.curves,
            "provenance": edit.provenance,
            "unsupported": unsupported,
        }))
    })?
}

fn review_status(host: &dyn Host, _: &Request, args: ReviewArgs) -> ToolResult {
    with(host, move |target| {
        let ledger = target.ledger();
        let state = ledger.state(args.review).ok_or_else(|| {
            ToolError::new(
                "not_found",
                format!("Review {} does not exist", args.review),
            )
        })?;
        let pending = ledger.pending().iter().find(|c| c.id == args.review);
        Ok(json!({
            "review": args.review,
            "revision": target.session().revision(),
            "state": state,
            "base_revision": pending.map(|c| c.base_revision),
            "stale": pending.map(|c| c.is_stale(target.session())),
        }))
    })?
}

fn activity(host: &dyn Host, _: &Request, args: HistoryArgs) -> ToolResult {
    let limit = args.limit.unwrap_or(20).min(200);
    with(host, move |target| {
        let (session, ledger) = target.control();
        ledger.sync(session);
        let revision = session.revision();
        let ledger = target.ledger();
        let entries: Vec<_> = ledger.entries().rev().take(limit).collect();
        json!({
            "revision": revision,
            "entries": entries,
            "dropped": ledger.dropped(),
            "pending": ledger.pending().iter().map(|c| json!({
                "review": c.id,
                "actor": c.actor.id,
                "tool": c.tool,
                "label": c.label,
                "base_revision": c.base_revision,
            })).collect::<Vec<_>>(),
        })
    })
}
