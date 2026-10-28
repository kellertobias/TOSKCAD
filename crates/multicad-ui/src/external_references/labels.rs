//! Cached occurrence names keep repeated copies distinct without hashing every
//! path for every open-menu row or letting long names hide the copy suffix.
use crate::{controls, egui};
use multicad_core::{
    document::{Document, EntityId},
    model::OccurrencePath,
    project_file::ContentHash,
};
use std::collections::{BTreeMap, BTreeSet};
pub(super) struct Label {
    pub full: String,
    pub help: String,
    component: String,
    key: String,
}
impl Label {
    pub fn compact(&self, ui: &egui::Ui) -> String {
        let suffix = format!(" ·{}", self.key);
        let font = egui::TextStyle::Button.resolve(ui.style());
        let width = (controls::inspector_width(ui) - 30.).max(0.);
        let fits = |value: &str| {
            ui.painter()
                .layout_no_wrap(value.into(), font.clone(), egui::Color32::WHITE)
                .size()
                .x
                <= width
        };
        let mut prefix = self.component.clone();
        let full = format!("{prefix}{suffix}");
        if fits(&full) {
            return full;
        }
        while !prefix.is_empty() {
            prefix.pop();
            let value = format!("{prefix}…{suffix}");
            if fits(&value) {
                return value;
            }
        }
        suffix.trim().into()
    }
}
#[derive(Default)]
pub(super) struct Labels(BTreeMap<(EntityId, OccurrencePath), Label>);
impl Labels {
    pub fn new(
        document: &Document,
        entries: impl IntoIterator<Item = (EntityId, OccurrencePath)>,
    ) -> Self {
        let entries: BTreeSet<_> = entries.into_iter().collect();
        let ids = |path: &OccurrencePath| {
            std::iter::once(path.root)
                .chain(path.members.iter().copied())
                .map(|id| id.to_string())
                .collect::<Vec<_>>()
                .join(" / ")
        };
        let hashes: BTreeMap<_, _> = entries
            .iter()
            .map(|(_, path)| {
                (
                    path.clone(),
                    ContentHash::of(ids(path).as_bytes()).to_string(),
                )
            })
            .collect();
        let mut widths: BTreeMap<_, _> =
            hashes.keys().map(|path| (path.clone(), 6_usize)).collect();
        loop {
            let mut counts = BTreeMap::<String, usize>::new();
            for (path, hash) in &hashes {
                *counts.entry(hash[..widths[path]].into()).or_default() += 1;
            }
            let mut changed = false;
            for (path, width) in &mut widths {
                if *width < 64 && counts[&hashes[path][..*width]] > 1 {
                    *width += 2;
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
        let mut full_counts = BTreeMap::<String, usize>::new();
        for hash in hashes.values() {
            *full_counts.entry(hash.clone()).or_default() += 1;
        }
        let labels = entries.into_iter().map(|(component, path)| {
            let full_ids = ids(&path);
            let digest = &hashes[&path];
            let mut key = digest[..widths[&path]].to_owned();
            if full_counts[digest] > 1 {
                key = full_ids.clone();
            }
            let component_name = document
                .get(component)
                .map_or("Missing Part", |entity| entity.name.as_str())
                .to_string();
            let names = std::iter::once(path.root)
                .chain(path.members.iter().copied())
                .map(|id| document.get(id).map_or("Missing occurrence", |entity| entity.name.as_str()))
                .collect::<Vec<_>>()
                .join(" / ");
            let placement_name = if path.root == component && path.members.is_empty() {
                "World Part"
            } else {
                names.rsplit(" / ").next().unwrap_or(&names)
            };
            let full = format!("{component_name} · {placement_name} [{key}]");
            let help = format!(
                "Part: {component_name}\nOccurrence: {names}\nFull path IDs: {full_ids}\nComponent ID: {component}"
            );
            ((component, path), Label { full, help, component: component_name, key })
        }).collect();
        Self(labels)
    }
    pub fn get(&self, component: EntityId, path: &OccurrencePath) -> Option<&Label> {
        self.0.get(&(component, path.clone()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::external_references::occurrence_context_tests::context_fixture;
    #[test]
    fn repeated_copy_names_keep_unique_ids_visible_in_a_narrow_selected_field() {
        let (session, _, body, contexts) = context_fixture();
        let mut entity = session.document().get(body).unwrap().clone();
        entity.name = "Cabinet frontage with a deliberately long component name".into();
        let document = session
            .document()
            .edited(&[multicad_core::document::Mutation::Replace(entity)])
            .unwrap();
        let entries = std::iter::once((
            body,
            OccurrencePath {
                root: body,
                members: vec![],
            },
        ))
        .chain(contexts.iter().map(|c| (c.component, c.path.clone())))
        .collect::<Vec<_>>();
        let labels = Labels::new(&document, entries.clone());
        assert_eq!(labels.0.len(), 3);
        let ctx = egui::Context::default();
        let mut visible = vec![];
        ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(230., 300.),
                )),
                ..Default::default()
            },
            |ui| {
                for (component, path) in &entries {
                    let name = labels.get(*component, path).unwrap();
                    let value = name.compact(ui);
                    assert!(value.starts_with("Cabinet"), "{value}");
                    assert!(
                        value.ends_with(&name.key),
                        "The copy ID must survive name truncation: {value}"
                    );
                    let font = egui::TextStyle::Button.resolve(ui.style());
                    assert!(
                        ui.painter()
                            .layout_no_wrap(value.clone(), font, egui::Color32::WHITE)
                            .size()
                            .x
                            <= controls::inspector_width(ui) - 30.
                    );
                    assert!(name.help.contains(&path.root.to_string()));
                    visible.push(value);
                }
            },
        )
        .drop_without_applying_deltas();
        assert_eq!(visible.iter().collect::<BTreeSet<_>>().len(), 3);
    }
    #[test]
    fn labels_keep_the_complete_nested_path_and_component_identity_in_hover_help() {
        let document = Document::new("Missing source labels").unwrap();
        let component = EntityId::new();
        let root = EntityId::new();
        let leaf = EntityId::new();
        let paths = [
            OccurrencePath {
                root,
                members: vec![EntityId::new(), leaf],
            },
            OccurrencePath {
                root,
                members: vec![EntityId::new(), leaf],
            },
        ];
        let labels = Labels::new(
            &document,
            paths.iter().cloned().map(|path| (component, path)),
        );
        let a = labels.get(component, &paths[0]).unwrap();
        let b = labels.get(component, &paths[1]).unwrap();
        assert_ne!(a.key, b.key);
        for path in paths {
            let label = labels.get(component, &path).unwrap();
            for id in std::iter::once(path.root).chain(path.members) {
                assert!(label.help.contains(&id.to_string()));
            }
            assert!(label.help.contains(&component.to_string()));
        }
    }
}
