use super::*;
use crate::{
    document::{Document, EntityData, EntityId},
    drawings,
    project_file::ContentHash,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
pub const MAX_MATCHES: usize = 4096;
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExportStamp {
    pub document: EntityId,
    pub revision: u64,
    pub source_hash: ContentHash,
}
impl ExportStamp {
    pub fn capture(document: &Document, revision: u64) -> Result<Self, ExportError> {
        Ok(Self {
            document: document.id(),
            revision,
            source_hash: ContentHash::document(document)
                .map_err(|e| ExportError::new(ErrorKind::Invalid, None, e))?,
        })
    }
    /// Explicit action boundary; UI paint should use its cached revision key.
    pub fn check(&self, document: &Document, revision: u64) -> Result<(), ExportError> {
        if self.document != document.id()
            || self.revision != revision
            || self.source_hash
                != ContentHash::document(document)
                    .map_err(|e| ExportError::new(ErrorKind::Invalid, None, e))?
        {
            Err(ExportError::new(
                ErrorKind::Stale,
                None,
                "Export preview is stale; resolve the current source again",
            ))
        } else {
            Ok(())
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PartRef {
    pub part: EntityId,
    pub occurrence_path: Vec<EntityId>,
    pub root_assembly: Option<EntityId>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResolvedPart {
    pub reference: PartRef,
    pub name: String,
    pub occurrence_names: Vec<String>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResolvedPage {
    pub reference: PageRef,
    pub number: String,
    pub title: String,
    pub paper_mm: [f64; 2],
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResolvedSet {
    pub stamp: ExportStamp,
    pub set: EntityId,
    pub name: String,
    pub parts: Vec<ResolvedPart>,
    pub pages: Vec<ResolvedPage>,
    pub recipes: Vec<Recipe>,
    pub naming: NamingRule,
}
fn missing(id: EntityId, role: &str) -> ExportError {
    ExportError::new(
        ErrorKind::Missing,
        Some(id),
        format!("Missing {role} {id}; scope was not broadened"),
    )
}
fn invalid(id: EntityId, role: &str) -> ExportError {
    ExportError::new(
        ErrorKind::Invalid,
        Some(id),
        format!("{id} is not a supported {role}"),
    )
}
struct Walker<'a> {
    document: &'a Document,
    parts: BTreeSet<PartRef>,
    steps: usize,
}
impl<'a> Walker<'a> {
    fn visit(
        &mut self,
        id: EntityId,
        path: &mut Vec<EntityId>,
        root: Option<EntityId>,
    ) -> Result<(), ExportError> {
        self.steps += 1;
        if self.steps > MAX_MATCHES * 4 || path.len() > 32 {
            return Err(ExportError::new(
                ErrorKind::Budget,
                Some(id),
                "Export occurrence traversal exceeds its bounded budget",
            ));
        }
        let entity = self
            .document
            .get(id)
            .ok_or_else(|| missing(id, "scope source"))?;
        match &entity.data {
            EntityData::Part { .. } => {
                if path.is_empty()
                    && crate::model::scale_component_owner(self.document, id)?.is_some()
                {
                    return Err(ExportError::new(
                        ErrorKind::Unsupported,
                        Some(id),
                        "Owned scale geometry must be exported through its explicit Instance",
                    ));
                }
                self.parts.insert(PartRef {
                    part: id,
                    occurrence_path: path.clone(),
                    root_assembly: root,
                });
                if self.parts.len() > MAX_MATCHES {
                    return Err(ExportError::new(
                        ErrorKind::Budget,
                        None,
                        "Export scope exceeds 4096 matched source occurrences",
                    ));
                }
            }
            EntityData::Instance { component, .. } => {
                if path.contains(&id) {
                    return Err(ExportError::new(
                        ErrorKind::Invalid,
                        Some(id),
                        "Cyclic occurrence path",
                    ));
                }
                path.push(id);
                self.visit(*component, path, root)?;
                path.pop();
            }
            EntityData::Assembly { instances } => {
                for instance in instances {
                    let member = self
                        .document
                        .get(*instance)
                        .ok_or_else(|| missing(*instance, "assembly occurrence"))?;
                    if !matches!(member.data, EntityData::Instance { .. }) {
                        return Err(invalid(*instance, "assembly occurrence"));
                    }
                    self.visit(*instance, path, root.or(Some(id)))?;
                }
            }
            _ => return Err(invalid(id, "Part, Instance or Assembly scope source")),
        };
        Ok(())
    }
}
pub fn resolve(
    document: &Document,
    revision: u64,
    set: EntityId,
) -> Result<ResolvedSet, ExportError> {
    let definition = read(document, set)?;
    let stamp = ExportStamp::capture(document, revision)?;
    let parts = resolve_parts(document, &definition.scope)?;
    let book_ids: BTreeSet<_> = match &definition.pages {
        Pages::None => BTreeSet::new(),
        Pages::Project => drawings::books(document).collect(),
        Pages::Selected { pages } => pages.iter().map(|p| p.book).collect(),
    };
    let mut books = BTreeMap::new();
    for book in book_ids {
        if document.get(book).is_none() {
            return Err(missing(book, "drawing book"));
        }
        books.insert(book, drawings::read(document, book)?);
    }
    let page_refs = match definition.pages {
        Pages::None => vec![],
        Pages::Selected { pages } => pages,
        Pages::Project => books
            .iter()
            .flat_map(|(book, value)| {
                value.pages.iter().map(move |page| PageRef {
                    book: *book,
                    page: page.id,
                })
            })
            .collect(),
    };
    if page_refs.len() > 1024 {
        return Err(ExportError::new(
            ErrorKind::Budget,
            None,
            "Export scope exceeds 1024 drawing pages",
        ));
    }
    let mut pages = Vec::new();
    for reference in page_refs {
        let book = books.get(&reference.book).expect("book list was validated");
        let page = book
            .pages
            .iter()
            .find(|p| p.id == reference.page)
            .ok_or_else(|| missing(reference.page, "drawing page"))?;
        pages.push(ResolvedPage {
            reference,
            number: page.number.clone(),
            title: page.info.title.clone(),
            paper_mm: page.settings.dimensions(),
        });
    }
    Ok(ResolvedSet {
        stamp,
        set,
        name: document.get(set).expect("read validated set").name.clone(),
        parts,
        pages,
        recipes: definition.recipes,
        naming: definition.naming,
    })
}

/// Resolve canonical part occurrences independently of a saved output set.
pub fn resolve_parts(document: &Document, scope: &Scope) -> Result<Vec<ResolvedPart>, ExportError> {
    let mut walker = Walker {
        document,
        parts: BTreeSet::new(),
        steps: 0,
    };
    match &scope.root {
        ScopeRoot::Project => {
            let mut members = BTreeSet::new();
            let mut components = BTreeSet::new();
            for entity in document.entities() {
                match &entity.data {
                    EntityData::Assembly { instances } => members.extend(instances.iter().copied()),
                    EntityData::Instance { component, .. } => {
                        components.insert(*component);
                    }
                    _ => {}
                }
            }
            for entity in document.entities() {
                match &entity.data {
                    EntityData::Part { .. }
                        if crate::model::scale_component_owner(document, entity.id)?.is_none() =>
                    {
                        walker.visit(entity.id, &mut vec![], None)?
                    }
                    EntityData::Instance { .. } if !members.contains(&entity.id) => {
                        walker.visit(entity.id, &mut vec![], None)?
                    }
                    EntityData::Assembly { .. } if !components.contains(&entity.id) => {
                        walker.visit(entity.id, &mut vec![], Some(entity.id))?
                    }
                    _ => {}
                }
            }
        }
        ScopeRoot::Assembly { assembly } => {
            if !matches!(
                document.get(*assembly).map(|e| &e.data),
                Some(EntityData::Assembly { .. })
            ) {
                return Err(if document.get(*assembly).is_none() {
                    missing(*assembly, "assembly")
                } else {
                    invalid(*assembly, "Assembly")
                });
            }
            walker.visit(*assembly, &mut vec![], Some(*assembly))?;
        }
        ScopeRoot::Explicit { entities } => {
            for id in entities {
                let Some(entity) = document.get(*id) else {
                    return Err(missing(*id, "explicit source"));
                };
                if !matches!(
                    entity.data,
                    EntityData::Part { .. } | EntityData::Instance { .. }
                ) {
                    return Err(invalid(*id, "explicit Part or Instance"));
                }
                walker.visit(*id, &mut vec![], None)?;
            }
        }
    }
    let matched = scope
        .tags
        .as_ref()
        .map(|q| q.select(document))
        .transpose()?
        .map(|ids| ids.into_iter().collect::<BTreeSet<_>>());
    let parts = walker
        .parts
        .into_iter()
        .filter(|p| matched.as_ref().is_none_or(|ids| ids.contains(&p.part)))
        .map(|reference| {
            let name = document
                .get(reference.part)
                .expect("walker validated part")
                .name
                .clone();
            let occurrence_names = reference
                .occurrence_path
                .iter()
                .map(|id| {
                    document
                        .get(*id)
                        .expect("walker validated occurrence")
                        .name
                        .clone()
                })
                .collect();
            ResolvedPart {
                reference,
                name,
                occurrence_names,
            }
        })
        .collect();
    Ok(parts)
}
