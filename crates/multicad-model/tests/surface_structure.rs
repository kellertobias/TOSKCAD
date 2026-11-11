//! Native face geometry, actual relief volumes and refusal before publication.
#[allow(dead_code)]
mod common;

use common::{SharedProfiles, actor};
use multicad_core::{
    commands::Session,
    document::{Document, EntityId},
    model::{
        self, BodyInput, BooleanRequest, Operation, PrimitiveFrame, PrimitiveParameters,
        PrimitiveRequest, SurfaceStructureDirection, SurfaceStructureParameters,
        SurfaceStructurePattern, SurfaceStructureRequest, angle, length,
    },
    project_file::{self, ProjectFile},
    references::{GeometryIntent, ReferenceFrame},
};
use multicad_kernel::{BooleanOperation, GeometryPolicy, RigidTransform, Solid};
use multicad_model::{Compiler, Model};
use std::f64::consts::PI;

fn compiler() -> Compiler {
    Compiler {
        omit_display: true,
        ..Default::default()
    }
}

fn native(session: &Session) -> Model {
    compiler()
        .compile(session.document(), session.revision())
        .unwrap()
}

fn near(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-6, "{a} != {b}");
}

fn stock(session: &mut Session, size: [f64; 3], origin: [f64; 3]) -> BodyInput {
    let request = PrimitiveRequest::new(
        "Coupon stock",
        PrimitiveParameters::Box {
            size: size.map(length),
            frame: PrimitiveFrame {
                origin: origin.map(length),
                ..Default::default()
            },
        },
    );
    let prepared =
        model::prepare_primitive(session, session.revision(), actor(), &request).unwrap();
    session.commit(prepared.prepared).unwrap();
    BodyInput {
        body: request.body,
        producer: request.feature,
    }
}

fn face(model: &Model, input: &BodyInput, axis: [f64; 3]) -> (GeometryIntent, ReferenceFrame) {
    let shape = &model.snapshot().shapes[&input.producer];
    let face = model
        .solid(input.producer)
        .unwrap()
        .faces()
        .unwrap()
        .into_iter()
        .filter(|f| {
            f.plane.is_some_and(|(_, n)| {
                n.into_iter().zip(axis).map(|(a, b)| a * b).sum::<f64>() > 1. - 1e-8
            })
        })
        .max_by(|a, b| {
            let support = |p: [f64; 3]| p.into_iter().zip(axis).map(|(p, n)| p * n).sum::<f64>();
            support(a.center_mm).total_cmp(&support(b.center_mm))
        })
        .unwrap();
    let element = shape
        .topology
        .elements
        .iter()
        .find(|element| {
            element.key.kind == multicad_core::references::TopologyKind::Face
                && element.key.ordinal == face.index
        })
        .unwrap();
    let capture = model::capture_profile_face(&shape.topology, element.key, None).unwrap();
    (capture.intent, element.frames[0].frame)
}

fn candidate(
    session: &Session,
    request: &SurfaceStructureRequest,
) -> Result<Model, multicad_model::ModelError> {
    let prepared =
        model::prepare_surface_structure(session, session.revision(), actor(), request).unwrap();
    compiler().compile(prepared.prepared.document(), session.revision() + 1)
}

fn material(solid: &Solid, center: [f64; 3]) -> bool {
    match solid.rust_normal_relief_contains(center) {
        Ok(material) => return material,
        Err(cause) if cause.kind == multicad_kernel::GeometryErrorKind::Unsupported => {}
        Err(cause) => panic!("{cause}"),
    }
    let policy = GeometryPolicy::default();
    let size = 0.02;
    let probe = Solid::box_solid([size; 3], policy)
        .unwrap()
        .transformed(RigidTransform {
            translation_mm: center.map(|c| c - size / 2.),
            ..Default::default()
        })
        .unwrap();
    solid
        .boolean(&probe, BooleanOperation::Intersect, policy)
        .unwrap()
        .is_some()
}

