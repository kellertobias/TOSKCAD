//! Actual native rack/pinion occurrences, shared drives and immutable exports.
use multicad_core::{
    commands::{Actor, Channel, Session},
    constraint_sketch::Dimension,
    document::Document,
    model::*,
    project_file::{self, ProjectFile},
};
use multicad_kernel::{BRepSelection, BooleanOperation, GeometryPolicy, Solid};
use multicad_model::{Compiler, EvaluationCache};
use std::f64::consts::PI;
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
fn request(cells: f64, rack_first: bool) -> GearPairRequest {
    let shared = GearParameters {
        module: length(1.),
        face_width: length(2.),
        ..Default::default()
    };
    let rack = GearRequest::new(
        "Finite rack",
        GearParameters {
            family: GearFamily::Rack,
            rack: Some(RackDimensions {
                backing_thickness: length(3.),
            }),
            teeth: count(cells),
            bore_diameter: length(0.),
            frame: PrimitiveFrame {
                origin: [length(3.), length(4.), length(5.)],
                u: [0., 1., 0.],
                v: [0., 0., 1.],
            },
            ..shared.clone()
        },
    );
    let pinion = GearRequest::new(
        "Spur pinion",
        GearParameters {
            frame: PrimitiveFrame {
                origin: [length(-7.), length(2.), length(9.)],
                u: [0., 0., 1.],
                v: [1., 0., 0.],
            },
            ..shared
        },
    );
    let mut result = GearPairRequest::new(
        "Native rack engagement",
        if rack_first {
            [rack, pinion]
        } else {
            [pinion, rack]
        },
    );
    result.parameters.frame = PrimitiveFrame {
        origin: [length(100.), length(200.), length(300.)],
        u: [0., 1., 0.],
        v: [0., 0., 1.],
    };
    result
}
fn near(a: f64, b: f64, tolerance: f64) {
    assert!(
        (a - b).abs() <= tolerance,
        "{a} != {b}; tolerance {tolerance}"
    );
}
#[test]
fn both_roles_and_odd_even_finite_racks_have_eleven_native_samples_two_physical_leaves_and_step_readback()
 {
    for cells in [20., 21.] {
        for rack_first in [false, true] {
            let mut session = Session::new(Document::new("Native finite rack").unwrap()).unwrap();
            let request = request(cells, rack_first);
            let before = session.document().clone();
            let prepared = prepare_gear_pair(&session, 0, actor(), &request).unwrap();
            let native = compiler().compile(prepared.prepared.document(), 1).unwrap();
            assert_eq!(session.document(), &before);
            assert_eq!(session.revision(), 0);
            let report = &native.gear_pair_checks()[&request.pair];
            assert_eq!(report.cycle_drive_angles_rad.len(), 11);
            assert!(report.current_pose_checked && !report.cycle_reused);
            assert_eq!(report.maximum_overlap_volume_mm3, 0.);
            near(report.center_distance_mm, 12., 1e-12);
            let half_period = PI / 24.;
            for (i, phase) in report.cycle_drive_angles_rad[..9].iter().enumerate() {
                near(*phase, half_period * (i as f64 / 4. - 1.), 1e-12);
            }
            let limit = (cells * PI / 2. - 13.) / 12.;
            near(report.cycle_drive_angles_rad[9], -limit, 1e-12);
            near(report.cycle_drive_angles_rad[10], limit, 1e-12);
            assert_eq!(native.snapshot().leaf_occurrences.len(), 2);
            let dir = tempfile::tempdir().unwrap();
            let mut physical = vec![];
            for (i, path) in native.snapshot().leaf_occurrences.keys().enumerate() {
                assert_eq!(path.root, request.placement.instance);
                let leaf = native.leaf_solid(path).unwrap();
                assert_eq!(leaf.solid_count().unwrap(), 1);
                assert!(leaf.volume_mm3().unwrap() > 0.);
                // Base-frame normal is world +X for BOTH independently framed definitions.
                let bounds = leaf.bounds().unwrap();
                near(bounds.min[0], 100., 1e-5);
                near(bounds.max[0], 102., 1e-5);
                if path
                    .members
                    .contains(&request.outputs.instances[usize::from(!rack_first)])
                {
                    near(bounds.min[1], 200. - cells * PI / 2., 1e-5);
                    near(bounds.max[1], 200. + cells * PI / 2., 1e-5);
                    near(bounds.min[2], 300. - 4.25, 1e-5);
                    near(bounds.max[2], 301., 1e-5);
                }
                let step = dir.path().join(format!("member-{i}.step"));
                leaf.write_step(&step).unwrap();
                let restored = Solid::read_step(&step, GeometryPolicy::default()).unwrap();
                assert_eq!(restored.solid_count().unwrap(), 1);
                near(
                    restored.volume_mm3().unwrap(),
                    leaf.volume_mm3().unwrap(),
                    1e-4,
                );
                let back_bounds = restored.bounds().unwrap();
                for axis in 0..3 {
                    near(back_bounds.min[axis], bounds.min[axis], 1e-5);
                    near(back_bounds.max[axis], bounds.max[axis], 1e-5);
                }
                physical.push(leaf);
            }
            let common = physical[0]
                .boolean(
                    physical[1],
                    BooleanOperation::Intersect,
                    GeometryPolicy::default(),
                )
                .unwrap();
            assert!(
                common.is_none(),
                "Current phase has actual positive-volume overlap"
            );
            let gap = physical[0]
                .minimum_distance(BRepSelection::Whole, physical[1], BRepSelection::Whole)
                .unwrap();
            assert!(
                gap.distance_mm <= 0.100001,
                "Pair is separated beyond total authored thinning"
            );
            session.commit(prepared.prepared).unwrap();
            assert_eq!(
                read_gear_pair(session.document(), request.pair)
                    .unwrap()
                    .outputs,
                request.outputs
            );
        }
    }
}
#[test]
fn drive_reuses_geometry_source_edit_invalidates_cycle_and_reopen_keeps_two_occurrences() {
    let mut session = Session::new(Document::new("Cached finite rack").unwrap()).unwrap();
    let request = request(20., true);
    session
        .commit(
            prepare_gear_pair(&session, 0, actor(), &request)
                .unwrap()
                .prepared,
        )
        .unwrap();
    let mut cache = EvaluationCache::default();
    let first = compiler()
        .compile_cached(&mut cache, session.document(), session.revision(), None)
        .unwrap();
    let rack_volume = first
        .solid(request.gears[0].feature)
        .unwrap()
        .volume_mm3()
        .unwrap();
    let mut drive = request.parameters.clone();
    drive.drive_angle = angle(10.);
    session
        .commit(
            prepare_gear_pair_edit(&session, session.revision(), actor(), request.pair, &drive)
                .unwrap()
                .prepared,
        )
        .unwrap();
    let driven = compiler()
        .compile_cached(&mut cache, session.document(), session.revision(), None)
        .unwrap();
    assert!(driven.gear_pair_checks()[&request.pair].cycle_reused);
    assert!(cache.last_compile().evaluated.is_empty());
    assert!(
        cache
            .last_compile()
            .reused
            .contains(&request.gears[0].feature)
    );
    assert!(
        cache
            .last_compile()
            .reused
            .contains(&request.gears[1].feature)
    );
    near(
        driven.gear_pair_checks()[&request.pair].current_drive_angle_rad,
        10_f64.to_radians(),
        1e-12,
    );
    let mut rack = request.gears[0].parameters.clone();
    rack.rack.as_mut().unwrap().backing_thickness = length(4.);
    session
        .commit(
            prepare_gear_edit(
                &session,
                session.revision(),
                actor(),
                request.gears[0].feature,
                &rack,
            )
            .unwrap()
            .prepared,
        )
        .unwrap();
    let edited = compiler()
        .compile_cached(&mut cache, session.document(), session.revision(), None)
        .unwrap();
    assert!(!edited.gear_pair_checks()[&request.pair].cycle_reused);
    assert!(
        cache
            .last_compile()
            .evaluated
            .contains(&request.gears[0].feature)
    );
    assert!(
        cache
            .last_compile()
            .reused
            .contains(&request.gears[1].feature)
    );
    let expected_added = 20. * PI * 2.;
    near(
        edited
            .solid(request.gears[0].feature)
            .unwrap()
            .volume_mm3()
            .unwrap()
            - rack_volume,
        expected_added,
        1e-4,
    );
    let good = session.document().clone();
    let mut invalid = drive;
    invalid.drive_angle = angle(1000.);
    assert!(
        prepare_gear_pair_edit(
            &session,
            session.revision(),
            actor(),
            request.pair,
            &invalid
        )
        .is_err()
    );
    assert_eq!(session.document(), &good);
    let mut bytes = vec![];
    project_file::write(
        &ProjectFile::new(good.clone()).unwrap(),
        &mut bytes,
        Default::default(),
    )
    .unwrap();
    let read = project_file::read(
        &mut bytes.as_slice(),
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap();
    let reopened = Session::new(read.project.document).unwrap();
    assert_eq!(reopened.document(), &good);
    let independent = compiler()
        .compile(reopened.document(), reopened.revision())
        .unwrap();
    assert_eq!(independent.snapshot().leaf_occurrences.len(), 2);
    assert_eq!(
        read_gear_pair(reopened.document(), request.pair)
            .unwrap()
            .outputs,
        request.outputs
    );
    assert!(!independent.gear_pair_checks()[&request.pair].cycle_reused);
    assert!(independent.gear_pair_checks()[&request.pair].current_pose_checked);
    for path in independent.snapshot().leaf_occurrences.keys() {
        let leaf = independent.leaf_solid(path).unwrap();
        assert_eq!(leaf.solid_count().unwrap(), 1);
        let prior = edited.leaf_solid(path).unwrap();
        near(
            leaf.volume_mm3().unwrap(),
            prior.volume_mm3().unwrap(),
            1e-4,
        );
    }
    session.undo(session.revision(), actor()).unwrap();
    session.undo(session.revision(), actor()).unwrap();
    assert_eq!(
        read_gear_pair(session.document(), request.pair)
            .unwrap()
            .parameters
            .drive_angle,
        request.parameters.drive_angle
    );
    session.redo(session.revision(), actor()).unwrap();
    session.redo(session.revision(), actor()).unwrap();
    assert_eq!(session.document(), &good);
}

#[test]
fn rack_and_actual_involute_pinion_print_files_read_back_as_two_closed_native_derived_meshes() {
    use multicad_core::exchange::{
        AidPolicy, MeshQuality, PrintFileKind, PrintRequest, PrintSource,
    };
    use multicad_model::export::{Exporter, mesh};
    let mut session = Session::new(Document::new("Rack print readback").unwrap()).unwrap();
    let mut request = request(20., true);
    request.parameters.frame = PrimitiveFrame::default();
    let prepared = prepare_gear_pair(&session, 0, actor(), &request).unwrap();
    session.commit(prepared.prepared).unwrap();
    let native = compiler().compile(session.document(), 1).unwrap();
    let expected_volume: f64 = native
        .snapshot()
        .leaf_occurrences
        .keys()
        .map(|path| native.leaf_solid(path).unwrap().volume_mm3().unwrap())
        .sum();
    let print = PrintRequest {
        source: PrintSource::Parts {
            parts: vec![request.placement.instance],
        },
        aids: AidPolicy::ExcludeRemovable,
        quality: MeshQuality {
            chord_deviation_mm: 0.002,
            angular_deviation_deg: 5.,
        },
        stl_per_part: true,
    };
    let exporter = Exporter::default();
    let preview = exporter
        .print_preview(session.document(), 1, &print, &actor(), None)
        .unwrap();
    assert!(preview.can_write(), "{:?}", preview.blocked);
    let directory = tempfile::tempdir().unwrap();
    let receipt = exporter
        .write_print(
            session.document(),
            1,
            &print,
            &preview.key,
            &directory.path().join("rack-pinion.3mf"),
            &actor(),
            None,
        )
        .unwrap();
    assert_eq!(receipt.files.len(), 3);
    let mut stl_volume = 0.;
    for file in receipt.files {
        let bytes = std::fs::read(&file.path).unwrap();
        let read = match file.kind {
            PrintFileKind::Stl => mesh::read_stl(&bytes).unwrap(),
            PrintFileKind::ThreeMf => mesh::read_three_mf(&bytes).unwrap(),
        };
        assert_eq!(read.triangles, file.triangles);
        near(read.volume_mm3(), file.mesh_volume_mm3, 1e-6);
        assert!(read.triangles > 100);
        if file.kind == PrintFileKind::Stl {
            stl_volume += read.volume_mm3();
        } else {
            near(read.volume_mm3(), expected_volume, expected_volume * 0.002);
        }
    }
    near(stl_volume, expected_volume, expected_volume * 0.002);
}
