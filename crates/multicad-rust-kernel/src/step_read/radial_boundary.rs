//! Actual finite boundary values; no authored thread parameters are inferred.
use super::*;
use crate::{radial_tensor as radial, tapered_thread::RetainedTensor};
use helical::{face, ring};

/// Decode one candidate boundary and prove its finite values and abstract
/// manifold ownership. This is not an occupied-solid import entry point.
pub fn read_radial_boundary(input: &str, tolerance: f64) -> Result<radial::BoundaryOwnership> {
    let mut members = compound::read_members_by(input, tolerance, true, true, |shell, id, _| {
        read_member(shell, id)
    })?;
    members.pop().ok_or(ReadError::Invalid)
}
fn failure(error: crate::tapered_thread::Error) -> ReadError {
    match error {
        crate::tapered_thread::Error::Limit => ReadError::Limit,
        crate::tapered_thread::Error::Invalid => ReadError::Invalid,
        _ => ReadError::Unsupported,
    }
}
fn axis(shell: &Shell<'_, '_>, id: u32) -> Result<radial::Axis> {
    let [Value::String(_), origin, normal, reference_axis] =
        shell.args(id, "AXIS2_PLACEMENT_3D")?
    else {
        return Err(ReadError::Invalid);
    };
    let vector = |id: &Value<'_>| -> Result<Point> {
        let [Value::String(_), values] = shell.args(reference(id)?, "DIRECTION")? else {
            return Err(ReadError::Invalid);
        };
        numbers(values)
    };
    Ok(radial::Axis {
        origin: shell.point(reference(origin)?)?,
        normal: vector(normal)?,
        reference: vector(reference_axis)?,
    })
}
fn curve(shell: &Shell<'_, '_>, id: u32) -> Result<radial::Curve> {
    match shell.simple(id)?.name {
        "CIRCLE" => {
            let [Value::String(_), a, Value::Number(r)] = shell.args(id, "CIRCLE")? else {
                return Err(ReadError::Invalid);
            };
            Ok(radial::Curve::Circle {
                axis: axis(shell, reference(a)?)?,
                radius_mm: *r,
            })
        }
        "B_SPLINE_CURVE_WITH_KNOTS" => {
            let [
                Value::String(_),
                Value::Number(d),
                poles,
                Value::Enum("UNSPECIFIED"),
                Value::Enum("F"),
                Value::Enum("F"),
                m,
                k,
                Value::Enum("UNSPECIFIED"),
            ] = shell.args(id, "B_SPLINE_CURVE_WITH_KNOTS")?
            else {
                return Err(ReadError::Invalid);
            };
            let ids = refs(poles)?;
            if *d != 9. || ids.len() < 10 || !(ids.len() - 1).is_multiple_of(9) {
                return Err(ReadError::Unsupported);
            }
            if ids.len() > 524288 {
                return Err(ReadError::Limit);
            }
            let spans = (ids.len() - 1) / 9;
            tapered_thread::knots(m, k, spans)?;
            let poles = ids
                .into_iter()
                .map(|id| shell.point(id))
                .collect::<Result<Vec<_>>>()?;
            Ok(radial::Curve::Spline { spans, poles })
        }
        _ => Err(ReadError::Unsupported),
    }
}
fn surface(shell: &Shell<'_, '_>, id: u32) -> Result<radial::Surface> {
    match shell.simple(id)?.name {
        "PLANE" => {
            let [Value::String(_), a] = shell.args(id, "PLANE")? else {
                return Err(ReadError::Invalid);
            };
            Ok(radial::Surface::Plane(axis(shell, reference(a)?)?))
        }
        "CYLINDRICAL_SURFACE" => {
            let [Value::String(_), a, Value::Number(r)] = shell.args(id, "CYLINDRICAL_SURFACE")?
            else {
                return Err(ReadError::Invalid);
            };
            Ok(radial::Surface::Cone {
                axis: axis(shell, reference(a)?)?,
                radius_mm: *r,
                semi_angle_rad: 0.,
            })
        }
        "CONICAL_SURFACE" => {
            let [Value::String(_), a, Value::Number(r), Value::Number(angle)] =
                shell.args(id, "CONICAL_SURFACE")?
            else {
                return Err(ReadError::Invalid);
            };
            if *angle <= 0. {
                return Err(ReadError::Invalid);
            }
            Ok(radial::Surface::Cone {
                axis: axis(shell, reference(a)?)?,
                radius_mm: *r,
                semi_angle_rad: *angle,
            })
        }
        "B_SPLINE_SURFACE_WITH_KNOTS" => {
            let (poles, spans) = tapered_thread::net(shell, id)?;
            Ok(radial::Surface::Tensor(
                RetainedTensor::new(spans, poles.into_iter().flatten().collect())
                    .map_err(failure)?,
            ))
        }
        _ => Err(ReadError::Unsupported),
    }
}
fn read_member(shell: &mut Shell<'_, '_>, id: u32) -> Result<radial::BoundaryOwnership> {
    let faces = shell.member_faces(id)?;
    if faces.len() > 34 {
        return Err(ReadError::Limit);
    }
    if faces.len() < 6 || !(faces.len() - 2).is_multiple_of(4) {
        return Err(ReadError::Unsupported);
    }
    let mut surface_ids = BTreeMap::<u32, usize>::new();
    let mut curve_ids = BTreeMap::<u32, usize>::new();
    let mut edge_ids = BTreeMap::<u32, usize>::new();
    let mut vertex_ids = BTreeMap::<u32, usize>::new();
    let mut input = radial::BoundaryInput {
        vertices: vec![],
        curves: vec![],
        edges: vec![],
        surfaces: vec![],
        faces: vec![],
    };
    let mut logical_controls = 0usize;
    let mut curve_controls = 0usize;
    for id in faces {
        let (surface_id, forward, bounds) = face(shell, id)?;
        let [bound] = bounds[..] else {
            return Err(ReadError::Unsupported);
        };
        let surface = if let Some(&index) = surface_ids.get(&surface_id) {
            index
        } else {
            // Count logical grid entries before copying their point values.
            if shell.simple(surface_id)?.name == "B_SPLINE_SURFACE_WITH_KNOTS" {
                let args = shell.args(surface_id, "B_SPLINE_SURFACE_WITH_KNOTS")?;
                let Some(Value::List(rows)) = args.get(3) else {
                    return Err(ReadError::Invalid);
                };
                for row in rows {
                    let Value::List(row) = row else {
                        return Err(ReadError::Invalid);
                    };
                    logical_controls = logical_controls
                        .checked_add(row.len())
                        .ok_or(ReadError::Limit)?;
                }
                if logical_controls > 524288
                    || logical_controls.saturating_add(curve_controls) > 1_048_576
                {
                    return Err(ReadError::Limit);
                }
            }
            let surface = surface(shell, surface_id)?;
            let index = input.surfaces.len();
            input.surfaces.push(surface);
            surface_ids.insert(surface_id, index);
            index
        };
        let mut outer = Vec::new();
        for (id, direction) in ring(shell, bound, true)? {
            let edge = if let Some(&index) = edge_ids.get(&id) {
                index
            } else {
                let [Value::String(_), a, b, c, fwd] = shell.args(id, "EDGE_CURVE")? else {
                    return Err(ReadError::Invalid);
                };
                let [a, b, c] = [reference(a)?, reference(b)?, reference(c)?];
                let fwd = sense(fwd)?;
                let mut ends = [0; 2];
                for (end, id) in [a, b].into_iter().enumerate() {
                    ends[end] = if let Some(&i) = vertex_ids.get(&id) {
                        i
                    } else {
                        if input.vertices.len() == 80 {
                            return Err(ReadError::Limit);
                        }
                        let p = shell.vertex(id)?;
                        let i = input.vertices.len();
                        input.vertices.push(p);
                        vertex_ids.insert(id, i);
                        i
                    };
                }
                let geometry = if let Some(&i) = curve_ids.get(&c) {
                    i
                } else {
                    if shell.simple(c)?.name == "B_SPLINE_CURVE_WITH_KNOTS" {
                        let args = shell.args(c, "B_SPLINE_CURVE_WITH_KNOTS")?;
                        let Some(Value::List(poles)) = args.get(2) else {
                            return Err(ReadError::Invalid);
                        };
                        curve_controls = curve_controls
                            .checked_add(poles.len())
                            .ok_or(ReadError::Limit)?;
                        if logical_controls.saturating_add(curve_controls) > 1_048_576 {
                            return Err(ReadError::Limit);
                        }
                    }
                    let geometry = curve(shell, c)?;
                    let i = input.curves.len();
                    input.curves.push(geometry);
                    curve_ids.insert(c, i);
                    i
                };
                if input.edges.len() == 112 {
                    return Err(ReadError::Limit);
                }
                let i = input.edges.len();
                input.edges.push(radial::Edge {
                    vertices: ends,
                    curve: geometry,
                    forward: fwd,
                });
                edge_ids.insert(id, i);
                i
            };
            outer.push((edge, direction));
        }
        input.faces.push(radial::Face {
            surface,
            outer,
            forward,
        });
    }
    radial::BoundaryOwnership::new(input, shell.tolerance, None).map_err(failure)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn replace_record(text: &str, name: &str, replacement: &str) -> String {
        let needle = format!("={name}(");
        let line = text.lines().find(|line| line.contains(&needle)).unwrap();
        let id = line.split('=').next().unwrap();
        text.replacen(line, &format!("{id}={replacement};"), 1)
    }
    #[test]
    fn malformed_finite_records_fail_without_partial_or_cached_import() {
        let material = crate::radial_tensor::tests::material();
        let text = crate::step::write_radial_boundary(&material, 1e-7).unwrap();
        for (name, replacement) in [
            ("AXIS2_PLACEMENT_3D", "AXIS2_PLACEMENT_3D('')"),
            ("DIRECTION", "DIRECTION('')"),
            ("CIRCLE", "CIRCLE('')"),
            ("B_SPLINE_CURVE_WITH_KNOTS", "B_SPLINE_CURVE_WITH_KNOTS('')"),
            ("B_SPLINE_CURVE_WITH_KNOTS", "LINE('')"),
            (
                "B_SPLINE_SURFACE_WITH_KNOTS",
                "B_SPLINE_SURFACE_WITH_KNOTS('')",
            ),
            ("PLANE", "PLANE('')"),
            ("CONICAL_SURFACE", "CONICAL_SURFACE('')"),
            ("CONICAL_SURFACE", "CYLINDRICAL_SURFACE('')"),
            ("CONICAL_SURFACE", "SPHERICAL_SURFACE('')"),
            ("EDGE_CURVE", "EDGE_CURVE('')"),
        ] {
            let malformed = replace_record(&text, name, replacement);
            assert!(read_radial_boundary(&malformed, 1e-7).is_err(), "{name}");
        }
        let point = text
            .lines()
            .find(|line| line.contains("=CARTESIAN_POINT("))
            .unwrap()
            .split('=')
            .next()
            .unwrap();
        for (name, replacement) in [
            (
                "B_SPLINE_CURVE_WITH_KNOTS",
                format!("B_SPLINE_CURVE_WITH_KNOTS('',9,({point}))"),
            ),
            (
                "B_SPLINE_SURFACE_WITH_KNOTS",
                format!(
                    "B_SPLINE_SURFACE_WITH_KNOTS('',9,9,({point}),.UNSPECIFIED.,.F.,.F.,.F.,(10,10),(10,10),(0.,1.),(0.,1.),.UNSPECIFIED.)"
                ),
            ),
        ] {
            assert_eq!(
                read_radial_boundary(&replace_record(&text, name, &replacement), 1e-7).err(),
                Some(ReadError::Invalid)
            );
        }
        let line = text
            .lines()
            .find(|line| line.contains("=CONICAL_SURFACE("))
            .unwrap();
        let (prefix, _) = line.rsplit_once(',').unwrap();
        let malformed = text.replacen(line, &format!("{prefix},-1.);"), 1);
        assert_eq!(
            read_radial_boundary(&malformed, 1e-7).err(),
            Some(ReadError::Invalid)
        );
        assert_eq!(failure(radial::Error::Limit), ReadError::Limit);
        assert_eq!(failure(radial::Error::Invalid), ReadError::Invalid);
        for error in [
            radial::Error::Unresolved,
            radial::Error::Unsupported,
            radial::Error::Cancelled,
        ] {
            assert_eq!(failure(error), ReadError::Unsupported);
        }
        assert_eq!(
            read_radial_boundary(&text, 1e-7).unwrap().input(),
            material.input()
        );
    }
    #[test]
    fn declared_control_buffers_are_bounded_before_copying_source_points() {
        let material = crate::radial_tensor::tests::material();
        let text = crate::step::write_radial_boundary(&material, 1e-7).unwrap();
        let curve = text
            .lines()
            .find(|line| line.contains("=B_SPLINE_CURVE_WITH_KNOTS("))
            .unwrap();
        let body = curve.split_once('=').unwrap().1.trim_end_matches(';');
        let (head, poles) = body.split_once(",(").unwrap();
        let (old, tail) = poles.split_once("),.UNSPECIFIED.").unwrap();
        let point = old.split(',').next().unwrap();
        let count = 9 * 58_255 + 1;
        let repeated = format!("{point},").repeat(count - 1) + point;
        let huge = format!("{head},({repeated}),.UNSPECIFIED.{tail}");
        let input = replace_record(&text, "B_SPLINE_CURVE_WITH_KNOTS", &huge);
        assert_eq!(
            read_radial_boundary(&input, 1e-7).err(),
            Some(ReadError::Limit)
        );
        let empty = format!("{head},(),.UNSPECIFIED.{tail}");
        let input = replace_record(&text, "B_SPLINE_CURVE_WITH_KNOTS", &empty);
        assert_eq!(
            read_radial_boundary(&input, 1e-7).err(),
            Some(ReadError::Unsupported)
        );
        let surface = text
            .lines()
            .find(|line| line.contains("=B_SPLINE_SURFACE_WITH_KNOTS("))
            .unwrap();
        let body = surface.split_once('=').unwrap().1.trim_end_matches(';');
        let (head, rows) = body.split_once(",((").unwrap();
        let (_, tail) = rows.split_once(")),.UNSPECIFIED.").unwrap();
        let row = format!("{point},").repeat(1024) + point;
        let rows = (0..512)
            .map(|_| format!("({row})"))
            .collect::<Vec<_>>()
            .join(",");
        let huge = format!("{head},({rows}),.UNSPECIFIED.{tail}");
        let input = replace_record(&text, "B_SPLINE_SURFACE_WITH_KNOTS", &huge);
        assert_eq!(
            read_radial_boundary(&input, 1e-7).err(),
            Some(ReadError::Limit)
        );
        // Individually legal curve buffers can still exceed the shared
        // source allowance. Full knot declarations reach that cumulative
        // gate before any later geometric endpoint/trim validation.
        let spans = 58_254usize;
        let count = 9 * spans + 1;
        let poles = format!("{point},").repeat(count - 1) + point;
        let mult = (0..=spans)
            .map(|i| if i == 0 || i == spans { "10" } else { "9" })
            .collect::<Vec<_>>()
            .join(",");
        let knots = (0..=spans)
            .map(|i| format!("{:.16E}", i as f64 / spans as f64))
            .collect::<Vec<_>>()
            .join(",");
        let body = format!(
            "B_SPLINE_CURVE_WITH_KNOTS('',9,({poles}),.UNSPECIFIED.,.F.,.F.,({mult}),({knots}),.UNSPECIFIED.)"
        );
        let mut input = text.clone();
        for line in text
            .lines()
            .filter(|line| line.contains("=B_SPLINE_CURVE_WITH_KNOTS("))
            .take(2)
        {
            let id = line.split('=').next().unwrap();
            input = input.replacen(line, &format!("{id}={body};"), 1);
        }
        assert_eq!(
            read_radial_boundary(&input, 1e-7).err(),
            Some(ReadError::Limit)
        );
        assert!(read_radial_boundary(&text, 1e-7).is_ok());
    }
    #[test]
    fn ordinary_analytic_stocks_keep_explicit_family_and_topology_limits() {
        use crate::{Rational as R, polygon::PolygonPrism};
        let r = |n| R::new(n, 1).unwrap();
        let write = |p: &PolygonPrism| {
            crate::step::write_polygon_prism(p, [0.; 3], [1., 0., 0.], [0., 1., 0.], false).unwrap()
        };
        let triangle =
            PolygonPrism::new(vec![vec![[r(0), r(0)], [r(1), r(0)], [r(0), r(1)]]], r(1)).unwrap();
        assert_eq!(
            read_radial_boundary(&write(&triangle), 1e-7).err(),
            Some(ReadError::Unsupported)
        );
        let mut ring = (0..=32).map(|i| [r(i), r(i * i)]).collect::<Vec<_>>();
        ring.extend([[r(32), r(-1)], [r(0), r(-1)]]);
        ring.reverse();
        let many = PolygonPrism::new(vec![ring], r(1)).unwrap();
        let text = write(&many);
        assert_eq!(text.matches("ADVANCED_FACE").count(), 37);
        assert_eq!(
            read_radial_boundary(&text, 1e-7).err(),
            Some(ReadError::Limit)
        );
        let ring = vec![
            [r(0), r(0)],
            [r(1), r(0)],
            [r(2), r(0)],
            [r(3), r(0)],
            [r(4), r(0)],
            [r(4), r(4)],
            [r(0), r(4)],
        ];
        let drilled = PolygonPrism::with_circular_holes(
            vec![ring],
            vec![crate::Circle::new([r(2), r(2)], R::new(1, 2).unwrap()).unwrap()],
            r(1),
        )
        .unwrap();
        let text = write(&drilled);
        assert_eq!(text.matches("ADVANCED_FACE").count(), 10);
        assert_eq!(
            read_radial_boundary(&text, 1e-7).err(),
            Some(ReadError::Unsupported)
        );
    }
    #[test]
    fn untrusted_topology_counts_distinct_ids_before_geometry_or_coordinate_welding() {
        let material = crate::radial_tensor::tests::material();
        let text = crate::step::write_radial_boundary(&material, 1e-7).unwrap();
        let entities = parse_entities(&text, 1e-7).unwrap();
        let shell = Shell::new(&entities, 1e-7, true);
        let solid = shell.represented_solids(true).unwrap()[0];
        let faces = shell.member_faces(solid).unwrap();
        let bound = face(&shell, faces[0]).unwrap().2[0];
        let wire = shell.bound(bound, false).unwrap().1;
        let edge = ring(&shell, bound, true).unwrap()[0].0;
        let args = shell.args(edge, "EDGE_CURVE").unwrap();
        let a = reference(&args[1]).unwrap();
        let b = reference(&args[2]).unwrap();
        let curve = reference(&args[3]).unwrap();
        let point = reference(&shell.args(a, "VERTEX_POINT").unwrap()[1]).unwrap();
        for vertices in [false, true] {
            let count = if vertices { 41 } else { 113 };
            let mut extra = String::new();
            let mut uses = Vec::new();
            for i in 0..count {
                let edge = 810_000 + i;
                let oriented = 820_000 + i;
                let (a, b) = if vertices {
                    let a = 800_000 + 2 * i;
                    let b = a + 1;
                    extra.push_str(&format!(
                        "#{a}=VERTEX_POINT('',#{point});\n#{b}=VERTEX_POINT('',#{point});\n"
                    ));
                    (a, b)
                } else {
                    (a as usize, b as usize)
                };
                extra.push_str(&format!("#{edge}=EDGE_CURVE('',#{a},#{b},#{curve},.T.);\n#{oriented}=ORIENTED_EDGE('',*,*,#{edge},.T.);\n"));
                uses.push(format!("#{oriented}"));
            }
            let old = text
                .lines()
                .find(|line| line.starts_with(&format!("#{wire}=")))
                .unwrap();
            let changed = text.replacen(
                old,
                &format!("#{wire}=EDGE_LOOP('',({}));", uses.join(",")),
                1,
            );
            let changed =
                changed.replacen("ENDSEC;\nEND-ISO", &format!("{extra}ENDSEC;\nEND-ISO"), 1);
            assert_eq!(
                read_radial_boundary(&changed, 1e-7).err(),
                Some(ReadError::Limit),
                "vertices{vertices}"
            );
        }
        assert!(read_radial_boundary(&text, 1e-7).is_ok());
    }
    #[test]
    fn finite_boundary_readback_retains_control_nets_without_a_thread_recipe() {
        for (k, left, starts) in [(0.1, false, 1), (-0.1, true, 3), (0., true, 1)] {
            let spec = crate::helical_thread::Spec {
                major_radius_mm: 4.,
                pitch_mm: 1.25,
                length_mm: 8.,
                depth_mm: 0.5,
                crest_width_mm: 0.25,
                lower_flank_deg: 12.,
                upper_flank_deg: 18.,
                starts,
                left_hand: left,
                root_radius_mm: None,
            };
            let t = crate::tapered_thread::Thread::new(spec, k, 1e-7, 1e-6).unwrap();
            let text = crate::step::write_tapered_thread(
                &t,
                [1000., -700., 300.],
                [0.8, 0.6, 0.],
                [-0.36, 0.48, 0.8],
                1e-7,
            )
            .unwrap();
            let decoded = read_radial_boundary(&text, 1e-7).unwrap();
            assert_eq!(decoded.input().faces.len(), 2 + 4 * starts as usize);
            assert_eq!(decoded.input().edges.len(), 14 * starts as usize);
            assert_eq!(decoded.input().vertices.len(), 10 * starts as usize);
            assert!(decoded.sewing_error_mm() < 1e-7 / 16.);
            let mut broken = decoded.input().clone();
            broken.faces[0].outer[0].1 = !broken.faces[0].outer[0].1;
            assert!(radial::BoundaryOwnership::new(broken, 1e-7, None).is_err());
            let trimmed = decoded.bind_surfaces(1e-7, None).unwrap();
            assert!(trimmed.boundary_error_mm() < 1e-7 / 16.);
            assert_eq!(
                trimmed
                    .tensor_edges()
                    .iter()
                    .filter(|e| e.is_some())
                    .count(),
                3 * starts as usize
            );
            let axial = trimmed.certify_axial(1e-7, 1e-6, None).unwrap();
            let h = axial.height_bounds_mm();
            assert!((h[0] - 8.).abs() < 1e-10 && (h[1] - 8.).abs() < 1e-10);
            assert!(axial.axial_parameter_error_mm() < 1e-7 / 32.);
            let material = axial
                .certify_caps(None)
                .unwrap()
                .certify_ordering(None)
                .unwrap()
                .certify_material(1e-7, None)
                .unwrap();
            let normal = [0.48, -0.64, 0.6];
            let point = |z: f64| std::array::from_fn(|i| [1000., -700., 300.][i] + z * normal[i]);
            assert_eq!(
                material.classify_core(point(4.), None).unwrap(),
                radial::Location::Inside
            );
            assert_eq!(
                material.classify_core(point(-0.1), None).unwrap(),
                radial::Location::Outside
            );
            assert_eq!(
                material.classify_core(point(0.), None).unwrap(),
                radial::Location::Unresolved
            );
            assert_eq!(
                material.classify_core(point(8.), None).unwrap(),
                radial::Location::Unresolved
            );
            assert_eq!(
                material.classify_core(point(4.), Some(&|| true)),
                Err(radial::Error::Cancelled)
            );
            let exported = crate::step::write_radial_boundary(&material, 1e-7).unwrap();
            let restored = read_radial_boundary(&exported, 1e-7).unwrap();
            assert_eq!(restored.input(), material.input());
            let source = t.support_bounds([1., 0., 0.], 1e-5).unwrap();
            let support = material.support_bounds([0.8, 0.6, 0.], 1e-5, None).unwrap();
            assert!(support[0] <= source[1] + 380. + 1e-7 && support[1] >= source[0] + 380. - 1e-7);
            let rate = (if left { -1. } else { 1. }) * starts as f64 * spec.pitch_mm
                / std::f64::consts::TAU;
            for fraction in [0.071, 0.287, 0.631, 0.893] {
                let phase = fraction * spec.pitch_mm;
                let theta = (4. - phase) / rate;
                let radius = t.radial_bounds(phase, 4., 1e-9).unwrap();
                for (r, expected) in [
                    (radius[0] - 0.02, radial::Location::Inside),
                    (radius[1] + 0.02, radial::Location::Outside),
                ] {
                    let xy = [r * theta.cos(), r * theta.sin()];
                    let p = std::array::from_fn(|i| {
                        point(4.)[i] + xy[0] * [0.8, 0.6, 0.][i] + xy[1] * [-0.36, 0.48, 0.8][i]
                    });
                    assert_eq!(
                        material.classify(p, None).unwrap(),
                        expected,
                        "k{k} left{left} starts{starts} phase{phase} r{r}"
                    );
                }
            }
        }
    }
}
