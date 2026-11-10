use super::*;
use crate::{planar_boolean::ConvexPiece, step::write_warped_union, warped_union::Boss};
use std::f64::consts::TAU;

const TOL: f64 = 1e-7;
const U: Point = [0.8, 0.6, 0.];
const V: Point = [-0.36, 0.48, 0.8];
const ORIGIN: Point = [13., -7., 42.];

fn fixture(with_boss: bool) -> warped_union::Shell {
    let stock = ConvexPiece::from_box([0., 0., 3.], [10., 80., 30.], TOL).unwrap();
    let base = warped_union::remap(
        &Union::new(&[stock], TOL).unwrap(),
        Band {
            center_z_mm: 0.,
            radius_mm: 2.,
        },
        TOL,
    )
    .unwrap();
    let base = if with_boss {
        base.attach_bosses(
            &[Boss {
                start_mm: [9.95, 20., 20.],
                end_mm: [13.5, 20., 20.],
                radius_mm: 5.5,
                source: 0,
                wall_source_face: 0,
                cap_source_face: 0,
            }],
            TOL,
        )
        .unwrap()
    } else {
        base
    };
    let rail = AxialCut {
        start_mm: [-0.05, 20., 20.],
        end_mm: [if with_boss { 13.55 } else { 10.05 }, 20., 20.],
        radius_mm: 1.5,
        source: 1,
        wall_source_face: 0,
        cap_source_faces: [1, 2],
        recess: with_boss.then(|| ConvexRecess {
            rim_mm: (0..6)
                .map(|i| {
                    let (sin, cos) = (TAU * i as f64 / 6.).sin_cos();
                    [11., 20. + 3. * cos, 20. + 3. * sin]
                })
                .collect(),
            travel_mm: [2.55, 0., 0.],
            source: 2,
            wall_source_faces: (0..6).collect(),
            cap_source_faces: [6, 7],
        }),
    };
    let blind = AxialCut {
        start_mm: [5., 50., 20.],
        end_mm: [-0.05, 50., 20.],
        radius_mm: 1.8,
        source: 3,
        wall_source_face: 0,
        cap_source_faces: [1, 2],
        recess: None,
    };
    base.cut_axial(&[rail, blind], TOL).unwrap()
}

fn parse<'a, 'input>(
    entities: &'a BTreeMap<u32, Vec<Entity<'input>>>,
) -> (Shell<'a, 'input>, Boundary) {
    let mut reader = Shell::new(entities, TOL, true);
    let solid = reader.represented_solids(true).unwrap()[0];
    let boundary = Boundary::parse(&mut reader, solid).unwrap();
    (reader, boundary)
}

#[test]
fn replay_preserves_blind_through_and_widened_boss_cuts_in_an_oblique_frame() {
    for with_boss in [false, true] {
        let original = fixture(with_boss);
        let text = write_warped_union(&original, ORIGIN, U, V).unwrap();
        let result = read_warped_solid(&text, TOL).unwrap();
        let actual = result.shell;
        assert_eq!(actual.faces().len(), original.faces().len());
        assert_eq!(actual.vertices().len(), original.vertices().len());
        assert_eq!(actual.edges().len(), original.edges().len());
        assert!((actual.volume_mm3() - original.volume_mm3()).abs() <= TOL);
        let area = |shell: &warped_union::Shell| {
            shell.faces().iter().map(|face| face.area_mm2).sum::<f64>()
        };
        assert!((area(&actual) - area(&original)).abs() <= TOL);
        assert_eq!(actual.solid_count(), 1);
    }
}

#[test]
fn replay_refuses_an_inconsistent_cylinder_sense_and_a_shifted_shoulder() {
    let original = fixture(true);
    let text = write_warped_union(&original, ORIGIN, U, V).unwrap();
    let entities = parse_entities(&text, TOL).unwrap();
    let (mut reader, boundary) = parse(&entities);
    let mut changed = boundary.clone();
    let inward = changed
        .faces
        .iter_mut()
        .find(|face| matches!(face.surface, Surface::Cylinder { outward: false, .. }))
        .unwrap();
    if let Surface::Cylinder { outward, .. } = &mut inward.surface {
        *outward = true;
    }
    assert!(reconstruct(&reader, &changed).is_err());

    let shoulder = boundary
        .faces
        .iter()
        .find(|face| face.loop_.len() == 6 && face.holes.len() == 1)
        .unwrap();
    let id = boundary.curves[&shoulder.loop_[0].0].0[0];
    reader.points.get_mut(&id).unwrap()[2] += 0.03;
    assert!(reconstruct(&reader, &boundary).is_err());
}

fn local_fixture(with_boss: bool) -> String {
    write_warped_union(&fixture(with_boss), [0.; 3], [1., 0., 0.], [0., 1., 0.]).unwrap()
}

