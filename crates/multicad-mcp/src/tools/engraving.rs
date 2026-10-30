//! Create Engraving: a sketch attached to a current planar face of a source
//! feature, through the same face-attached `prepare_new` as the window.
//! Only the sketch is defined; no depth, material removal or toolpath.
use super::{Request, ToolResult, commit, editable, id, revision_guard};
use crate::{
    error::ToolError,
    host::{Host, with},
};
use multicad_core::{
    document::EntityData,
    references::{TopologyKey, TopologyKind},
    sketch::{self, CreationPlane},
};
use multicad_model::Compiler;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::json;

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EngravingArgs {
    pub expected_revision: u64,
    /// Source feature that produces the face.
    pub source: String,
    /// Face ordinal from reference_candidates at expected_revision.
    pub face: u32,
    /// Sketch name; defaults to "Engraving N".
    #[serde(default)]
    pub name: Option<String>,
}

pub(super) fn create_engraving_sketch(
    host: &dyn Host,
    call: &Request,
    args: EngravingArgs,
) -> ToolResult {
    let source = id(&args.source)?;
    let (document, revision) = with(host, |target| {
        (
            target.session().document().clone(),
            target.session().revision(),
        )
    })?;
    if revision != args.expected_revision {
        return Err(ToolError::new(
            "stale_revision",
            format!("The document is at revision {revision}; read the face again"),
        )
        .at_revision(revision));
    }
    if !matches!(
        document.get(source).map(|e| &e.data),
        Some(EntityData::Feature { .. })
    ) {
        return Err(
            ToolError::invalid(format!("{source} is not a source feature")).involving(source),
        );
    }
    let interrupt = || call.cancelled();
    let model = call.finish(Compiler::default().compile_sources_interruptible(
        &document,
        revision,
        &[source],
        Some(&interrupt),
    ))?;
    let topology = model
        .snapshot()
        .shapes
        .get(&source)
        .map(|shape| shape.topology.clone())
        .ok_or_else(|| {
            ToolError::new(
                "model_failed",
                "Source feature has no current native topology",
            )
            .involving(source)
        })?;
    let key = TopologyKey {
        kind: TopologyKind::Face,
        ordinal: args.face,
    };
    if !topology.elements.iter().any(|e| e.key == key) {
        return Err(ToolError::invalid(format!(
            "Face {} does not exist on the current source; list faces with reference_candidates",
            args.face
        ))
        .involving(source));
    }
    let name = args.name.unwrap_or_else(|| {
        let count = document
            .entities()
            .filter(|e| matches!(e.data, EntityData::Sketch { .. }))
            .count();
        format!("Engraving {}", count + 1)
    });
    let call = call.clone();
    with(host, move |target| {
        revision_guard(target, args.expected_revision)?;
        editable(target)?;
        let new = sketch::prepare_new(
            target.session(),
            args.expected_revision,
            call.actor.clone(),
            &name,
            CreationPlane::Face {
                snapshot: &topology,
                key,
                anchor: None,
            },
            Default::default(),
        )
        .map_err(|e| ToolError::invalid(e.to_string()).involving(source))?;
        let (sketch, plane) = (new.sketch, new.plane);
        let revision = commit(target, &call, new.prepared)?;
        Ok(json!({ "revision": revision, "sketch": sketch, "plane": plane, "name": name }))
    })?
}
