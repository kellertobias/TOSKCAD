//! Independent meridian/section oracles for retained screw head and drive profiles.
use multicad_core::{
    commands::{Actor, Channel, Session},
    document::Document,
    model::{
        self, MechanicalPartParameters, MechanicalPartRequest, PhillipsHParameters, PhillipsHSize,
        PhillipsInnerForm, ResolvedMechanicalPart, ResolvedPhillipsInnerForm, ResolvedScrewDrive,
        ResolvedScrewHead, ScrewDrive, ScrewHead, ThreadRepresentation, angle, length,
    },
    project_file::{self, ProjectFile},
    references::TopologyKind,
};
use multicad_kernel::{BooleanOperation, GeometryPolicy, RigidTransform, Solid};
use multicad_model::{Compiler, EvaluationCache};
use std::f64::consts::PI;
fn actor() -> Actor {
    Actor::local(Channel::Mcp)
}
fn session() -> Session {
    Session::new(Document::new("Native screw profiles").unwrap()).unwrap()
}
fn compiler() -> Compiler {
    Compiler {
        omit_display: true,
        ..Default::default()
    }
}
fn torx(depth: f64) -> ScrewDrive {
    ScrewDrive::Hexalobular {
        profile: Box::new(model::HexalobularDriveParameters {
            major_diameter: length(2.8),
            minor_diameter: length(2.05),
            outer_lobe_radius: length(0.28),
            depth: length(depth),
            bottom_angle: angle(120.),
        }),
    }
}
fn pan() -> ScrewHead {
    ScrewHead::Pan {
        profile: Box::new(model::CrownedScrewHeadParameters {
            diameter: length(5.6),
            height: length(2.4),
            crown_radius: length(5.),
            top_land_diameter: length(4.1),
            neck_radius: length(0.1),
        }),
    }
}
fn button() -> ScrewHead {
    ScrewHead::Button {
        profile: Box::new(model::CrownedScrewHeadParameters {
            diameter: length(5.7),
            height: length(1.65),
            crown_radius: length(3.5),
            top_land_diameter: length(3.3),
            neck_radius: length(0.1),
        }),
    }
}
fn csk() -> ScrewHead {
    ScrewHead::Countersunk {
        profile: Box::new(model::CountersunkScrewHeadParameters {
            diameter: length(5.5),
            height: length(1.65),
            included_angle: angle(90.),
            edge_chamfer: length(0.1),
            neck_radius: length(0.1),
        }),
    }
}
fn h(size: PhillipsHSize, dimensions: [f64; 4], inner: PhillipsInnerForm, beta: f64) -> ScrewDrive {
    let [m, r, b, g] = dimensions;
    ScrewDrive::PhillipsH {
        profile: Box::new(PhillipsHParameters {
            size,
            mouth_diameter: length(m),
            mouth_radius: length(r),
            b_width: length(b),
            g_diameter: length(g),
            tilt_angle: angle(beta),
            outer_flank_angle: angle(26.5),
            floor_angle: angle(28.),
            outer_corner_angle: angle(92.),
            inner,
        }),
    }
}
fn h1(m: f64) -> ScrewDrive {
    h(
        PhillipsHSize::H1,
        [m, 0.5, 0.955, 1.27],
        PhillipsInnerForm::ShallowV {
            angle: angle(138.),
            e_width: length(0.435),
        },
        7.,
    )
}
fn recipe(
    head: ScrewHead,
    drive: ScrewDrive,
    nominal: f64,
    pitch: f64,
) -> MechanicalPartParameters {
    let mut p = model::default_machine_screw();
    let MechanicalPartParameters::MachineScrew {
        underhead_length,
        shank_diameter,
        head: hd,
        drive: dr,
        thread,
        ..
    } = &mut p
    else {
        unreachable!()
    };
    *underhead_length = Some(length(6.));
    *shank_diameter = length(nominal);
    *hd = head;
    *dr = drive;
    thread.major_diameter = length(nominal);
    thread.pitch = length(pitch);
    thread.length = length(2. * pitch);
    thread.representation = ThreadRepresentation::Modeled;
    p
}
fn add(s: &mut Session, p: MechanicalPartParameters) -> MechanicalPartRequest {
    let r = MechanicalPartRequest::new("Profile screw", p);
    s.commit(
        model::prepare_mechanical_part(s, s.revision(), actor(), &r)
            .unwrap()
            .prepared,
    )
    .unwrap();
    r
}
fn probe(s: &Solid, p: [f64; 3], inside: bool) {
    let d = 0.002;
    let q = Solid::box_solid([d; 3], GeometryPolicy::default())
        .unwrap()
        .transformed(RigidTransform {
            translation_mm: p.map(|v| v - d / 2.),
            ..Default::default()
        })
        .unwrap();
    let v = s
        .boolean(&q, BooleanOperation::Intersect, GeometryPolicy::default())
        .unwrap()
        .map(|s| s.volume_mm3().unwrap())
        .unwrap_or(0.)
        / d.powi(3);
    assert!(
        if inside { v > 0.999 } else { v < 0.001 },
        "Actual section {p:?}: fraction {v}, expected material {inside}"
    );
}
fn integral(f: &impl Fn(f64) -> f64, a: f64, b: f64) -> f64 {
    fn rec(
        f: &impl Fn(f64) -> f64,
        interval: [f64; 2],
        samples: [f64; 3],
        s: f64,
        eps: f64,
        n: u32,
    ) -> f64 {
        let [a, b] = interval;
        let [fa, fb, fm] = samples;
        let m = (a + b) / 2.;
        let fl = f((a + m) / 2.);
        let fr = f((m + b) / 2.);
        let l = (m - a) * (fa + 4. * fl + fm) / 6.;
        let r = (b - m) * (fm + 4. * fr + fb) / 6.;
        if n == 0 || (l + r - s).abs() < 15. * eps {
            l + r + (l + r - s) / 15.
        } else {
            rec(f, [a, m], [fa, fm, fl], l, eps / 2., n - 1)
                + rec(f, [m, b], [fm, fb, fr], r, eps / 2., n - 1)
        }
    }
    if a == b {
        return 0.;
    }
    let fa = f(a);
    let fb = f(b);
    let fm = f((a + b) / 2.);
    rec(
        f,
        [a, b],
        [fa, fb, fm],
        (b - a) * (fa + 4. * fm + fb) / 6.,
        1e-10,
        30,
    )
}
/// Independent analytic head/neck meridian, integrated without native queries.
fn body_volume(head: &ResolvedScrewHead, shank: f64, plain: f64) -> f64 {
    let k = head.height_mm();
    let mut knots = vec![-k, 0., plain];
    let radius: Box<dyn Fn(f64) -> f64> = match *head {
        ResolvedScrewHead::Pan {
            radius_mm: r,
            crown_radius_mm: c,
            top_land_radius_mm: l,
            neck_radius_mm: n,
            ..
        }
        | ResolvedScrewHead::Button {
            radius_mm: r,
            crown_radius_mm: c,
            top_land_radius_mm: l,
            neck_radius_mm: n,
            ..
        } => {
            let center = -k + (c * c - l * l).sqrt();
            let skirt = center - (c * c - r * r).sqrt();
            knots.extend([skirt, n]);
            Box::new(move |z| {
                if z < skirt {
                    (c * c - (z - center).powi(2)).sqrt()
                } else if z < 0. {
                    r
                } else if n > 0. && z < n {
                    shank + n - (n * n - (z - n).powi(2)).max(0.).sqrt()
                } else {
                    shank
                }
            })
        }
        ResolvedScrewHead::Countersunk {
            radius_mm: r,
            included_angle_deg,
            edge_chamfer_mm: ch,
            neck_radius_mm: n,
            ..
        } => {
            let t = (included_angle_deg / 2.).to_radians();
            let rim = k - (r - shank) / t.tan();
            let center = n * (t / 2.).tan();
            let tangent = center - n * t.sin();
            knots.extend([-k + ch, -k + rim, tangent, center]);
            Box::new(move |z| {
                if n > 0. && z > tangent && z < center {
                    shank + n - (n * n - (z - center).powi(2)).max(0.).sqrt()
                } else if z >= center {
                    shank
                } else if z < -k + ch {
                    r - ch + z + k
                } else {
                    r.min(shank - z * t.tan())
                }
            })
        }
        ResolvedScrewHead::SocketCap { radius_mm: r, .. } => {
            Box::new(move |z| if z < 0. { r } else { shank })
        }
        _ => unreachable!(),
    };
    knots.retain(|z| *z >= -k && *z <= plain);
    knots.sort_by(f64::total_cmp);
    knots.dedup();
    knots
        .windows(2)
        .map(|w| integral(&|z| PI * radius(z).powi(2), w[0], w[1]))
        .sum()
}
fn thread_area(d: f64, p: f64) -> f64 {
    let r = d / 2.;
    let root = r - 5. * 3_f64.sqrt() / 16. * p;
    let q = (r * r - root * root) / 16.
        + 2. / (3_f64.sqrt() * p)
            * (r * (r * r - root * root) / 2. - (r.powi(3) - root.powi(3)) / 3.);
    PI * root * root + 2. * PI * q
}
fn torx_removed(major: f64, minor: f64, re: f64, depth: f64, angle: f64) -> f64 {
    let a = 2. * major;
    let b = 2. * minor;
    let ri = -((b + 3_f64.sqrt() * (2. * re - a)) * b + (a - 4. * re) * a)
        / (4. * b - 2. * 3_f64.sqrt() * a + (4. * 3_f64.sqrt() - 8.) * re);
    let ce = major - re;
    let d = minor + ri;
    let dist = ((d * (PI / 6.).cos() - ce).powi(2) + (d / 2.).powi(2)).sqrt();
    let q = [
        ce + re * (d * (PI / 6.).cos() - ce) / dist,
        re * d / 2. / dist,
    ];
    let theta = q[1].atan2(q[0]);
    let ray = |t: f64| {
        if t <= theta {
            ce * t.cos() + (re * re - ce * ce * t.sin().powi(2)).sqrt()
        } else {
            let u = PI / 6. - t;
            d * u.cos() - (ri * ri - d * d * u.sin().powi(2)).sqrt()
        }
    };
    let f = |t: f64| {
        let r = ray(t);
        (depth + major / (angle / 2.).to_radians().tan()) * r * r / 2.
            - r.powi(3) / (3. * (angle / 2.).to_radians().tan())
    };
    12. * (integral(&f, 0., theta) + integral(&f, theta, PI / 6.))
}
fn h_removed(p: &model::ResolvedPhillipsHParameters) -> Option<f64> {
    let ResolvedPhillipsInnerForm::ShallowV {
        angle_deg,
        e_width_mm: e,
    } = p.inner
    else {
        return None;
    };
    let alpha = (angle_deg / 2.).to_radians();
    let beta = p.tilt_angle_deg.to_radians();
    let a = p.outer_flank_angle_deg.to_radians();
    let c = p.floor_angle_deg.to_radians();
    let theta = (p.outer_corner_angle_deg / 2.).to_radians();
    let m = p.mouth_diameter_mm;
    let r = p.mouth_radius_mm;
    let b = p.b_width_mm;
    let tg = (m - p.g_diameter_mm) / (2. * a.tan());
    let total = tg + p.g_diameter_mm / 2. * c.tan();
    let tb = tg + (p.g_diameter_mm - b) / 2. * c.tan();
    let join = r * (1. - a.sin());
    let entry = m / 2. + r * (1. - a.sin()) / a.cos();
    let de = e / (2. * alpha.tan() * beta.cos());
    let first = 1. / (alpha.tan() * beta.cos());
    let second = 1. / (theta.tan() * beta.cos());
    let area = |depth: f64| {
        let rr = if depth <= join {
            entry - (r * r - (depth - r).powi(2)).max(0.).sqrt()
        } else if depth <= tg {
            m / 2. - depth * a.tan()
        } else {
            (total - depth).max(0.) / c.tan()
        };
        if rr == 0. {
            return 0.;
        }
        let rb = b / 2. + (tb - depth) * beta.tan();
        let primitive = |y: f64| {
            let y = y.clamp(0., rr);
            (y * (rr * rr - y * y).max(0.).sqrt() + rr * rr * (y / rr).asin()) / 2.
        };
        let part = |aa: f64, bb: f64, lo: f64, upper: f64| {
            let disc = rr * rr * (1. + bb * bb) - aa * aa;
            if disc <= 0. {
                return 0.;
            }
            let hi = upper.min(rr).min((-aa * bb + disc.sqrt()) / (1. + bb * bb));
            if hi <= lo {
                0.
            } else {
                primitive(hi) - primitive(lo) - aa * (hi - lo) - bb * (hi * hi - lo * lo) / 2.
            }
        };
        let diagonal = rr / 2_f64.sqrt();
        let xmin = if diagonal <= e / 2. {
            rb + first * diagonal
        } else {
            rb + de + second * (diagonal - e / 2.)
        };
        assert!(xmin > diagonal, "Corner quadrants must not overlap");
        PI * rr * rr
            - 8. * (part(rb, first, 0., rr.min(e / 2.))
                + part(rb + de - second * e / 2., second, e / 2., rr))
    };
    Some(
        [0., join, tg, total]
            .windows(2)
            .map(|w| integral(&area, w[0], w[1]))
            .sum(),
    )
}
fn check_step(solid: &Solid, dir: &std::path::Path, index: usize) -> Solid {
    let path = dir.join(format!("case-{index}.step"));
    solid.write_step(&path).unwrap();
    let read = Solid::read_step(&path, GeometryPolicy::default()).unwrap();
    assert_eq!(read.solid_count().unwrap(), 1);
    let area = solid
        .faces()
        .unwrap()
        .iter()
        .map(|f| f.area_mm2)
        .sum::<f64>()
        + read
            .faces()
            .unwrap()
            .iter()
            .map(|f| f.area_mm2)
            .sum::<f64>();
    let v = solid.volume_mm3().unwrap();
    let rv = read.volume_mm3().unwrap();
    assert!((v - rv).abs() <= 1e-7 * area + 1e-7 * (v.abs() + rv.abs()));
    read
}

