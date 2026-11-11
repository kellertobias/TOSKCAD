//! Full untapered source law and finite terminal material identity.
use super::*;
use crate::thread_end_relief::Relief;
pub fn thread_end_relief_import_matches(
    imported: &ImportSolid,
    relief: &Relief,
    frame: [Point; 3],
    tolerance: f64,
) -> bool {
    let ImportProfile::ThreadEndRelief {
        tool_spec,
        material_length_mm,
        neck_radius_mm,
    } = imported.profile
    else {
        return false;
    };
    if imported.height_mm != tool_spec.length_mm {
        return false;
    }
    let Ok(tool) = Thread::new(tool_spec) else {
        return false;
    };
    let Ok(candidate) = Relief::from_material(tool, material_length_mm, neck_radius_mm) else {
        return false;
    };
    // Compare only the actual retained thread nets. Full-tool intent remains
    // separate and its constructor budget was validated before restriction.
    if !thread_nets_match(
        relief.material_thread(),
        candidate.material_thread(),
        [None, None],
        [frame, [imported.origin_mm, imported.u, imported.v]],
        tolerance,
    ) {
        return false;
    }
    let error = super::cylindrical_closure::error(
        super::cylindrical_closure::Cylinder {
            frame,
            radius_mm: relief.neck_radius_mm(),
            axial_mm: [relief.material_length_mm(), relief.full_length_mm()],
        },
        super::cylindrical_closure::Cylinder {
            frame: [imported.origin_mm, imported.u, imported.v],
            radius_mm: neck_radius_mm,
            axial_mm: [material_length_mm, tool_spec.length_mm],
        },
    );
    error.is_finite() && error <= tolerance / 8.
}

#[cfg(test)]
mod tests {
    use super::*;
    fn body() -> Relief {
        let root = 1. / (4. * 3_f64.sqrt());
        Relief::new(
            Thread::new(Spec {
                major_radius_mm: 3.,
                pitch_mm: 1.,
                length_mm: 6.6,
                depth_mm: 0.875 * 3_f64.sqrt() / 2. - root,
                crest_width_mm: 0.125,
                lower_flank_deg: 30.,
                upper_flank_deg: 30.,
                starts: 2,
                left_hand: false,
                root_radius_mm: Some(root),
            })
            .unwrap(),
            0.6,
            0.1,
        )
        .unwrap()
    }
    #[test]
    fn matcher_preserves_actual_prefix_station_neck_and_unchanged_source_law() {
        let body = body();
        let frame = [[0.; 3], [1., 0., 0.], [0., 1., 0.]];
        let text = crate::step::write_thread_end_relief(&body, frame[0], frame[1], frame[2], 1e-7)
            .unwrap();
        let (_, source) = read_import_solid(&text, 1e-7).unwrap();
        let matches = |candidate: &ImportSolid| {
            thread_end_relief_import_matches(candidate, &body, frame, 1e-7)
        };
        assert!(matches(&source));
        let mut changed = source.clone();
        changed.height_mm += 1.;
        assert!(!matches(&changed));
        changed = source.clone();
        changed.profile = ImportProfile::HelicalThread {
            spec: body.source_thread().spec(),
            stock_radius_mm: None,
        };
        assert!(!matches(&changed));
        for (f, r) in [
            (0.5, body.neck_radius_mm()),
            (body.material_length_mm(), 3.),
            (body.material_length_mm() + 0.1, body.neck_radius_mm()),
            (body.material_length_mm(), body.neck_radius_mm() - 0.1),
        ] {
            changed = source.clone();
            changed.profile = ImportProfile::ThreadEndRelief {
                tool_spec: body.source_thread().spec(),
                material_length_mm: f,
                neck_radius_mm: r,
            };
            assert!(!matches(&changed));
        }
        let mut spec = body.source_thread().spec();
        spec.pitch_mm = f64::NAN;
        changed = source.clone();
        changed.profile = ImportProfile::ThreadEndRelief {
            tool_spec: spec,
            material_length_mm: body.material_length_mm(),
            neck_radius_mm: body.neck_radius_mm(),
        };
        assert!(!matches(&changed));
        changed = source.clone();
        changed.u = [0., 1., 0.];
        assert!(!matches(&changed));
        changed = source.clone();
        changed.origin_mm[0] += 0.01;
        assert!(!matches(&changed));
    }
    #[test]
    fn long_unseen_tool_extension_does_not_consume_material_step_budget() {
        let original = body();
        let tool = Thread::new(Spec {
            major_radius_mm: 1e4,
            length_mm: 100.,
            starts: 1,
            ..original.source_thread().spec()
        })
        .unwrap();
        let relief = Relief::new(tool, 99., 0.1).unwrap();
        let frame = [[0.; 3], [1., 0., 0.], [0., 1., 0.]];
        assert!(!crate::step::supports_helical_thread(
            &tool, None, frame[0], 1e-7
        ));
        let text =
            crate::step::write_thread_end_relief(&relief, frame[0], frame[1], frame[2], 1e-7)
                .unwrap();
        let (_, imported) = read_import_solid(&text, 1e-7).unwrap();
        assert!(thread_end_relief_import_matches(
            &imported, &relief, frame, 1e-7
        ));
    }

