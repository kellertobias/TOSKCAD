//! Unplaced pinned definitions are checked on the native owner thread too.
use crate::{Compiler, ModelError, error};
use multicad_core::{
    document::{Document, EntityId},
    project_file::ContentHash,
    symbols,
};
use std::collections::BTreeSet;
const MAX_DEFINITIONS: usize = 64;
const MAX_TEMPLATE_ENTITIES: usize = 16_384;

pub(crate) fn validate_definitions(
    compiler: &Compiler,
    document: &Document,
    revision: u64,
    interrupt: Option<&dyn Fn() -> bool>,
) -> Result<(), ModelError> {
    let definitions = symbols::definitions(document).map_err(|e| error(None, e))?;
    if definitions.len() > MAX_DEFINITIONS {
        return Err(error(
            None,
            "Embedded symbol definition count exceeds the native evaluation budget",
        ));
    }
    let mut entities = 0usize;
    let mut checked = BTreeSet::<ContentHash>::new();
    for (id, definition) in definitions {
        if interrupt.is_some_and(|f| f()) {
            return Err(error(Some(id), "Symbol definition evaluation superseded"));
        }
        entities = entities
            .checked_add(definition.template.entities().count())
            .ok_or_else(|| error(Some(id), "Symbol definition entity budget overflow"))?;
        if entities > MAX_TEMPLATE_ENTITIES {
            return Err(error(
                Some(id),
                "Embedded symbol templates exceed the native evaluation entity budget",
            ));
        }
        let pin = definition.pin().map_err(|e| error(Some(id), e))?;
        if !checked.insert(pin) {
            continue;
        }
        if symbols::architecture::blueprint_for_definition(&definition)
            .map_err(|e| error(Some(id), e))?
            .is_some()
        {
            // Core validates a construction-only template and its real role.
            // Its exact solids depend on a receiving path/context. The ordinary
            // wall/fence providers preflight every placed member below; an
            // unplaced construction pin carries no native geometry receipt.
            continue;
        }
        // Core rejects nested symbol providers. This recursive call therefore
        // evaluates a bounded ordinary source graph, not another symbol graph.
        let native = compiler
            .compile_interruptible(&definition.template, revision, interrupt)
            .map_err(|e| {
                // An interrupted check says nothing about the definition.
                if crate::compiler::is_superseded(&e) {
                    return error(Some(id), "Symbol definition evaluation superseded");
                }
                error(
                    Some(id),
                    format!(
                        "Pinned symbol '{}' has invalid native geometry: {e}",
                        definition.name
                    ),
                )
            })?;
        if native.solid(definition.root).is_none() {
            return Err(error(
                Some(id),
                "A pinned symbol must have an exact nonempty Part or Assembly root",
            ));
        }
        require_references(id, &definition.template, &native.snapshot().references)?;
    }
    Ok(())
}
fn require_references(
    id: EntityId,
    document: &Document,
    report: &multicad_core::references::ReferenceReport,
) -> Result<(), ModelError> {
    if !report
        .for_model(document)
        .map_err(|e| error(Some(id), e))?
        .all_resolved()
    {
        return Err(error(
            Some(id),
            "Pinned symbol has unresolved driving references",
        ));
    }
    Ok(())
}