#[test]
fn real_head_lands_necks_and_hexalobular_or_hex_recesses_match_independent_volumes_and_step() {
    let hex = || ScrewDrive::HexSocket {
        across_flats: length(2.04),
        depth: length(1.04),
    };
    let cases = [
        recipe(pan(), torx(1.1), 3., 0.5),
        recipe(button(), hex(), 3., 0.5),
        recipe(button(), torx(1.01), 3., 0.5),
        recipe(csk(), hex(), 3., 0.5),
        recipe(csk(), torx(0.76), 3., 0.5),
        recipe(
            ScrewHead::SocketCap {
                diameter: length(5.68),
                height: length(3.),
            },
            torx(1.1),
            3.,
            0.5,
        ),
    ];
    let dir = tempfile::tempdir().unwrap();
    for (index, p) in cases.into_iter().enumerate() {
        let mut s = session();
        let r = add(&mut s, p);
        let resolved = r.parameters.resolve(s.parameters()).unwrap();
        let ResolvedMechanicalPart::MachineScrew {
            head,
            drive,
            thread,
            shank_radius_mm,
            underhead_length_mm,
            ..
        } = resolved
        else {
            unreachable!()
        };
        let plain = underhead_length_mm - thread.length_mm;
        let body = body_volume(&head, shank_radius_mm, plain)
            + thread_area(thread.nominal_major_radius_mm * 2., thread.pitch_mm) * thread.length_mm;
        let removed = match drive {
            ResolvedScrewDrive::HexSocket {
                across_flats_mm,
                depth_mm,
            } => 3_f64.sqrt() / 2. * across_flats_mm.powi(2) * depth_mm,
            ResolvedScrewDrive::Hexalobular {
                major_radius_mm,
                minor_radius_mm,
                outer_lobe_radius_mm,
                depth_mm,
                bottom_angle_deg,
            } => torx_removed(
                major_radius_mm,
                minor_radius_mm,
                outer_lobe_radius_mm,
                depth_mm,
                bottom_angle_deg,
            ),
            _ => unreachable!(),
        };
        let model = compiler().compile(s.document(), s.revision()).unwrap();
        let solid = model.solid(r.feature).unwrap();
        let v = solid.volume_mm3().unwrap();
        let area = solid
            .faces()
            .unwrap()
            .iter()
            .map(|f| f.area_mm2)
            .sum::<f64>();
        assert!(
            (v - (body - removed)).abs() <= 1e-7 * area + 1e-7 * (v.abs() + (body - removed).abs()),
            "case{index}: native{v} expected{}",
            body - removed
        );
        let read = check_step(solid, dir.path(), index);
        for actual in [solid, &read] {
            probe(actual, [0., 0., -head.height_mm() + 0.01], false);
            probe(actual, [0., 0., -head.height_mm() + 0.3], false);
            probe(actual, [shank_radius_mm - 0.05, 0., plain - 0.1], true);
            let entry = head.top_land_radius_mm();
            probe(
                actual,
                [
                    (entry + drive.entry_radius_mm()) / 2.,
                    0.,
                    -head.height_mm() + 0.005,
                ],
                true,
            );
        }
        if matches!(drive, ResolvedScrewDrive::Hexalobular { .. }) {
            let snapshot = solid.topology_snapshot(r.feature, s.revision()).unwrap();
            for sector in 0..6 {
                for curve in ["lobe", "valley"] {
                    let role = format!("screw.hexalobular/{curve}/{sector}");
                    assert!(
                        snapshot
                            .elements
                            .iter()
                            .filter(|e| e.key.kind == TopologyKind::Face)
                            .any(|e| e.origins.iter().any(|o| o.role == role)),
                        "Missing named actual cavity face {role}"
                    );
                }
            }
        }
        let topology = solid.topology_snapshot(r.feature, s.revision()).unwrap();
        assert!(
            topology
                .elements
                .iter()
                .filter(|e| e.key.kind == TopologyKind::Face)
                .all(|e| !e.origins.is_empty())
        );
        assert!(
            topology
                .elements
                .iter()
                .flat_map(|e| &e.origins)
                .any(|o| o.role == "screw.head/top.land" || o.role.starts_with("screw.head/"))
        );
    }
}

