//! Complete thread-net and full exterior closure comparison for a blind receiver.
use super::*;
use crate::helical_receiver::Receiver;
pub fn helical_receiver_import_matches(
    imported: &ImportSolid,
    receiver: &Receiver,
    frame: [Point; 3],
    tolerance: f64,
) -> bool {
    let ImportProfile::HelicalReceiver {
        spec,
        stock_radius_mm,
        stock_length_mm,
    } = imported.profile
    else {
        return false;
    };
    if imported.height_mm != stock_length_mm {
        return false;
    }
    let Ok(candidate) = Thread::new(spec) else {
        return false;
    };
    if Receiver::new(candidate, stock_radius_mm, stock_length_mm).is_err() {
        return false;
    }
    let thread = ImportSolid {
        height_mm: spec.length_mm,
        profile: ImportProfile::HelicalThread {
            spec,
            stock_radius_mm: None,
        },
        ..imported.clone()
    };
    if !helical_import_matches(&thread, receiver.thread(), None, frame, tolerance) {
        return false;
    }
    let error = super::cylindrical_closure::error(
        super::cylindrical_closure::Cylinder {
            frame,
            radius_mm: receiver.radius_mm(),
            axial_mm: [0., receiver.stock_length_mm()],
        },
        super::cylindrical_closure::Cylinder {
            frame: [imported.origin_mm, imported.u, imported.v],
            radius_mm: stock_radius_mm,
            axial_mm: [0., stock_length_mm],
        },
    );
    error.is_finite() && error <= tolerance / 8.
}

