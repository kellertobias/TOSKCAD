//! Exact planar closure of a fully threaded external hex screw.
use super::*;
use crate::headed_screw::hex_vertices;

pub(super) struct LineUse {
    oriented: EdgeRef,
    vertices: [u32; 2],
}

pub(super) fn verify(
    shell: &mut Shell<'_, '_>,
    faces: &[u32],
    shoulder: u32,
    shoulder_center: Point,
    [reference_dir, axis]: [Point; 2],
    [across_flats_mm, height_mm]: [f64; 2],
    world: impl Fn(Point2, f64) -> Point,
) -> Result<f64> {
    let (surface, forward, bounds) = face(shell, faces[0])?;
    let [bottom] = bounds[..] else {
        return Err(ReadError::Invalid);
    };
    require(forward)?;
    let center = world([0.; 2], -height_mm);
    let plane = plane(shell, surface)?;
    let down = axis.map(|n| -n);
    let cap = check_axis(shell, plane, center, down, reference_dir)?;
    let points = hex_vertices(across_flats_mm);
    let lower = hex_rim(
        shell,
        bottom,
        true,
        false,
        points.map(|p| world(p, -height_mm)),
    )?;
    let upper = hex_rim(shell, shoulder, true, true, points.map(|p| world(p, 0.)))?;
    let area = walls_area(
        shell,
        &faces[1..],
        [&lower, &upper],
        cap.origin,
        axis,
        false,
    )?;
    let height = dot(sub(cap.origin, shoulder_center), cap.normal);
    let volume = area * height;
    require(volume.is_finite() && volume > 0.)?;
    Ok(volume)
}

pub(super) fn walls_area(
    shell: &mut Shell<'_, '_>,
    faces: &[u32],
    [lower, upper]: [&[LineUse]; 2],
    origin: Point,
    axis: Point,
    cavity: bool,
) -> Result<f64> {
    let mut rails = [[(0, false); 2]; 6];
    let mut area = 0.;
    for (i, (&id, (lower, upper))) in faces.iter().zip(lower.iter().zip(upper)).enumerate() {
        let (surface, forward, bounds) = face(shell, id)?;
        let [bound] = bounds[..] else {
            return Err(ReadError::Invalid);
        };
        require(forward)?;
        let [start, end] = lower.vertices;
        let a = shell.vertex(start)?;
        let b = shell.vertex(end)?;
        let tangent = unit(sub(b, a))?;
        check_axis(
            shell,
            self::plane(shell, surface)?,
            a,
            cross(tangent, axis).map(|n| if cavity { -n } else { n }),
            tangent,
        )?;
        let edges = orient(&ring(shell, bound, true)?, !cavity);
        let [bottom, end_rail, top, start_rail] = edges[..] else {
            return Err(ReadError::Invalid);
        };
        require(bottom == lower.oriented && top == reversed(upper.oriented))?;
        require(line(shell, end_rail)?.vertices == [end, upper.vertices[1]])?;
        require(line(shell, start_rail)?.vertices == [upper.vertices[0], start])?;
        rails[i] = [start_rail, end_rail];
        area += dot(cross(sub(a, origin), sub(b, origin)), axis) / 2.;
    }
    require((0..6).all(|i| rails[i][1] == reversed(rails[(i + 1) % 6][0])))?;
    Ok(area)
}

fn reversed((edge, forward): EdgeRef) -> EdgeRef {
    (edge, !forward)
}

pub(super) fn plane<'a, 'input>(
    shell: &'a Shell<'_, 'input>,
    id: u32,
) -> Result<&'a Value<'input>> {
    let [Value::String(_), axis] = shell.args(id, "PLANE")? else {
        return Err(ReadError::Invalid);
    };
    Ok(axis)
}

fn line(shell: &mut Shell<'_, '_>, oriented: EdgeRef) -> Result<LineUse> {
    let (edge, forward) = oriented;
    let [a, b] = shell.edge(edge)?;
    require(matches!(shell.edges[&edge].curve, Curve::Line))?;
    Ok(LineUse {
        oriented,
        vertices: if forward { [a, b] } else { [b, a] },
    })
}

