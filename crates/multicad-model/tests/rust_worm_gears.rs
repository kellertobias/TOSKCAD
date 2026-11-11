//! Rust ZA worm and generated wheel pairs: compile, certify, edit, history
//! and saved files through the Rust geometry backend.
use multicad_core::{
    commands::{Actor, Channel, Session},
    constraint_sketch::Dimension,
    document::{Document, EntityId},
    model::{self, *},
    project_file::{self, FileLimits, ProjectFile},
    references::TopologyKind,
};
use multicad_kernel::{
    BooleanOperation, RigidTransform, Solid, rust_kernel::worm_gear::clearance::ResultKind,
};
use multicad_model::{Compiler, GeometryBackend};
use std::{collections::BTreeSet, f64::consts::PI};

fn actor() -> Actor {
    Actor::local(Channel::Ui)
}

fn count(n: f64) -> Dimension {
    Dimension::Literal {
        value: n,
        unit: "count".into(),
    }
}

fn member(wheel: bool, teeth: f64, width: f64, bore: f64, backlash: f64) -> GearParameters {
    GearParameters {
        family: GearFamily::Worm,
        worm: Some(WormDimensions {
            role: if wheel {
                WormMember::Wheel
            } else {
                WormMember::Worm
            },
            tooth_system: WormToothSystem::ZaAxialStraight,
            mate_teeth: count(if wheel { 1. } else { teeth }),
            diameter_quotient: ratio(10.),
        }),
        module_system: GearModuleSystem::Axial,
        module: length(1.),
        teeth: count(if wheel { teeth } else { 1. }),
        hand: GearHand::Right,
        face_width: length(width),
        bore_diameter: length(bore),
        root_fillet_ratio: ratio(0.),
        backlash: length(backlash),
        fabrication_allowance: length(0.02),
        frame: PrimitiveFrame {
            origin: [length(4.), length(3.), length(-4.)],
            u: [0., 1., 0.],
            v: [0., 0., 1.],
        },
        ..Default::default()
    }
}

fn members(teeth: f64, wheel_width: f64, backlash: f64) -> [GearParameters; 2] {
    [
        member(false, teeth, 20., 2., backlash),
        member(true, teeth, wheel_width, 6., backlash),
    ]
}

fn roles(solid: &Solid, owner: EntityId) -> BTreeSet<String> {
    solid
        .topology_snapshot(owner, 0)
        .unwrap()
        .elements
        .into_iter()
        .filter(|e| e.key.kind == TopologyKind::Face)
        .flat_map(|e| e.origins.into_iter().map(|o| o.role))
        .collect()
}

