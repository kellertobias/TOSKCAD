//! Independently infer an authored one-start thread from its complete spline
//! shell, then prove every Bernstein net and shared physical boundary.
use super::*;
use crate::{
    helical_thread::{Spec, Thread},
    step::helical::{Closure, DEGREE},
};
use helical::{face, require};
use std::f64::consts::TAU;

pub(super) fn read_member(
    shell: &mut Shell<'_, '_>,
    solid: u32,
    faces: &[u32],
) -> Result<Option<(Readback, ImportSolid)>> {
    if !matches!(faces.len(), 6 | 7) {
        return Ok(None);
    }
    let first = face(shell, faces[0])?;
    let entity = shell.simple(first.0)?;
    if entity.name != "B_SPLINE_SURFACE_WITH_KNOTS"
        || entity.args.get(1..3)
            != Some(&[Value::Number(DEGREE as f64), Value::Number(DEGREE as f64)])
    {
        return Ok(None);
    }
    let mut grids = Vec::with_capacity(4);
    let mut radial = Vec::with_capacity(4);
    let mut axial = None;
    for &id in &faces[..4] {
        let (id, _, _) = face(shell, id)?;
        let args = shell.args(id, "B_SPLINE_SURFACE_WITH_KNOTS")?;
        let [
            Value::String(_),
            Value::Number(a),
            Value::Number(b),
            Value::List(rows),
            _,
            _,
            _,
            _,
            _,
            _,
            r,
            z,
            _,
        ] = args
        else {
            return Err(ReadError::Invalid);
        };
        require(*a == DEGREE as f64 && *b == DEGREE as f64 && rows.len() > DEGREE)?;
        radial.push(domain(r)?);
        let domain = domain(z)?;
        if let Some(previous) = axial {
            require(previous == domain)?;
        } else {
            axial = Some(domain);
        }
        grids.push(rows);
    }
    let plane = |id| -> Result<Plane> {
        let (surface, forward, _) = face(shell, id)?;
        require(forward)?;
        let [Value::String(_), axis] = shell.args(surface, "PLANE")? else {
            return Err(ReadError::Invalid);
        };
        shell.placement(reference(axis)?)
    };
    let bottom = plane(faces[4])?;
    let top = plane(faces[5])?;
    let origin = bottom.origin;
    let u = bottom.u;
    let v = bottom.v.map(|n| -n);
    let n = bottom.normal.map(|n| -n);
    let height = dot(sub(top.origin, origin), n);
    require(height.is_finite() && height > shell.tolerance)?;
    require(
        length(sub(
            top.origin,
            std::array::from_fn(|i| origin[i] + n[i] * height),
        )) <= shell.tolerance / 16.,
    )?;
    let [start, spin] = axial.ok_or(ReadError::Invalid)?;
    require(start == 0. && spin > 0.)?;
    let rate = height / spin;
    let pitch = TAU * rate;
    let radius = |band: usize, last: bool| -> Result<f64> {
        let rows = grids[band];
        let row = &rows[if last { rows.len() - 1 } else { 0 }];
        let ids = refs(row)?;
        let point = shell.point(*ids.first().ok_or(ReadError::Invalid)?)?;
        Ok(length(sub(point, origin)))
    };
    let major = radius(3, false)?;
    let upper_radius = radius(0, true)?;
    let lower_radius = radius(2, false)?;
    let phase_start = radial[0][0] * rate;
    let phase_upper = radial[0][1] * rate;
    let phase_lower = radial[2][0] * rate;
    let phase_crest = radial[2][1] * rate;
    let upper_run = (phase_upper - phase_start) / (major - upper_radius);
    let lower_run = (phase_crest - phase_lower) / (major - lower_radius);
    let (depth, root_radius) = if radial[1][0] < 0. {
        let [a, b] = radial[1];
        require(
            a < 0. && b > 0. && a > -std::f64::consts::FRAC_PI_2 && b < std::f64::consts::FRAC_PI_2,
        )?;
        let f = (phase_lower - phase_upper) / (b.sin() - a.sin());
        let center = upper_radius + f * a.cos();
        (major - center + f, Some(f))
    } else {
        (major - upper_radius, None)
    };
    let stock = if faces.len() == 7 {
        let (surface, outward, _) = face(shell, faces[6])?;
        require(outward)?;
        let [Value::String(_), _, Value::Number(radius)] =
            shell.args(surface, "CYLINDRICAL_SURFACE")?
        else {
            return Err(ReadError::Invalid);
        };
        Some(*radius)
    } else {
        None
    };
    let thread = Thread::new(Spec {
        major_radius_mm: major,
        pitch_mm: pitch,
        length_mm: height,
        depth_mm: depth,
        crest_width_mm: 2. * phase_start,
        lower_flank_deg: lower_run.atan().to_degrees(),
        upper_flank_deg: upper_run.atan().to_degrees(),
        starts: 1,
        left_hand: first.1 != stock.is_some(),
        root_radius_mm: root_radius,
    })
    .map_err(|e| match e {
        crate::helical_thread::ThreadError::Invalid => ReadError::Invalid,
        crate::helical_thread::ThreadError::Limit => ReadError::Limit,
    })?;
    // Clamped C0 degree-11 segments are Bézier patches. Per-control world
    // residual bounds the complete patch under piecewise affine chart maps;
    // independent knot lengths do not change a segment's geometric image.
    // Exact edge/surface control equality and shared vertices remain mandatory.
    let volume = helical::verify_member(
        shell,
        solid,
        &thread,
        Closure::stock(stock),
        [origin, u, v],
        Some(shell.tolerance.min(1e-7) / 16.),
    )?;
    let support = |direction: Point| {
        stock.map_or_else(
            || thread.support(direction),
            |radius| radius * direction[0].hypot(direction[1]) + (height * direction[2]).max(0.),
        )
    };
    let bounds = [-1., 1.].map(|sign| {
        std::array::from_fn(|axis| {
            let d = [u[axis] * sign, v[axis] * sign, n[axis] * sign];
            origin[axis] + sign * support(d)
        })
    });
    let circles = stock.map_or_else(Vec::new, |radius| {
        [origin, top.origin]
            .map(|center_mm| CircleEdge {
                center_mm,
                normal: n,
                radius_mm: radius,
            })
            .to_vec()
    });
    let readback = Readback {
        bounds_mm: bounds,
        volume_mm3: volume,
        face_count: faces.len(),
        edges_mm: vec![],
        circles,
        semicircles: vec![],
        arcs: vec![],
        ellipses: vec![],
        splines: vec![],
        solid_count: 1,
    };
    Ok(Some((
        readback,
        ImportSolid {
            origin_mm: origin,
            u,
            v,
            height_mm: height,
            profile: ImportProfile::HelicalThread {
                spec: thread.spec(),
                stock_radius_mm: stock,
            },
        },
    )))
}
/// Certify that a separately reconstructed import has the same bounded
/// analytic thread as an existing owned source. This compares complete world
/// Bernstein nets, not aggregate dimensions or sampled display geometry.
pub fn helical_import_matches(
    imported: &ImportSolid,
    thread: &Thread,
    stock: Option<f64>,
    [origin, u, v]: [Point; 3],
    tolerance: f64,
) -> bool {
    let frame_valid = |[origin, u, v]: [Point; 3]| {
        origin.into_iter().chain(u).chain(v).all(f64::is_finite)
            && (dot(u, u) - 1.).abs() <= 1e-10
            && (dot(v, v) - 1.).abs() <= 1e-10
            && dot(u, v).abs() <= 1e-10
    };
    if !frame_valid([origin, u, v]) || !frame_valid([imported.origin_mm, imported.u, imported.v]) {
        return false;
    }
    let ImportProfile::HelicalThread {
        spec,
        stock_radius_mm,
    } = imported.profile
    else {
        return false;
    };
    let Ok(other) = Thread::new(spec) else {
        return false;
    };
    if spec.length_mm != imported.height_mm
        || spec.left_hand != thread.spec().left_hand
        || spec.root_radius_mm.is_some() != thread.spec().root_radius_mm.is_some()
        || stock.is_some() != stock_radius_mm.is_some()
    {
        return false;
    }
    if stock
        .zip(stock_radius_mm)
        .is_some_and(|(a, b)| (a - b).abs() > tolerance / 16.)
    {
        return false;
    }
    if let Some((a, b)) = stock.zip(stock_radius_mm) {
        let na = cross(u, v);
        let nb = cross(imported.u, imported.v);
        let end_a = std::array::from_fn(|i| origin[i] + na[i] * thread.spec().length_mm);
        let end_b = std::array::from_fn(|i| imported.origin_mm[i] + nb[i] * spec.length_mm);
        let closure_error = length(sub(origin, imported.origin_mm)).max(length(sub(end_a, end_b)))
            + a.max(b) * (length(sub(u, imported.u)) + length(sub(v, imported.v)))
            + (a - b).abs() * (length(imported.u) + length(imported.v));
        if !closure_error.is_finite() || closure_error > tolerance / 8. {
            return false;
        }
    }
    let Ok(a) = crate::step::helical::prepare(thread, Closure::stock(stock), origin, tolerance)
    else {
        return false;
    };
    let Ok(b) = crate::step::helical::prepare(
        &other,
        Closure::stock(stock_radius_mm),
        imported.origin_mm,
        tolerance,
    ) else {
        return false;
    };
    let world = |origin: Point, u: Point, v: Point, radial: Point2, axial: Point2, z: f64| {
        let p = crate::step::helical::product(radial, axial);
        let n = cross(u, v);
        std::array::from_fn::<_, 3, _>(|i| origin[i] + u[i] * p[0] + v[i] * p[1] + n[i] * z)
    };
    if a.axial.poles.len() != b.axial.poles.len() {
        return false;
    }
    for (a_band, b_band) in a.bands.iter().zip(&b.bands) {
        if a_band.poles.len() != b_band.poles.len() {
            return false;
        }
        for (&ra, &rb) in a_band.poles.iter().zip(&b_band.poles) {
            for ((&aa, &az), (&ba, &bz)) in a
                .axial
                .poles
                .iter()
                .zip(&a.heights)
                .zip(b.axial.poles.iter().zip(&b.heights))
            {
                let a = world(origin, u, v, ra, aa, az);
                let b = world(imported.origin_mm, imported.u, imported.v, rb, ba, bz);
                let error = length(sub(a, b));
                if !error.is_finite() || error > tolerance / 8. {
                    return false;
                }
            }
        }
    }
    true
}

