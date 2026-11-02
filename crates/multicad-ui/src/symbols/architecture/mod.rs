//! Shared graphical controls for the canonical architectural symbol contract.
pub(crate) mod author;
mod binding;
mod fields;
pub(crate) use binding::Binding;
pub use binding::Command;
pub(crate) use fields::Fields;

#[cfg(test)]
pub(crate) mod test_support;

/// A constant-size projection from an already validated catalogue snapshot.
/// Full interface/provider validation remains in core preparation.
pub(crate) fn role(
    definition: &multicad_core::symbols::SymbolDefinition,
) -> Option<multicad_core::symbols::architecture::ArchitectureRole> {
    let multicad_core::document::EntityData::Extension { definition, .. } =
        &definition.template.get(definition.root)?.data
    else {
        return None;
    };
    if definition.type_id != multicad_core::symbols::architecture::BLUEPRINT_TYPE
        || definition.schema_version != 1
    {
        return None;
    }
    match definition.data.get("role")?.as_str()? {
        "wall" => Some(multicad_core::symbols::architecture::ArchitectureRole::Wall),
        "fence" => Some(multicad_core::symbols::architecture::ArchitectureRole::Fence),
        _ => None,
    }
}
