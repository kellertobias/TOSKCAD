//! Certify two full periodic seams bounding one complete regular torus.
use super::*;
use crate::torus::Torus;
use std::f64::consts::TAU;

pub(super) fn read_member(
    shell: &mut Shell<'_, '_>,
    faces: &[u32],
) -> Result<Option<(Readback, ImportSolid)>> {
    let [face] = faces else {
        return Ok(None);
    };
    let [Value::String(_), bounds, support, orientation] = shell.args(*face, "ADVANCED_FACE")?
    else {
        return Err(ReadError::Invalid);
    };
    let support = reference(support)?;
    let surface = shell.simple(support)?;
    if surface.name != "TOROIDAL_SURFACE" {
        return Ok(None);
    }
    let [
        Value::String(_),
        placement,
        Value::Number(major),
        Value::Number(minor),
    ] = surface.args.as_slice()
    else {
        return Err(ReadError::Invalid);
    };
    let [major, minor] = [*major, *minor];
    let torus = Torus::new(major, minor).ok_or(ReadError::Invalid)?;
    let frame = shell.placement(reference(placement)?)?;
    let [bound] = refs(bounds)?.try_into().map_err(|_| ReadError::Invalid)?;
    let (_, wire, forward) = shell.bound(bound, true)?;
    if !forward || !sense(orientation)? {
        return Err(ReadError::Invalid);
    }
    let [Value::String(_), loop_edges] = shell.args(wire, "EDGE_LOOP")? else {
        return Err(ReadError::Invalid);
    };
    let edges = refs(loop_edges)?;
    if edges.len() != 4 || edges.iter().collect::<BTreeSet<_>>().len() != 4 {
        return Err(ReadError::Invalid);
    }
    let uses = edges
        .iter()
        .map(|&id| edge_use(shell, id))
        .collect::<Result<Vec<_>>>()?;
    let [a, b, c, d] = uses.try_into().map_err(|_| ReadError::Invalid)?;
    if a.0 != c.0 || b.0 != d.0 || a.0 == b.0 || a.1 == c.1 || b.1 == d.1 {
        return Err(ReadError::Invalid);
    }

    let outer = major + minor;
    let linear = shell.tolerance.min(minor * 0.001);
    let angular = (linear / outer).clamp(64. * f64::EPSILON, 1e-8);
    let axis_point = |r: f64| std::array::from_fn(|i| frame.origin[i] + r * frame.u[i]);
    let [equator, meridian] = [a.0, b.0].map(|edge| seam(shell, edge, support, angular));
    let mut seams = [equator?, meridian?];
    seams.sort_unstable_by_key(|seam| seam.moving);
    let [equator, meridian] = &seams;
    if equator.moving != 0 || meridian.moving != 1 || equator.vertex != meridian.vertex {
        return Err(ReadError::Invalid);
    }
    let expected = [
        (frame.origin, frame.normal, outer),
        (axis_point(major), frame.v.map(|x| -x), minor),
    ];
    for (seam, (center, normal, radius)) in seams.iter().zip(expected) {
        if length(sub(seam.frame.origin, center)) > linear
            || length(sub(seam.frame.normal, normal)) > angular
            || length(sub(seam.frame.u, frame.u)) > angular
            || (seam.radius - radius).abs() > linear
        {
            return Err(ReadError::Invalid);
        }
    }
    if length(sub(shell.vertex(equator.vertex)?, axis_point(outer))) > linear {
        return Err(ReadError::Invalid);
    }

    // With +U and +V on the corresponding master curves, this wire must
    // be a cyclic rotation of the outward rectangular chart boundary.
    let mut uses = [a, b, c, d];
    let start = uses
        .iter()
        .position(|&item| item == (equator.edge, true))
        .ok_or(ReadError::Invalid)?;
    uses.rotate_left(start);
    if uses
        != [
            (equator.edge, true),
            (meridian.edge, true),
            (equator.edge, false),
            (meridian.edge, false),
        ]
    {
        return Err(ReadError::Invalid);
    }
    let extent: Point = std::array::from_fn(|i| major * frame.u[i].hypot(frame.v[i]) + minor);
    let bounds_mm =
        [-1., 1.].map(|sign| std::array::from_fn(|i| frame.origin[i] + sign * extent[i]));
    if bounds_mm[0]
        .into_iter()
        .zip(bounds_mm[1])
        .any(|(lo, hi)| lo >= hi)
    {
        return Err(ReadError::Invalid);
    }
    let proof = Readback {
        bounds_mm,
        volume_mm3: torus.volume_mm3(),
        face_count: 1,
        edges_mm: vec![],
        circles: expected
            .into_iter()
            .map(|(center_mm, normal, radius_mm)| CircleEdge {
                center_mm,
                normal,
                radius_mm,
            })
            .collect(),
        semicircles: vec![],
        arcs: vec![],
        ellipses: vec![],
        splines: vec![],
        solid_count: 1,
    };
    let body = ImportSolid {
        origin_mm: frame.origin,
        u: frame.u,
        v: frame.v,
        height_mm: 0.,
        profile: ImportProfile::Torus {
            major_radius_mm: major,
            minor_radius_mm: minor,
        },
    };
    Ok(Some((proof, body)))
}

