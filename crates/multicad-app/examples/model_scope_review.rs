//! Generate a canonical stacked-panel fixture for native cut-scope and end-limit
//! review. Every panel is checked by the production exact compiler before save.
use multicad_core::{
    commands::{Actor, Channel, Request, Session},
    constraint_sketch::source::Source,
    document::{Document, Entity, EntityData, EntityId, Mutation},
    hierarchy::{self, Action, ContainerKind},
    model::{
        self, CutTargetQuery, Extent, ExtrudeParameters, ExtrudeRequest, PrimitiveFrame,
        PrimitiveParameters, PrimitiveRequest, Termination, length,
    },
    project_file::{self, ProjectFile},
    sketch::Datum,
};
use multicad_model::Compiler;
use std::{io::Write, path::PathBuf};
type Error = Box<dyn std::error::Error>;
fn actor() -> Actor {
    Actor::local(Channel::CommandLine)
}
fn hierarchy(session: &mut Session, action: Action) -> Result<Option<EntityId>, Error> {
    let prepared = hierarchy::prepare(session, session.revision(), actor(), action)?;
    let created = prepared.created;
    session.commit(prepared.prepared)?;
    Ok(created)
}
fn container(
    session: &mut Session,
    name: &str,
    kind: ContainerKind,
    parent: Option<EntityId>,
) -> Result<EntityId, Error> {
    hierarchy(
        session,
        Action::Create {
            kind,
            name: name.into(),
            parent,
        },
    )?
    .ok_or_else(|| "Container identity missing".into())
}
fn main() -> Result<(), Error> {
    let args = std::env::args_os().skip(1).collect::<Vec<_>>();
    let [path] = args.as_slice() else {
        return Err("Usage: model_scope_review NEW_PROJECT.multicad".into());
    };
    let path = PathBuf::from(path);
    let mut session = Session::new(Document::new("Native cut scope review")?)?;
    let active = container(&mut session, "Cabinet A", ContainerKind::Component, None)?;
    let group = container(
        &mut session,
        "Organized panels",
        ContainerKind::Group,
        Some(active),
    )?;
    let nested = container(
        &mut session,
        "Nested cabinet",
        ContainerKind::Component,
        Some(active),
    )?;
    let sibling = container(&mut session, "Cabinet B", ContainerKind::Component, None)?;
    for (index, (name, component, parent, visible)) in [
        ("A visible panel", Some(active), Some(active), true),
        ("A grouped panel", Some(active), Some(group), true),
        ("A hidden panel", Some(active), Some(active), false),
        ("Nested panel", Some(nested), Some(nested), true),
        ("B foreign panel", Some(sibling), Some(sibling), true),
        ("Document root panel", None, None, true),
    ]
    .into_iter()
    .enumerate()
    {
        hierarchy(&mut session, Action::SetActive { component })?;
        let request = PrimitiveRequest::new(
            name,
            PrimitiveParameters::Box {
                size: [length(80.), length(60.), length(10.)],
                frame: PrimitiveFrame {
                    origin: [length(0.), length(0.), length(index as f64 * 20.)],
                    ..Default::default()
                },
            },
        );
        let prepared = model::prepare_primitive(&session, session.revision(), actor(), &request)?;
        session.commit(prepared.prepared)?;
        if parent != component {
            hierarchy(
                &mut session,
                Action::Move {
                    entries: vec![request.body, request.feature],
                    parent,
                    before: None,
                },
            )?;
        }
        if !visible {
            hierarchy(
                &mut session,
                Action::SetVisible {
                    id: request.body,
                    visible: false,
                },
            )?;
        }
    }
    hierarchy(
        &mut session,
        Action::SetActive {
            component: Some(active),
        },
    )?;
    let (sketch, edits) = Source::create(Datum::Xy, "Cut profile 20 x 20")?;
    let prepared = session.prepare_scoped(Request {
        expected_revision: session.revision(),
        actor: actor(),
        label: "Create cut sketch".into(),
        edits,
    })?;
    session.commit(prepared)?;
    let mut source = Source::read(session.document(), sketch)?.0;
    source.add_rectangle([10., 10.], [30., 30.])?;
    let limit = Entity::new(
        "Limit at Z 110 mm",
        EntityData::Plane {
            origin_mm: [0., 0., 110.],
            normal: [0., 0., 1.],
            anchor: None,
        },
    );
    let mut edits = source.replace(session.document(), sketch)?;
    edits.push(Mutation::Insert(limit));
    let prepared = session.prepare_scoped(Request {
        expected_revision: session.revision(),
        actor: actor(),
        label: "Define cut review source".into(),
        edits,
    })?;
    session.commit(prepared)?;
    let exact = Compiler::default().compile(session.document(), session.revision())?;
    if exact.snapshot().bodies.len() != 6
        || exact
            .snapshot()
            .shapes
            .values()
            .any(|shape| shape.solid_count != 1 || (shape.volume_mm3 - 48000.).abs() > 1e-6)
    {
        return Err("Stacked-panel native geometry verification failed".into());
    }
    let regions = multicad_core::sketch_regions::extract(&source);
    if regions.regions.len() != 1 || !regions.diagnostics.is_empty() {
        return Err("Review cut profile must be one exact closed region".into());
    }
    let extrusion = ExtrudeRequest::new(
        sketch,
        regions
            .regions
            .iter()
            .map(|region| region.intent())
            .collect(),
        ExtrudeParameters {
            start_offset: length(-5.),
            extent: Extent::OneSided {
                end: Termination::Distance {
                    distance: length(120.),
                },
            },
            ..Default::default()
        },
    );
    let candidate = model::prepare_extrude(&session, session.revision(), actor(), &extrusion)?;
    let candidate_model =
        Compiler::default().compile(candidate.prepared.document(), session.revision())?;
    let mut query = CutTargetQuery {
        tools: extrusion
            .outputs
            .iter()
            .map(|output| output.feature)
            .collect(),
        component: Some(active),
        include: vec![],
        exclude: vec![],
    };
    let names = |selection: model::CutTargetSelection| -> std::collections::BTreeSet<String> {
        selection
            .targets
            .iter()
            .filter_map(|target| {
                session
                    .document()
                    .get(target.body)
                    .map(|entity| entity.name.clone())
            })
            .collect()
    };
    let selection = candidate_model.cut_targets(
        candidate.prepared.document(),
        session.revision(),
        &query,
        None,
    )?;
    if names(selection) != ["A visible panel".into(), "A grouped panel".into()].into() {
        return Err(
            "Default native cut scope must include exactly the two visible Cabinet A panels".into(),
        );
    }
    query.include.push(sibling);
    query.exclude.push(
        session
            .document()
            .entities()
            .find(|entity| entity.name == "A grouped panel")
            .ok_or("Grouped target missing")?
            .id,
    );
    let selection = candidate_model.cut_targets(
        candidate.prepared.document(),
        session.revision(),
        &query,
        None,
    )?;
    if names(selection) != ["A visible panel".into(), "B foreign panel".into()].into() {
        return Err("Explicit native include/exclude scope verification failed".into());
    }
    let project = ProjectFile::new(session.document().clone())?;
    let mut bytes = Vec::new();
    project_file::write(&project, &mut bytes, Default::default())?;
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| std::path::Path::new("."));
    let mut file = tempfile::NamedTempFile::new_in(parent)?;
    file.write_all(&bytes)?;
    file.as_file().sync_all()?;
    file.persist_noclobber(&path)?;
    println!(
        "{}: six exact panels, active Cabinet A, grouped/hidden/nested/foreign/root scopes; 20 x 20 mm cut profile; Z110 limit. Preview a cut from offset -5 mm for 120 mm: default targets are A visible panel and A grouped panel.",
        path.display()
    );
    Ok(())
}