#[test]
fn rust_worm_pairs_certify_saved_edited_motion_and_refuse_wrong_poses() {
    let mut session = Session::new(Document::new("Rust worm pair").unwrap()).unwrap();
    let mut request = GearPairRequest::new(
        "ZA pair",
        members(40., 1.5, 0.03).map(|p| GearRequest::new("ZA member", p)),
    );
    request.parameters.frame = PrimitiveFrame {
        origin: [length(30.), length(40.), length(50.)],
        u: [0., 1., 0.],
        v: [0.8, 0., 0.6],
    };
    let draft = model::prepare_gear_pair(&session, 0, actor(), &request).unwrap();
    let compiler = Compiler {
        omit_display: true,
        ..Compiler::default()
    };
    let document = draft.prepared.document();
    let rust = compiler
        .compile_with_backend(document, 1, GeometryBackend::Rust, None)
        .unwrap();
    let report = &rust.gear_pair_checks()[&request.pair];
    assert_eq!(report.cycle_drive_angles_rad.len(), 9);
    assert!(report.current_pose_checked && report.worm.is_some());
    let witness = report.current_engagement.as_ref().unwrap();
    // Total thinning: (0.03 + 0.02) per member.
    assert!(
        witness.distance_mm > 0. && witness.distance_mm <= 0.1 + 1e-5,
        "{witness:?}"
    );
    let [worm, wheel] = request.outputs.instances;
    for instance in [worm, wheel] {
        let solid = rust.solid(instance).unwrap();
        assert!(solid.uses_rust_geometry());
        assert_eq!(solid.solid_count().unwrap(), 1);
        solid
            .topology_snapshot(instance, 1)
            .unwrap()
            .validate()
            .unwrap();
    }
    assert!(roles(rust.solid(worm).unwrap(), worm).contains("worm/flank.lower"));
    assert!(roles(rust.solid(wheel).unwrap(), wheel).contains("worm_wheel/sector/39/flank.plus"));

    // A half-pitch wheel turn about its own axis puts teeth into the
    // thread; a moved wheel stays clear without engagement.
    let placed_wheel = rust.solid(wheel).unwrap();
    let axis_frame = placed_wheel
        .faces()
        .unwrap()
        .into_iter()
        .find_map(|f| f.plane)
        .unwrap();
    let spin = |angle: f64| {
        let ([px, py, pz], [x, y, z]) = axis_frame;
        let (s, c) = angle.sin_cos();
        let t = 1. - c;
        let rotation = [
            [c + x * x * t, x * y * t - z * s, x * z * t + y * s],
            [y * x * t + z * s, c + y * y * t, y * z * t - x * s],
            [z * x * t - y * s, z * y * t + x * s, c + z * z * t],
        ];
        // The end-plane point lies on the wheel axis.
        let point = [px, py, pz];
        let center = {
            let bounds = placed_wheel.bounds().unwrap();
            let middle: [f64; 3] = std::array::from_fn(|i| (bounds.min[i] + bounds.max[i]) / 2.);
            let offset: f64 = (0..3).map(|i| (middle[i] - point[i]) * [x, y, z][i]).sum();
            std::array::from_fn(|i| point[i] + offset * [x, y, z][i])
        };
        let center: [f64; 3] = center;
        let moved: [f64; 3] = std::array::from_fn(|i| {
            center[i] - (0..3).map(|j| rotation[i][j] * center[j]).sum::<f64>()
        });
        RigidTransform {
            rotation,
            translation_mm: moved,
        }
    };
    let half = placed_wheel.transformed(spin(PI / 40.)).unwrap();
    let worm_solid = rust.solid(worm).unwrap();
    let (result, _) = worm_solid
        .certify_rust_worm_pair(&half, compiler.policy, 0.2, None)
        .unwrap();
    assert_eq!(result, ResultKind::CommonInterior);
    let far = placed_wheel
        .transformed(RigidTransform {
            rotation: [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]],
            translation_mm: [1000., 0., 0.],
        })
        .unwrap();
    assert_eq!(
        worm_solid
            .certify_rust_worm_pair(&far, compiler.policy, 0.2, None)
            .unwrap(),
        (ResultKind::Clear, None)
    );
    assert!(
        worm_solid
            .certify_rust_worm_pair(worm_solid, compiler.policy, 0.2, None)
            .is_err()
    );

    if multicad_kernel::OCCT_LINKED {
        let native = compiler
            .compile_with_backend(document, 1, GeometryBackend::OpenCascade, None)
            .unwrap();
        let oracle = &native.gear_pair_checks()[&request.pair];
        assert_eq!(oracle.cycle_drive_angles_rad, report.cycle_drive_angles_rad);
        println!(
            "{{\"rust_cycle_ms\":{},\"rust_current_ms\":{},\"native_cycle_ms\":{},\"native_current_ms\":{},\"rust_witness_mm\":{},\"native_witness_mm\":{}}}",
            report.sampled_cycle_ms,
            report.current_pose_ms,
            oracle.sampled_cycle_ms,
            oracle.current_pose_ms,
            witness.distance_mm,
            oracle.current_engagement.as_ref().unwrap().distance_mm
        );
        for instance in [worm, wheel] {
            let [r, n] = [&rust, &native].map(|c| c.solid(instance).unwrap().volume_mm3().unwrap());
            assert!((r - n).abs() / n < 1e-6, "{r} {n}");
            let [rb, nb] = [&rust, &native].map(|c| c.solid(instance).unwrap().bounds().unwrap());
            for i in 0..3 {
                assert!(
                    (rb.min[i] - nb.min[i]).abs() < 5e-3 && (rb.max[i] - nb.max[i]).abs() < 5e-3,
                    "{rb:?} {nb:?}"
                );
            }
            assert_eq!(
                roles(rust.solid(instance).unwrap(), instance),
                roles(native.solid(instance).unwrap(), instance)
            );
        }
        // OpenCascade confirms the deliberately wrong pose overlaps.
        let wrong = native
            .solid(wheel)
            .unwrap()
            .transformed(spin(PI / 40.))
            .unwrap();
        let overlap = native
            .solid(worm)
            .unwrap()
            .boolean(&wrong, BooleanOperation::Intersect, compiler.policy)
            .unwrap()
            .unwrap()
            .volume_mm3()
            .unwrap();
        assert!(overlap > 0.1, "{overlap}");
    }

    session.commit(draft.prepared).unwrap();
    // Edit drive and the wheel face width in one checked transaction.
    let mut parameters = request.parameters.clone();
    parameters.drive_angle = angle(30.);
    let gears = members(40., 1.2, 0.03);
    let edited = model::prepare_gear_pair_geometry_edit(
        &session,
        session.revision(),
        actor(),
        request.pair,
        &parameters,
        &gears,
    )
    .unwrap();
    session.commit(edited.prepared).unwrap();
    let after = compiler
        .compile_with_backend(
            session.document(),
            session.revision(),
            GeometryBackend::Rust,
            None,
        )
        .unwrap();
    let moved = &after.gear_pair_checks()[&request.pair];
    assert!((moved.current_drive_angle_rad - 30_f64.to_radians()).abs() < 1e-12);
    let narrower = after.solid(wheel).unwrap().volume_mm3().unwrap();
    let wider = rust.solid(wheel).unwrap().volume_mm3().unwrap();
    assert!(narrower < wider);
    assert_eq!(
        roles(after.solid(wheel).unwrap(), wheel),
        roles(rust.solid(wheel).unwrap(), wheel)
    );

    // Undo restores the previous recipe and its proof; redo restores the edit.
    session.undo(session.revision(), actor()).unwrap();
    let undone = compiler
        .compile_with_backend(
            session.document(),
            session.revision(),
            GeometryBackend::Rust,
            None,
        )
        .unwrap();
    assert!((undone.solid(wheel).unwrap().volume_mm3().unwrap() - wider).abs() < 1e-9);
    session.redo(session.revision(), actor()).unwrap();

    // Zero thinning leaves conjugate contact that Rust cannot certify.
    let revision = session.revision();
    let mut zero = gears.clone();
    for gear in &mut zero {
        gear.backlash = length(0.);
        gear.fabrication_allowance = length(0.);
    }
    let candidate = model::prepare_gear_pair_geometry_edit(
        &session,
        revision,
        actor(),
        request.pair,
        &parameters,
        &zero,
    )
    .unwrap();
    let Err(failure) = compiler.compile_with_backend(
        candidate.prepared.document(),
        revision,
        GeometryBackend::Rust,
        None,
    ) else {
        panic!("zero-thinning worm pair must not be certified");
    };
    assert!(
        failure.to_string().contains("certified positive clearance"),
        "{failure}"
    );
    assert_eq!(session.revision(), revision);

    let mut bytes = Vec::new();
    project_file::write(
        &ProjectFile::new(session.document().clone()).unwrap(),
        &mut bytes,
        FileLimits::default(),
    )
    .unwrap();
    let reopened = project_file::read(
        &mut bytes.as_slice(),
        FileLimits::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap();
    let readback = compiler
        .compile_with_backend(&reopened.project.document, 0, GeometryBackend::Rust, None)
        .unwrap();
    let proof = &readback.gear_pair_checks()[&request.pair];
    assert_eq!(proof.cycle_drive_angles_rad, moved.cycle_drive_angles_rad);
    assert!((readback.solid(wheel).unwrap().volume_mm3().unwrap() - narrower).abs() < 1e-9);
}