#[test]
fn true_cc_phillips_h0_through_h4_preserve_real_wings_rounding_depth_and_step() {
    let cases = [
        recipe(
            ScrewHead::Pan {
                profile: Box::new(model::CrownedScrewHeadParameters {
                    diameter: length(4.),
                    height: length(1.6),
                    crown_radius: length(3.2),
                    top_land_diameter: length(2.5),
                    neck_radius: length(0.1),
                }),
            },
            h(
                PhillipsHSize::H0,
                [1.9, 0.3, 0.61, 0.81],
                PhillipsInnerForm::Rounded {
                    radius: length(0.25),
                },
                7.,
            ),
            2.,
            0.4,
        ),
        recipe(pan(), h1(3.), 3., 0.5),
        recipe(csk(), h1(3.2), 3., 0.5),
        recipe(
            ScrewHead::Pan {
                profile: Box::new(model::CrownedScrewHeadParameters {
                    diameter: length(8.),
                    height: length(3.1),
                    crown_radius: length(6.5),
                    top_land_diameter: length(6.),
                    neck_radius: length(0.2),
                }),
            },
            h(
                PhillipsHSize::H2,
                [4.4, 0.6, 1.455, 2.29],
                PhillipsInnerForm::ShallowV {
                    angle: angle(140.),
                    e_width: length(0.815),
                },
                5.75,
            ),
            4.,
            0.7,
        ),
        recipe(
            ScrewHead::Pan {
                profile: Box::new(model::CrownedScrewHeadParameters {
                    diameter: length(12.),
                    height: length(4.6),
                    crown_radius: length(10.),
                    top_land_diameter: length(8.4),
                    neck_radius: length(0.25),
                }),
            },
            h(
                PhillipsHSize::H3,
                [6.9, 0.8, 2.395, 3.81],
                PhillipsInnerForm::ShallowV {
                    angle: angle(146.),
                    e_width: length(2.005),
                },
                5.75,
            ),
            6.,
            1.,
        ),
        recipe(
            ScrewHead::Pan {
                profile: Box::new(model::CrownedScrewHeadParameters {
                    diameter: length(16.),
                    height: length(6.),
                    crown_radius: length(13.),
                    top_land_diameter: length(10.8),
                    neck_radius: length(0.4),
                }),
            },
            h(
                PhillipsHSize::H4,
                [9., 1., 3.465, 5.08],
                PhillipsInnerForm::ShallowV {
                    angle: angle(153.),
                    e_width: length(2.415),
                },
                7.,
            ),
            8.,
            1.25,
        ),
    ];
    let dir = tempfile::tempdir().unwrap();
    for (index, p) in cases.into_iter().enumerate() {
        let mut s = session();
        let r = add(&mut s, p);
        let ResolvedMechanicalPart::MachineScrew {
            head,
            drive,
            thread,
            underhead_length_mm,
            shank_radius_mm,
            ..
        } = r.parameters.resolve(s.parameters()).unwrap()
        else {
            unreachable!()
        };
        let ResolvedScrewDrive::PhillipsH { profile: p } = &drive else {
            unreachable!()
        };
        let model = compiler().compile(s.document(), s.revision()).unwrap();
        let solid = model.solid(r.feature).unwrap();
        let read = check_step(solid, dir.path(), index);
        let top = -head.height_mm();
        let tg = (p.mouth_diameter_mm - p.g_diameter_mm)
            / (2. * p.outer_flank_angle_deg.to_radians().tan());
        let tip = top + tg + p.g_diameter_mm / 2. * p.floor_angle_deg.to_radians().tan();
        for actual in [solid, &read] {
            for depth in [0.35 * tg, 0.65 * tg] {
                let rho =
                    p.mouth_diameter_mm / 2. - depth * p.outer_flank_angle_deg.to_radians().tan();
                for i in 0..4 {
                    let t = PI / 4. + i as f64 * PI / 2.;
                    probe(
                        actual,
                        [0.85 * rho * t.cos(), 0.85 * rho * t.sin(), top + depth],
                        false,
                    );
                    let t = i as f64 * PI / 2.;
                    probe(
                        actual,
                        [0.85 * rho * t.cos(), 0.85 * rho * t.sin(), top + depth],
                        true,
                    );
                }
            }
            probe(actual, [0., 0., tip - 0.03], false);
            probe(actual, [0., 0., tip + 0.03], true);
        }
        let snapshot = solid.topology_snapshot(r.feature, s.revision()).unwrap();
        let rounded = matches!(p.inner, ResolvedPhillipsInnerForm::Rounded { .. });
        for corner in 0..4 {
            for curve in if rounded {
                vec!["inner.round"]
            } else {
                vec!["inner.upper", "inner.lower"]
            } {
                let role = format!("screw.phillips.corner.{corner}/{curve}");
                assert!(
                    snapshot
                        .elements
                        .iter()
                        .filter(|e| e.key.kind == TopologyKind::Face)
                        .any(|e| e.origins.iter().any(|o| o.role == role)),
                    "Missing physical C-C curve ancestry {role}"
                );
            }
        }
        let forbidden = if rounded {
            "inner.upper"
        } else {
            "inner.round"
        };
        assert!(
            !snapshot
                .elements
                .iter()
                .flat_map(|e| &e.origins)
                .any(|o| o.role.ends_with(forbidden)),
            "Changing the inner construction must not alias its old face role"
        );
        if let Some(removed) = h_removed(p) {
            let expected = body_volume(
                &head,
                shank_radius_mm,
                underhead_length_mm - thread.length_mm,
            ) + thread_area(thread.nominal_major_radius_mm * 2., thread.pitch_mm)
                * thread.length_mm
                - removed;
            let v = solid.volume_mm3().unwrap();
            let area = solid
                .faces()
                .unwrap()
                .iter()
                .map(|f| f.area_mm2)
                .sum::<f64>();
            assert!(
                (v - expected).abs() <= 1e-7 * area + 1e-7 * (v.abs() + expected.abs()),
                "Hprofile{index}: native{v}, independent{expected}"
            );
        }
        assert_eq!(
            model::read_mechanical_part(s.document(), r.feature)
                .unwrap()
                .parameters,
            r.parameters
        );
    }
}