fn edge_use(shell: &Shell<'_, '_>, id: u32) -> Result<(u32, bool)> {
    let [
        Value::String(_),
        Value::Derived,
        Value::Derived,
        edge,
        sense_value,
    ] = shell.args(id, "ORIENTED_EDGE")?
    else {
        return Err(ReadError::Invalid);
    };
    Ok((reference(edge)?, sense(sense_value)?))
}

struct Seam {
    edge: u32,
    vertex: u32,
    moving: usize,
    frame: Plane,
    radius: f64,
}

fn seam(shell: &Shell<'_, '_>, edge: u32, support: u32, tolerance: f64) -> Result<Seam> {
    let [Value::String(_), start, end, geometry, Value::Enum("T")] =
        shell.args(edge, "EDGE_CURVE")?
    else {
        return Err(ReadError::Invalid);
    };
    let vertex = reference(start)?;
    if vertex != reference(end)? {
        return Err(ReadError::Invalid);
    }
    let [Value::String(_), circle, pcurves, Value::Enum("PCURVE_S1")] =
        shell.args(reference(geometry)?, "SEAM_CURVE")?
    else {
        return Err(ReadError::Unsupported);
    };
    let [a, b] = refs(pcurves)?.try_into().map_err(|_| ReadError::Invalid)?;
    if a == b {
        return Err(ReadError::Invalid);
    }
    let decode = |id| {
        let (surface, line) = shell.pcurve_definition(id)?;
        if surface != support {
            return Err(ReadError::Invalid);
        }
        shell.line(line, true)
    };
    let (low, direction, scale) = decode(a)?;
    let (high, high_direction, high_scale) = decode(b)?;
    let moving = usize::from(direction[1].abs() > direction[0].abs());
    let fixed = 1 - moving;
    // Both pcurve copies use the same 3D circle parameter. A complete
    // seam spans one unit-speed period with its stationary coordinate
    // fixed at the opposite sides of the fundamental UV rectangle.
    if (scale - 1.).abs().max((high_scale - 1.).abs()) > tolerance
        || (direction[moving] - 1.).abs() > tolerance
        || direction[fixed].abs() > tolerance
        || length(sub(direction, high_direction)) > tolerance
        || length(low) > tolerance
        || high[moving].abs() > tolerance
        || (high[fixed] - TAU).abs() > tolerance
    {
        return Err(ReadError::Invalid);
    }
    let [Value::String(_), frame, Value::Number(radius)] =
        shell.args(reference(circle)?, "CIRCLE")?
    else {
        return Err(ReadError::Invalid);
    };
    Ok(Seam {
        edge,
        vertex,
        moving,
        frame: shell.placement(reference(frame)?)?,
        radius: *radius,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        Rational,
        step::{StepError, Writer},
    };

    const ORIGIN: Point = [17., 4., -12.];
    const U: Point = [0.8, 0., -0.6];
    const V: Point = [0., 1., 0.];
    const TOLERANCE: f64 = 1e-7;

    fn specimen() -> String {
        let mut writer = Writer::new();
        writer
            .append_torus(Torus::new(5., 1.5).unwrap(), ORIGIN, U, V)
            .unwrap();
        writer.finish().unwrap()
    }

    fn record<'a>(source: &'a str, entity: &str, occurrence: usize) -> &'a str {
        source
            .lines()
            .filter(|line| {
                line.split_once('=')
                    .is_some_and(|(_, data)| data.starts_with(entity))
            })
            .nth(occurrence)
            .unwrap()
    }

    fn edit(source: &str, entity: &str, change: impl FnOnce(&str) -> String) -> String {
        let old = record(source, entity, 0);
        source.replacen(old, &change(old), 1)
    }

    #[test]
    fn full_torus_preserves_oblique_metrics_and_the_two_seams_in_a_compound() {
        let text = specimen();
        assert!(text.contains("TOROIDAL_SURFACE("));
        assert_eq!(text.matches("SEAM_CURVE(").count(), 2);
        assert_eq!(text.matches("PCURVE(").count(), 4);
        let (proof, body) = read_import_solid(&text, TOLERANCE).unwrap();
        assert_eq!(proof, read_analytic_solid(&text, TOLERANCE).unwrap());
        // Independently authored with OCCT 7.9.3 BRepPrimAPI_MakeTorus.
        let native = include_str!("tests/fixtures/occt-regular-torus.step");
        assert_eq!(
            read_import_solid(native, TOLERANCE).unwrap(),
            (proof.clone(), body.clone())
        );
        assert_eq!(body.origin_mm, ORIGIN);
        assert_eq!(body.u, U);
        assert_eq!(body.v, V);
        assert_eq!(body.height_mm, 0.);
        assert_eq!(
            body.profile,
            ImportProfile::Torus {
                major_radius_mm: 5.,
                minor_radius_mm: 1.5,
            }
        );
        assert_eq!(proof.bounds_mm, [[11.5, -2.5, -16.5], [22.5, 10.5, -7.5]]);
        assert_eq!([proof.solid_count as usize, proof.face_count], [1, 1]);
        assert!((proof.volume_mm3 - 22.5 * std::f64::consts::PI.powi(2)).abs() < 1e-12);
        assert_eq!(proof.circles[0].radius_mm, 6.5);
        assert_eq!(proof.circles[0].center_mm, ORIGIN);
        assert_eq!(proof.circles[1].radius_mm, 1.5);
        assert_eq!(proof.circles[1].center_mm, [21., 4., -15.]);
        assert_eq!(proof.circles[1].normal, [0., -1., 0.]);
        assert!(read_planar_solid(&text, TOLERANCE).is_err());

        let rotated_loop = edit(&text, "EDGE_LOOP(", |line| {
            let (start, refs) = line.split_once("',(").unwrap();
            let [a, b, c, d]: [&str; 4] = refs
                .strip_suffix("));")
                .unwrap()
                .split(',')
                .collect::<Vec<_>>()
                .try_into()
                .unwrap();
            format!("{start}',({b},{c},{d},{a}));")
        });
        assert_eq!(
            read_import_solid(&rotated_loop, TOLERANCE).unwrap(),
            (proof.clone(), body.clone())
        );

        let mut writer = Writer::new();
        writer
            .append_box([Rational::new(4, 1).unwrap(); 3], [0.; 3], [1., 0., 0.], V)
            .unwrap();
        let torus = Torus::new(5., 1.5).unwrap();
        assert_eq!(
            writer.append_torus(torus, ORIGIN, U, U),
            Err(StepError::InvalidPlacement)
        );
        assert_eq!(
            writer.append_torus(torus, [1e12, 0., 0.], U, V),
            Err(StepError::InvalidPlacement)
        );
        writer.append_torus(torus, ORIGIN, U, V).unwrap();
        let compound = writer.finish().unwrap();
        let decoded = read_import_compound(&compound, TOLERANCE).unwrap();
        assert_eq!(decoded.len(), 2);
        assert_eq!(decoded[1], (proof, body));
        assert_eq!(
            read_analytic_compound(&compound, TOLERANCE).unwrap()[1],
            decoded[1].0
        );
    }

    #[test]
    fn rejects_unsupported_or_inconsistent_periodic_trims() {
        let text = specimen();
        let invalid = |malformed: &str| {
            assert_ne!(text, malformed);
            assert!(
                read_analytic_solid(malformed, TOLERANCE).is_err(),
                "{malformed}"
            );
            assert!(read_import_solid(malformed, TOLERANCE).is_err());
        };
        let entity_id = |entity| record(&text, entity, 0).split_once('=').unwrap().0;
        let strip_args = |line: &str| {
            let (entity, body) = line.split_once('(').unwrap();
            assert!(!body.is_empty());
            format!("{entity}('');")
        };
        for entity in [
            "TOROIDAL_SURFACE(",
            "EDGE_LOOP(",
            "ORIENTED_EDGE(",
            "EDGE_CURVE(",
            "SEAM_CURVE(",
            "CIRCLE(",
        ] {
            invalid(&edit(&text, entity, strip_args));
        }
        let flip = |line: &str| line.replace(".T.", ".F.");
        for entity in [
            "ADVANCED_FACE(",
            "FACE_BOUND(",
            "ORIENTED_EDGE(",
            "EDGE_CURVE(",
        ] {
            invalid(&edit(&text, entity, flip));
        }
        invalid(&text.replace("5.0000000000000000E0,1.5000000000000000E0", "1.,2."));
        invalid(&text.replace("6.2831853071795862E0", "5."));
        invalid(&text.replace("6.5000000000000000E0", "6.6"));
        invalid(&text.replace(".RADIAN.", ".DEGREE."));
        invalid(&edit(&text, "PCURVE(", |line| {
            line.replace(entity_id("TOROIDAL_SURFACE("), entity_id("CIRCLE("))
        }));
        invalid(&edit(&text, "VECTOR(", |line| {
            line.replace(",1.);", ",2.);")
        }));
        invalid(&edit(&text, "SEAM_CURVE(", |line| {
            let (_, tail) = line.split_once(",(").unwrap();
            let [first, second] = tail
                .split_once(')')
                .unwrap()
                .0
                .split_once(',')
                .map(|(a, b)| [a, b])
                .unwrap();
            line.replace(
                &format!("({first},{second})"),
                &format!("({first},{first})"),
            )
        }));
        invalid(&edit(&text, "EDGE_CURVE(", |line| {
            line.replacen(
                &format!(",{},", entity_id("VERTEX_POINT(")),
                &format!(",{},", entity_id("EDGE_LOOP(")),
                1,
            )
        }));
        invalid(&edit(&text, "EDGE_LOOP(", |line| {
            let (start, refs) = line.split_once("',(").unwrap();
            let [a, b, c, _]: [&str; 4] = refs
                .strip_suffix("));")
                .unwrap()
                .split(',')
                .collect::<Vec<_>>()
                .try_into()
                .unwrap();
            format!("{start}',({a},{b},{c},{a}));")
        }));
        invalid(&edit(&text, "EDGE_LOOP(", |line| {
            let (start, refs) = line.split_once("',(").unwrap();
            let [a, b, c, d]: [&str; 4] = refs
                .strip_suffix("));")
                .unwrap()
                .split(',')
                .collect::<Vec<_>>()
                .try_into()
                .unwrap();
            format!("{start}',({a},{d},{c},{b}));")
        }));
        invalid(&text.replace("2.2199999999999999E1", "2.3E1"));
        // Sharing a coordinate is insufficient. Both seams must share a
        // single topological vertex to close the fundamental rectangle.
        let vertex = record(&text, "VERTEX_POINT(", 0);
        let (vertex_id, definition) = vertex.split_once('=').unwrap();
        let meridian = record(&text, "EDGE_CURVE(", 1);
        let split_corner = text
            .replacen(meridian, &meridian.replace(vertex_id, "#999999"), 1)
            .replace(
                "ENDSEC;\nEND-ISO",
                &format!("#999999={definition}\nENDSEC;\nEND-ISO"),
            );
        invalid(&split_corner);
        let malformed = edit(&text, "ADVANCED_FACE(", strip_args);
        let entities = parse_entities(&malformed, TOLERANCE).unwrap();
        let mut shell = Shell::new(&entities, TOLERANCE, true);
        let faces = shell.shell_faces().unwrap();
        assert!(matches!(
            read_member(&mut shell, &faces),
            Err(ReadError::Invalid)
        ));
    }

    #[test]
    fn rejects_a_world_placement_that_collapses_a_torus_extent() {
        let mut writer = Writer::new();
        let tiny = Torus::new(2e-6, 1e-6).unwrap();
        assert_eq!(
            writer.append_torus(tiny, [1e11, 0., 0.], [0., 1., 0.], [0., 0., 1.]),
            Err(StepError::InvalidPlacement)
        );
        writer
            .append_torus(tiny, [0.; 3], [0., 1., 0.], [0., 0., 1.])
            .unwrap();
        let source = writer.finish().unwrap();
        let translated = source
            .lines()
            .map(|line| {
                if line.contains("CARTESIAN_POINT") && line.matches(',').count() == 3 {
                    line.replacen("(0.0000000000000000E0,", "(1.0E11,", 1)
                } else {
                    line.to_string()
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(
            read_analytic_solid(&translated, TOLERANCE),
            Err(ReadError::Invalid)
        );
    }
}
