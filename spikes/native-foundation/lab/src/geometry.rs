use multicad_kernel_proof::{Error, Mesh, Shape};
use serde::{Deserialize, Serialize};
use std::{path::Path, time::Instant};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Definition {
    pub width: f64,
    pub depth: f64,
    pub heights: [f64; 2],
    pub cut_width: f64,
}
impl Default for Definition {
    fn default() -> Self {
        Self {
            width: 120.0,
            depth: 80.0,
            heights: [24.0, 12.0],
            cut_width: 30.0,
        }
    }
}
pub struct Built {
    pub meshes: Vec<Mesh>,
    pub volumes: [f64; 2],
    pub elapsed_ms: f64,
}

pub fn build(definition: &Definition) -> Result<Built, String> {
    let start = Instant::now();
    let [w, d] = crate::solver::rectangle(definition.width, definition.depth)?;
    if !definition.cut_width.is_finite() || definition.cut_width <= 0.0 || definition.cut_width >= w
    {
        return Err("Cut width must be positive and smaller than sketch width".into());
    }
    let mut meshes = Vec::new();
    let mut volumes = [0.0; 2];
    for (index, height) in definition.heights.iter().copied().enumerate() {
        let shape = Shape::extrude(w, d, height).map_err(|e| e.to_string())?;
        let cut = shape
            .cut(w - definition.cut_width, definition.cut_width, d, height)
            .map_err(|e| e.to_string())?;
        if !cut.valid().map_err(|e| e.to_string())? {
            return Err("Invalid solid".into());
        }
        volumes[index] = cut.volume().map_err(|e| e.to_string())?;
        let expected = (w - definition.cut_width) * d * height;
        if (volumes[index] - expected).abs() > expected * 1e-9 {
            return Err("Unexpected cut volume".into());
        }
        let mesh = cut.mesh(0.1).map_err(|e| e.to_string())?;
        if mesh.vertices.iter().any(|v| !v.is_finite())
            || mesh
                .indices
                .iter()
                .any(|i| *i as usize >= mesh.vertices.len() / 3)
            || mesh.indices.is_empty()
        {
            return Err("Invalid display mesh".into());
        }
        let mesh_volume = mesh
            .indices
            .as_chunks::<3>()
            .0
            .iter()
            .map(|triangle| {
                let p: Vec<_> = triangle
                    .iter()
                    .map(|index| {
                        let i = *index as usize * 3;
                        [mesh.vertices[i], mesh.vertices[i + 1], mesh.vertices[i + 2]]
                    })
                    .collect();
                let (a, b, c) = (p[0], p[1], p[2]);
                (a[0] * (b[1] * c[2] - b[2] * c[1])
                    + a[1] * (b[2] * c[0] - b[0] * c[2])
                    + a[2] * (b[0] * c[1] - b[1] * c[0]))
                    / 6.0
            })
            .sum::<f64>();
        if (mesh_volume - expected).abs() > expected * 1e-9 {
            return Err("Display mesh volume/winding does not match exact solid".into());
        }
        meshes.push(mesh);
    }
    Ok(Built {
        meshes,
        volumes,
        elapsed_ms: start.elapsed().as_secs_f64() * 1000.0,
    })
}

pub fn verify(directory: &Path) -> Result<(), Error> {
    std::fs::create_dir_all(directory)?;
    let definition = Definition::default();
    let warm = build(&definition).map_err(|e| -> Error { e.into() })?;
    let mut samples = Vec::new();
    for i in 0..25 {
        let mut edit = definition.clone();
        edit.width += i as f64 * 0.125;
        samples.push(build(&edit).map_err(|e| -> Error { e.into() })?.elapsed_ms);
    }
    let shape = Shape::extrude(120.0, 80.0, 24.0)?.cut(90.0, 30.0, 80.0, 24.0)?;
    let path = directory.join("sketch-cut.step");
    shape.write_step(&path)?;
    let roundtrip = Shape::read_step(&path)?;
    if !roundtrip.valid()? || (roundtrip.volume()? - shape.volume()?).abs() > 1e-5 {
        return Err("STEP round-trip mismatch".into());
    }
    let base = Shape::extrude(100.0, 80.0, 20.0)?;
    let mut alignment = Vec::new();
    for delta in [0.0_f64, 1e-8, 1e-6, 1e-4, -1e-8, -1e-6, -1e-4] {
        let cut = base.cut(100.0 + delta, 10.0, 80.0, 20.0)?;
        let volume = cut.volume()?;
        let expected = 160000.0 - (-delta).max(0.0) * 80.0 * 20.0;
        if !cut.valid()? || (volume - expected).abs() > 2.0 * 1e-7 * 80.0 * 20.0 {
            return Err("Near-contact Boolean exceeds fixture tolerance".into());
        }
        alignment.push(serde_json::json!({"offset_mm":delta,"volume_mm3":volume,"analytic_volume_mm3":expected,"valid":true}));
    }
    if Shape::extrude(f64::NAN, 2.0, 3.0).is_ok() || Shape::extrude(-1.0, 2.0, 3.0).is_ok() {
        return Err("Invalid input accepted".into());
    }
    if shape
        .write_step(&directory.join("missing-directory/failure.step"))
        .is_ok()
    {
        return Err("Export failure was hidden".into());
    }
    samples.sort_by(f64::total_cmp);
    let result = serde_json::json!({"stage":"CAD-49 bounded proof, not production acceptance","occt":"7.9.3","solver":"sketch-solver 0.1.0","ui":"eframe 0.36.2 + egui_dock 0.21.1","platform":std::env::consts::OS,"arch":std::env::consts::ARCH,"shared_profile_independent_solids":2,"triangles":warm.meshes.iter().map(|m|m.indices.len()/3).sum::<usize>(),"volumes_mm3":warm.volumes,"rebuild_samples_ms":samples,"rebuild_median_ms":samples[12],"rebuild_max_ms":samples[24],"alignment":alignment,"step_roundtrip":true});
    std::fs::write(
        directory.join("geometry-report.json"),
        serde_json::to_vec_pretty(&result)?,
    )?;
    println!("{}", serde_json::to_string_pretty(&result)?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn failed_rebuild_does_not_mutate_source_definition() {
        let original = Definition::default();
        let mut bad = original.clone();
        bad.cut_width = 999.0;
        assert!(build(&bad).is_err());
        assert_eq!(original, Definition::default());
        assert!(build(&original).is_ok());
    }
}
