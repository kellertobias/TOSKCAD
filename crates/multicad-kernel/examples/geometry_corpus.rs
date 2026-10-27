//! Reproducible local/native-runner evidence, separate from UI acceptance.
use multicad_kernel::*;
use serde_json::json;
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let directory = std::env::args_os()
        .nth(1)
        .ok_or("Pass an output directory")?;
    let directory = std::path::PathBuf::from(directory);
    std::fs::create_dir_all(&directory)?;
    let policy = GeometryPolicy::default();
    let mut cases = vec![];
    for width in [10., 10.1, 25., 100.] {
        for dz in [0., 1e-9, -1e-9, 1e-7, -1e-7, 1e-5, -1e-5] {
            let source = Solid::box_solid([width, 10., 10.], policy)?;
            let tool =
                Solid::box_solid([width, 10., 10.], policy)?.transformed(RigidTransform {
                    translation_mm: [width * 0.5, 0., dz],
                    ..Default::default()
                })?;
            let result = match source.boolean(&tool, BooleanOperation::Subtract, policy) {
                Ok(Some(solid)) => {
                    let volume = solid.volume_mm3()?;
                    let expected = width * 50. + width * 5. * dz.abs();
                    if (volume - expected).abs() > 1e-4 {
                        return Err(
                            "Near-coplanar result exceeds volume regression tolerance".into()
                        );
                    }
                    json!({"status":"valid","volume_mm3":volume,"expected_mm3":expected,"solid_count":solid.solid_count()?,"faces":solid.faces()?.len()})
                }
                Ok(None) => {
                    return Err(
                        "Near-coplanar subtraction unexpectedly removed all material".into(),
                    );
                }
                Err(e) => {
                    json!({"status":"explicit_error","operation":e.operation,"kind":format!("{:?}",e.kind),"message":e.message})
                }
            };
            cases.push(json!({"width_mm":width,"offset_z_mm":dz,"result":result}));
        }
    }
    let mut profile = Profile::rectangle(100., 60.);
    profile.holes.push(vec![ProfileEdge::Circle {
        center: [50., 30.],
        radius_mm: 10.,
    }]);
    let solid = Solid::extrude_profile(&profile, 18., policy)?;
    let before = solid.volume_mm3()?;
    let mesh = solid.mesh(MeshOptions::default())?;
    let path = directory.join("panel-with-hole.step");
    solid.write_step(&path)?;
    let reopened = Solid::read_step(&path, policy)?;
    if (reopened.volume_mm3()? - before).abs() > 1e-5 {
        return Err("STEP round trip changed the fixture volume".into());
    }
    let report = json!({"occt_source_identity":source_identity(),"target_os":std::env::consts::OS,"target_arch":std::env::consts::ARCH,"policy":{"linear_tolerance_mm":policy.linear_tolerance_mm,"boolean_fuzzy_mm":policy.boolean_fuzzy_mm,"minimum_feature_mm":policy.minimum_feature_mm,"maximum_coordinate_mm":policy.maximum_coordinate_mm},"near_coplanar_cases":cases,"exchange_fixture":{"file":"panel-with-hole.step","volume_mm3":before,"reimported_volume_mm3":reopened.volume_mm3()?,"triangles":mesh.indices.len()/3,"volume_after_meshing_mm3":solid.volume_mm3()?}});
    std::fs::write(
        directory.join("geometry-report.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    Ok(())
}
