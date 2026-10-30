//! Reviewed workbench artwork (`assets/icons`, CAD-135) bound to registered
//! ribbon tools, plus truthful explanations for tools whose name alone is
//! ambiguous. Tools without a reviewed asset keep their retained vector
//! symbol from `icons.rs`. Artwork for a proposed tool never makes that tool
//! available: only tools already in the registry are mapped here.
use crate::{registry::Mode, svg_icon::SvgIcon};
use std::sync::OnceLock;

macro_rules! artwork {
    ($($id:literal),* $(,)?) => {
        &[$((
            $id,
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../assets/icons/",
                $id,
                ".svg"
            )),
        )),*]
    };
}

/// Every catalog asset the native ribbon embeds. Adding catalog entries does
/// not require changes here; only newly bound tools do.
pub static SOURCES: &[(&str, &str)] = artwork![
    "apply-rebuild",
    "engraving",
    "discard",
    "preview-model",
    "preview",
    "finish",
    "import",
    "erase",
    "wall-2d",
    "levels-2d",
    "project-reference",
    "trim",
    "rectangle-corners",
    "rectangle-center",
    "rectangle-center-two",
    "rectangle-three",
    "circle-radius",
    "circle-two-radius",
    "circle-three",
    "dimension",
    "construction",
    "fillet",
    "chamfer",
    "extrude",
    "revolve",
    "wall-3d",
    "levels-3d",
    "boolean-union",
    "boolean-subtract",
    "boolean-intersect",
    "fillet-model",
    "draft",
    "align-frames",
    "drill",
    "finger-joint",
    "weld-joint",
    "tube",
    "miter",
    "place-component",
    "create-assembly",
    "relationship",
    "joint-motion",
    "named-pose",
    "interference",
    "motion-range",
    "explode",
    "new-drawing",
    "add-page",
    "duplicate-page",
    "remove-page",
    "page-setup",
    "model-dimension",
    "wood-panel",
    "fdm-support",
];

/// Catalog asset for a registered tool in a workspace. Shared action meanings
/// stay consistent: checked hammer Apply/Rebuild, stop sign Discard, box and
/// magnifier for the Parameters model Preview, eye for other previews, green
/// check Finish sketch / Apply page and eraser Delete.
pub fn artwork_id(mode: Mode, tool: &str) -> Option<&'static str> {
    Some(match (mode, tool) {
        (_, "apply") => "apply-rebuild",
        (_, "discard" | "abort-sketch" | "page-discard" | "export-set-discard") => "discard",
        (Mode::Parameters, "preview") => "preview-model",
        (_, "preview" | "export-preview" | "print-preview") => "preview",
        (_, "finish-sketch" | "page-apply") => "finish",
        (_, "import-artwork") => "import",
        (_, "delete") => "erase",
        (Mode::Sketch, "wall") => "wall-2d",
        (Mode::Sketch, "level") => "levels-2d",
        (_, "wall") => "wall-3d",
        (_, "level") => "levels-3d",
        (_, "project-reference") => "project-reference",
        (_, "trim") => "trim",
        (_, "rectangle") => "rectangle-corners",
        (_, "rectangle-center") => "rectangle-center",
        (_, "rectangle-center-two") => "rectangle-center-two",
        (_, "rectangle-three") => "rectangle-three",
        (_, "circle") => "circle-radius",
        (_, "circle-two-radius") => "circle-two-radius",
        (_, "circle-three") => "circle-three",
        (_, "sketch-fillet") => "fillet",
        (_, "sketch-chamfer") => "chamfer",
        (_, "dimension") => "dimension",
        (_, "construction") => "construction",
        (_, "extrude") => "extrude",
        (_, "revolve") => "revolve",
        (_, "boolean-join") => "boolean-union",
        (_, "boolean-subtract") => "boolean-subtract",
        (_, "boolean-intersect") => "boolean-intersect",
        (_, "edge-treatment") => "fillet-model",
        (_, "draft") => "draft",
        (_, "align") => "align-frames",
        (_, "drill") => "drill",
        (_, "joint") => "finger-joint",
        (_, "weld-joint") => "weld-joint",
        (_, "tube") => "tube",
        (_, "cope") => "miter",
        (_, "place-component") => "place-component",
        (_, "create-assembly") => "create-assembly",
        (_, "relationship") => "relationship",
        (_, "joint-motion") => "joint-motion",
        (_, "named-pose") => "named-pose",
        (_, "interference") => "interference",
        (_, "motion-range") => "motion-range",
        (_, "explode") => "explode",
        (_, "drawing-new") => "new-drawing",
        (_, "sheet-page") => "add-page",
        (_, "page-duplicate") => "duplicate-page",
        (_, "page-remove") => "remove-page",
        (_, "page-setup") => "page-setup",
        (_, "drawing-dimension") => "model-dimension",
        (_, "panel") => "wood-panel",
        (_, "engraving") => "engraving",
        (_, "fdm-aid") => "fdm-support",
        _ => return None,
    })
}

/// Truthful explanation appended to a tool's hover text.
pub fn description(tool: &str) -> Option<&'static str> {
    Some(match tool {
        "construction" => {
            "Construction: helper geometry that can carry constraints and dimensions for the sketch but is excluded from solid profiles. While a drawing tool is active (or no curve is selected) this toggles construction creation mode for new curves; otherwise it toggles the selected curve."
        }
        "draft" => {
            "Draft: tapers the selected faces by an angle about a neutral plane, measured against the pull direction (for example a mould release taper)."
        }
        "align" => {
            "Align: moves one object so a source frame, axis or position anchor matches a target anchor. It does not distribute rows of objects."
        }
        "apply" => "Apply and rebuild the canonical document with the pending edits.",
        "discard" | "page-discard" | "export-set-discard" => {
            "Discard the pending edits; the canonical document stays unchanged."
        }
        "abort-sketch" => "Abort the sketch session without applying its pending edits.",
        "engraving" => {
            "Create engraving: opens a sketch on the planar face selected in the 3D view. It defines only the sketch; no depth or material removal."
        }
        "boolean-join" => "Boolean Join: unite the tool bodies with each target body.",
        "boolean-subtract" => "Boolean Subtract: remove the tool bodies from each target body.",
        "boolean-intersect" => {
            "Boolean Intersect: keep only the region shared by each target and the tool bodies."
        }
        _ => return None,
    })
}