#[test]
fn rust_worm_pair_members_draw_along_their_placed_axes() {
    use multicad_core::drawings::{VectorCurve, VectorRole};
    use multicad_kernel::{DrawingProjection, DrawingSolid, PlaneFrame, project_drawing};
    let session = Session::new(Document::new("Rust worm drawing").unwrap()).unwrap();
    let request = GearPairRequest::new(
        "ZA pair",
        members(40., 1.5, 0.03).map(|p| GearRequest::new("ZA member", p)),
    );
    let draft = model::prepare_gear_pair(&session, 0, actor(), &request).unwrap();
    let compiled = Compiler {
        omit_display: true,
        ..Compiler::default()
    }
    .compile_with_backend(draft.prepared.document(), 1, GeometryBackend::Rust, None)
    .unwrap();
    let [worm, wheel] = request.outputs.instances;
    // A view frame whose normal is the placed axis through an end plane.
    let axial = |solid: &Solid| {
        let (point, normal) = solid
            .faces()
            .unwrap()
            .into_iter()
            .find_map(|face| face.plane)
            .unwrap();
        let helper = if normal[0].abs() < 0.9 {
            [1., 0., 0.]
        } else {
            [0., 1., 0.]
        };
        let dot: f64 = (0..3).map(|i| helper[i] * normal[i]).sum();
        let u: [f64; 3] = std::array::from_fn(|i| helper[i] - dot * normal[i]);
        let length = u.iter().map(|x| x * x).sum::<f64>().sqrt();
        let u = u.map(|x| x / length);
        let v = [
            normal[1] * u[2] - normal[2] * u[1],
            normal[2] * u[0] - normal[0] * u[2],
            normal[0] * u[1] - normal[1] * u[0],
        ];
        PlaneFrame {
            origin: point,
            u,
            v,
        }
    };
    let worm_solid = compiled.solid(worm).unwrap();
    let frame = axial(worm_solid);
    let view = project_drawing(
        &[DrawingSolid {
            solid: worm_solid,
            hatch: None,
        }],
        DrawingProjection {
            frame,
            hidden_lines: true,
            ..Default::default()
        },
    )
    .unwrap();
    let circles = |role: VectorRole| {
        view.iter()
            .filter(|edge| edge.role == role)
            .filter_map(|edge| match edge.curve {
                VectorCurve::Ellipse { major_mm, .. } => Some(major_mm),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    // Crest radius 6 m and bore radius 1 mm are visible; the root circle
    // outside the near root land is hidden.
    let mut visible = circles(VectorRole::Visible);
    visible.sort_by(f64::total_cmp);
    assert_eq!(visible.len(), 2);
    assert!((visible[0] - 1.).abs() < 1e-9 && (visible[1] - 6.).abs() < 1e-9);
    assert_eq!(circles(VectorRole::Hidden).len(), 1);
    let wheel_solid = compiled.solid(wheel).unwrap();
    let cut = project_drawing(
        &[DrawingSolid {
            solid: wheel_solid,
            hatch: None,
        }],
        DrawingProjection {
            frame: axial(wheel_solid),
            section_offset_mm: Some(-0.75),
            section_only: true,
            ..Default::default()
        },
    )
    .unwrap();
    assert!(cut.len() > 40 && cut.iter().all(|edge| edge.role == VectorRole::Section));
}
