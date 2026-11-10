use super::*;
use crate::{
    planar_boolean::ConvexPiece,
    step::write_warped_union,
    step_read::{ImportProfile, read_import_solid},
};

const TOL: f64 = 1e-7;
const U: Point = [0.8, 0.6, 0.];
const V: Point = [-0.36, 0.48, 0.8];

fn fixture(center_z: f64) -> warped_union::Shell {
    let stock = ConvexPiece::from_box([0.; 3], [10., 20., 8.], TOL).unwrap();
    let shell = warped_union::from_planar_union(&Union::new(&[stock], TOL).unwrap(), TOL).unwrap();
    shell
        .cut_open_axial(
            &AxialCut {
                start_mm: [10.05, 10., center_z],
                end_mm: [5., 10., center_z],
                radius_mm: 2.,
                source: 1,
                wall_source_face: 2,
                cap_source_faces: [0, 1],
                recess: None,
            },
            OpenBoreSide {
                point_mm: [0., 0., 8.],
                normal: [0., 0., 1.],
            },
            TOL,
        )
        .unwrap()
}

fn text() -> String {
    write_warped_union(&fixture(6.5), [0.; 3], [1., 0., 0.], [0., 1., 0.]).unwrap()
}

fn parse<'a, 'input>(
    entities: &'a BTreeMap<u32, Vec<Entity<'input>>>,
) -> (Shell<'a, 'input>, Boundary) {
    let mut reader = Shell::new(entities, TOL, true);
    let id = reader.represented_solids(true).unwrap()[0];
    let boundary = Boundary::parse(&mut reader, id).unwrap();
    (reader, boundary)
}

#[test]
fn reimports_major_and_minor_open_bores_in_a_rotated_placement() {
    for center_z in [6.5, 9.5] {
        let expected = fixture(center_z);
        let text = write_warped_union(&expected, [13., -7., 42.], U, V).unwrap();
        let (readback, import) = read_import_solid(&text, TOL).unwrap();
        let ImportProfile::WarpedUnion { shell } = import.profile else {
            panic!("expected open bore")
        };
        assert_eq!(shell.faces().len(), expected.faces().len());
        assert_eq!(shell.vertices().len(), expected.vertices().len());
        assert_eq!(shell.edges().len(), expected.edges().len());
        assert_eq!(shell.bounds_mm(), readback.bounds_mm);
        assert!((shell.volume_mm3() - expected.volume_mm3()).abs() < TOL);
        let area = |s: &warped_union::Shell| s.faces().iter().map(|f| f.area_mm2).sum::<f64>();
        assert!((area(&shell) - area(&expected)).abs() < TOL);
    }
}

#[test]
fn requires_both_contiguous_notches_to_restore_the_shared_edge() {
    let source = text();
    let entities = parse_entities(&source, TOL).unwrap();
    let (reader, boundary) = parse(&entities);
    let bore = Bore::read(&boundary, TOL).unwrap().unwrap();
    let rings = bore.restore(&reader, &boundary).unwrap();
    assert_eq!(rings.len(), 6);
    let mut changed = boundary.clone();
    let roof = &mut changed.faces[bore.roof];
    let position = roof
        .loop_
        .iter()
        .position(|(id, _)| *id == bore.chord)
        .unwrap();
    let outside = (position + 3) % roof.loop_.len();
    roof.loop_.swap(position, outside);
    assert_eq!(bore.restore(&reader, &changed), Err(ReadError::Invalid));

    let wire = [(1, true), (2, true), (3, true), (4, true)];
    assert_eq!(span(&wire, &BTreeSet::new()), Err(ReadError::Invalid));
    assert_eq!(
        span(&wire, &BTreeSet::from([1, 3])),
        Err(ReadError::Invalid)
    );
    assert_eq!(span(&wire, &BTreeSet::from([5])), Err(ReadError::Invalid));
    assert_eq!(span(&wire, &BTreeSet::from([1, 4])), Ok([3, 0]));
    assert_eq!(
        boundary_error(planar_boolean::Error::Limit),
        ReadError::Limit
    );
    assert_eq!(
        boundary_error(planar_boolean::Error::Invalid),
        ReadError::Invalid
    );
}

#[test]
fn rejects_a_wall_with_incorrect_radius_or_sense() {
    let source = text();
    let entities = parse_entities(&source, TOL).unwrap();
    let (_, boundary) = parse(&entities);
    let bore = Bore::read(&boundary, TOL).unwrap().unwrap();
    let first = *bore.walls.first().unwrap();
    let last = *bore.walls.last().unwrap();
    for index in [first, last] {
        let mut changed = boundary.clone();
        if let Surface::Cylinder { outward, .. } = &mut changed.faces[index].surface {
            *outward = true
        }
        assert!(Bore::read(&changed, TOL).unwrap().is_none());
    }
    let mut changed = boundary.clone();
    if let Surface::Cylinder { radius, .. } = &mut changed.faces[last].surface {
        *radius *= 2.
    }
    assert!(Bore::read(&changed, TOL).unwrap().is_none());
}

