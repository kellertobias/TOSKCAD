//! Native intersecting-axis geometry and canonical bevel pair transactions.
use multicad_core::{
    commands::{Actor, Channel, Session},
    constraint_sketch::Dimension,
    document::Document,
    model::*,
    project_file::{self, ProjectFile},
};
use multicad_kernel::{BRepSelection, GeometryPolicy, Solid};
use multicad_model::{Compiler, EvaluationCache};
fn actor() -> Actor {
    Actor::local(Channel::Mcp)
}
fn compiler() -> Compiler {
    Compiler {
        omit_display: true,
        ..Default::default()
    }
}
fn count(value: f64) -> Dimension {
    Dimension::Literal {
        value,
        unit: "count".into(),
    }
}
fn recipes() -> [GearParameters; 2] {
    std::array::from_fn(|i| GearParameters {
        family: GearFamily::Bevel,
        module: length(1.),
        teeth: count(if i == 0 { 24. } else { 32. }),
        face_width: length(2.),
        bore_diameter: length(2.),
        bevel: Some(BevelDimensions {
            tooth_system: BevelToothSystem::SphericalInvoluteStraight,
            mate_teeth: count(if i == 0 { 32. } else { 24. }),
            shaft_angle: angle(90.),
        }),
        // Deliberately unrelated definition frames. Assembly placement must
        // remove these before applying the common pitch apex and shaft axes.
        frame: PrimitiveFrame {
            origin: [length(i as f64 * 7.), length(3.), length(-4.)],
            u: [0., 1., 0.],
            v: [0., 0., 1.],
        },
        ..Default::default()
    })
}
fn request() -> GearPairRequest {
    let members = recipes().map(|p| GearRequest::new("Bevel", p));
    let mut request = GearPairRequest::new("Intersecting shafts", members);
    request.parameters.frame.origin = [length(30.), length(40.), length(50.)];
    request
}
fn near(a: f64, b: f64, tolerance: f64) {
    assert!(
        (a - b).abs() <= tolerance,
        "{a} != {b}, tolerance {tolerance}"
    );
}
#[test]
fn native_bevel_pair_has_intersecting_axes_engagement_and_two_step_leaves() {
    let session = Session::new(Document::new("Native bevel pair").unwrap()).unwrap();
    let request = request();
    let before = session.document().clone();
    let candidate = prepare_gear_pair(&session, 0, actor(), &request).unwrap();
    let model = compiler()
        .compile(candidate.prepared.document(), 1)
        .unwrap();
    assert_eq!(session.document(), &before);
    let check = &model.gear_pair_checks()[&request.pair];
    assert_eq!(check.center_distance_mm, 0.);
    let cone = check.bevel.as_ref().expect("truthful cone context");
    near(cone.shaft_angle_rad, std::f64::consts::FRAC_PI_2, 1e-12);
    near(cone.cone_distance_mm, 20., 1e-12);
    near(
        cone.pitch_cone_angles_rad.iter().sum(),
        std::f64::consts::FRAC_PI_2,
        1e-12,
    );
    assert_eq!(check.cycle_drive_angles_rad.len(), 9);
    assert!(check.current_pose_checked && !check.cycle_reused);
    let witness = check
        .current_engagement
        .as_ref()
        .expect("actual native witness");
    assert_eq!(witness.kind, "bevel_flank_faces");
    assert!(witness.faces.is_some());
    assert!(witness.distance_mm <= 0.100001);
    near(
        (0..3)
            .map(|i| (witness.points_mm[0][i] - witness.points_mm[1][i]).powi(2))
            .sum::<f64>()
            .sqrt(),
        witness.distance_mm,
        4e-7,
    );
    assert!(check.scope.contains("spherical-involute bevel"));
    assert_eq!(model.snapshot().leaf_occurrences.len(), 2);
    let leaves = model
        .snapshot()
        .leaf_occurrences
        .keys()
        .map(|path| {
            assert_eq!(path.root, request.placement.instance);
            model.leaf_solid(path).unwrap()
        })
        .collect::<Vec<_>>();
    let gap = leaves[0]
        .minimum_distance(BRepSelection::Whole, leaves[1], BRepSelection::Whole)
        .unwrap();
    assert!(
        gap.distance_mm <= 0.100001,
        "separated pair: {}",
        gap.distance_mm
    );
    let dir = tempfile::tempdir().unwrap();
    for (i, solid) in leaves.iter().enumerate() {
        assert_eq!(solid.solid_count().unwrap(), 1);
        let path = dir.path().join(format!("bevel-{i}.step"));
        solid.write_step(&path).unwrap();
        let imported = Solid::read_step(&path, GeometryPolicy::default()).unwrap();
        assert_eq!(imported.solid_count().unwrap(), 1);
        near(
            imported.volume_mm3().unwrap(),
            solid.volume_mm3().unwrap(),
            1e-4,
        );
        let a = solid.bounds().unwrap();
        let b = imported.bounds().unwrap();
        for axis in 0..3 {
            near(a.min[axis], b.min[axis], 1e-4);
            near(a.max[axis], b.max[axis], 1e-4);
        }
    }
}
#[test]
fn bevel_drive_reuses_geometry_and_grouped_edit_history_reopen_preserve_identities() {
    let mut session = Session::new(Document::new("Bevel lifecycle").unwrap()).unwrap();
    let request = request();
    session
        .commit(
            prepare_gear_pair(&session, 0, actor(), &request)
                .unwrap()
                .prepared,
        )
        .unwrap();
    let mut cache = EvaluationCache::default();
    let first = compiler()
        .compile_cached(&mut cache, session.document(), 1, None)
        .unwrap();
    let mut drive = request.parameters.clone();
    drive.drive_angle = angle(10.);
    session
        .commit(
            prepare_gear_pair_edit(&session, 1, actor(), request.pair, &drive)
                .unwrap()
                .prepared,
        )
        .unwrap();
    let driven = compiler()
        .compile_cached(&mut cache, session.document(), 2, None)
        .unwrap();
    assert!(driven.gear_pair_checks()[&request.pair].cycle_reused);
    assert!(driven.gear_pair_checks()[&request.pair].current_pose_checked);
    assert!(cache.last_compile().evaluated.is_empty());
    for member in &request.gears {
        assert!(cache.last_compile().reused.contains(&member.feature));
    }
    let before_edit = session.document().clone();
    let mut edited = recipes();
    edited[0].teeth = count(28.);
    let edited = couple_bevel_pair_recipes(&edited, &angle(90.)).unwrap();
    assert!(prepare_gear_edit(&session, 2, actor(), request.gears[0].feature, &edited[0]).is_err());
    let transaction =
        prepare_gear_pair_geometry_edit(&session, 2, actor(), request.pair, &drive, &edited)
            .unwrap();
    let candidate = compiler()
        .compile_cached(&mut cache, transaction.prepared.document(), 3, None)
        .unwrap();
    assert!(!candidate.gear_pair_checks()[&request.pair].cycle_reused);
    for member in &request.gears {
        assert!(cache.last_compile().evaluated.contains(&member.feature));
    }
    assert_eq!(session.document(), &before_edit);
    session.commit(transaction.prepared).unwrap();
    let committed = session.document().clone();
    assert_eq!(
        read_gear_pair(&committed, request.pair).unwrap().outputs,
        request.outputs
    );
    session.undo(3, actor()).unwrap();
    assert_eq!(session.document(), &before_edit);
    session.redo(4, actor()).unwrap();
    assert_eq!(session.document(), &committed);
    let mut bytes = vec![];
    project_file::write(
        &ProjectFile::new(committed.clone()).unwrap(),
        &mut bytes,
        Default::default(),
    )
    .unwrap();
    let reopened = project_file::read(
        &mut bytes.as_slice(),
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap()
    .project;
    assert_eq!(reopened.document, committed);
    let cold = compiler().compile(&reopened.document, 0).unwrap();
    assert_eq!(cold.snapshot().leaf_occurrences.len(), 2);
    assert!(!cold.gear_pair_checks()[&request.pair].cycle_reused);
    for feature in request.parameters.gears {
        near(
            cold.solid(feature).unwrap().volume_mm3().unwrap(),
            candidate.solid(feature).unwrap().volume_mm3().unwrap(),
            1e-5,
        );
    }
    assert!(
        (first
            .solid(request.gears[1].feature)
            .unwrap()
            .volume_mm3()
            .unwrap()
            - cold
                .solid(request.gears[1].feature)
                .unwrap()
                .volume_mm3()
                .unwrap())
        .abs()
            > 1.
    );
}
