use multicad_core::project_file;
use multicad_model::Compiler;

#[test]
fn full_front_panel_schedule_matches_unfold_and_scoped_evaluation_omits_it() {
    let document = project_file::read(
        &mut include_bytes!("../../../examples/front-panel.tcd").as_slice(),
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap()
    .project
    .document;
    let part = document
        .entities()
        .find(|entity| entity.name == "Front panel sheet")
        .unwrap()
        .id;
    let compiler = Compiler {
        omit_display: true,
        ..Default::default()
    };
    let full = compiler.compile(&document, 0).unwrap();
    let expected: Vec<_> = full
        .unfold(&document, part)
        .unwrap()
        .bends
        .into_iter()
        .map(|bend| bend.record)
        .collect();
    assert!(!expected.is_empty());
    assert_eq!(full.sheet_bend_schedules(&document), [(part, expected)]);

    let scoped = compiler.compile_sources(&document, 0, &[part]).unwrap();
    assert!(scoped.sheet_bend_schedules(&document).is_empty());
}