fn domain(value: &Value<'_>) -> Result<[f64; 2]> {
    let Value::List(values) = value else {
        return Err(ReadError::Invalid);
    };
    let [Some(Value::Number(a)), Some(Value::Number(b))] = [values.first(), values.last()] else {
        return Err(ReadError::Invalid);
    };
    require(a.is_finite() && b.is_finite() && b > a)?;
    Ok([*a, *b])
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reconstructs_flat_rounded_metric_and_custom_threads_from_actual_nets() {
        for (origin, u, v) in [
            ([0.; 3], [1., 0., 0.], [0., 1., 0.]),
            ([17., -8., 13.], [0.8, 0.6, 0.], [-0.36, 0.48, 0.8]),
        ] {
            for left_hand in [false, true] {
                for stock in [None, Some(6.)] {
                    for rounded in [false, true] {
                        let f = 1. / (4. * 3_f64.sqrt());
                        let crest = 0.13;
                        let spec = Spec {
                            major_radius_mm: 3.,
                            pitch_mm: 1.,
                            length_mm: 4.2,
                            depth_mm: if rounded {
                                (1. - crest) * 3_f64.sqrt() / 2. - f
                            } else {
                                0.54
                            },
                            crest_width_mm: crest,
                            lower_flank_deg: 30.,
                            upper_flank_deg: 30.,
                            starts: 1,
                            left_hand,
                            root_radius_mm: rounded.then_some(f),
                        };
                        let thread = Thread::new(spec).unwrap();
                        let text =
                            crate::step::write_helical_thread(&thread, stock, origin, u, v, 1e-7)
                                .unwrap();
                        let (proof, imported) = read_import_solid(&text, 1e-7).unwrap();
                        assert!(
                            matches!(imported.profile,ImportProfile::HelicalThread{spec:actual,stock_radius_mm} if stock_radius_mm==stock&&actual.left_hand==left_hand&&actual.root_radius_mm.is_some()==rounded)
                        );
                        assert!(helical_import_matches(
                            &imported,
                            &thread,
                            stock,
                            [origin, u, v],
                            1e-7
                        ));
                        assert!(read_analytic_solid(&text, 1e-7).is_ok());
                        let expected = stock.map_or(thread.volume_mm3(), |r| {
                            std::f64::consts::PI * r * r * spec.length_mm - thread.volume_mm3()
                        });
                        assert!(
                            (proof.volume_mm3 - expected).abs() < 1e-5,
                            "{} {expected}",
                            proof.volume_mm3
                        );
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod rejection_tests {
    use super::*;
    fn thread() -> Thread {
        Thread::new(Spec {
            major_radius_mm: 3.,
            pitch_mm: 1.,
            length_mm: 4.2,
            depth_mm: 0.54,
            crest_width_mm: 0.13,
            lower_flank_deg: 30.,
            upper_flank_deg: 30.,
            starts: 1,
            left_hand: false,
            root_radius_mm: None,
        })
        .unwrap()
    }
    fn fixture(stock: Option<f64>) -> String {
        crate::step::write_helical_thread(
            &thread(),
            stock,
            [0.; 3],
            [1., 0., 0.],
            [0., 1., 0.],
            1e-7,
        )
        .unwrap()
    }
    fn render(v: &Value<'_>) -> String {
        match v {
            Value::String(s) => format!("'{s}'"),
            Value::Enum(s) => format!(".{s}."),
            Value::Number(n) => format!("{n:.16E}"),
            Value::Reference(n) => format!("#{n}"),
            Value::List(v) => format!("({})", v.iter().map(render).collect::<Vec<_>>().join(",")),
            Value::Omitted => "$".into(),
            Value::Derived => "*".into(),
            Value::Typed(e) => format!(
                "{}({})",
                e.name,
                e.args.iter().map(render).collect::<Vec<_>>().join(",")
            ),
        }
    }
    fn ids(text: &str, name: &str) -> Vec<u32> {
        parse_entities(text, 1e-7)
            .unwrap()
            .iter()
            .filter_map(|(&id, parts)| matches!(parts.as_slice(),[e] if e.name==name).then_some(id))
            .collect()
    }
    fn edit(text: &str, id: u32, change: impl FnOnce(&mut Entity<'_>)) -> String {
        let mut entities = parse_entities(text, 1e-7).unwrap();
        let e = &mut entities.get_mut(&id).unwrap()[0];
        change(e);
        let line = text
            .lines()
            .find(|line| line.starts_with(&format!("#{id}=")))
            .unwrap();
        text.replacen(
            line,
            &format!(
                "#{id}={}({});",
                e.name,
                e.args.iter().map(render).collect::<Vec<_>>().join(",")
            ),
            1,
        )
    }
    fn bad(text: &str) {
        assert!(read_import_solid(text, 1e-7).is_err());
        assert!(read_analytic_solid(text, 1e-7).is_err());
    }
    fn number_list(value: &Value<'_>) -> Vec<Value<'static>> {
        render(value)
            .strip_prefix('(')
            .unwrap()
            .strip_suffix(')')
            .unwrap()
            .split(',')
            .map(|n| Value::Number(n.parse::<f64>().unwrap()))
            .collect()
    }
    fn numeric_list(value: &Value<'_>) -> Vec<f64> {
        render(value)
            .strip_prefix('(')
            .unwrap()
            .strip_suffix(')')
            .unwrap()
            .split(',')
            .map(|n| n.parse().unwrap())
            .collect()
    }
    fn rows(value: &Value<'_>) -> Vec<Vec<u32>> {
        render(value)
            .strip_prefix("((")
            .unwrap()
            .strip_suffix("))")
            .unwrap()
            .split("),(")
            .map(|row| {
                row.split(',')
                    .map(|n| n.strip_prefix('#').unwrap().parse().unwrap())
                    .collect()
            })
            .collect()
    }
    #[test]
    fn malformed_domains_caps_supports_and_profiles_are_never_partly_imported() {
        let text = fixture(None);
        let surfaces = ids(&text, "B_SPLINE_SURFACE_WITH_KNOTS");
        bad(&edit(&text, surfaces[1], |e| e.args.truncate(1)));
        for value in [
            Value::Omitted,
            Value::Derived,
            Value::Typed(Entity {
                name: "UNPROVED",
                args: vec![Value::Number(0.)],
            }),
        ] {
            bad(&edit(&text, surfaces[0], |e| e.args[10] = value));
        }
        let planes = ids(&text, "PLANE");
        bad(&edit(&text, planes[0], |e| e.args.truncate(1)));
        bad(&edit(&text, surfaces[0], |e| {
            e.args[10] = Value::Number(0.)
        }));
        bad(&edit(&text, surfaces[0], |e| {
            e.args[10] = Value::List(vec![Value::String("bad")])
        }));
        bad(&edit(&text, surfaces[0], |e| e.args[8] = Value::Number(0.)));
        bad(&edit(&text, surfaces[0], |e| {
            let mut k = number_list(&e.args[11]);
            k[1] = Value::String("bad");
            e.args[11] = Value::List(k);
        }));
        bad(&edit(&text, surfaces[0], |e| {
            let mut k = number_list(&e.args[11]);
            k[1] = Value::Number(0.);
            e.args[11] = Value::List(k);
        }));
        bad(&edit(&text, surfaces[0], |e| {
            let mut k = number_list(&e.args[10]);
            k[0] = Value::Number(0.);
            e.args[10] = Value::List(k);
        }));
        let mut limit = text.clone();
        for &surface in &surfaces {
            limit = edit(&limit, surface, |e| {
                let mut k = numeric_list(&e.args[11]);
                *k.last_mut().unwrap() *= 100.;
                e.args[11] = Value::List(k.into_iter().map(Value::Number).collect());
            });
        }
        let parsed = parse_entities(&text, 1e-7).unwrap();
        let axis = reference(&parsed[&planes[1]][0].args[1]).unwrap();
        let center = reference(&parsed[&axis][0].args[1]).unwrap();
        limit = edit(&limit, center, |e| {
            let mut p = numbers(&e.args[1]).unwrap();
            p[2] = 420.;
            e.args[1] = Value::List(p.into_iter().map(Value::Number).collect());
        });
        assert_eq!(
            read_import_solid(&limit, 1e-7).unwrap_err(),
            ReadError::Limit
        );
        let stock = fixture(Some(6.));
        bad(&edit(&stock, ids(&stock, "CYLINDRICAL_SURFACE")[0], |e| {
            e.args.truncate(1)
        }));
        bad(&edit(&text, center, |e| {
            let mut p = numbers(&e.args[1]).unwrap();
            p[0] = 1.;
            e.args[1] = Value::List(p.into_iter().map(Value::Number).collect());
        }));
        let direction = reference(&parsed[&axis][0].args[3]).unwrap();
        bad(&edit(&text, direction, |e| {
            e.args[1] = Value::List([0., 1., 0.].into_iter().map(Value::Number).collect())
        }));
        let f = 1. / (4. * 3_f64.sqrt());
        let rounded = Thread::new(Spec {
            root_radius_mm: Some(f),
            depth_mm: (1. - 0.13) * 3_f64.sqrt() / 2. - f,
            ..thread().spec()
        })
        .unwrap();
        let rounded = crate::step::write_helical_thread(
            &rounded,
            None,
            [0.; 3],
            [1., 0., 0.],
            [0., 1., 0.],
            1e-7,
        )
        .unwrap();
        let root = ids(&rounded, "B_SPLINE_SURFACE_WITH_KNOTS")[1];
        bad(&edit(&rounded, root, |e| {
            let mut k = number_list(&e.args[10]);
            k[0] = Value::Number(-2.);
            e.args[10] = Value::List(k);
        }));
        assert!(read_import_solid(&text, 1e-7).is_ok());
    }
    #[test]
    fn entire_control_nets_and_exact_shared_boundaries_are_required() {
        let text = fixture(None);
        let parsed = parse_entities(&text, 1e-7).unwrap();
        let surface = ids(&text, "B_SPLINE_SURFACE_WITH_KNOTS")[0];
        let rows = rows(&parsed[&surface][0].args[3]);
        let interior = rows[1][1];
        bad(&edit(&text, interior, |e| {
            let mut p = numbers(&e.args[1]).unwrap();
            p[0] = 100.;
            e.args[1] = Value::List(p.into_iter().map(Value::Number).collect());
        }));
        let tiny = edit(&text, interior, |e| {
            let mut p = numbers(&e.args[1]).unwrap();
            p[0] += 1e-10;
            e.args[1] = Value::List(p.into_iter().map(Value::Number).collect());
        });
        assert!(read_import_solid(&tiny, 1e-7).is_ok());
        let boundary = rows[0][1];
        let clone_id = parsed.keys().next_back().unwrap() + 1;
        let shifted = edit(&text, boundary, |e| {
            let mut p = numbers(&e.args[1]).unwrap();
            p[0] += 1e-10;
            e.args[1] = Value::List(p.into_iter().map(Value::Number).collect());
        });
        let replacement = shifted
            .lines()
            .find(|line| line.starts_with(&format!("#{boundary}=")))
            .unwrap()
            .replacen(&format!("#{boundary}="), &format!("#{clone_id}="), 1);
        let extra = text.replacen(
            "ENDSEC;\nEND-ISO",
            &format!("{replacement}\nENDSEC;\nEND-ISO"),
            1,
        );
        bad(&edit(&extra, surface, |e| {
            let mut grid = rows
                .iter()
                .map(|row| {
                    row.iter()
                        .copied()
                        .map(Value::Reference)
                        .collect::<Vec<_>>()
                })
                .collect::<Vec<_>>();
            grid[0][1] = Value::Reference(clone_id);
            e.args[3] = Value::List(grid.into_iter().map(Value::List).collect());
        }));
        assert!(
            read_import_solid(
                &text.replace("MultiCAD", "deliberately misleading name"),
                1e-7
            )
            .is_ok()
        );
    }
    #[test]
    fn matching_uses_full_world_nets_and_refuses_wrong_shapes_or_invalid_contracts() {
        let text = fixture(None);
        let (_, original) = read_import_solid(&text, 1e-7).unwrap();
        let source = thread();
        let frame = [[0.; 3], [1., 0., 0.], [0., 1., 0.]];
        assert!(helical_import_matches(
            &original, &source, None, frame, 1e-7
        ));
        let mut wrong = original.clone();
        wrong.profile = ImportProfile::Cylinder { radius_mm: 3. };
        assert!(!helical_import_matches(&wrong, &source, None, frame, 1e-7));
        let mut wrong = original.clone();
        if let ImportProfile::HelicalThread { spec, .. } = &mut wrong.profile {
            spec.major_radius_mm = 0.;
        }
        assert!(!helical_import_matches(&wrong, &source, None, frame, 1e-7));
        let mut wrong = original.clone();
        wrong.height_mm += 1.;
        assert!(!helical_import_matches(&wrong, &source, None, frame, 1e-7));
        assert!(!helical_import_matches(
            &original,
            &source,
            Some(6.),
            frame,
            1e-7
        ));
        let (_, stock) = read_import_solid(&fixture(Some(6.)), 1e-7).unwrap();
        assert!(!helical_import_matches(
            &stock,
            &source,
            Some(6.1),
            frame,
            1e-7
        ));
        assert!(!helical_import_matches(
            &original, &source, None, frame, 1e-100
        ));
        let mut wrong = original.clone();
        if let ImportProfile::HelicalThread { spec, .. } = &mut wrong.profile {
            spec.starts = 2;
        }
        assert!(!helical_import_matches(&wrong, &source, None, frame, 1e-7));
        let mut wrong = original.clone();
        wrong.height_mm = 10.;
        if let ImportProfile::HelicalThread { spec, .. } = &mut wrong.profile {
            spec.length_mm = 10.;
        }
        assert!(!helical_import_matches(&wrong, &source, None, frame, 1e-7));
        let mut wrong = original.clone();
        if let ImportProfile::HelicalThread { spec, .. } = &mut wrong.profile {
            spec.crest_width_mm = 0.01;
            spec.depth_mm = 0.8;
            spec.lower_flank_deg = 31.5;
            spec.upper_flank_deg = 31.5;
        }
        assert!(!helical_import_matches(&wrong, &source, None, frame, 1e-7));
        let mut wrong = original.clone();
        wrong.origin_mm[0] += 0.01;
        assert!(!helical_import_matches(&wrong, &source, None, frame, 1e-7));
        let mut wrong = original.clone();
        wrong.u = [0.; 3];
        assert!(!helical_import_matches(&wrong, &source, None, frame, 1e-7));
    }
}

#[cfg(test)]
mod more_tests {
    use super::*;
    #[test]
    fn custom_asymmetric_flanks_and_large_stock_are_certified_in_full() {
        let (lower, upper) = (15_f64.to_radians(), 40_f64.to_radians());
        let pitch = 1.3;
        let crest = 0.2;
        let f = 0.08;
        let intersection = (pitch - crest) / (lower.tan() + upper.tan());
        let depth =
            intersection - f * (((upper - lower) / 2.).cos() / ((upper + lower) / 2.).sin() - 1.);
        for root in [None, Some(f)] {
            let source = Thread::new(Spec {
                major_radius_mm: 4.,
                pitch_mm: pitch,
                length_mm: 5.,
                depth_mm: if root.is_some() { depth } else { 0.3 },
                crest_width_mm: crest,
                lower_flank_deg: 15.,
                upper_flank_deg: 40.,
                starts: 1,
                left_hand: true,
                root_radius_mm: root,
            })
            .unwrap();
            let text = crate::step::write_helical_thread(
                &source,
                None,
                [0.; 3],
                [1., 0., 0.],
                [0., 1., 0.],
                1e-7,
            )
            .unwrap();
            let (_, imported) = read_import_solid(&text, 1e-7).unwrap();
            assert!(helical_import_matches(
                &imported,
                &source,
                None,
                [[0.; 3], [1., 0., 0.], [0., 1., 0.]],
                1e-7
            ));
        }
        let source = Thread::new(Spec {
            major_radius_mm: 3.,
            pitch_mm: 1.,
            length_mm: 4.2,
            depth_mm: 0.54,
            crest_width_mm: 0.13,
            lower_flank_deg: 30.,
            upper_flank_deg: 30.,
            starts: 1,
            left_hand: false,
            root_radius_mm: None,
        })
        .unwrap();
        let mut imports = vec![];
        for stock in [None, Some(1e5)] {
            let text = crate::step::write_helical_thread(
                &source,
                stock,
                [0.; 3],
                [1., 0., 0.],
                [0., 1., 0.],
                1e-7,
            )
            .unwrap();
            let (_, imported) = read_import_solid(&text, 1e-7).unwrap();
            assert!(helical_import_matches(
                &imported,
                &source,
                stock,
                [[0.; 3], [1., 0., 0.], [0., 1., 0.]],
                1e-7
            ));
            imports.push(imported);
        }
        let (s, c) = 5e-11_f64.sin_cos();
        for imported in &mut imports {
            imported.u = [c, 0., -s];
        }
        assert!(helical_import_matches(
            &imports[0],
            &source,
            None,
            [[0.; 3], [1., 0., 0.], [0., 1., 0.]],
            1e-7
        ));
        assert!(!helical_import_matches(
            &imports[1],
            &source,
            Some(1e5),
            [[0.; 3], [1., 0., 0.], [0., 1., 0.]],
            1e-7
        ));
    }
}
