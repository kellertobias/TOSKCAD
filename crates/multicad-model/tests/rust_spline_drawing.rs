use multicad_core::{
    commands::{Actor, Channel, Session},
    document::{Document, EntityId},
    drawings::{
        self, DrawingView, Page, Scale, VectorCurve, VectorRole, ViewEvaluation, ViewKind,
        ViewSource,
    },
    model::imports::prepare_step_import,
    project_file::{self, ProjectFile},
    references::ReferenceFrame,
};
use multicad_kernel::{
    DrawingProjection, DrawingSolid, GeometryErrorKind, GeometryPolicy, PlaneFrame, Solid,
    project_drawing,
    rust_kernel::{
        spur_gear::{Profile, Spec},
        step::write_spur_gear,
        step_read::read_analytic_solid,
    },
};
use multicad_model::{
    Compiler, DrawingProviders, GeometryBackend, imports::read_step_file_with_backend,
};
use std::f64::consts::{FRAC_1_SQRT_2, PI, TAU};

const PLACEMENT: PlaneFrame = PlaneFrame {
    origin: [11., -7., 23.],
    u: [FRAC_1_SQRT_2, 0., FRAC_1_SQRT_2],
    v: [0., 1., 0.],
};
const CHORD_MM: f64 = 0.01;
const TOL: f64 = 1e-7;

fn step_fixture() -> String {
    let pressure = 20_f64.to_radians();
    let profile = Profile::new(
        Spec {
            teeth: 20,
            pitch_radius_mm: 20.,
            base_radius_mm: 20. * pressure.cos(),
            tip_radius_mm: 22.,
            root_radius_mm: 17.5,
            root_fillet_radius_mm: 0.4,
            half_base_angle_rad: PI / 40. + pressure.tan() - pressure - 0.05 / 40.,
            face_width_mm: 8.,
            bore_radius_mm: 2.,
        },
        1e-5,
        1e7,
    )
    .unwrap();
    write_spur_gear(&profile, PLACEMENT.origin, PLACEMENT.u, PLACEMENT.v, TOL).unwrap()
}

fn view_frame(front: bool) -> PlaneFrame {
    PlaneFrame {
        u: PLACEMENT.v,
        v: if front {
            PLACEMENT.u.map(|v| -v)
        } else {
            PLACEMENT.u
        },
        ..PLACEMENT
    }
}

fn point(frame: PlaneFrame, p: [f64; 3]) -> [f64; 2] {
    [frame.u, frame.v].map(|axis| {
        axis.into_iter()
            .enumerate()
            .map(|(i, u)| u * (p[i] - frame.origin[i]))
            .sum()
    })
}

fn distance(point: [f64; 2], [a, b]: [[f64; 2]; 2]) -> f64 {
    let dot = |a: [f64; 2], b: [f64; 2]| a[0] * b[0] + a[1] * b[1];
    let delta = [b[0] - a[0], b[1] - a[1]];
    let offset = [point[0] - a[0], point[1] - a[1]];
    let t = (dot(offset, delta) / dot(delta, delta)).clamp(0., 1.);
    (offset[0] - t * delta[0]).hypot(offset[1] - t * delta[1])
}

fn near(a: [f64; 2], b: [f64; 2]) -> bool {
    (a[0] - b[0]).hypot(a[1] - b[1]) < 1e-9
}

