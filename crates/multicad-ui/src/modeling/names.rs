use multicad_core::document::Document;
use std::collections::BTreeSet;

pub(super) struct GeneratedNames(BTreeSet<String>);
impl GeneratedNames {
    pub fn new(document: &Document) -> Self {
        Self(
            document
                .entities()
                .map(|entity| entity.name.clone())
                .collect(),
        )
    }
    /// Only generated defaults use this allocator. Deliberately entered names
    /// remain editable and may duplicate another canonical entity's name.
    pub fn numbered(&mut self, prefix: &str, producer_suffix: &str) -> String {
        for number in 1.. {
            let name = format!("{prefix} {number}");
            let producer = format!("{name}{producer_suffix}");
            if !self.0.contains(&name) && !self.0.contains(&producer) {
                self.0.extend([name.clone(), producer]);
                return name;
            }
        }
        unreachable!("A finite document cannot exhaust generated names")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use multicad_core::document::{Entity, EntityData};
    #[test]
    fn generated_names_reserve_body_and_producer_labels_without_renaming_source() {
        let mut document = Document::new("Name reservations").unwrap();
        for name in ["Box 1", "Box 2 feature"] {
            document
                .insert(Entity::new(name, EntityData::Group {}))
                .unwrap();
        }
        let before = document.clone();
        let mut names = GeneratedNames::new(&document);
        assert_eq!(names.numbered("Box", " feature"), "Box 3");
        assert_eq!(names.numbered("Box", " feature"), "Box 4");
        assert_eq!(document, before);
    }
}