fn move_use(boundary: &mut Boundary, id: u32, from: usize, to: usize) {
    let wire = &mut boundary.faces[from].loop_;
    let index = wire.iter().position(|(edge, _)| *edge == id).unwrap();
    let usage = wire.remove(index);
    boundary.faces[to].loop_.push(usage);
}

fn change_circle(boundary: &mut Boundary, edge: u32, shift: f64, scale: f64) {
    let trim = &mut boundary.curves.get_mut(&edge).unwrap().1;
    let TrimCurve::Arc(arc) = *trim else {
        panic!("expected circular edge")
    };
    let ellipse = arc.ellipse();
    let center = std::array::from_fn(|i| ellipse.center_mm()[i] + shift * ellipse.normal()[i]);
    let shifted = Ellipse::new(
        center,
        ellipse.normal(),
        ellipse.major_axis(),
        ellipse.semiaxes_mm().map(|r| r * scale),
    )
    .unwrap();
    *trim = TrimCurve::Arc(EllipseArc::new(shifted, arc.start_rad(), arc.sweep_rad()).unwrap());
}

#[test]
fn recovers_the_same_stock_when_step_lists_the_floor_before_the_mouth() {
    let source = text();
    let entities = parse_entities(&source, TOL).unwrap();
    let (reader, mut boundary) = parse(&entities);
    boundary.faces.reverse();
    assert!(reconstruct(&reader, &boundary).unwrap().is_some());
}

#[test]
fn rejects_bad_axial_groups_and_the_wrong_roof_incidence() {
    let source = text();
    let entities = parse_entities(&source, TOL).unwrap();
    let (_, boundary) = parse(&entities);
    let bore = Bore::read(&boundary, TOL).unwrap().unwrap();
    let entry_arc = *bore.entry_rim.edges.first().unwrap();
    let rail = *bore.rails.first().unwrap();
    let other = (0..boundary.faces.len())
        .find(|i| ![bore.entry, bore.floor, bore.roof].contains(i) && !bore.walls.contains(i))
        .unwrap();
    for (shift, scale) in [(0., 2.), (0.1, 1.)] {
        let mut changed = boundary.clone();
        change_circle(&mut changed, entry_arc, shift, scale);
        assert!(Bore::read(&changed, TOL).unwrap().is_none());
    }
    let mut changed = boundary.clone();
    changed.curves.get_mut(&entry_arc).unwrap().1 = TrimCurve::Line;
    assert!(Bore::read(&changed, TOL).unwrap().is_none());

    let mut changed = boundary.clone();
    move_use(&mut changed, rail, bore.roof, other);
    assert!(Bore::read(&changed, TOL).unwrap().is_none());
    assert!(
        single_roof(
            &boundary,
            BTreeMap::from([(*bore.walls.first().unwrap(), bore.rails.clone())])
        )
        .is_none()
    );

    let mut changed = boundary.clone();
    move_use(&mut changed, entry_arc, bore.entry, other);
    assert!(Bore::read(&changed, TOL).unwrap().is_none());

    let mut changed = boundary.clone();
    changed.curves.get_mut(&bore.chord).unwrap().1 = boundary.curves[&entry_arc].1;
    assert!(Bore::read(&changed, TOL).unwrap().is_none());

    let mut changed = boundary.clone();
    move_use(&mut changed, bore.chord, bore.roof, other);
    assert!(Bore::read(&changed, TOL).unwrap().is_none());
}

#[test]
fn refuses_unrestored_curves_and_a_broken_straight_wire() {
    let source = text();
    let entities = parse_entities(&source, TOL).unwrap();
    let (reader, boundary) = parse(&entities);
    let bore = Bore::read(&boundary, TOL).unwrap().unwrap();
    let wire = &boundary.faces[bore.entry].loop_;
    let [first, _] = span(wire, &bore.entry_rim.edges).unwrap();
    let tail = wire[(first + wire.len() - 1) % wire.len()].0;
    let circle = boundary.curves[&wire[first].0].1;

    let mut changed = boundary.clone();
    changed.curves.get_mut(&tail).unwrap().1 = circle;
    assert_eq!(bore.restore(&reader, &changed), Err(ReadError::Invalid));

    let other = (0..boundary.faces.len())
        .find(|i| ![bore.entry, bore.floor, bore.roof].contains(i) && !bore.walls.contains(i))
        .unwrap();
    let mut changed = boundary.clone();
    let edge = changed.faces[other].loop_[0].0;
    changed.curves.get_mut(&edge).unwrap().1 = circle;
    assert_eq!(bore.restore(&reader, &changed), Err(ReadError::Unsupported));

    let mut changed = boundary.clone();
    changed.faces[other].loop_[0].1 ^= true;
    assert_eq!(bore.restore(&reader, &changed), Err(ReadError::Invalid));
}
