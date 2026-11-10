use multicad_core::{document::EntityId, project_file};
use multicad_kernel::Solid;
use multicad_model::{Compiler, GeometryBackend, export::verify_rust_step_shape};

#[test]
fn saved_toolcase_crops_the_exact_finished_drawer_to_its_front_coupon() {
    let document = project_file::read(
        &mut include_bytes!("../../../examples/toolcase.tcd").as_slice(),
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap()
    .project
    .document;
    let feature = EntityId::parse("771b0fdf-af00-48ca-81c1-04416f269879").unwrap();
    let crop_tool = EntityId::parse("58a01de2-71dc-4147-9b6c-8d35185600d4").unwrap();
    let compiler = Compiler {
        omit_display: true,
        ..Compiler::default()
    };
    let [rust, native] = [GeometryBackend::Rust, GeometryBackend::OpenCascade].map(|backend| {
        compiler
            .compile_sources_with_backend(&document, 0, &[feature], backend, None)
            .unwrap_or_else(|error| panic!("{backend:?}: {error}"))
    });
    let [actual, expected] = [&rust, &native].map(|model| model.solid(feature).unwrap());
    assert!(actual.uses_rust_geometry());
    let near = |a: f64, b: f64| assert!((a - b).abs() < 1e-7 * b.abs().max(1.), "{a} != {b}");
    let area = |solid: &Solid| {
        solid
            .faces()
            .unwrap()
            .iter()
            .map(|face| face.area_mm2)
            .sum()
    };
    let expected_bounds = expected.bounds().unwrap();
    near(expected_bounds.max[1], 30.);
    assert!(
        actual
            .topology_snapshot(feature, 0)
            .unwrap()
            .elements
            .iter()
            .any(|element| element
                .origins
                .iter()
                .any(|origin| origin.source == crop_tool))
    );
    assert!(
        !actual
            .manufacturing_mesh_with_receipt(Default::default())
            .unwrap()
            .mesh
            .indices
            .is_empty()
    );
    let scratch = tempfile::tempdir().unwrap();
    let path = scratch.path().join("drawer-coupon.step");
    actual.write_step(&path).unwrap();
    let independent = Solid::read_step(&path, compiler.policy).unwrap();
    for solid in [actual, &independent] {
        assert_eq!(solid.solid_count().unwrap(), 1);
        near(solid.volume_mm3().unwrap(), expected.volume_mm3().unwrap());
        near(area(solid), area(expected));
        let bounds = solid.bounds().unwrap();
        for i in 0..3 {
            near(bounds.min[i], expected_bounds.min[i]);
            near(bounds.max[i], expected_bounds.max[i]);
        }
    }
    let receipt = verify_rust_step_shape(
        &path,
        &rust.snapshot().shapes[&feature],
        compiler.policy.linear_tolerance_mm,
    )
    .unwrap();
    near(receipt, actual.volume_mm3().unwrap());
}
#[test]
fn aligned_box_intersection_handles_multiple_sides_and_empty_results() {
    use multicad_core::{
        commands::{Actor, Channel, Session},
        model::{self, PrimitiveFrame, PrimitiveParameters, PrimitiveRequest, length},
    };
    use multicad_kernel::GeometryPolicy;

    let feature = EntityId::parse("737d2801-98a7-47dc-8bb8-b544e7055f51").unwrap();
    let output = EntityId::parse("771b0fdf-af00-48ca-81c1-04416f269879").unwrap();
    let document = project_file::read(
        &mut include_bytes!("../../../examples/toolcase.tcd").as_slice(),
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap()
    .project
    .document;
    let mut session = Session::new(document).unwrap();
    let mut stock_box = |name, origin: [f64; 3], size: [f64; 3]| {
        let request = PrimitiveRequest::new(
            name,
            PrimitiveParameters::Box {
                size: size.map(length),
                frame: PrimitiveFrame {
                    origin: origin.map(length),
                    u: [1., 0., 0.],
                    v: [0., 1., 0.],
                },
            },
        );
        let prepared = model::prepare_primitive(
            &session,
            session.revision(),
            Actor::local(Channel::Ui),
            &request,
        )
        .unwrap();
        session.commit(prepared.prepared).unwrap();
        request.feature
    };
    let containing = stock_box("Containing", [220., -2., 110.], [120., 120., 60.]);
    let disjoint = stock_box("Disjoint", [500., 0., 110.], [20., 30., 60.]);
    let narrow = stock_box("Narrow", [230., -1., 110.], [96., 31., 60.]);
    let compiler = Compiler {
        omit_display: true,
        ..Compiler::default()
    };
    let model = compiler
        .compile_sources_with_backend(
            session.document(),
            session.revision(),
            &[feature, containing, disjoint, narrow],
            GeometryBackend::Rust,
            None,
        )
        .unwrap();
    let [body, containing, disjoint, narrow] =
        [feature, containing, disjoint, narrow].map(|id| model.solid(id).unwrap());
    let policy = GeometryPolicy::default();
    let same = Solid::intersect_warped_box(&[body, containing], output, policy)
        .unwrap()
        .unwrap();
    assert_eq!(same.volume_mm3().unwrap(), body.volume_mm3().unwrap());

    assert!(
        Solid::intersect_warped_box(&[disjoint, body], output, policy)
            .unwrap()
            .is_err()
    );

    let sliced = Solid::intersect_warped_box(&[body, narrow], output, policy)
        .unwrap()
        .unwrap();
    let bounds = sliced.bounds().unwrap();
    for (actual, expected) in [
        (bounds.min[0], 230.),
        (bounds.max[0], 326.),
        (bounds.max[1], 30.),
    ] {
        assert!((actual - expected).abs() < 1e-7);
    }
    assert!(
        sliced
            .manufacturing_mesh_with_receipt(Default::default())
            .is_ok()
    );
}