fn patches(source: &Boundary) -> Vec<Option<Patch>> {
    source
        .faces
        .iter()
        .map(|face| Patch::read(source, face, TOL))
        .collect()
}

fn removal_from(
    first: usize,
    patches: &[Option<Patch>],
    incidence: &Incidence,
) -> Result<Option<Removal>> {
    Removal::read(
        first,
        patches[first].unwrap(),
        patches,
        incidence,
        &mut BTreeSet::new(),
        TOL,
    )
}

fn at_center(arc: EllipseArc, center: Point) -> EllipseArc {
    let ellipse = arc.ellipse();
    let translated = Ellipse::new(
        center,
        ellipse.normal(),
        ellipse.major_axis(),
        ellipse.semiaxes_mm(),
    )
    .unwrap();
    EllipseArc::new(translated, arc.start_rad(), arc.sweep_rad()).unwrap()
}

#[test]
fn rejects_unpaired_edges_incomplete_rings_and_inconsistent_rim_levels() {
    let source = local_fixture(true);
    let entities = parse_entities(&source, TOL).unwrap();
    let (_, boundary) = parse(&entities);
    let mut incidence = Incidence::new(&boundary);
    let mut candidates = patches(&boundary);
    let first = candidates.iter().position(Option::is_some).unwrap();
    let component = removal_from(first, &candidates, &incidence)
        .unwrap()
        .unwrap();
    let rail = candidates[first].unwrap().rails[0];
    let old = incidence.0[&rail].clone();
    incidence.0.get_mut(&rail).unwrap().pop();
    assert_eq!(incidence.adjacent(rail, first), Err(ReadError::Invalid));
    incidence.0.insert(rail, old.clone());
    assert_eq!(
        incidence.adjacent(rail, usize::MAX),
        Err(ReadError::Invalid)
    );

    let external = candidates.iter().position(Option::is_none).unwrap();
    let neighbor = incidence.adjacent(rail, first).unwrap().face;
    let extra = u32::MAX;
    let original_neighbor = candidates[neighbor].unwrap();
    *candidates[neighbor]
        .as_mut()
        .unwrap()
        .rails
        .iter_mut()
        .find(|id| **id == rail)
        .unwrap() = extra;
    incidence
        .0
        .get_mut(&rail)
        .unwrap()
        .iter_mut()
        .find(|usage| usage.face != first)
        .unwrap()
        .face = external;
    incidence.0.insert(
        extra,
        vec![
            Use {
                face: neighbor,
                hole: None,
            },
            Use {
                face: external,
                hole: None,
            },
        ],
    );
    assert!(matches!(
        removal_from(first, &candidates, &incidence),
        Err(ReadError::Invalid)
    ));
    candidates[neighbor] = Some(original_neighbor);
    incidence.0.insert(rail, old.clone());

    let fifth = candidates.len();
    let neighbor = incidence.adjacent(rail, first).unwrap().face;
    let extra = u32::MAX;
    incidence
        .0
        .get_mut(&rail)
        .unwrap()
        .iter_mut()
        .find(|u| u.face == neighbor)
        .unwrap()
        .face = fifth;
    incidence.0.insert(
        extra,
        vec![
            Use {
                face: fifth,
                hole: None,
            },
            Use {
                face: neighbor,
                hole: None,
            },
        ],
    );
    let saved = candidates[neighbor].unwrap();
    let old_rail = candidates[neighbor]
        .as_mut()
        .unwrap()
        .rails
        .iter_mut()
        .find(|id| **id == rail)
        .unwrap();
    *old_rail = extra;
    let mut added = candidates[first].unwrap();
    added.rails = [rail, extra];
    candidates.push(Some(added));
    assert!(matches!(
        removal_from(first, &candidates, &incidence),
        Err(ReadError::Unsupported)
    ));
    candidates.pop();
    candidates[neighbor] = Some(saved);
    incidence.0.insert(rail, old);

    let first_rim = candidates[first].as_mut().unwrap();
    let arc = first_rim.rims[0].1;
    let moved = std::array::from_fn(|i| arc.ellipse().center_mm()[i] + first_rim.axis[i] * 0.01);
    first_rim.rims[0].1 = at_center(arc, moved);
    assert!(matches!(
        removal_from(first, &candidates, &incidence),
        Err(ReadError::Invalid)
    ));
    for index in component.remove {
        let patch = candidates[index].as_mut().unwrap();
        patch.rims.sort_by(|a, b| {
            dot(a.1.ellipse().center_mm(), patch.axis)
                .total_cmp(&dot(b.1.ellipse().center_mm(), patch.axis))
        });
        for (_, arc) in &mut patch.rims {
            *arc = at_center(*arc, component.ends[0].center);
        }
    }
    assert!(matches!(
        removal_from(first, &candidates, &incidence),
        Err(ReadError::Invalid)
    ));
}

