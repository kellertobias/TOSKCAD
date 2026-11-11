use super::*;
#[test]
fn partial_raw_correspondence_is_not_a_relief_and_resources_recover() {
    let relief = super::super::tests::fixture(Kind::Raised);
    let text =
        crate::step::write_normal_relief(&relief, [0.; 3], [1., 0., 0.], [0., 1., 0.]).unwrap();
    let entities = parse_entities(&text, 1e-7).unwrap();
    let mut shell = Shell::new(&entities, 1e-7, true);
    let ids = shell.shell_faces().unwrap();
    let faces = shell.validate_faces(ids.clone()).unwrap();
    let readback = shell.readback(&faces).unwrap();
    assert_eq!(
        recognize_with_work(&shell, &ids, &faces, &readback, &mut Work(0)),
        Err(ReadError::Limit)
    );
    assert!(
        recognize_with_work(&shell, &ids, &faces, &readback, &mut Work(MAX_WORK))
            .unwrap()
            .is_some()
    );
    let frame = Frame::new([0.; 3], [1., 0., 0.], [0., 1., 0.]).unwrap();
    let top = faces
        .iter()
        .enumerate()
        .max_by_key(|(i, _)| loops(&shell, ids[*i], &mut Work(MAX_WORK)).unwrap().len())
        .unwrap()
        .0;
    let wires = outer_loops(&shell, ids[top], &mut Work(MAX_WORK)).unwrap();
    assert!(
        profile(
            &shell,
            &wires[0],
            frame,
            false,
            4.,
            1e-8,
            &mut Work(MAX_WORK)
        )
        .is_ok()
    );
    let mut face_edges = vec![vec![]; faces.len()];
    for (&id, edge) in &shell.edges {
        for use_ in &edge.uses {
            face_edges[use_.face].push(id);
        }
    }
    let source = wires.last().unwrap();
    let edge_id = source[0].0;
    let previous = std::mem::take(&mut shell.edges.get_mut(&edge_id).unwrap().uses);
    assert!(
        adjacent_cap(
            &shell,
            &faces,
            source,
            &face_edges,
            top,
            [0., 0., 1.],
            &mut Work(MAX_WORK)
        )
        .is_err()
    );
    shell.edges.get_mut(&edge_id).unwrap().uses = previous;
    let mut empty_edges = face_edges.clone();
    for edges in &mut empty_edges {
        edges.clear();
    }
    assert!(
        adjacent_cap(
            &shell,
            &faces,
            source,
            &empty_edges,
            top,
            [0., 0., 1.],
            &mut Work(MAX_WORK)
        )
        .is_err()
    );
    assert!(
        recognize_with_work(&shell, &ids, &faces, &readback, &mut Work(MAX_WORK))
            .unwrap()
            .is_some()
    );
}
#[test]
fn actual_relief_constructor_refusals_retain_typed_reader_errors() {
    let source = super::super::tests::fixture(Kind::Raised);
    let error = Relief::new(
        source.base().clone(),
        vec![source.motifs()[0].clone(); 193],
        Kind::Raised,
        1e-7,
        1e-5,
    )
    .unwrap_err();
    assert_eq!(relief_error(error), ReadError::Limit);
    let error = Relief::new(source.base().clone(), vec![], Kind::Raised, 1e-7, 1e-5).unwrap_err();
    assert_eq!(relief_error(error), ReadError::Invalid);
}
#[test]
fn an_ordinary_straight_extrusion_has_no_relief_cap() {
    let mut writer = crate::step::Writer::new();
    writer
        .append_box(
            [crate::Rational::new(2, 1).unwrap(); 3],
            [0.; 3],
            [1., 0., 0.],
            [0., 1., 0.],
        )
        .unwrap();
    let source = writer.finish().unwrap();
    let entities = parse_entities(&source, 1e-7).unwrap();
    let mut shell = Shell::new(&entities, 1e-7, true);
    let ids = shell.shell_faces().unwrap();
    let faces = shell.validate_faces(ids.clone()).unwrap();
    let readback = shell.readback(&faces).unwrap();
    assert!(
        recognize(&shell, &ids, &faces, &readback)
            .unwrap()
            .is_none()
    );
}
