use super::*;
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HierarchyDiagnostic {
    SharedLegacyInstance {
        entry: EntityId,
        parents: Vec<EntityId>,
    },
}
pub struct Index<'a> {
    document: &'a Document,
    parents: BTreeMap<EntityId, BTreeSet<EntityId>>,
    children: BTreeMap<EntityId, Vec<EntityId>>,
}
impl<'a> Index<'a> {
    pub fn new(document: &'a Document) -> Result<Self, HierarchyError> {
        validate(document)?;
        let children = combined_children(document);
        let mut parents: BTreeMap<_, BTreeSet<_>> = BTreeMap::new();
        for (parent, members) in &children {
            for id in members {
                parents.entry(*id).or_default().insert(*parent);
            }
        }
        Ok(Self {
            document,
            parents,
            children,
        })
    }
    pub fn diagnostics(&self) -> Vec<HierarchyDiagnostic> {
        self.parents
            .iter()
            .filter(|(_, parents)| parents.len() > 1)
            .map(
                |(entry, parents)| HierarchyDiagnostic::SharedLegacyInstance {
                    entry: *entry,
                    parents: parents.iter().copied().collect(),
                },
            )
            .collect()
    }
    pub fn children(&self, container: Option<EntityId>) -> Result<Vec<EntityId>, HierarchyError> {
        let id = container.unwrap_or(self.document.id());
        if id != self.document.id()
            && !self
                .document
                .get(id)
                .is_some_and(|entity| container_kind(&entity.data).is_some())
        {
            return Err(invalid("Choose a component, group or root"));
        }
        let mut children = self.children.get(&id).cloned().unwrap_or_default();
        if id == self.document.id() {
            children.extend(
                self.document
                    .entities()
                    .filter(|entity| {
                        is_model_entry(&entity.data) && !self.parents.contains_key(&entity.id)
                    })
                    .map(|entity| entity.id),
            );
        }
        Ok(children)
    }
    pub fn parent(&self, entry: EntityId) -> Result<Option<EntityId>, HierarchyError> {
        if entry == self.document.id() {
            return Ok(None);
        }
        if self.document.get(entry).is_none() {
            return Err(invalid(format!("Unknown model entry {entry}")));
        }
        let Some(parents) = self.parents.get(&entry) else {
            return Ok(None);
        };
        if parents.len() > 1 {
            return Err(HierarchyError::AmbiguousOwner {
                entry,
                parents: parents.iter().copied().collect(),
            });
        }
        Ok(parents
            .first()
            .copied()
            .filter(|id| *id != self.document.id()))
    }
    pub fn owning_component(&self, entry: EntityId) -> Result<Option<EntityId>, HierarchyError> {
        let mut current = entry;
        while let Some(parent) = self.parent(current)? {
            if self
                .document
                .get(parent)
                .is_some_and(|entity| matches!(entity.data, EntityData::Assembly { .. }))
            {
                return Ok(Some(parent));
            }
            current = parent;
        }
        Ok(None)
    }
    pub fn effective_visibility(&self, entry: EntityId) -> Result<bool, HierarchyError> {
        if entry == self.document.id() {
            return Ok(!self.document.hierarchy().hidden.contains(&entry));
        }
        if self.document.get(entry).is_none() {
            return Err(invalid(format!("Unknown model entry {entry}")));
        }
        if self
            .document
            .hierarchy()
            .hidden
            .contains(&self.document.id())
        {
            return Ok(false);
        }
        let mut current = Some(entry);
        while let Some(id) = current {
            if self.document.hierarchy().hidden.contains(&id) {
                return Ok(false);
            }
            current = self.parent(id)?;
        }
        Ok(true)
    }
    /// Canonical Part definitions, excluding nested/sibling component boundaries.
    /// Instance placement does not transfer ownership of its source definition.
    pub fn default_cut_bodies(
        &self,
        component: Option<EntityId>,
    ) -> Result<Vec<EntityId>, HierarchyError> {
        if let Some(id) = component
            && !self
                .document
                .get(id)
                .is_some_and(|entity| matches!(entity.data, EntityData::Assembly { .. }))
        {
            return Err(invalid("Default cut scope must be a component or root"));
        }
        let mut bodies = Vec::new();
        for entity in self.document.entities() {
            if matches!(entity.data, EntityData::Part { .. })
                && self.owning_component(entity.id)? == component
                && self.effective_visibility(entity.id)?
            {
                bodies.push(entity.id);
            }
        }
        Ok(bodies)
    }
    pub fn active_cut_bodies(&self) -> Result<Vec<EntityId>, HierarchyError> {
        self.default_cut_bodies(self.document.hierarchy().active_component)
    }
}