pub(super) fn hex_rim(
    shell: &mut Shell<'_, '_>,
    bound: u32,
    exterior: bool,
    forward: bool,
    expected: [Point; 6],
) -> Result<Vec<LineUse>> {
    let edges = ring(shell, bound, exterior)?;
    require(edges.len() == expected.len())?;
    // The bottom rim is oriented clockwise starting at corner 5. Normalize
    // it to the same first corner and counterclockwise order as the shoulder.
    let mut edges = orient(&edges, forward);
    if !forward {
        edges.rotate_left(1);
    }
    let mut uses = Vec::with_capacity(expected.len());
    for (i, edge) in edges.into_iter().enumerate() {
        let line = line(shell, edge)?;
        let [a, b] = line.vertices;
        require(shell.vertex(a)? == expected[i] && shell.vertex(b)? == expected[(i + 1) % 6])?;
        uses.push(line);
    }
    require((0..6).all(|i| uses[i].vertices[1] == uses[(i + 1) % 6].vertices[0]))?;
    Ok(uses)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        headed_screw::{Screw, ScrewHead, ScrewSpec},
        helical_thread::{Spec, Thread},
        step::{supports_helical_screw, write_helical_screw},
    };

    fn screw(hand: bool) -> Screw {
        let thread = Thread::new(Spec {
            major_radius_mm: 3.,
            pitch_mm: 1.,
            length_mm: 3.7,
            depth_mm: 5. * 3_f64.sqrt() / 16.,
            crest_width_mm: 0.125,
            lower_flank_deg: 30.,
            upper_flank_deg: 30.,
            starts: 1,
            left_hand: hand,
            root_radius_mm: None,
        })
        .unwrap();
        Screw::new(
            thread,
            ScrewSpec {
                head: ScrewHead::ExternalHex {
                    across_flats_mm: 10.,
                    height_mm: 4.,
                },
                shank_length_mm: 0.,
                shank_radius_mm: 3.,
            },
        )
        .unwrap()
    }

    #[test]
    fn rejects_changed_hex_head_faces_and_incidence() {
        let [origin, u, v] = [[0.; 3], [1., 0., 0.], [0., 1., 0.]];
        let screw = screw(false);
        let encoded = write_helical_screw(&screw, origin, u, v, 1e-7).unwrap();
        let data = parse_entities(&encoded, 1e-7).unwrap();
        let reader = Shell::new(&data, 1e-7, true);
        let root = reader.represented_solids(true).unwrap()[0];
        let faces = reader.member_faces(root).unwrap();
        let mut rewrite = Vec::new();
        for id in &faces[6..8] {
            let (surface, _, bounds) = face(&reader, *id).unwrap();
            rewrite.push((*id, format!("ADVANCED_FACE('',(),#{surface},.T.);")));
            let (_, wire, _) = reader.bound(bounds[0], false).unwrap();
            rewrite.push((wire, "EDGE_LOOP('',());".into()));
            rewrite.push((surface, "PLANE('');".into()));
            let position = reference(plane(&reader, surface).unwrap()).unwrap();
            let placement = reader.args(position, "AXIS2_PLACEMENT_3D").unwrap();
            let [origin, normal, u] = [1, 2, 3].map(|index| reference(&placement[index]).unwrap());
            rewrite.push((
                position,
                format!("AXIS2_PLACEMENT_3D('',#{origin},#{u},#{normal});"),
            ));
        }
        for (id, record) in rewrite {
            let key = format!("#{id}=");
            let original = encoded.lines().find(|line| line.starts_with(&key)).unwrap();
            let changed = encoded.replacen(original, &format!("{key}{record}"), 1);
            assert!(verify_helical_screw(&changed, &screw, origin, u, v, 1e-7).is_err());
        }
    }

    #[test]
    fn both_handings_keep_a_closed_hex_head_and_decoded_volume() {
        let [origin, u, v] = [[0.; 3], [1., 0., 0.], [0., 1., 0.]];
        let write = |screw: &Screw| write_helical_screw(screw, origin, u, v, 1e-7);
        for hand in [false, true] {
            let screw = screw(hand);
            assert!(supports_helical_screw(&screw, origin, 1e-7));
            let step = write(&screw).unwrap();
            let actual = verify_helical_screw(&step, &screw, origin, u, v, 1e-7).unwrap();
            assert!((actual - screw.volume_mm3()).abs() < 1e-6);
            assert_eq!(step.matches("ADVANCED_FACE").count(), 13);
            assert_eq!(step.matches("PLANE(").count(), 9);
        }
        let authored = screw(false);
        let partial = Screw::new(
            *authored.thread(),
            ScrewSpec {
                shank_length_mm: 1.,
                ..authored.spec()
            },
        )
        .unwrap();
        let encoded = write(&authored).unwrap();
        assert!(!supports_helical_screw(&partial, origin, 1e-7));
        assert_eq!(write(&partial), Err(StepError::Unsupported));
        assert_eq!(
            verify_helical_screw(&encoded, &partial, origin, u, v, 1e-7),
            Err(ReadError::Unsupported)
        );
        let thin_seat = Screw::new(
            *authored.thread(),
            ScrewSpec {
                head: ScrewHead::ExternalHex {
                    across_flats_mm: 6.000_000_01,
                    height_mm: 4.,
                },
                ..authored.spec()
            },
        )
        .unwrap();
        assert_eq!(write(&thin_seat), Err(StepError::Limit));
    }
}