    #[test]
    fn changed_shoulder_neck_caps_and_shared_periodic_wire_fail_both_readers() {
        let body = body();
        let frame = [[0.; 3], [1., 0., 0.], [0., 1., 0.]];
        let text = crate::step::write_thread_end_relief(&body, frame[0], frame[1], frame[2], 1e-7)
            .unwrap();
        let data = parse_entities(&text, 1e-7).unwrap();
        let shell = Shell::new(&data, 1e-7, true);
        let solid = shell.represented_solids(true).unwrap()[0];
        let faces = shell.member_faces(solid).unwrap();
        let count = 4 * body.source_thread().spec().starts as usize;
        let reject = |changed: String| {
            assert!(
                crate::step_read::verify_thread_end_relief(
                    &changed, &body, frame[0], frame[1], frame[2], 1e-7
                )
                .is_err()
            );
            assert!(read_import_solid(&changed, 1e-7).is_err());
        };
        for &id in &faces[count..] {
            reject(super::super::rejection_tests::edit(&text, id, |e| {
                e.args[3] = Value::Enum("F")
            }));
            reject(super::super::rejection_tests::edit(&text, id, |e| {
                e.args[1] = Value::List(vec![])
            }));
        }
        for index in [count, count + 1, count + 3] {
            let (surface, _, bounds) = face(&shell, faces[index]).unwrap();
            let axis = reference(&shell.args(surface, "PLANE").unwrap()[1]).unwrap();
            let center = reference(&shell.args(axis, "AXIS2_PLACEMENT_3D").unwrap()[1]).unwrap();
            reject(super::super::rejection_tests::edit(&text, center, |e| {
                e.args[1] = Value::List(vec![
                    Value::Number(0.),
                    Value::Number(0.),
                    Value::Number(9.),
                ])
            }));
            if index != count {
                let bound = bounds[0];
                let hole = index == count + 1;
                let (edge, _) = helical::ring(&shell, bound, !hole).unwrap()[0];
                let curve = reference(&shell.args(edge, "EDGE_CURVE").unwrap()[3]).unwrap();
                reject(super::super::rejection_tests::edit(&text, curve, |e| {
                    e.args[2] = Value::Number(8.)
                }));
            }
        }
        let (surface, _, bounds) = face(&shell, faces[count + 2]).unwrap();
        for radius in [body.material_thread().root_radius_mm(), -1.] {
            reject(super::super::rejection_tests::edit(&text, surface, |e| {
                e.args[2] = Value::Number(radius)
            }));
        }
        let loop_id = reference(&shell.args(bounds[0], "FACE_OUTER_BOUND").unwrap()[1]).unwrap();
        for id in refs(&shell.args(loop_id, "EDGE_LOOP").unwrap()[1]).unwrap() {
            reject(super::super::rejection_tests::edit(&text, id, |e| {
                e.args[4] = if e.args[4] == Value::Enum("T") {
                    Value::Enum("F")
                } else {
                    Value::Enum("T")
                }
            }));
        }
        let (surface, _, _) = face(&shell, faces[count + 3]).unwrap();
        let axis = reference(&shell.args(surface, "PLANE").unwrap()[1]).unwrap();
        let center = reference(&shell.args(axis, "AXIS2_PLACEMENT_3D").unwrap()[1]).unwrap();
        reject(super::super::rejection_tests::edit(&text, center, |e| {
            e.args[1] = Value::List(vec![
                Value::Number(0.),
                Value::Number(0.),
                Value::Number(5.),
            ])
        }));
    }
}