#[test]
fn rejects_malformed_end_support_and_reusing_faces_or_holes() {
    let source = local_fixture(true);
    let entities = parse_entities(&source, TOL).unwrap();
    let (reader, boundary) = parse(&entities);
    let incidence = Incidence::new(&boundary);
    let candidates = patches(&boundary);
    let first = candidates.iter().position(Option::is_some).unwrap();
    let removal = || {
        removal_from(first, &candidates, &incidence)
            .unwrap()
            .unwrap()
    };
    let mut malformed = boundary.clone();
    let end = removal().ends[0];
    malformed.faces[end.at.face].surface = boundary.faces[first].surface;
    assert!(matches!(
        reconstruct(&reader, &malformed),
        Err(ReadError::Unsupported)
    ));
    assert!(matches!(
        recover(
            &reader,
            &malformed,
            &incidence,
            removal(),
            &mut BTreeSet::new(),
            &mut BTreeSet::new()
        ),
        Err(ReadError::Unsupported)
    ));

    let mut floor = removal();
    floor.ends[0].at.hole = None;
    assert!(matches!(
        recover(
            &reader,
            &boundary,
            &incidence,
            floor,
            &mut BTreeSet::new(),
            &mut BTreeSet::new()
        ),
        Err(ReadError::Unsupported)
    ));

    let mut removed = BTreeSet::from([first]);
    assert!(matches!(
        recover(
            &reader,
            &boundary,
            &incidence,
            removal(),
            &mut removed,
            &mut BTreeSet::new()
        ),
        Err(ReadError::Invalid)
    ));

    let mut filled = BTreeSet::new();
    let good = recover(
        &reader,
        &boundary,
        &incidence,
        removal(),
        &mut BTreeSet::new(),
        &mut filled,
    )
    .unwrap();
    assert!(matches!(
        recover(
            &reader,
            &boundary,
            &incidence,
            removal(),
            &mut BTreeSet::new(),
            &mut filled
        ),
        Err(ReadError::Invalid)
    ));
    assert!(matches!(
        uncut(
            &reader,
            &boundary,
            &BTreeSet::from([filled.first().unwrap().face]),
            &filled
        ),
        Err(ReadError::Invalid)
    ));

    let widening = good.widen.unwrap();
    assert_eq!(
        retain_widening(&mut Some(widening.clone()), widening.clone()),
        Err(ReadError::Unsupported)
    );
    let mut recipes = (0..64)
        .map(|_| Recipe {
            ends: good.ends,
            radial: good.radial,
            radius: good.radius,
            widen: None,
        })
        .collect();
    assert_eq!(
        add_recipe(
            &mut recipes,
            Recipe {
                ends: good.ends,
                radial: good.radial,
                radius: good.radius,
                widen: Some(widening)
            }
        ),
        Err(ReadError::Limit)
    );
    assert_eq!(replay_error(warped_union::Error::Limit), ReadError::Limit);
    assert_eq!(
        replay_error(warped_union::Error::Unsupported),
        ReadError::Invalid
    );
}

