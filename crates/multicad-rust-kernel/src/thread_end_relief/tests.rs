use super::*;
use crate::{display::Tessellation, helical_thread::Spec, helical_thread_mesh, manufacturing};
pub(super) fn tool(pitch: f64, length: f64, left: bool, starts: u32, rounded: bool) -> Thread {
    let root = rounded.then_some(pitch / (4. * 3_f64.sqrt()));
    Thread::new(Spec {
        major_radius_mm: 3. * pitch,
        pitch_mm: pitch,
        length_mm: length,
        depth_mm: root.map_or(5. * 3_f64.sqrt() * pitch / 16., |r| {
            0.875 * 3_f64.sqrt() * pitch / 2. - r
        }),
        crest_width_mm: pitch / 8.,
        lower_flank_deg: 30.,
        upper_flank_deg: 30.,
        starts,
        left_hand: left,
        root_radius_mm: root,
    })
    .unwrap()
}
#[test]
fn terminal_neck_retains_source_phase_root_intent_and_one_manufacturing_shell() {
    let options = Tessellation {
        deflection: 0.04,
        angle: 0.3,
        triangle_limit: 150_000,
    };
    for pitch in [0.5, 1.] {
        for left in [false, true] {
            for starts in [1, 2, 8] {
                for rounded in [false, true] {
                    let source = tool(pitch, 6.6 * pitch, left, starts, rounded);
                    let width = 0.6 * pitch;
                    let depth = 0.1 * pitch;
                    let body = Relief::new(source, width, depth).unwrap();
                    assert_eq!(*body.source_thread(), source);
                    assert_eq!(
                        body.intent(),
                        Some(Intent {
                            width_mm: width,
                            radial_depth_mm: depth
                        })
                    );
                    assert_eq!(body.material_length_mm(), source.spec().length_mm - width);
                    assert_eq!(body.neck_radius_mm(), source.root_radius_mm() - depth);
                    assert_eq!(body.support([0., 0., 1.]), source.spec().length_mm);
                    assert_eq!(body.support([0., 0., -1.]), 0.);
                    for angle in [-6., 0., 0.1, 3., 7.] {
                        for z in [
                            0.,
                            body.material_length_mm() / 3.,
                            body.material_length_mm(),
                        ] {
                            assert_eq!(
                                body.material_thread().radius(angle, z),
                                source.radius(angle, z)
                            );
                        }
                    }
                    let z = (body.material_length_mm() + body.full_length_mm()) / 2.;
                    assert!(body.contains([body.neck_radius_mm() / 2., 0., z], 0.));
                    assert!(!body.contains([source.root_radius_mm(), 0., z], 0.));
                    assert!(!body.contains([0., 0., body.full_length_mm() + 1.], 0.));
                    assert!(!body.contains([0., 0., f64::NAN], 0.));
                    assert!(!body.contains([0., 0., 0.], -1.));
                    let volume = body.material_thread().volume_mm3()
                        + std::f64::consts::PI
                            * body.neck_radius_mm().powi(2)
                            * (body.full_length_mm() - body.material_length_mm());
                    assert_eq!(body.volume_mm3(), volume);
                    let mesh = helical_thread_mesh::tessellate_end_relief(&body, options).unwrap();
                    let proof = manufacturing::certify(&mesh).unwrap();
                    assert!(
                        (proof.signed_volume_mm3 - volume).abs()
                            <= helical_thread_mesh::end_relief_volume_error_bound(&body, options)
                    );
                    assert_eq!(
                        mesh.face_indices
                            .iter()
                            .copied()
                            .collect::<std::collections::BTreeSet<_>>(),
                        (0..body.face_count()).collect()
                    );
                    let imported = Relief::from_material(
                        source,
                        body.material_length_mm(),
                        body.neck_radius_mm(),
                    )
                    .unwrap();
                    assert_eq!(imported.intent(), None);
                    assert_eq!(imported.volume_mm3(), body.volume_mm3());
                    for [o, u, v] in [
                        [[0.; 3], [1., 0., 0.], [0., 1., 0.]],
                        [[2., -3., 4.], [0.8, 0.6, 0.], [-0.36, 0.48, 0.8]],
                    ] {
                        let step =
                            crate::step::write_thread_end_relief(&body, o, u, v, 1e-7).unwrap();
                        let volume =
                            crate::step_read::verify_thread_end_relief(&step, &body, o, u, v, 1e-7)
                                .unwrap();
                        assert!((volume - body.volume_mm3()).abs() < 1e-6);
                        let (metrics, imported) =
                            crate::step_read::read_import_solid(&step, 1e-7).unwrap();
                        assert!((metrics.volume_mm3 - body.volume_mm3()).abs() < 1e-6);
                        assert!(crate::step_read::thread_end_relief_import_matches(
                            &imported,
                            &body,
                            [o, u, v],
                            1e-7
                        ));
                    }
                }
            }
        }
    }
}
#[test]
fn canonical_maximum_width_keeps_literal_rounded_station_instead_of_clamping() {
    let source = tool(0.1, 1.01, false, 1, true);
    let body = Relief::new(source, 0.91, 0.01).unwrap();
    assert!(body.material_length_mm() < source.spec().pitch_mm);
    assert_eq!(body.material_length_mm(), 1.01 - 0.91);
    assert_eq!(
        source.axial_prefix(body.material_length_mm()),
        Err(ThreadError::Limit)
    );
    assert!(
        helical_thread_mesh::tessellate_end_relief(
            &body,
            Tessellation {
                deflection: 0.01,
                angle: 0.3,
                triangle_limit: 150_000
            }
        )
        .is_ok()
    );
    let text =
        crate::step::write_thread_end_relief(&body, [0.; 3], [1., 0., 0.], [0., 1., 0.], 1e-7)
            .unwrap();
    assert!(
        crate::step_read::verify_thread_end_relief(
            &text,
            &body,
            [0.; 3],
            [1., 0., 0.],
            [0., 1., 0.],
            1e-7
        )
        .is_ok()
    );
    let (_, imported) = crate::step_read::read_import_solid(&text, 1e-7).unwrap();
    assert!(crate::step_read::thread_end_relief_import_matches(
        &imported,
        &body,
        [[0.; 3], [1., 0., 0.], [0., 1., 0.]],
        1e-7
    ));
}
#[test]
fn invalid_or_unresolved_reliefs_leave_tool_and_mesh_budget_intact() {
    let source = tool(1., 6.6, false, 1, true);
    for (width, depth) in [
        (f64::NAN, 0.1),
        (0.6, f64::NAN),
        (0., 0.1),
        (0.6, 0.),
        (6., 0.1),
        (0.6, source.root_radius_mm()),
    ] {
        assert_eq!(Relief::new(source, width, depth), Err(ThreadError::Invalid));
    }
    for (length, radius) in [
        (f64::NAN, 1.),
        (-1., 1.),
        (6.6, 1.),
        (0.5, 1.),
        (5., f64::NAN),
        (5., 0.),
        (5., source.root_radius_mm()),
    ] {
        assert_eq!(
            Relief::from_material(source, length, radius),
            Err(ThreadError::Invalid)
        );
    }
    let body = Relief::new(source, 0.6, 0.1).unwrap();
    let options = Tessellation {
        deflection: 0.04,
        angle: 0.3,
        triangle_limit: 150_000,
    };
    let mesh = helical_thread_mesh::tessellate_end_relief(&body, options).unwrap();
    for limit in [1, mesh.face_indices.len() - 1] {
        assert_eq!(
            helical_thread_mesh::tessellate_end_relief(
                &body,
                Tessellation {
                    triangle_limit: limit,
                    ..options
                }
            ),
            Err(crate::display::MeshError::BudgetExceeded)
        );
    }
    assert_eq!(*body.source_thread(), source);
    assert!(crate::step::supports_thread_end_relief(
        &body, [0.; 3], 1e-7
    ));
    assert!(!crate::step::supports_thread_end_relief(
        &body, [0.; 3], 1e-16
    ));
    let tiny = Relief::from_material(
        source,
        source.spec().length_mm - 1e-8,
        body.neck_radius_mm(),
    )
    .unwrap();
    let mut writer = crate::step::Writer::new();
    writer
        .append_thread_end_relief(&body, [0.; 3], [1., 0., 0.], [0., 1., 0.], 1e-7)
        .unwrap();
    assert_eq!(
        writer.append_thread_end_relief(&tiny, [0.; 3], [1., 0., 0.], [0., 1., 0.], 1e-7),
        Err(crate::step::StepError::Limit)
    );
    assert_eq!(
        writer.append_thread_end_relief(&body, [0.; 3], [1., 0., 0.], [0., 1., 0.], f64::NAN),
        Err(crate::step::StepError::InvalidGeometry)
    );
    writer
        .append_thread_end_relief(&body, [10., 0., 0.], [1., 0., 0.], [0., 1., 0.], 1e-7)
        .unwrap();
    assert_eq!(
        crate::step_read::read_import_compound(&writer.finish().unwrap(), 1e-7)
            .unwrap()
            .len(),
        2
    );
}
