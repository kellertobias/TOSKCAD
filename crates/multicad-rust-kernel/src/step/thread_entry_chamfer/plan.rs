//! Stage the full source boundary before mutating a compound writer.
use super::*;

pub(crate) struct Group {
    pub trims: Vec<Trim>,
    pub wall: usize,
    pub cone: usize,
    pub active: bool,
}
pub(crate) struct Plan {
    pub source: helical::ThreadSplines,
    pub cone_angle: f64,
    pub groups: Vec<Group>,
    pub points: HashMap<(u32, u64), Point>,
}

pub(super) fn key(body: &Chamfer, start: u32, phase: f64) -> (u32, u64) {
    if phase == body.source_thread().spec().pitch_mm {
        (
            (start + 1) % body.source_thread().spec().starts,
            0_f64.to_bits(),
        )
    } else {
        (start, phase.to_bits())
    }
}

fn qualify_trim(body: &Chamfer, trim: &Trim, tolerance: f64) -> Result<(), Error> {
    if trim.error_bound(body)? > tolerance / 16. {
        Err(Error::Unsupported)
    } else {
        Ok(())
    }
}
fn cone_angle(body: &Chamfer, tolerance: f64) -> Result<f64, Error> {
    let r = body.source_thread().spec().major_radius_mm;
    let r0 = body.entry_radius_mm();
    let c = body.entry_length_mm();
    let angle = ((r - r0) / c).atan();
    if !(angle > 0. && angle < std::f64::consts::FRAC_PI_2) {
        return Err(Error::Unsupported);
    }
    let angle_i = trim::I::point(angle);
    let denominator = trim::I::point(angle.cos());
    let exact = (trim::I::point(r) - trim::I::point(r0)) / trim::I::point(c);
    let error = trim::I::point(2.)
        * trim::I::point(c)
        * ((angle_i.sin() / denominator - exact).abs()
            + (angle_i.cos() / denominator - trim::I::point(1.)).abs());
    if error.checked()?.hi > tolerance / 128. {
        return Err(Error::Unsupported);
    }
    Ok(angle)
}
pub(super) fn prepare(
    body: &Chamfer,
    placement: &Placement,
    tolerance: f64,
    interrupt: Option<&dyn Fn() -> bool>,
) -> Result<Plan, Error> {
    if !tolerance.is_finite() || tolerance <= 0. {
        return Err(Error::InvalidGeometry);
    }
    if interrupt.is_some_and(|callback| callback()) {
        return Err(Error::Cancelled);
    }
    let tolerance = tolerance.min(1e-7);
    let cone_angle = cone_angle(body, tolerance)?;
    let source = helical::prepare(
        body.source_thread(),
        helical::Closure::Shaft,
        placement.origin,
        tolerance / 4.,
    )?;
    let trims = trim::prepare(
        body,
        QueryOptions {
            tolerance: tolerance / 64.,
            maximum_cells: crate::step_read::MAX_STEP_ENTITIES / 64,
        },
        interrupt,
    )?;
    let count = body.source_thread().spec().starts;
    let mut points = HashMap::new();
    for trim in &trims {
        for (q, point) in trim.phase.into_iter().zip([trim.poles[0], trim.poles[3]]) {
            points.entry(key(body, trim.start, q)).or_insert(point);
        }
    }
    let mut groups: Vec<Group> = Vec::new();
    for trim in trims {
        if interrupt.is_some_and(|callback| callback()) {
            return Err(Error::Cancelled);
        }
        let ends = trim.phase.map(|q| points[&key(body, trim.start, q)]);
        let wall = match trim.band {
            0 => 4 * ((trim.start + count - 1) % count) as usize + 3,
            1 => 4 * trim.start as usize,
            2 => 4 * trim.start as usize + 1,
            3 => 4 * trim.start as usize + 2,
            _ => 4 * trim.start as usize + 3,
        };
        let period = if trim.band == 0 && trim.start == 0 {
            count
        } else {
            trim.start
        };
        let band = &source.bands[wall];
        let domain = [band.knots[0], *band.knots.last().expect("validated spline")];
        let trim = trim
            .with_endpoints(body, ends)?
            .surface_branch(body, period, domain)?;
        qualify_trim(body, &trim, tolerance)?;
        let cone = 2 * trim.start as usize + usize::from(trim.half == ConeHalf::Falling);
        if let Some(group) = groups.last_mut()
            && group.wall == wall
            && group.cone == cone
            && group.active == trim.active
            && group.trims.len() < 1024
            && group.trims.last().expect("nonempty group").phase[1] == trim.phase[0]
        {
            group.trims.push(trim);
        } else {
            groups.push(Group {
                wall,
                cone,
                active: trim.active,
                trims: vec![trim],
            });
        }
    }
    Ok(Plan {
        source,
        cone_angle,
        groups,
        points,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn staged_boundary_keeps_source_face_roles_and_shared_periodic_vertices() {
        for left in [false, true] {
            for rounded in [false, true] {
                let root = rounded.then_some(1. / (4. * 3_f64.sqrt()));
                let source = crate::helical_thread::Thread::new(crate::helical_thread::Spec {
                    major_radius_mm: 3.,
                    pitch_mm: 1.,
                    length_mm: 6.6,
                    depth_mm: root
                        .map_or(5. * 3_f64.sqrt() / 16., |r| 0.875 * 3_f64.sqrt() / 2. - r),
                    crest_width_mm: 0.125,
                    lower_flank_deg: 30.,
                    upper_flank_deg: 30.,
                    starts: 2,
                    left_hand: left,
                    root_radius_mm: root,
                })
                .unwrap();
                for fraction in [0.1, 1., 1.2] {
                    let body =
                        Chamfer::new(source, fraction * source.spec().depth_mm, None).unwrap();
                    let p = Placement::new([0.; 3], [1., 0., 0.], [0., 1., 0.]).unwrap();
                    let plan = prepare(&body, &p, 1e-7, None).unwrap();
                    assert_eq!(plan.source.bands.len(), 8);
                    let mut wall = [false; 8];
                    let mut cone = [false; 4];
                    for group in plan.groups {
                        wall[group.wall] = true;
                        if group.active {
                            cone[group.cone] = true;
                        }
                        for trim in group.trims {
                            assert!(trim.error_bound(&body).unwrap() <= 1e-7 / 16.);
                            assert_eq!(
                                trim.poles[0],
                                plan.points[&key(&body, trim.start, trim.phase[0])]
                            );
                            assert_eq!(
                                trim.poles[3],
                                plan.points[&key(&body, trim.start, trim.phase[1])]
                            );
                        }
                    }
                    assert_eq!(wall, [true; 8]);
                    assert_eq!(cone, [true; 4]);
                    assert_eq!(key(&body, 1, source.spec().pitch_mm), key(&body, 0, 0.));
                }
            }
        }
    }
}

#[cfg(test)]
mod cancellation_tests {
    use super::*;
    #[test]
    fn cancellation_during_source_and_staged_trim_work_never_returns_a_partial_plan() {
        let thread = crate::helical_thread::Thread::new(crate::helical_thread::Spec {
            major_radius_mm: 3.,
            pitch_mm: 1.,
            length_mm: 6.6,
            depth_mm: 0.5,
            crest_width_mm: 0.125,
            lower_flank_deg: 30.,
            upper_flank_deg: 30.,
            starts: 1,
            left_hand: false,
            root_radius_mm: None,
        })
        .unwrap();
        let body = Chamfer::new(thread, 0.4, None).unwrap();
        let placement = Placement::new([0.; 3], [1., 0., 0.], [0., 1., 0.]).unwrap();
        assert!(matches!(
            prepare(&body, &placement, 1e-7, Some(&|| true)),
            Err(Error::Cancelled)
        ));
        let calls = std::cell::Cell::new(0usize);
        let stop = || {
            calls.set(calls.get() + 1);
            calls.get() == 2
        };
        assert!(matches!(
            prepare(&body, &placement, 1e-7, Some(&stop)),
            Err(Error::Cancelled)
        ));
        let count = std::cell::Cell::new(0usize);
        let count_steps = || {
            count.set(count.get() + 1);
            false
        };
        trim::prepare(
            &body,
            QueryOptions {
                tolerance: 1e-7 / 64.,
                maximum_cells: crate::step_read::MAX_STEP_ENTITIES / 64,
            },
            Some(&count_steps),
        )
        .unwrap();
        let after = count.get() + 2;
        calls.set(0);
        let stop = || {
            calls.set(calls.get() + 1);
            calls.get() == after
        };
        assert!(matches!(
            prepare(&body, &placement, 1e-7, Some(&stop)),
            Err(Error::Cancelled)
        ));
        assert!(prepare(&body, &placement, 1e-7, None).is_ok());
    }
}

#[cfg(test)]
mod sharing_tests {
    use super::*;
    #[test]
    fn shared_endpoint_displacement_is_requalified_before_publishing_a_source_plan() {
        let thread = crate::helical_thread::Thread::new(crate::helical_thread::Spec {
            major_radius_mm: 3.,
            pitch_mm: 1.,
            length_mm: 6.6,
            depth_mm: 0.5,
            crest_width_mm: 0.125,
            lower_flank_deg: 30.,
            upper_flank_deg: 30.,
            starts: 1,
            left_hand: false,
            root_radius_mm: None,
        })
        .unwrap();
        let body = Chamfer::new(thread, 0.4, None).unwrap();
        let original = trim::prepare(
            &body,
            QueryOptions {
                tolerance: 1e-7 / 64.,
                maximum_cells: 100_000,
            },
            None,
        )
        .unwrap()
        .remove(0);
        qualify_trim(&body, &original, 1e-7).unwrap();
        let mut endpoints = [original.poles[0], original.poles[3]];
        for p in &mut endpoints {
            p[0] += 1e-6;
        }
        let shifted = original.with_endpoints(&body, endpoints).unwrap();
        assert!(matches!(
            qualify_trim(&body, &shifted, 1e-7),
            Err(Error::Unsupported)
        ));
    }
}

#[cfg(test)]
mod angle_tests {
    use super::*;
    #[test]
    fn finite_source_cones_that_round_to_a_degenerate_step_semiangle_are_not_published() {
        let source = crate::helical_thread::Thread::new(crate::helical_thread::Spec {
            major_radius_mm: 3.,
            pitch_mm: 1.,
            length_mm: 6.6,
            depth_mm: 0.5,
            crest_width_mm: 0.125,
            lower_flank_deg: 30.,
            upper_flank_deg: 30.,
            starts: 1,
            left_hand: false,
            root_radius_mm: None,
        })
        .unwrap();
        for c in [1e-300, 1e-15] {
            let body = Chamfer::from_material(source, 2.6, c, None).unwrap();
            assert!(matches!(cone_angle(&body, 1e-7), Err(Error::Unsupported)));
        }
        assert!(cone_angle(&Chamfer::new(source, 0.4, None).unwrap(), 1e-7).is_ok());
    }
}
