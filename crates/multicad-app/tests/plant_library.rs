#[allow(dead_code)]
#[path = "../examples/plant_library.rs"]
mod library;
use multicad_core::{
    plants::{self, PlantKind},
    project_file,
};
#[test]
fn included_plant_projects_match_native_generation_and_the_creation_tool_pin() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../library");
    for (kind, name) in [
        (PlantKind::Tree, "plant-generic-tree.tcd"),
        (PlantKind::Shrub, "plant-generic-shrub.tcd"),
    ] {
        let project = library::build(kind).unwrap();
        let mut bytes = vec![];
        project_file::write(&project, &mut bytes, Default::default()).unwrap();
        assert_eq!(bytes, std::fs::read(root.join(name)).unwrap());
        let definitions = multicad_core::symbols::definitions(&project.document).unwrap();
        assert_eq!(definitions.len(), 1);
        assert_eq!(
            definitions[0].1.pin().unwrap(),
            plants::generic_definition(kind).unwrap().pin().unwrap()
        );
        let template = plants::template_for(&definitions[0].1.template, definitions[0].1.root)
            .unwrap()
            .unwrap();
        assert_eq!(template.kind, kind);
        assert!(plants::required_capability(&project.document));
    }
}
