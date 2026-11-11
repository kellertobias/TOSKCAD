#[allow(dead_code)]
#[path = "../examples/equipment_library.rs"]
mod library;
use multicad_core::{
    equipment::{self, Kind},
    project_file, symbols,
};
#[test]
fn included_equipment_files_match_native_generation_and_graphical_tool_pins() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../library");
    for kind in Kind::ALL {
        let project = library::build(kind).unwrap();
        let mut bytes = vec![];
        project_file::write(&project, &mut bytes, Default::default()).unwrap();
        let retained = std::fs::read(root.join(library::file_name(kind))).unwrap();
        assert!(
            bytes == retained,
            "{kind:?}: generated {}, retained {}",
            project_file::ContentHash::of(&bytes),
            project_file::ContentHash::of(&retained)
        );
        let definitions = symbols::definitions(&project.document).unwrap();
        assert_eq!(definitions.len(), 1);
        assert_eq!(
            definitions[0].1.pin().unwrap(),
            equipment::generic_definition(kind).unwrap().pin().unwrap()
        );
        assert!(
            project_file::info::read(&project.document)
                .unwrap()
                .preview
                .is_some()
        );
    }
}
