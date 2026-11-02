use super::*;
fn public_inputs(definition: &symbols::SymbolDefinition) -> ToolResult {
    let blueprint = blueprint_for_definition(definition)?
        .ok_or_else(|| ToolError::invalid("Choose an architectural definition"))?;
    let mut source = ArchitectureConstruction::read(
        &definition.template,
        blueprint.role,
        blueprint.construction,
    )?;
    let params = multicad_core::parameters::evaluate(&definition.template)?;
    let mut dimensions = Vec::new();
    for slot in &blueprint.dimensions {
        let value = slot
            .target
            .dimension_mut(&mut source)?
            .value_in(multicad_core::parameters::Unit::Millimeter, Some(&params))
            .map_err(|e| ToolError::invalid(e.to_string()))?;
        dimensions.push(json!({"id":slot.id,"name":slot.name,"target":slot.target,"default":{"value":value,"unit":"mm"}}));
    }
    let mut materials = Vec::new();
    for slot in &blueprint.materials {
        let default = *slot.target.material_mut(&mut source)?;
        let choices = slot
            .choices
            .iter()
            .map(|id| json!({"material":id,"name":definition.template.get(*id).map(|e|&e.name)}))
            .collect::<Vec<_>>();
        materials.push(json!({"id":slot.id,"name":slot.name,"target":slot.target,"default":{"kind":"preset","material":default},"choices":choices,"allow_project_materials":slot.allow_project_materials}));
    }
    let mut options = Vec::new();
    for slot in &blueprint.options {
        options.push(json!({"id":slot.id,"name":slot.name,"target":slot.target,"default":slot.target.value(&source)?,"choices":slot.choices}));
    }
    let mut parameters = Vec::new();
    for id in &definition.exposed {
        let entity = definition
            .template
            .get(*id)
            .ok_or_else(|| ToolError::invalid("Exposed parameter is missing"))?;
        let multicad_core::document::EntityData::Parameter { unit, .. } = &entity.data else {
            return Err(ToolError::invalid("Exposed input is not a parameter"));
        };
        let parsed = multicad_core::parameters::Unit::parse(unit)
            .ok_or_else(|| ToolError::invalid("Exposed parameter unit is unsupported"))?;
        parameters.push(json!({"id":id,"name":entity.name,"default":{"value":params.value(*id,parsed)?,"unit":unit}}));
    }
    Ok(
        json!({"parameters":parameters,"dimensions":dimensions,"materials":materials,"options":options}),
    )
}
fn current_context(doc: &Document, output: &ArchitectureOutput) -> ToolResult {
    let q = |value| super::fences::quantities(value);
    match output {
        ArchitectureOutput::Wall { feature, .. } => {
            let wall = read_wall(doc, *feature)?;
            Ok(q(
                json!({"role":"wall","path":wall.path,"closed":wall.closed,"level":wall.level,"base_z":wall.base_z,"flipped":wall.layer_placement.is_some_and(|v|v.flipped)}),
            ))
        }
        ArchitectureOutput::Fence { run, .. } => {
            let fence = read_fence_run(doc, *run)?;
            let mut positions = Vec::new();
            for position in fence.post_positions {
                let FencePostKey::Interior {
                    start,
                    end,
                    ordinal,
                } = position.post
                else {
                    return Err(ToolError::invalid(
                        "Only interior posts accept position overrides",
                    ));
                };
                positions.push(
                    json!({"start":start,"end":end,"ordinal":ordinal,"offset":position.offset}),
                );
            }
            Ok(q(
                json!({"role":"fence","vertices":fence.path.vertices,"closed":fence.path.closed,"base_z":fence.path.base_z,"level":fence.level,"post_positions":positions}),
            ))
        }
    }
}
pub(in crate::tools) fn inspect(host: &dyn Host, _: &Request, args: InspectArgs) -> ToolResult {
    let (doc, revision, _) = snapshot(host)?;
    if let Some(entity) = args.entity {
        let entity = id(&entity)?;
        if let Some((record, instance)) = architecture_instance_for_output(&doc, entity)? {
            let definition = symbols::read_definition(&doc, instance.definition)?;
            return Ok(
                json!({"revision":revision,"record":record,"instance":super::fences::quantities(serde_json::to_value(&instance).expect("architectural source serializes")),"context":current_context(&doc,&instance.output)?,"public_inputs":public_inputs(&definition)?,"interface":blueprint_for_definition(&definition)?,"native_geometry_checked":false}),
            );
        }
        let definition = symbols::read_definition(&doc, entity)?;
        let interface = blueprint_for_definition(&definition)?
            .ok_or_else(|| ToolError::invalid("Choose an architectural definition"))?;
        return Ok(
            json!({"revision":revision,"definition":entity,"symbol":definition.symbol,"name":definition.name,"source_revision":definition.revision,"pin":definition.pin()?,"interface":interface,"public_inputs":public_inputs(&definition)?,"instances":architecture_instances(&doc,entity)?,"native_geometry_checked":false}),
        );
    }
    let definitions=symbols::definitions(&doc)?.into_iter().filter_map(|(id,d)|match blueprint_for_definition(&d){Ok(Some(bp))=>Some(Ok(json!({"definition":id,"symbol":d.symbol,"name":d.name,"revision":d.revision,"interface":bp}))),Ok(None)=>None,Err(e)=>Some(Err(ToolError::from(e)))}).collect::<Result<Vec<_>,_>>()?;
    Ok(json!({"revision":revision,"definitions":definitions,"native_geometry_checked":false}))
}