#[test]
fn raised_builtin_patterns_change_native_material_and_keep_the_part_identity() {
    use SurfaceStructurePattern::{Ribs, Stipple, Waves};
    let mut session = Session::new(Document::new("Native relief coupons").unwrap()).unwrap();
    let input = stock(&mut session, [30., 20., 4.], [7., 11., 13.]);
    let original = session.document().clone();
    let (top, frame) = face(&native(&session), &input, [0., 0., 1.]);
    let mut request = SurfaceStructureRequest {
        feature: EntityId::new(),
        input,
        faces: vec![top],
        parameters: SurfaceStructureParameters {
            amplitude: length(0.6),
            pitch: length(4.),
            size: [length(16.), length(8.)],
            offset: [length(1.), length(-1.)],
            rotation: angle(30.),
            ..Default::default()
        },
    };
    for (pattern, relief_area) in [
        (Ribs, 2. * 16. * 1.8),
        (Waves, 2. * 16. * 1.2),
        (Stipple, 8. * PI * 0.88_f64.powi(2)),
    ] {
        request.parameters.pattern = pattern;
        let model = candidate(&session, &request)
            .unwrap_or_else(|e| panic!("{:?}: {e}", request.parameters.pattern));
        assert_eq!(session.document(), &original);
        assert_eq!(
            model.snapshot().bodies[&request.input.body],
            request.feature
        );
        let result = model.solid(request.feature).unwrap();
        assert_eq!(result.solid_count().unwrap(), 1);
        near(
            result.volume_mm3().unwrap(),
            30. * 20. * 4. + 0.6 * relief_area,
        );
        let bounds = result.bounds().unwrap();
        for (actual, expected) in bounds
            .min
            .into_iter()
            .chain(bounds.max)
            .zip([7., 11., 13., 37., 31., 17.6])
        {
            near(actual, expected);
        }
        assert!(!material(result, [7.1, 11.1, 17.3]));
        if matches!(request.parameters.pattern, Ribs) {
            let half_sqrt_three = 3_f64.sqrt() / 2.;
            let u = 7. * half_sqrt_three;
            let v = 2.5 + 2. * half_sqrt_three;
            let v_sign = if frame.normal()[2] < 0. { -1. } else { 1. };
            let point = std::array::from_fn(|i| {
                [22., 21., 17.3][i] + u * frame.u[i] + v_sign * v * frame.v[i]
            });
            // The rotated and translated rib contains this point. Its V
            // coordinate is outside the original, unrotated window.
            assert!(material(result, point));
        }
    }
    let prepared =
        model::prepare_surface_structure(&session, session.revision(), actor(), &request).unwrap();
    session.commit(prepared.prepared).unwrap();
    let committed = session.document().clone();
    session.undo(session.revision(), actor()).unwrap();
    assert_eq!(session.document(), &original);
    session.redo(session.revision(), actor()).unwrap();
    assert_eq!(session.document(), &committed);
}