/// Parsed artwork for a catalog id, cached for the process lifetime.
pub fn artwork(id: &str) -> Option<&'static SvgIcon> {
    static PARSED: OnceLock<Vec<(&'static str, Option<SvgIcon>)>> = OnceLock::new();
    PARSED
        .get_or_init(|| {
            SOURCES
                .iter()
                .map(|(id, source)| (*id, crate::svg_icon::parse(source).ok()))
                .collect()
        })
        .iter()
        .find(|(candidate, _)| *candidate == id)
        .and_then(|(_, icon)| icon.as_ref())
}

/// Artwork for a registered tool in a workspace, if a reviewed asset exists.
pub fn tool_artwork(mode: Mode, tool: &str) -> Option<(&'static str, &'static SvgIcon)> {
    let id = artwork_id(mode, tool)?;
    artwork(id).map(|icon| (id, icon))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::Registry;
    use std::collections::BTreeSet;

    fn catalog() -> BTreeSet<String> {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/icons/catalog.json"
        );
        let value: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        value["icons"]
            .as_array()
            .unwrap()
            .iter()
            .map(|icon| icon["id"].as_str().unwrap().to_string())
            .collect()
    }

    #[test]
    fn every_embedded_asset_is_a_catalog_entry_that_parses_and_renders() {
        let catalog = catalog();
        for (id, source) in SOURCES {
            assert!(catalog.contains(*id), "{id} is not in catalog.json");
            let icon = crate::svg_icon::parse(source)
                .unwrap_or_else(|error| panic!("{id} failed to parse: {error}"));
            assert_eq!(icon.view_box, [0., 0., 64., 64.], "{id}");
            assert!(!icon.title.is_empty(), "{id} needs an accessible title");
            for size in [16, 24, 48] {
                let mask = icon.rasterize(size);
                assert!(mask.coverage() > 0, "{id} renders empty at {size}px");
                assert!(
                    mask.coverage() < size * size * 3 / 4,
                    "{id} floods the {size}px canvas"
                );
            }
        }
    }

    #[test]
    fn every_registered_ribbon_tool_binding_resolves_to_parsed_artwork() {
        let registry = Registry::default();
        let mut bound = 0;
        for workspace in &registry.workspaces {
            for tab in &workspace.tabs {
                for tool in &tab.tools {
                    assert!(registry.tool(tool).is_some());
                    if let Some(id) = artwork_id(workspace.mode, tool) {
                        assert!(
                            tool_artwork(workspace.mode, tool).is_some(),
                            "{tool} maps to {id}, which is not embedded or does not parse"
                        );
                        bound += 1;
                    }
                }
            }
        }
        assert!(
            bound >= 30,
            "only {bound} ribbon entries use reviewed artwork"
        );
    }

    #[test]
    fn shared_action_motifs_stay_consistent_across_workbenches() {
        for mode in Mode::ALL {
            assert_eq!(artwork_id(mode, "apply"), Some("apply-rebuild"));
            assert_eq!(artwork_id(mode, "discard"), Some("discard"));
            assert_eq!(artwork_id(mode, "abort-sketch"), Some("discard"));
            assert_eq!(artwork_id(mode, "delete"), Some("erase"));
            assert_eq!(artwork_id(mode, "finish-sketch"), Some("finish"));
        }
        assert_eq!(
            artwork_id(Mode::Parameters, "preview"),
            Some("preview-model")
        );
        assert_eq!(artwork_id(Mode::Sketch, "preview"), Some("preview"));
        assert_eq!(artwork_id(Mode::Export, "export-preview"), Some("preview"));
        assert_eq!(artwork_id(Mode::Drawings, "page-apply"), Some("finish"));
        assert_eq!(artwork_id(Mode::Drawings, "page-discard"), Some("discard"));
        assert_eq!(artwork_id(Mode::Sketch, "wall"), Some("wall-2d"));
        assert_eq!(artwork_id(Mode::Model, "wall"), Some("wall-3d"));
        assert_eq!(
            artwork("finish").and_then(|icon| icon.color),
            Some([0x24, 0xaa, 0x60]),
            "Finish keeps its reviewed success green"
        );
        // Proposed tools without behavior have no binding.
        for proposed in ["door", "window-tool", "vertical-cutout", "fold", "unfold"] {
            assert_eq!(artwork_id(Mode::Sketch, proposed), None);
            assert_eq!(artwork_id(Mode::Model, proposed), None);
        }
    }

    #[test]
    fn construction_draft_and_align_explanations_are_truthful() {
        let construction = description("construction").unwrap();
        assert!(construction.contains("excluded from solid profiles"));
        assert!(construction.contains("constraints"));
        assert!(construction.contains("drawing tool is active"));
        assert!(construction.contains("selected curve"));
        let draft = description("draft").unwrap();
        assert!(draft.contains("neutral plane") && draft.contains("pull direction"));
        let align = description("align").unwrap();
        assert!(align.contains("frame, axis or position"));
        assert!(align.contains("does not distribute rows"));
    }
}
