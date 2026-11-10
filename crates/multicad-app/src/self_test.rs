//! A headless packaged-runtime smoke check, not a CAD workspace acceptance test.
use multicad_kernel::{Error, Mesh, Shape};
use std::path::Path;

pub fn run(directory: &Path) -> Result<(), Error> {
    std::fs::create_dir_all(directory)?;
    if multicad_kernel::OCCT_LINKED {
        native_smoke(directory)?;
    } else {
        rust_smoke(directory)?;
    }
    crate::modeling::smoke(directory).map_err(|e| -> Error { e.into() })?;
    // The packaged executable must find and open its bundled examples.
    let (examples, searched) = multicad_ui::examples::locate_bundled();
    let receipt = crate::bundled_examples::verify(examples.as_deref(), &searched)
        .map_err(|e| -> Error { e.into() })?;
    println!("{receipt}");
    println!("Native runtime smoke check passed");
    Ok(())
}

fn native_smoke(directory: &Path) -> Result<(), Error> {
    let [width, depth] = multicad_solver::rectangle(120.0, 80.0)?;
    let solid = Shape::extrude(width, depth, 24.0)?.cut(90.0, 30.0, 80.0, 24.0)?;
    let expected = 172800.0;
    if !solid.valid()? || (solid.volume()? - expected).abs() > 1e-5 {
        return Err("Native solid validity/volume smoke check failed".into());
    }
    verify_mesh(&solid.mesh(0.1)?)?;
    let step = directory.join("native-smoke.step");
    solid.write_step(&step)?;
    let reimported = Shape::read_step(&step)?;
    if !reimported.valid()? || (reimported.volume()? - expected).abs() > 1e-5 {
        return Err("STEP round-trip failed".into());
    }
    std::fs::write(
        directory.join("native-smoke.txt"),
        "OCCT 7.9.3; constrained rectangle; valid solid; Boolean cut; tessellation; STEP round-trip: passed\n",
    )?;
    Ok(())
}

fn rust_smoke(directory: &Path) -> Result<(), Error> {
    use multicad_core::document::EntityId;
    use multicad_kernel::{
        BooleanOperation, GeometryPolicy, PlaneFrame, Solid, rust_kernel::Rational,
    };

    let policy = GeometryPolicy::default();
    let box_at = |id, size: [i64; 3], origin| {
        Solid::rust_box(
            id,
            size.map(|value| Rational::new(value, 1).expect("small integer dimension")),
            PlaneFrame {
                origin,
                ..Default::default()
            },
            policy,
        )
    };
    let stock = box_at(1, [120, 80, 24], [0.; 3])?;
    let tool = box_at(2, [30, 80, 24], [90., 0., 0.])?;
    let cut = stock
        .boolean(&tool, BooleanOperation::Subtract, policy)?
        .ok_or("Rust Boolean cut returned no material")?;
    let verify = |solid: &Solid| -> Result<(), Error> {
        if solid.solid_count()? != 1 || (solid.volume_mm3()? - 172800.).abs() > 1e-5 {
            return Err("Rust solid volume or identity smoke check failed".into());
        }
        Ok(())
    };
    verify(&cut)?;
    verify_mesh(&cut.mesh(Default::default())?)?;
    let step = directory.join("native-smoke.step");
    cut.write_step(&step)?;
    let imported =
        Solid::import_rust_step(&std::fs::read_to_string(&step)?, EntityId::new(), policy)?;
    verify(&imported)?;
    std::fs::write(
        directory.join("native-smoke.txt"),
        "MultiCAD Rust; valid solid; Boolean cut; tessellation; STEP round-trip: passed\n",
    )?;
    Ok(())
}

fn verify_mesh(mesh: &Mesh) -> Result<(), Error> {
    if mesh.indices.is_empty()
        || !mesh.indices.as_chunks::<3>().1.is_empty()
        || mesh.vertices.iter().any(|v| !v.is_finite())
        || mesh
            .indices
            .iter()
            .any(|i| *i as usize >= mesh.vertices.len() / 3)
    {
        return Err("Native tessellation smoke check failed".into());
    }
    Ok(())
}