#[test]
fn changed_profile_inputs_invalidate_native_cache_with_identity_history_and_independent_reopen() {
    let mut s = session();
    let r = add(&mut s, recipe(pan(), torx(1.1), 3., 0.5));
    let mut cache = EvaluationCache::default();
    let first = compiler()
        .compile_cached(&mut cache, s.document(), s.revision(), None)
        .unwrap();
    let before = s.document().clone();
    let mut p = r.parameters.clone();
    let MechanicalPartParameters::MachineScrew {
        head: ScrewHead::Pan { profile: hp },
        drive: ScrewDrive::Hexalobular { profile: dp },
        ..
    } = &mut p
    else {
        unreachable!()
    };
    hp.crown_radius = length(5.1);
    hp.top_land_diameter = length(4.2);
    hp.neck_radius = length(0.11);
    dp.outer_lobe_radius = length(0.27);
    dp.depth = length(1.12);
    dp.bottom_angle = angle(118.);
    let preview =
        model::prepare_mechanical_part_edit(&s, s.revision(), actor(), r.feature, &p).unwrap();
    let candidate = compiler()
        .compile(preview.prepared.document(), s.revision() + 1)
        .unwrap();
    assert_ne!(
        candidate.solid(r.feature).unwrap().volume_mm3().unwrap(),
        first.solid(r.feature).unwrap().volume_mm3().unwrap()
    );
    assert_eq!(s.document(), &before);
    s.commit(preview.prepared).unwrap();
    compiler()
        .compile_cached(&mut cache, s.document(), s.revision(), None)
        .unwrap();
    assert!(cache.last_compile().evaluated.contains(&r.feature));
    assert_eq!(
        model::read_mechanical_part(s.document(), r.feature)
            .unwrap()
            .output
            .body,
        r.body
    );
    let after = s.document().clone();
    s.undo(s.revision(), actor()).unwrap();
    assert_eq!(s.document(), &before);
    s.redo(s.revision(), actor()).unwrap();
    assert_eq!(s.document(), &after);
    let mut bytes = vec![];
    project_file::write(
        &ProjectFile::new(after.clone()).unwrap(),
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
    assert_eq!(reopened.document, after);
    let independent = compiler().compile(&reopened.document, 0).unwrap();
    assert_eq!(
        independent.solid(r.feature).unwrap().volume_mm3().unwrap(),
        candidate.solid(r.feature).unwrap().volume_mm3().unwrap()
    );
}

#[test]
fn native_profile_minimum_feature_rejects_thin_annulus_and_floor_but_allows_countersunk_neck_depth()
{
    let minimum = 0.01;
    let mut thin_land = recipe(pan(), torx(1.1), 3., 0.5);
    let MechanicalPartParameters::MachineScrew {
        head: ScrewHead::Pan { profile },
        ..
    } = &mut thin_land
    else {
        unreachable!()
    };
    profile.top_land_diameter = length(2.81); //0.005mm actual annulus, below native0.01mm policy.
    let mut thin_floor = recipe(pan(), torx(1.1), 3., 0.5);
    let MechanicalPartParameters::MachineScrew {
        drive: ScrewDrive::Hexalobular { profile },
        ..
    } = &mut thin_floor
    else {
        unreachable!()
    };
    profile.major_diameter = length(1.8);
    profile.minor_diameter = length(1.3);
    profile.outer_lobe_radius = length(0.18);
    profile.depth = length(2.4 + 6. - 0.005 - 0.9 / 3_f64.sqrt());
    let strict = Compiler {
        policy: GeometryPolicy {
            minimum_feature_mm: minimum,
            ..Default::default()
        },
        omit_display: true,
        ..Default::default()
    };
    for p in [thin_land, thin_floor] {
        let mut s = session();
        let r = add(&mut s, p);
        let before = s.document().clone();
        // Canonical source remains a valid positive-clearance configured recipe.
        assert!(r.parameters.resolve(s.parameters()).is_ok());
        let error = strict.compile(s.document(), s.revision()).err().expect(
            "Configured positive clearance below the native policy must not produce geometry",
        );
        assert!(error.to_string().contains("minimum feature"), "{error}");
        assert_eq!(s.document(), &before);
    }
    let mut s = session();
    let r = add(&mut s, recipe(csk(), h1(3.2), 3., 0.5));
    let ResolvedMechanicalPart::MachineScrew { head, drive, .. } =
        r.parameters.resolve(s.parameters()).unwrap()
    else {
        unreachable!()
    };
    assert!(
        drive.removed_depth_mm() > head.height_mm(),
        "Countercase must genuinely reach into the nominal neck"
    );
    let model = strict.compile(s.document(), s.revision()).unwrap();
    let solid = model.solid(r.feature).unwrap();
    let tip = -head.height_mm() + drive.removed_depth_mm();
    probe(solid, [0., 0., tip - 0.03], false);
    probe(solid, [0., 0., tip + 0.03], true);
}
