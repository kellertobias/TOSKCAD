use super::{DrawingBook, Page, TYPE_ID, read, source::invalid};
use crate::{
    commands::{Actor, Prepared, Request, Session},
    document::{Document, DocumentError, EntityData, EntityId, Mutation},
};

#[derive(Clone, Debug)]
pub enum PageAction {
    Add(Page),
    Duplicate {
        page: EntityId,
        new_page: EntityId,
    },
    Remove(EntityId),
    Move {
        page: EntityId,
        before: Option<EntityId>,
    },
    Replace(Page),
}
/// Prepare a complete book edit against a revision. Applying it uses the same
/// Session commit API as all other source changes; no GUI or cache state enters.
pub fn prepare(
    session: &Session,
    revision: u64,
    actor: Actor,
    book_id: EntityId,
    action: PageAction,
) -> Result<Prepared, DocumentError> {
    let mut book = read(session.document(), book_id)?;
    let label = match action {
        PageAction::Add(page) => {
            book.pages.push(page);
            "Add drawing page"
        }
        PageAction::Duplicate { page, new_page } => {
            if new_page.is_nil() {
                return Err(invalid("Duplicate page needs an identity"));
            }
            let i = position(&book, page)?;
            let mut copy = book.pages[i].clone();
            copy.id = new_page;
            copy.number = next_number(&book);
            for note in &mut copy.notes {
                note.id = EntityId::new();
            }
            book.pages.insert(i + 1, copy);
            "Duplicate drawing page"
        }
        PageAction::Remove(id) => {
            let i = position(&book, id)?;
            book.pages.remove(i);
            "Remove drawing page"
        }
        PageAction::Move { page, before } => {
            let i = position(&book, page)?;
            if before == Some(page) {
                return Err(invalid("Choose another page position"));
            }
            let value = book.pages.remove(i);
            let index = before
                .map(|id| position(&book, id))
                .transpose()?
                .unwrap_or(book.pages.len());
            book.pages.insert(index, value);
            "Reorder drawing pages"
        }
        PageAction::Replace(page) => {
            let i = position(&book, page.id)?;
            book.pages[i] = page;
            "Edit drawing page"
        }
    };
    book.validate()?;
    let mut entity = session
        .document()
        .get(book_id)
        .ok_or(DocumentError::Missing(book_id))?
        .clone();
    let EntityData::Drawing { definition, .. } = &mut entity.data else {
        unreachable!()
    };
    definition.data = serde_json::to_value(book).map_err(invalid)?;
    session
        .prepare(Request {
            expected_revision: revision,
            actor,
            label: label.into(),
            edits: vec![Mutation::Replace(entity)],
        })
        .map_err(invalid)
}
pub fn prepare_create(
    session: &Session,
    revision: u64,
    actor: Actor,
    name: &str,
    page: Option<Page>,
) -> Result<(EntityId, Prepared), DocumentError> {
    let book = DrawingBook {
        pages: page.into_iter().collect(),
    };
    let entity = book.entity(name)?;
    let id = entity.id;
    Ok((
        id,
        session
            .prepare(Request {
                expected_revision: revision,
                actor,
                label: "Create drawing document".into(),
                edits: vec![Mutation::Insert(entity)],
            })
            .map_err(invalid)?,
    ))
}
pub fn next_number(book: &DrawingBook) -> String {
    (1..)
        .map(|i| i.to_string())
        .find(|number| book.pages.iter().all(|page| &page.number != number))
        .expect("bounded page count")
}
pub fn replace_edits(
    document: &Document,
    id: EntityId,
    book: &DrawingBook,
) -> Result<Vec<Mutation>, DocumentError> {
    book.validate()?;
    let mut entity = document.get(id).ok_or(DocumentError::Missing(id))?.clone();
    let EntityData::Drawing { definition, .. } = &mut entity.data else {
        return Err(invalid("Select a drawing document"));
    };
    if definition.type_id != TYPE_ID {
        return Err(invalid("Unsupported drawing provider"));
    }
    definition.data = serde_json::to_value(book).map_err(invalid)?;
    Ok(vec![Mutation::Replace(entity)])
}
fn position(book: &DrawingBook, id: EntityId) -> Result<usize, DocumentError> {
    book.pages
        .iter()
        .position(|page| page.id == id)
        .ok_or(DocumentError::Missing(id))
}
