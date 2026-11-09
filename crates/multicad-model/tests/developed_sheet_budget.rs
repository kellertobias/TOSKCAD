use multicad_core::{
    commands::{Actor, Channel, Session},
    model::{self, Cutout, CutoutFace, CutoutShape, length},
    project_file,
};
use multicad_kernel::{
    CurvedEdge2d, CurvedProfile, GeometryErrorKind, GeometryPolicy, PlaneFrame, Solid,
};
use multicad_model::Compiler;

#[test]
fn shipped_c_channel_develops_seventeen_openings_after_two_folded_cutouts() {
    let project = project_file::read(
        &mut include_bytes!("../../../examples/front-panel.tcd").as_slice(),
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap()
    .project;
    let session = Session::new(project.document).unwrap();
    let part = session
        .document()
        .entities()
        .find(|e| e.name == "Front panel sheet")
        .unwrap()
        .id;
    let feature = model::sheet_feature(session.document(), part).unwrap();
    let mut sheet = model::read_sheet(session.document(), feature).unwrap();
    let original_hardware = model::sheet_hardware(session.document(), feature).unwrap();
    sheet.cutouts.extend([
        Cutout {
            face: CutoutFace::Base,
            shape: CutoutShape::Circle {
                center: [length(20.), length(15.)],
                diameter: length(5.),
            },
        },
        Cutout {
            face: CutoutFace::Flange { index: 0 },
            shape: CutoutShape::Rectangle {
                center: [length(45.), length(25.)],
                size: [length(8.), length(6.)],
            },
        },
    ]);
    let prepared = model::prepare_sheet_edit(
        &session,
        session.revision(),
        Actor::local(Channel::Ui),
        feature,
        &sheet,
    )
    .unwrap();
    let candidate = prepared.prepared.document();
    let compiler = Compiler {
        omit_display: true,
        ..Default::default()
    };
    let baseline = compiler
        .compile(session.document(), session.revision())
        .unwrap();
    let baseline = baseline.unfold(session.document(), part).unwrap();
    assert_eq!(
        baseline.cut.loops.iter().filter(|wire| !wire.outer).count(),
        15
    );

    let model = compiler.compile(candidate, session.revision()).unwrap();
    let flat = model.unfold(candidate, part).unwrap();
    assert_eq!(flat.feature, feature);
    assert_eq!(flat.cut.loops.iter().filter(|wire| !wire.outer).count(), 17);
    assert_eq!(flat.bends, baseline.bends);
    assert_eq!(flat.drills.len(), baseline.drills.len() + 1);
    assert!(
        flat.drills
            .iter()
            .any(|drill| drill.center == [20., 15.] && drill.diameter_mm == 5.)
    );
    assert_eq!(
        model::sheet_hardware(candidate, feature).unwrap(),
        original_hardware
    );
    let area_removed = 25. * std::f64::consts::PI / 4. + 8. * 6.;
    assert!((baseline.area_mm2 - flat.area_mm2 - area_removed).abs() < 1e-6);
}

#[test]
fn larger_developed_profile_keeps_exact_overlap_validation_and_the_query_limit() {
    let boundary = [[0., 0.], [200., 0.], [200., 40.], [0., 40.]];
    let mut profile = CurvedProfile {
        frame: PlaneFrame::default(),
        outer: (0..4)
            .map(|i| CurvedEdge2d::Line {
                a: boundary[i],
                b: boundary[(i + 1) % 4],
            })
            .collect(),
        holes: (0..17)
            .map(|i| {
                vec![CurvedEdge2d::Circle {
                    center: [10. + 10. * f64::from(i), 20.],
                    radius_mm: 2.,
                }]
            })
            .collect(),
    };
    let policy = GeometryPolicy::default();
    let displacement = [0., 0., 1.];
    let error = Solid::extrude_curved_profile_vector(&profile, displacement, policy)
        .err()
        .unwrap();
    assert_eq!(error.kind, GeometryErrorKind::Limit);
    assert!(error.message.contains("16-hole query budget"));
    assert_eq!(
        profile.area_mm2(policy).unwrap_err().kind,
        GeometryErrorKind::Limit
    );
    let developed = Solid::extrude_developed_sheet_profile(&profile, displacement, policy).unwrap();
    assert_eq!(developed.solid_count().unwrap(), 1);
    let expected = 200. * 40. - 17. * 4. * std::f64::consts::PI;
    assert!((developed.volume_mm3().unwrap() - expected).abs() < 1e-6);
    profile.holes[16] = profile.holes[15].clone();
    let error = Solid::extrude_developed_sheet_profile(&profile, displacement, policy)
        .err()
        .unwrap();
    assert!(error.message.contains("Overlapping"), "{error}");
}
