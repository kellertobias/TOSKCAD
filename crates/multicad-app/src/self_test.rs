//! A headless packaged-runtime smoke check, not a CAD workspace acceptance test.
use multicad_kernel::{Error, Shape};
use std::path::Path;

pub fn run(directory: &Path) -> Result<(), Error> {
    std::fs::create_dir_all(directory)?;
    let [width, depth] = multicad_solver::rectangle(120.0, 80.0)?;
    let solid = Shape::extrude(width, depth, 24.0)?.cut(90.0, 30.0, 80.0, 24.0)?;
    let expected = 172800.0;
    if !solid.valid()? || (solid.volume()? - expected).abs() > 1e-5 {
        return Err("Native solid validity/volume smoke check failed".into());
    }
    let mesh = solid.mesh(0.1)?;
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
    crate::modeling::smoke(directory).map_err(|e| -> Error { e.into() })?;
    println!("Native runtime smoke check passed");
    Ok(())
}