#[cfg(test)]
mod tests {
    use super::*;
    fn receiver() -> Receiver {
        Receiver::new(
            Thread::new(Spec {
                major_radius_mm: 3.,
                pitch_mm: 1.,
                length_mm: 2.5,
                depth_mm: 5. * 3_f64.sqrt() / 16.,
                crest_width_mm: 0.125,
                lower_flank_deg: 30.,
                upper_flank_deg: 30.,
                starts: 1,
                left_hand: false,
                root_radius_mm: None,
            })
            .unwrap(),
            5.,
            4.,
        )
        .unwrap()
    }
    #[test]
    fn finite_receiver_identity_covers_the_complete_outer_stock_and_declared_floor() {
        let receiver = receiver();
        let frame = [[0.; 3], [1., 0., 0.], [0., 1., 0.]];
        let text =
            crate::step::write_helical_receiver(&receiver, frame[0], frame[1], frame[2], 1e-7)
                .unwrap();
        let (_, imported) = read_import_solid(&text, 1e-7).unwrap();
        assert!(helical_receiver_import_matches(
            &imported, &receiver, frame, 1e-7
        ));
        let mut wrong = imported.clone();
        wrong.height_mm = 5.;
        assert!(!helical_receiver_import_matches(
            &wrong, &receiver, frame, 1e-7
        ));
        wrong = imported.clone();
        wrong.profile = ImportProfile::HelicalThread {
            spec: receiver.thread().spec(),
            stock_radius_mm: Some(5.),
        };
        assert!(!helical_receiver_import_matches(
            &wrong, &receiver, frame, 1e-7
        ));
        for (radius, height) in [(6., 4.), (5., 5.), (5., 1.)] {
            wrong = imported.clone();
            wrong.height_mm = height;
            wrong.profile = ImportProfile::HelicalReceiver {
                spec: receiver.thread().spec(),
                stock_radius_mm: radius,
                stock_length_mm: height,
            };
            assert!(!helical_receiver_import_matches(
                &wrong, &receiver, frame, 1e-7
            ));
        }
        for invalid in [f64::NAN, -1.] {
            let mut spec = receiver.thread().spec();
            spec.pitch_mm = invalid;
            wrong = imported.clone();
            wrong.profile = ImportProfile::HelicalReceiver {
                spec,
                stock_radius_mm: 5.,
                stock_length_mm: 4.,
            };
            assert!(!helical_receiver_import_matches(
                &wrong, &receiver, frame, 1e-7
            ));
        }
        wrong = imported.clone();
        wrong.u = [0., 1., 0.];
        assert!(!helical_receiver_import_matches(
            &wrong, &receiver, frame, 1e-7
        ));
        wrong = imported.clone();
        wrong.origin_mm[0] += 0.01;
        assert!(!helical_receiver_import_matches(
            &wrong, &receiver, frame, 1e-7
        ));
        let large = Receiver::new(*receiver.thread(), 1e6, 4.).unwrap();
        wrong = imported.clone();
        wrong.profile = ImportProfile::HelicalReceiver {
            spec: receiver.thread().spec(),
            stock_radius_mm: 1e6,
            stock_length_mm: 4.,
        };
        assert!(helical_receiver_import_matches(&wrong, &large, frame, 1e-7));
        wrong.u[0] += 4e-14;
        assert!(!helical_receiver_import_matches(
            &wrong, &large, frame, 1e-7
        ));
    }
    #[test]
    fn blind_receiver_far_cap_identity_retains_low_components_before_world_rounding() {
        let source = receiver();
        let height: f64 = 1e7;
        let lower = height.next_down();
        let origin = [0., 0., (height - lower) / 2.];
        assert_eq!(origin[2] + height, origin[2] + lower);
        let receiver = Receiver::new(*source.thread(), source.radius_mm(), height).unwrap();
        let frame = [origin, [1., 0., 0.], [0., 1., 0.]];
        let mut imported = ImportSolid {
            origin_mm: origin,
            u: frame[1],
            v: frame[2],
            height_mm: height,
            profile: ImportProfile::HelicalReceiver {
                spec: receiver.thread().spec(),
                stock_radius_mm: receiver.radius_mm(),
                stock_length_mm: height,
            },
        };
        assert!(helical_receiver_import_matches(
            &imported, &receiver, frame, 1e-10
        ));
        imported.height_mm = lower;
        imported.profile = ImportProfile::HelicalReceiver {
            spec: receiver.thread().spec(),
            stock_radius_mm: receiver.radius_mm(),
            stock_length_mm: lower,
        };
        assert!(!helical_receiver_import_matches(
            &imported, &receiver, frame, 1e-10
        ));
    }
    #[test]
    fn changed_floor_far_cap_and_outer_wall_cannot_be_blindly_imported() {
        let receiver = receiver();
        let frame = [[0.; 3], [1., 0., 0.], [0., 1., 0.]];
        let text =
            crate::step::write_helical_receiver(&receiver, frame[0], frame[1], frame[2], 1e-7)
                .unwrap();
        let data = parse_entities(&text, 1e-7).unwrap();
        let shell = Shell::new(&data, 1e-7, true);
        let solid = shell.represented_solids(true).unwrap()[0];
        let faces = shell.member_faces(solid).unwrap();
        for &face in &faces[5..] {
            let changed =
                super::super::rejection_tests::edit(&text, face, |e| e.args[3] = Value::Enum("F"));
            assert!(read_import_solid(&changed, 1e-7).is_err());
            assert!(
                crate::step_read::verify_helical_receiver(
                    &changed, &receiver, frame[0], frame[1], frame[2], 1e-7
                )
                .is_err()
            );
        }
        let (surface, _, _) = face(&shell, faces[7]).unwrap();
        let reject = |changed: &str| {
            assert!(
                crate::step_read::verify_helical_receiver(
                    changed, &receiver, frame[0], frame[1], frame[2], 1e-7
                )
                .is_err()
            );
            assert!(read_import_solid(changed, 1e-7).is_err());
        };
        reject(&super::super::rejection_tests::edit(&text, surface, |e| {
            e.args.truncate(1)
        }));
        let (_, _, bounds) = face(&shell, faces[7]).unwrap();
        let ring_entity =
            reference(&shell.args(bounds[0], "FACE_OUTER_BOUND").unwrap()[1]).unwrap();
        let oriented = refs(&shell.args(ring_entity, "EDGE_LOOP").unwrap()[1]).unwrap()[0];
        reject(&super::super::rejection_tests::edit(&text, oriented, |e| {
            e.args[4] = Value::Enum("F")
        }));
        let far_edge = reference(&shell.args(oriented, "ORIENTED_EDGE").unwrap()[3]).unwrap();
        let curve = reference(&shell.args(far_edge, "EDGE_CURVE").unwrap()[3]).unwrap();
        let circle_axis = reference(&shell.args(curve, "CIRCLE").unwrap()[1]).unwrap();
        let fresh = data.keys().copied().max().unwrap() + 1;
        let changed = super::super::rejection_tests::edit(&text, circle_axis, |e| {
            e.args[1] = Value::Reference(fresh)
        });
        let end = changed.rfind("ENDSEC;").unwrap();
        let changed = format!(
            "{}#{fresh}=CARTESIAN_POINT('',(0.,0.,4.00000001));\n{}",
            &changed[..end],
            &changed[end..]
        );
        reject(&changed);
        let (_, _, wall_bounds) = face(&shell, faces[6]).unwrap();
        let wall = helical::ring(&shell, wall_bounds[0], true).unwrap();
        let seam = wall
            .into_iter()
            .map(|(edge, _)| shell.args(edge, "EDGE_CURVE").unwrap())
            .find(|args| reference(&args[1]).unwrap() != reference(&args[2]).unwrap())
            .unwrap();
        let [a, b, line] = [1, 2, 3].map(|i| reference(&seam[i]).unwrap());
        let forward = if sense(&seam[4]).unwrap() { "T" } else { "F" };
        reject(&super::super::rejection_tests::edit(&text, far_edge, |e| {
            e.args[1] = Value::Reference(a);
            e.args[2] = Value::Reference(b);
            e.args[3] = Value::Reference(line);
            e.args[4] = Value::Enum(forward);
        }));
        let placement = reference(&shell.args(surface, "PLANE").unwrap()[1]).unwrap();
        let center = reference(&shell.args(placement, "AXIS2_PLACEMENT_3D").unwrap()[1]).unwrap();
        for height in [2., 2e7] {
            let changed = super::super::rejection_tests::edit(&text, center, |e| {
                e.args[1] = Value::List(vec![
                    Value::Number(0.),
                    Value::Number(0.),
                    Value::Number(height),
                ])
            });
            assert!(read_import_solid(&changed, 1e-7).is_err());
        }
    }
}