#[test]
fn recessed_height_map_preserves_tones_and_refuses_breakthrough_or_internal_voids() {
    let mut session = Session::new(Document::new("Stepped grayscale relief").unwrap()).unwrap();
    let input = stock(&mut session, [30., 20., 4.], [0.; 3]);
    let (top, frame) = face(&native(&session), &input, [0., 0., 1.]);
    let mut request = SurfaceStructureRequest {
        feature: EntityId::new(),
        input,
        faces: vec![top],
        parameters: SurfaceStructureParameters {
            pattern: SurfaceStructurePattern::HeightMap {
                width: 2,
                height: 2,
                samples: vec![255, 128, 0, 64],
                repeat: [1, 1],
            },
            direction: SurfaceStructureDirection::Recessed,
            amplitude: length(1.),
            size: [length(12.), length(8.)],
            ..Default::default()
        },
    };
    let model = candidate(&session, &request)
        .unwrap_or_else(|e| panic!("{:?}: {e}", request.parameters.pattern));
    let solid = model.solid(request.feature).unwrap();
    near(
        solid.volume_mm3().unwrap(),
        30. * 20. * 4. - 5.4 * 3.6 * (447. / 255.),
    );
    // Positive V is the top image row. Sample inside each tile at a depth
    // which distinguishes its native cut from the next nonzero gray tone.
    let up = if frame.normal()[2] < 0. { -1. } else { 1. };
    let point = |u: f64, v: f64, depth: f64| {
        let mut p =
            std::array::from_fn(|i| [15., 10., 4.][i] + u * frame.u[i] + up * v * frame.v[i]);
        p[2] -= depth;
        p
    };
    assert!(!material(solid, point(-3., 2., 0.75)));
    assert!(material(solid, point(3., 2., 0.75)));
    assert!(!material(solid, point(3., 2., 0.40)));
    assert!(material(solid, point(-3., -2., 0.10)));
    assert!(!material(solid, point(3., -2., 0.10)));
    assert!(material(solid, point(3., -2., 0.40)));

    let before = session.document().clone();
    request.parameters.amplitude = length(3.95);
    let problem = candidate(&session, &request).err().unwrap();
    assert!(problem.message.contains("break through"), "{problem}");
    assert_eq!(session.document(), &before);

    request.parameters.amplitude = length(1.);
    let prepared =
        model::prepare_surface_structure(&session, session.revision(), actor(), &request).unwrap();
    session.commit(prepared.prepared).unwrap();
    let file = ProjectFile::new(session.document().clone()).unwrap();
    let mut bytes = vec![];
    project_file::write(&file, &mut bytes, Default::default()).unwrap();
    let reopened = project_file::read(
        &mut bytes.as_slice(),
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap()
    .project;
    assert_eq!(
        model::read_surface_structure(&reopened.document, request.feature).unwrap(),
        request
    );

    // An enclosed cavity has stock both before the first cut and beneath its
    // nominal floor, so entry/floor-only sampling would give a false pass.
    let mut void_session = Session::new(Document::new("Internal void coupon").unwrap()).unwrap();
    let base = stock(&mut void_session, [30., 20., 4.], [0.; 3]);
    let cutter = stock(&mut void_session, [4., 4., 0.4], [13., 8., 3.2]);
    let cut = BooleanRequest::new(Operation::Cut, base, vec![cutter]);
    let draft = model::prepare_booleans(
        &void_session,
        void_session.revision(),
        actor(),
        std::slice::from_ref(&cut),
    )
    .unwrap();
    void_session.commit(draft.prepared).unwrap();
    let input = BodyInput {
        body: cut.target.body,
        producer: cut.feature,
    };
    let current = native(&void_session);
    let (top, _) = face(&current, &input, [0., 0., 1.]);
    let refused = SurfaceStructureRequest {
        feature: EntityId::new(),
        input,
        faces: vec![top],
        parameters: request.parameters,
    };
    let problem = candidate(&void_session, &refused).err().unwrap();
    assert!(problem.message.contains("internal void"), "{problem}");
}

#[test]
fn exact_face_openings_and_adjacent_swept_windows_are_respected() {
    let mut session = Session::new(Document::new("Adjacent faces").unwrap()).unwrap();
    let input = stock(&mut session, [20.; 3], [0.; 3]);
    let original = native(&session);
    let (top, _) = face(&original, &input, [0., 0., 1.]);
    let (side, _) = face(&original, &input, [1., 0., 0.]);
    let mut request = SurfaceStructureRequest {
        feature: EntityId::new(),
        input,
        faces: vec![top, side],
        parameters: SurfaceStructureParameters {
            direction: SurfaceStructureDirection::Recessed,
            amplitude: length(1.),
            pitch: length(2.),
            size: [length(6.), length(6.)],
            ..Default::default()
        },
    };
    let model = candidate(&session, &request)
        .unwrap_or_else(|e| panic!("{:?}: {e}", request.parameters.pattern));
    near(
        model.solid(request.feature).unwrap().volume_mm3().unwrap(),
        8000. - 2. * 3. * 6. * 0.9,
    );
    request.parameters.size = [length(14.), length(14.)];
    request.parameters.amplitude = length(4.);
    let problem = candidate(&session, &request).err().unwrap();
    assert!(problem.message.contains("windows overlap"), "{problem}");
    request.parameters.size = [length(18.), length(18.)];
    request.parameters.pitch = length(0.5);
    request.parameters.pattern = SurfaceStructurePattern::Stipple;
    let problem = candidate(&session, &request).err().unwrap();
    assert!(problem.message.contains("192-motif"), "{problem}");

    let holed = SharedProfiles::new(Default::default());
    let input = BodyInput {
        body: holed.outer().body,
        producer: holed.outer().feature,
    };
    let (top, frame) = face(&native(&holed.session), &input, [0., 0., 1.]);
    let desired_center = [20., 20., 0.];
    let baseline = [40., 30., 0.];
    let v_sign = if frame.normal()[2] > 0. { 1. } else { -1. };
    let delta: [f64; 3] = std::array::from_fn(|i| desired_center[i] - baseline[i]);
    let offset = |axis: [f64; 3]| delta.into_iter().zip(axis).map(|(a, b)| a * b).sum::<f64>();
    let mut request = SurfaceStructureRequest {
        feature: EntityId::new(),
        input,
        faces: vec![top],
        parameters: SurfaceStructureParameters {
            size: [length(8.), length(8.)],
            offset: [length(offset(frame.u)), length(v_sign * offset(frame.v))],
            ..Default::default()
        },
    };
    let problem = candidate(&holed.session, &request).err().unwrap();
    assert!(
        problem.message.contains("boundary or an opening"),
        "{problem}"
    );
    request.parameters.offset = [length(0.), length(0.)];
    candidate(&holed.session, &request).unwrap();
}

#[test]
fn a_dense_relief_candidate_can_be_superseded_without_publishing() {
    use std::cell::Cell;
    let mut session = Session::new(Document::new("Interrupted relief").unwrap()).unwrap();
    let input = stock(&mut session, [30., 30., 4.], [0.; 3]);
    let checks = Cell::new(0);
    let baseline = compiler()
        .compile_interruptible(
            session.document(),
            session.revision(),
            Some(&|| {
                checks.set(checks.get() + 1);
                false
            }),
        )
        .unwrap();
    let base_checkpoints = checks.get();
    let (top, _) = face(&baseline, &input, [0., 0., 1.]);
    let request = SurfaceStructureRequest {
        feature: EntityId::new(),
        input,
        faces: vec![top],
        parameters: SurfaceStructureParameters {
            pattern: SurfaceStructurePattern::Stipple,
            size: [length(12.), length(12.)],
            pitch: length(1.),
            ..Default::default()
        },
    };
    let original = session.document().clone();
    let prepared =
        model::prepare_surface_structure(&session, session.revision(), actor(), &request).unwrap();
    checks.set(0);
    let error = compiler()
        .compile_interruptible(
            prepared.prepared.document(),
            session.revision() + 1,
            Some(&|| {
                checks.set(checks.get() + 1);
                checks.get() > base_checkpoints + 16
            }),
        )
        .err()
        .expect("dense native build observes the cancellation checkpoint");
    assert!(multicad_model::is_superseded(&error), "{error}");
    assert!(checks.get() > base_checkpoints);
    assert_eq!(session.document(), &original);
    assert_eq!(session.revision(), 1);
}