#[test]
fn saved_imported_step_drawing_projects_exact_splines_from_both_ends_without_meshes() {
    let source = step_fixture();
    let analytic = read_analytic_solid(&source, TOL).unwrap();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("placed-spur.step");
    std::fs::write(&path, source).unwrap();
    let file = read_step_file_with_backend(&path, GeometryBackend::Rust).unwrap();
    let mut session = Session::new(Document::new("Imported spur drawing").unwrap()).unwrap();
    let actor = Actor::local(Channel::Ui);
    let import = prepare_step_import(&session, 0, actor.clone(), &file).unwrap();
    let part = import.model.outputs[0].body;
    session.commit(import.model.prepared).unwrap();

    let mut page = Page::new("1");
    let views = [true, false].map(|front| {
        let frame = view_frame(front);
        let id = EntityId::new();
        page.views.push(DrawingView {
            id,
            name: if front { "Front" } else { "Rear" }.into(),
            source: ViewSource::Model {
                entities: vec![part],
            },
            frame: ReferenceFrame {
                origin_mm: frame.origin,
                u: frame.u,
                v: frame.v,
            },
            kind: ViewKind::Projection,
            position_mm: [if front { 70. } else { 160. }, 70.],
            scale: Scale {
                numerator: 1,
                denominator: 1,
            },
            hidden_lines: true,
            tangent_edges: true,
            paper_chord_tolerance_mm: CHORD_MM,
            guide: None,
            equipment_categories: None,
        });
        (id, frame)
    });
    let (_, prepared) =
        drawings::prepare_create(&session, session.revision(), actor, "Plans", Some(page)).unwrap();
    session.commit(prepared).unwrap();

    let project = ProjectFile::new(session.document().clone()).unwrap();
    let mut bytes = Vec::new();
    project_file::write(&project, &mut bytes, Default::default()).unwrap();
    std::fs::remove_file(path).unwrap();
    let reopened = project_file::read(
        &mut bytes.as_slice(),
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap()
    .project
    .document;
    let model = Compiler {
        omit_display: true,
        ..Default::default()
    }
    .compile_with_backend(&reopened, 0, GeometryBackend::Rust, None)
    .unwrap();
    assert!(
        model
            .snapshot()
            .shapes
            .values()
            .all(|shape| shape.mesh.indices.is_empty())
    );
    let evaluation = model
        .drawings(&reopened, 0, &DrawingProviders::default())
        .unwrap();
    for (id, frame) in views {
        let ViewEvaluation::Ready { vectors, .. } = &evaluation.views[&id] else {
            panic!("view must evaluate: {:?}", evaluation.views[&id]);
        };
        assert_eq!(vectors.len(), 161);
        assert!(vectors.iter().all(|v| {
            v.source == part && v.material_part == part && v.role == VectorRole::Visible
        }));
        let mut cubics = 0;
        let mut bores = 0;
        for vector in vectors {
            match &vector.curve {
                VectorCurve::BSpline {
                    degree,
                    periodic,
                    poles,
                    weights,
                    knots,
                    multiplicities,
                    first,
                    last,
                } => {
                    cubics += 1;
                    assert_eq!(*degree, 3);
                    assert!(!periodic);
                    assert!(weights.iter().all(|w| *w == 1.));
                    let curve = analytic
                        .splines
                        .iter()
                        .find(|curve| {
                            poles.len() == curve.poles_mm().len()
                                && poles
                                    .iter()
                                    .zip(curve.poles_mm())
                                    .all(|(&a, &b)| near(a, point(frame, b)))
                        })
                        .unwrap();
                    assert_eq!(knots, curve.knots());
                    assert_eq!(
                        *multiplicities,
                        curve
                            .multiplicities()
                            .iter()
                            .copied()
                            .map(u32::from)
                            .collect::<Vec<_>>()
                    );
                    assert_eq!([*first, *last], [knots[0], knots[knots.len() - 1]]);
                    assert!(near(vector.preview_mm[0], poles[0]));
                    assert!(near(
                        *vector.preview_mm.last().unwrap(),
                        *poles.last().unwrap()
                    ));
                    for span in curve.bezier_spans() {
                        for t in [0.25_f64, 0.5, 0.75] {
                            let s = 1. - t;
                            let weights = [s.powi(3), 3. * s * s * t, 3. * s * t * t, t.powi(3)];
                            let p = std::array::from_fn(|i| {
                                span.iter()
                                    .zip(weights)
                                    .map(|(pole, weight)| pole[i] * weight)
                                    .sum()
                            });
                            let sample = point(frame, p);
                            assert!(
                                vector
                                    .preview_mm
                                    .windows(2)
                                    .any(|pair| distance(sample, [pair[0], pair[1]])
                                        <= CHORD_MM + 1e-9)
                            );
                        }
                    }
                }
                VectorCurve::Ellipse {
                    center,
                    major_mm,
                    minor_mm,
                    start_rad,
                    end_rad,
                    ..
                } if (*end_rad - *start_rad - TAU).abs() < 1e-9 => {
                    bores += 1;
                    assert!(near(*center, [0.; 2]));
                    assert_eq!([*major_mm, *minor_mm], [2.; 2]);
                }
                _ => {}
            }
        }
        assert_eq!(cubics, 40);
        assert_eq!(bores, 1);
    }
}

#[test]
fn spline_projection_keeps_budgets_and_refuses_unproved_visibility() {
    let source = step_fixture();
    let solid =
        Solid::import_rust_step(&source, EntityId::new(), GeometryPolicy::default()).unwrap();
    let inputs = [DrawingSolid {
        solid: &solid,
        hatch: None,
    }];
    let options = DrawingProjection {
        frame: view_frame(true),
        ..Default::default()
    };
    let vectors = project_drawing(&inputs, options).unwrap();
    let point_count = vectors
        .iter()
        .map(|vector| vector.preview_mm.len() as u32)
        .sum::<u32>();
    assert_eq!(
        vectors,
        project_drawing(
            &inputs,
            DrawingProjection {
                hidden_lines: true,
                ..options
            }
        )
        .unwrap(),
    );
    for limited in [
        DrawingProjection {
            maximum_curves: vectors.len() as u32 - 1,
            ..options
        },
        DrawingProjection {
            maximum_points: point_count - 1,
            ..options
        },
        DrawingProjection {
            maximum_points: 1,
            ..options
        },
    ] {
        assert_eq!(
            project_drawing(&inputs, limited).unwrap_err().kind,
            GeometryErrorKind::Limit
        );
    }
    for unsupported in [
        DrawingProjection {
            frame: PlaneFrame::default(),
            ..options
        },
        DrawingProjection {
            detail: Some(([0.; 2], 4.)),
            ..options
        },
        DrawingProjection {
            section_offset_mm: Some(2.),
            ..options
        },
    ] {
        let error = project_drawing(&inputs, unsupported).unwrap_err();
        assert_eq!(error.kind, GeometryErrorKind::Unsupported);
        assert!(error.message.contains("uncropped axial"));
    }
    let repeated = [
        DrawingSolid {
            solid: &solid,
            hatch: None,
        },
        DrawingSolid {
            solid: &solid,
            hatch: None,
        },
    ];
    let error = project_drawing(&repeated, options).unwrap_err();
    assert_eq!(error.kind, GeometryErrorKind::Unsupported);
    assert!(error.message.contains("scene occlusion"));
}