#[test]
fn rejects_recess_walls_that_do_not_share_one_convex_mouth() {
    let source = local_fixture(true);
    let entities = parse_entities(&source, TOL).unwrap();
    let (reader, boundary) = parse(&entities);
    let incidence = Incidence::new(&boundary);
    let candidates = patches(&boundary);
    let first = candidates.iter().position(Option::is_some).unwrap();
    let removal = removal_from(first, &candidates, &incidence)
        .unwrap()
        .unwrap();
    let (index, end) = removal
        .ends
        .into_iter()
        .enumerate()
        .find(|(_, end)| boundary.faces[end.at.face].loop_.len() == 6)
        .unwrap();
    let direction = [-1., 1.][index];
    let read = |boundary, incidence, direction| {
        recess::read(&reader, boundary, incidence, end, removal.axis, direction)
            .unwrap()
            .is_some()
    };
    assert!(read(&boundary, &incidence, direction));
    assert!(!read(&boundary, &incidence, -direction));
    let shoulder_edge = boundary.faces[end.at.face].loop_[0].0;
    let side = incidence.adjacent(shoulder_edge, end.at.face).unwrap().face;
    let circle = candidates[first].unwrap().rims[0].1;
    let mut changed = boundary.clone();
    changed.curves.get_mut(&shoulder_edge).unwrap().1 = TrimCurve::Arc(circle);
    assert!(!read(&changed, &incidence, direction));

    let mut changed = boundary.clone();
    changed.faces[side].surface = boundary.faces[first].surface;
    assert!(!read(&changed, &incidence, direction));

    let height = |id| dot(sub(reader.points[&id], end.center), removal.axis);
    let far = |face: usize| {
        boundary.faces[face]
            .loop_
            .iter()
            .copied()
            .find(|&(edge, _)| {
                let [a, b] = boundary.curves[&edge].0.map(height);
                (a - b).abs() <= TOL && a * direction > TOL
            })
            .unwrap()
    };
    let far_edge = far(side);
    let mut changed = boundary.clone();
    changed.curves.get_mut(&far_edge.0).unwrap().1 = TrimCurve::Arc(circle);
    assert!(!read(&changed, &incidence, direction));

    let mut changed = boundary.clone();
    changed.faces[side].loop_[0] = far_edge;
    changed.faces[side].loop_[1] = far_edge;
    assert!(!read(&changed, &incidence, direction));

    let mut changed_incidence = Incidence::new(&boundary);
    changed_incidence
        .0
        .get_mut(&far_edge.0)
        .unwrap()
        .iter_mut()
        .find(|usage| usage.face != side)
        .unwrap()
        .hole = None;
    assert!(!read(&boundary, &changed_incidence, direction));

    let second_edge = boundary.faces[end.at.face].loop_[1].0;
    let second_side = incidence.adjacent(second_edge, end.at.face).unwrap().face;
    let second_far = far(second_side).0;
    let mut changed_incidence = Incidence::new(&boundary);
    changed_incidence
        .0
        .get_mut(&second_far)
        .unwrap()
        .iter_mut()
        .find(|usage| usage.face != second_side)
        .unwrap()
        .hole = Some(usize::MAX);
    assert!(!read(&boundary, &changed_incidence, direction));
}

#[test]
fn rejects_duplicate_identities_in_replayed_wires() {
    let original = fixture(true);
    let text = local_fixture(true);
    let entities = parse_entities(&text, TOL).unwrap();
    let (mut reader, boundary) = parse(&entities);
    let chart = Chart {
        origin: [0.; 3],
        u: [1., 0., 0.],
        v: [0., 1., 0.],
        up: [0., 0., 1.],
        band: Band {
            center_z_mm: 0.,
            radius_mm: 2.,
        },
    };
    verify::replayed(&reader, &boundary, chart, &original).unwrap();
    assert_eq!(verify::point_budget(5000), Err(ReadError::Limit));
    let face = &original.faces()[0];
    assert_eq!(
        verify::face_uses(&[face.clone(), face.clone()]),
        Err(ReadError::Invalid)
    );

    let mut changed = boundary.clone();
    changed.faces.pop();
    assert_eq!(
        verify::replayed(&reader, &changed, chart, &original),
        Err(ReadError::Invalid)
    );

    let ids: Vec<_> = reader.points.keys().copied().take(2).collect();
    let old = reader.points[&ids[1]];
    reader.points.insert(ids[1], reader.points[&ids[0]]);
    assert_eq!(
        verify::replayed(&reader, &boundary, chart, &original),
        Err(ReadError::Invalid)
    );
    reader.points.insert(ids[1], old);

    let mut changed = boundary.clone();
    let ids: Vec<_> = changed.curves.keys().copied().take(2).collect();
    changed.curves.insert(ids[1], changed.curves[&ids[0]]);
    assert_eq!(
        verify::replayed(&reader, &changed, chart, &original),
        Err(ReadError::Invalid)
    );

    let mut changed = boundary.clone();
    changed.faces[0].surface = Surface::Plane {
        origin: [0.; 3],
        normal: [1., 0., 0.],
    };
    assert_eq!(
        verify::replayed(&reader, &changed, chart, &original),
        Err(ReadError::Invalid)
    );
}

#[test]
fn invalid_radial_reference_and_mismatched_arc_support_are_rejected() {
    let original = fixture(true);
    let cut = AxialCut {
        start_mm: [-0.05, 60., 20.],
        end_mm: [5., 60., 20.],
        radius_mm: 1.5,
        source: 1,
        wall_source_face: 0,
        cap_source_faces: [1, 2],
        recess: None,
    };
    assert_eq!(
        original.cut_axial_with_reference(&[(cut, [1., 0., 0.])], TOL),
        Err(warped_union::Error::Invalid)
    );

    let text = local_fixture(true);
    let entities = parse_entities(&text, TOL).unwrap();
    let (_, mut boundary) = parse(&entities);
    let index = patches(&boundary).iter().position(Option::is_some).unwrap();
    if let Surface::Cylinder { radius, .. } = &mut boundary.faces[index].surface {
        *radius *= 2.;
    }
    assert!(Patch::read(&boundary, &boundary.faces[index], TOL).is_none());
}
