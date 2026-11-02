//! An authored interface and its immutable pin are prepared together. The
//! blueprint links the live canonical construction; sample geometry is outside
//! its captured dependency closure.
use super::*;
#[derive(Clone, Debug, PartialEq)]
pub struct ArchitectureAuthorRequest {
    pub blueprint: EntityId,
    pub interface: ArchitectureBlueprint,
    pub symbol: SymbolAuthorRequest,
}
pub fn prepare_architecture_author(
    session: &Session,
    revision: u64,
    actor: Actor,
    request: &ArchitectureAuthorRequest,
) -> Result<SymbolPrepared, CommandError> {
    super::super::prepare::stale(session, revision)?;
    if request.blueprint.is_nil() || request.blueprint == session.document().id() {
        return Err(invalid("Architectural interface requires a nonempty identity").into());
    }
    request.interface.validate(session.document())?;
    let mut blueprint = super::super::source::entity(
        request.blueprint,
        &request.symbol.name,
        EntityData::Extension {
            definition: request.interface.definition()?,
            inputs: request.interface.inputs(),
        },
    );
    let edit = if let Some(previous) = session.document().get(request.blueprint) {
        let old = read_blueprint(session.document(), request.blueprint)?;
        if old.role != request.interface.role || old.construction != request.interface.construction
        {
            return Err(invalid(
                "An existing interface retains its canonical construction and role",
            )
            .into());
        }
        blueprint.description = previous.description.clone();
        Mutation::Replace(blueprint)
    } else {
        Mutation::Insert(blueprint)
    };
    let stage = session.document().edited(&[edit])?;
    let mut symbol = request.symbol.clone();
    symbol.root = request.blueprint;
    symbol.kind = SymbolKind::Dynamic;
    symbol.plan = None;
    let value = super::super::source::author(&stage, &symbol)?;
    let staged = Session::new(stage)?;
    let result = super::super::prepare::prepare_import_at(
        &staged,
        staged.revision(),
        actor.clone(),
        &value,
        symbol.definition,
    )?;
    let candidate = result.prepared.document();
    let mut edits = Vec::new();
    for entity in candidate.entities() {
        match session.document().get(entity.id) {
            None => edits.push(Mutation::Insert(entity.clone())),
            Some(old) if old != entity => edits.push(Mutation::Replace(entity.clone())),
            _ => {}
        }
    }
    let prepared = session.prepare(Request {
        expected_revision: revision,
        actor,
        label: "Author reusable architectural construction".into(),
        edits,
    })?;
    Ok(SymbolPrepared {
        prepared,
        definition: result.definition,
        instances: vec![],
        plans: Default::default(),
    })
}
