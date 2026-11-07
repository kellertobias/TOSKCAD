//! Generate the owned item Line 8 profile Library project.
#[allow(dead_code)]
#[path = "support/native_preview.rs"]
mod native_preview;

use multicad_core::{
    commands::{Actor, Channel, Request, Session},
    constraint_sketch::Dimension,
    document::{DOCUMENT_SCHEMA, Document, Entity, EntityData, EntityId, Mutation},
    materials::{LibraryDefinition, ProfileShape, Stock, StockKind},
    model::{self, BodyOutput, MemberDefinition, MemberPath, MemberRequest, length},
    parameters::BoundExpression,
    project_file::{self, ProjectFile, info::FileInfo},
    symbols::{self, SymbolAuthorRequest, SymbolKind},
};
use serde_json::json;
use std::{io::Write, path::Path};

type Error = Box<dyn std::error::Error>;
const NAME: &str = "item Line 8 profile 40 × 40";
const FILE: &str = "profile-item-line8-40x40.tcd";
const DEFAULT_LENGTH_MM: f64 = 160.;

fn identity(role: u32) -> Result<EntityId, Error> {
    Ok(EntityId::parse(&format!(
        "cad20800-0000-4000-8000-{role:012x}"
    ))?)
}

fn build() -> Result<ProjectFile, Error> {
    let document: Document = serde_json::from_value(json!({
        "schema_version": DOCUMENT_SCHEMA,
        "id": identity(1)?,
        "name": NAME,
        "entities": {},
    }))?;
    let mut session = Session::new(document)?;
    let actor = Actor::local(Channel::Mcp);
    let mut material = Entity::new(
        "Natural anodized aluminum",
        EntityData::Material {
            density_kg_m3: None,
            color_rgba: [166, 173, 181, 255],
            section_hatch: None,
        },
    );
    material.id = identity(2)?;
    let stock = LibraryDefinition::Stock(Stock {
        material: material.id,
        specification: StockKind::Profile {
            designation: "item 0.0.026.03".into(),
            length_mm: 6000.,
            width_mm: model::ITEM_LINE_8_WIDTH_MM,
            height_mm: model::ITEM_LINE_8_WIDTH_MM,
            wall_mm: None,
            shape: ProfileShape::ItemLine8_40x40,
        },
    })
    .entity(identity(3)?, "Line 8 40 × 40 stock")?;
    let mut parameter = Entity::new(
        "Length",
        EntityData::Parameter {
            expression: DEFAULT_LENGTH_MM.to_string(),
            unit: "mm".into(),
            exposed: true,
            inputs: vec![],
        },
    );
    parameter.id = identity(4)?;
    let parameter_id = parameter.id;
    let stock_id = stock.id;
    session.apply(Request {
        expected_revision: session.revision(),
        actor: actor.clone(),
        label: "Line 8 stock and length".into(),
        edits: vec![
            Mutation::Insert(material),
            Mutation::Insert(stock),
            Mutation::Insert(parameter),
        ],
    })?;
    let bound_length = Dimension::Expression {
        expression: BoundExpression::bind(session.document(), "=$Length", "mm", None)?,
    };
    let mut request = MemberRequest::new(
        NAME,
        MemberDefinition {
            output: BodyOutput {
                body: EntityId::nil(),
            },
            stock: stock_id,
            path: MemberPath::Points {
                start: [length(0.), length(0.), length(0.)],
                end: [bound_length, length(0.), length(0.)],
            },
            anchor: Default::default(),
            rotation: None,
            extend_start: None,
            extend_end: None,
        },
    );
    request.feature = identity(5)?;
    request.body = identity(6)?;
    request.member.output.body = request.body;
    let prepared = model::prepare_members(
        &session,
        session.revision(),
        actor.clone(),
        std::slice::from_ref(&request),
        None,
    )?;
    session.commit(prepared.prepared)?;

    // The interactive Member transaction allocates this metadata ID. Give it
    // a retained ID before extracting the template so a rebuild keeps the pin.
    let document = session.document();
    let (allocated, _) = multicad_core::materials::part_properties(document, request.body)?
        .ok_or("The canonical member has no stock properties")?;
    let mut properties = document
        .get(allocated)
        .cloned()
        .ok_or("The canonical stock properties are missing")?;
    properties.id = identity(7)?;
    session.apply(Request {
        expected_revision: session.revision(),
        actor: actor.clone(),
        label: "Retain profile property identity".into(),
        edits: vec![Mutation::Remove(allocated), Mutation::Insert(properties)],
    })?;

    let mut author = SymbolAuthorRequest::new(
        NAME,
        request.body,
        SymbolKind::Dynamic,
        vec![parameter_id],
        None,
    );
    author.definition = identity(8)?;
    author.symbol = identity(9)?;
    let prepared = symbols::prepare_author(&session, session.revision(), actor.clone(), &author)?;
    session.commit(prepared.prepared)?;
    let preview =
        native_preview::render_symbol_preview(session.document(), author.definition, |_| {
            [0.65, 0.69, 0.74]
        })?;
    let info = FileInfo {
        name: NAME.into(),
        description: "item 0.0.026.03 mounting dimensions. Set Length when placing the member. Four open grooves, rounded outer corners and a through core. The modeled groove flanks use a simplified taper.".into(),
        preview: Some(preview),
        capture_on_save: true,
    };
    let mut edits = project_file::info::edits(session.document(), &info)?;
    for edit in &mut edits {
        if let Mutation::Insert(entity) = edit {
            entity.id = identity(10)?;
        }
    }
    session.apply(Request {
        expected_revision: session.revision(),
        actor,
        label: "Line 8 Library card".into(),
        edits,
    })?;
    Ok(ProjectFile::new(session.document().clone())?)
}

fn main() -> Result<(), Error> {
    let output = std::env::args_os()
        .nth(1)
        .ok_or("Usage: profile_library OUTPUT_FOLDER")?;
    let output = Path::new(&output);
    std::fs::create_dir_all(output)?;
    let project = build()?;
    let definitions = symbols::definitions(&project.document)?;
    let mut bytes = vec![];
    project_file::write(&project, &mut bytes, Default::default())?;
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output.join(FILE))?
        .write_all(&bytes)?;
    println!(
        "{}",
        json!({
            "file": FILE,
            "document": project.document.id(),
            "sha256": project_file::ContentHash::of(&bytes).to_string(),
            "pin": definitions[0].1.pin()?.to_string(),
        })
    );
    Ok(())
}
