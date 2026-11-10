use super::super::tests::boss_step_fixture;
use super::*;

const TOL: f64 = 1e-7;

fn parse<'a, 'input>(
    entities: &'a BTreeMap<u32, Vec<Entity<'input>>>,
) -> (Shell<'a, 'input>, Boundary, Vec<Attachment>) {
    let mut reader = Shell::new(entities, TOL, true);
    let solid = reader.represented_solids(true).unwrap()[0];
    let boundary = Boundary::parse(&mut reader, solid).unwrap();
    let bosses = attachments(&reader, &boundary).unwrap();
    (reader, boundary, bosses)
}

fn circle(boundary: &mut Boundary, edge: u32, center: Point, radius: f64) {
    let curve = &mut boundary.curves.get_mut(&edge).unwrap().1;
    let TrimCurve::Arc(old) = *curve else {
        unreachable!()
    };
    let ellipse = Ellipse::new(
        center,
        old.ellipse().normal(),
        old.ellipse().major_axis(),
        [radius; 2],
    )
    .unwrap();
    *curve = TrimCurve::Arc(EllipseArc::new(ellipse, old.start_rad(), old.sweep_rad()).unwrap());
}

fn cap_edge(boundary: &Boundary, boss: &Attachment, side: usize) -> u32 {
    let wall = &boundary.faces[boss.walls[side]];
    let cap = &boundary.faces[boss.cap];
    wall.loop_
        .iter()
        .find(|&&(id, _)| cap.loop_.iter().any(|&(cap_id, _)| id == cap_id))
        .unwrap()
        .0
}

#[test]
fn rejects_non_cylindrical_and_inconsistent_boss_incidence() {
    let (_, step) = boss_step_fixture();
    let entities = parse_entities(&step, TOL).unwrap();
    let (reader, original, bosses) = parse(&entities);
    let boss = &bosses[0];
    let host = boss.host;
    let wall = boss.walls[0];
    let cap = boss.cap;
    let rim = original.faces[host].holes[0][3];
    let far = cap_edge(&original, boss, 0);
    let wall_rail = original.faces[wall]
        .loop_
        .iter()
        .find(|&&(id, _)| matches!(original.curves[&id].1, TrimCurve::Line))
        .unwrap()
        .0;

    let mut changed = original.clone();
    changed.faces[host].surface = original.faces[wall].surface;
    assert!(matches!(
        attachments(&reader, &changed),
        Err(ReadError::Unsupported)
    ));
    let mut changed = original.clone();
    changed.faces[host].holes[0].pop();
    assert!(matches!(
        attachments(&reader, &changed),
        Err(ReadError::Unsupported)
    ));
    let mut changed = original.clone();
    changed.curves.get_mut(&rim.0).unwrap().1 = TrimCurve::Line;
    assert!(matches!(
        attachments(&reader, &changed),
        Err(ReadError::Unsupported)
    ));
    let mut changed = original.clone();
    changed.faces[wall].surface = original.faces[host].surface;
    assert!(matches!(
        attachments(&reader, &changed),
        Err(ReadError::Unsupported)
    ));

    let mut changed = original.clone();
    changed.faces[wall].loop_.retain(|&(id, _)| id != rim.0);
    assert!(matches!(
        attachments(&reader, &changed),
        Err(ReadError::Invalid)
    ));
    changed.faces[host].loop_.push(rim);
    assert!(matches!(
        attachments(&reader, &changed),
        Err(ReadError::Invalid)
    ));

    let mut changed = original.clone();
    changed.curves.get_mut(&far).unwrap().1 = TrimCurve::Line;
    assert!(matches!(
        attachments(&reader, &changed),
        Err(ReadError::Unsupported)
    ));
    let mut changed = original.clone();
    changed.curves.get_mut(&wall_rail).unwrap().1 = original.curves[&far].1;
    assert!(matches!(
        attachments(&reader, &changed),
        Err(ReadError::Unsupported)
    ));
    let mut changed = original.clone();
    circle(
        &mut changed,
        cap_edge(&original, boss, 2),
        boss.top.map(|x| x + 1.),
        boss.radius,
    );
    assert!(matches!(
        attachments(&reader, &changed),
        Err(ReadError::Invalid)
    ));
    let mut changed = original.clone();
    changed.faces[cap].surface = original.faces[wall].surface;
    assert!(matches!(
        attachments(&reader, &changed),
        Err(ReadError::Unsupported)
    ));

    let mut changed = original.clone();
    let other = std::mem::take(&mut changed.faces[bosses[1].cap].loop_);
    changed.faces[cap].loop_.extend(other);
    assert_eq!(
        reconstruct(&reader, &changed).unwrap_err(),
        ReadError::Invalid
    );
}

#[test]
fn rejects_bosses_that_cannot_be_replayed_on_the_same_certified_stock() {
    let (_, step) = boss_step_fixture();
    let entities = parse_entities(&step, TOL).unwrap();
    let (mut reader, original, bosses) = parse(&entities);
    let boss = &bosses[0];
    let first = original.faces[boss.host].holes[0][3].0;
    let second = original.faces[boss.host].holes[0][2].0;

    let mut changed = original.clone();
    circle(&mut changed, first, boss.mouth, 1e9);
    assert_eq!(
        reconstruct(&reader, &changed).unwrap_err(),
        ReadError::Limit
    );
    circle(&mut changed, first, boss.mouth, 100.);
    assert_eq!(
        reconstruct(&reader, &changed).unwrap_err(),
        ReadError::Invalid
    );
    let mut changed = original.clone();
    circle(
        &mut changed,
        second,
        boss.mouth.map(|x| x + 1.),
        boss.radius,
    );
    assert_eq!(
        reconstruct(&reader, &changed).unwrap_err(),
        ReadError::Invalid
    );

    let mut changed = original.clone();
    let wall = &mut changed.faces[boss.walls[0]];
    let Surface::Cylinder { radius, .. } = &mut wall.surface else {
        unreachable!()
    };
    *radius *= 2.;
    assert_eq!(
        reconstruct(&reader, &changed).unwrap_err(),
        ReadError::Invalid
    );
    let mut changed = original.clone();
    let wall = &mut changed.faces[boss.walls[0]];
    let rail = *wall
        .loop_
        .iter()
        .find(|&&(id, _)| matches!(changed.curves[&id].1, TrimCurve::Line))
        .unwrap();
    wall.loop_.push(rail);
    assert_eq!(
        reconstruct(&reader, &changed).unwrap_err(),
        ReadError::Invalid
    );

    reader.points.insert(u32::MAX, [1000.; 3]);
    assert_eq!(
        reconstruct(&reader, &original).unwrap_err(),
        ReadError::Invalid
    );
}
