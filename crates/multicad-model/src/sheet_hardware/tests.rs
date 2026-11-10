use super::*;

#[test]
fn saved_front_panel_hardware_matches_native_on_the_actual_formed_layout() {
    use multicad_core::project_file;
    let document = project_file::read(
        &mut include_bytes!("../../../../examples/front-panel.tcd").as_slice(),
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap()
    .project
    .document;
    let compiler = crate::Compiler {
        omit_display: true,
        ..Default::default()
    };
    let native = compiler.compile(&document, 0).unwrap();
    let layouts = &native.snapshot().sheet_layouts;
    let mut counts = Vec::new();
    let near = |a: f64, b: f64| {
        assert!((a - b).abs() < 1e-6 * b.abs().max(1.), "{a} != {b}");
    };
    for entity in model::hardware_features(&document) {
        let item = model::read_hardware(&document, entity.id).unwrap().item;
        let legacy = native.solid(entity.id).unwrap();
        let rust = build(
            &document,
            entity.id,
            layouts,
            compiler.policy,
            GeometryBackend::Rust,
        )
        .unwrap_or_else(|e| panic!("{}: {e}", entity.name));
        assert!(rust.uses_rust_geometry());
        let count = rust.solid_count().unwrap();
        counts.push(count);
        assert_eq!(count, legacy.solid_count().unwrap(), "{}", entity.name);
        assert_eq!(
            rust.faces().unwrap().len(),
            legacy.faces().unwrap().len(),
            "{}",
            entity.name
        );
        near(rust.volume_mm3().unwrap(), legacy.volume_mm3().unwrap());
        let [a, b] = [rust.bounds().unwrap(), legacy.bounds().unwrap()];
        for (x, y) in a
            .min
            .into_iter()
            .chain(a.max)
            .zip(b.min.into_iter().chain(b.max))
        {
            near(x, y);
        }
        assert!(!rust.mesh(Default::default()).unwrap().indices.is_empty());
        if item == HardwareItem::Screw {
            let receipt = rust
                .manufacturing_mesh_with_receipt(Default::default())
                .unwrap();
            assert!(!receipt.mesh.indices.is_empty());
        }
    }
    counts.sort_unstable();
    assert_eq!(counts, [2, 2, 4]);
}
