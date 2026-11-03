//! Output-format routes for the tool finder (CAD-157).
//!
//! Users search by the file they want ("STL", "3MF", "STEP"), not by the
//! command that writes it. Each route names the implemented workflows that
//! produce that format and a short note shown as guidance in the finder.
//! Routes only point at registered catalog tools; no exporter is implied.

/// One searchable output format and the workflows that produce it.
pub struct FormatRoute {
    /// Display name, also matched case-insensitively.
    pub format: &'static str,
    /// Further spellings, for example `stp` for STEP.
    pub aliases: &'static [&'static str],
    /// Catalog tool id and guidance, in result order.
    pub tools: &'static [(&'static str, &'static str)],
}
const PRINT_PREVIEW: &str = "print-preview";
const PRINT_WRITE: &str = "print-write";
const EXPORT_SET: &str = "export-set";
pub const FORMAT_ROUTES: &[FormatRoute] = &[
    FormatRoute {
        format: "STL",
        aliases: &[],
        tools: &[
            (
                PRINT_PREVIEW,
                "Print packaging · tick \"Also write one STL per Part\" to add STL files beside the 3MF package",
            ),
            (
                PRINT_WRITE,
                "Print packaging · writes the 3MF package and, when ticked, one STL per part",
            ),
            (EXPORT_SET, "Export set · add an STL mesh recipe"),
        ],
    },
    FormatRoute {
        format: "3MF",
        aliases: &[],
        tools: &[
            (
                PRINT_PREVIEW,
                "Print packaging · preview the 3MF print package",
            ),
            (PRINT_WRITE, "Print packaging · write the 3MF print package"),
            (EXPORT_SET, "Export set · add a 3MF mesh recipe"),
        ],
    },
    FormatRoute {
        format: "STEP",
        aliases: &["stp"],
        tools: &[(EXPORT_SET, "Export set · add a STEP solid recipe")],
    },
    FormatRoute {
        format: "OBJ",
        aliases: &[],
        tools: &[(EXPORT_SET, "Export set · add an OBJ mesh recipe")],
    },
    FormatRoute {
        format: "GLB",
        aliases: &["gltf"],
        tools: &[(EXPORT_SET, "Export set · add a GLB mesh recipe")],
    },
    FormatRoute {
        format: "PDF",
        aliases: &[],
        tools: &[
            (
                EXPORT_SET,
                "Export set · add a PDF recipe; it needs at least one drawing page",
            ),
            (
                "drawing-pdf",
                "Drawings · export the current sheet, drawing or project sheets to one PDF",
            ),
        ],
    },
    FormatRoute {
        format: "DXF",
        aliases: &[],
        tools: &[(
            EXPORT_SET,
            "Export set · add a planar-profile or drawing-page DXF recipe",
        )],
    },
    FormatRoute {
        // "export" alone should also surface print packaging, which lives on
        // Export → Print rather than in the Common export commands.
        format: "Export",
        aliases: &["print"],
        tools: &[(
            PRINT_PREVIEW,
            "Print packaging · 3MF with optional STL per part",
        )],
    },
];
impl FormatRoute {
    /// A query names this format when one of its words equals the format
    /// or an alias, or begins one of them with at least two characters.
    fn matches(&self, query: &str) -> bool {
        query.split_whitespace().any(|word| {
            std::iter::once(self.format)
                .chain(self.aliases.iter().copied())
                .map(str::to_lowercase)
                .any(|name| name == word || (word.len() >= 2 && name.starts_with(word)))
        })
    }
}
/// Routes named by `query` (already trimmed and lower-cased).
pub fn routes(query: &str) -> impl Iterator<Item = &'static FormatRoute> + '_ {
    FORMAT_ROUTES
        .iter()
        .filter(move |route| route.matches(query))
}
/// Guidance for `id` among the routes named by `query`, prefixed by the
/// format, for example `STL: Print packaging · …`.
pub fn note(query: &str, id: &str) -> Option<String> {
    let query = query.trim().to_lowercase();
    routes(&query).find_map(|route| {
        route
            .tools
            .iter()
            .find(|(tool, _)| *tool == id)
            .map(|(_, note)| format!("{}: {note}", route.format))
    })
}
