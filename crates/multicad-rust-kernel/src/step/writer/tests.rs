use super::*;

const X: Point = [1., 0., 0.];
const Y: Point = [0., 1., 0.];
const ORIGIN: Point = [0.; 3];

fn size() -> [Rational; 3] {
    [5, 7, 9].map(|value| Rational::new(value, 1).unwrap())
}

#[test]
fn orthogonal_through_bores_share_two_stock_mouths_and_coexist_with_blind_pockets() {
    use crate::{
        orthogonal::{Box3, Operation, boolean},
        orthogonal_boundary::build,
        step_read::{ReadError, read_analytic_solid, read_import_solid},
    };
    let stock = boolean(
        &[Box3 {
            min: ORIGIN,
            max: [20., 16., 10.],
        }],
        &[],
        Operation::Union,
        1e-7,
    )
    .unwrap()
    .unwrap();
    let boundary = build(stock.quads()).unwrap();
    let side = |normal| {
        boundary
            .faces
            .iter()
            .position(|face| face.normal == normal)
            .unwrap()
    };
    let bore = OrthogonalThroughBore {
        faces: [side([-1., 0., 0.]), side(X)],
        centers_mm: [[0., 5., 4.], [20., 5., 4.]],
        radius_mm: 1.5,
    };
    let blind = OrthogonalPocket {
        face: side([0., 0., 1.]),
        center_mm: [16., 12., 10.],
        radius_mm: 2.,
        depth_mm: 3.,
    };
    let place = |[x, y, z]: Point| [z + 18., x - 2., y + 7.];
    assert!(supports_drilled_orthogonal(&boundary, &[blind], &[bore]));
    let write = |through: &[OrthogonalThroughBore]| {
        write_drilled_orthogonal(&stock, &boundary, &[blind], through, &place)
    };
    let source = write(&[bore]).unwrap();
    let read = read_analytic_solid(&source, 1e-7).unwrap();
    assert_eq!(read.face_count, 9);
    assert_eq!(read.circles.len(), 4);
    let expected =
        stock.volume_mm3() - (20. * 1.5_f64.powi(2) + 3. * 2_f64.powi(2)) * std::f64::consts::PI;
    assert!((read.volume_mm3 - expected).abs() < 1e-7);
    let mut reversed = bore;
    reversed.faces.reverse();
    reversed.centers_mm.reverse();
    let reversed = read_analytic_solid(&write(&[reversed]).unwrap(), 1e-7).unwrap();
    assert!((reversed.volume_mm3 - expected).abs() < 1e-7);
    for center in bore.centers_mm.map(place) {
        assert!(
            reversed
                .circles
                .iter()
                .any(|rim| rim.center_mm == center && rim.radius_mm == bore.radius_mm)
        );
    }
    let slot = OrthogonalSlot {
        face: blind.face,
        centers_mm: [[3., 10., 10.], [7., 10., 10.]],
        radius_mm: 1.,
        depth_mm: 2.,
    };
    // A valid boundary using both cuts is recognizable STEP. The current
    // Rust solid representation deliberately rejects the combined profile.
    let mut foreign = Writer::new();
    foreign
        .append(|file| encode_orthogonal(file, &boundary, &[blind], &[slot], &[bore], &place))
        .unwrap();
    let foreign = foreign.finish().unwrap();
    assert_eq!(read_analytic_solid(&foreign, 1e-7).unwrap().face_count, 14);
    assert_eq!(
        read_import_solid(&foreign, 1e-7),
        Err(ReadError::Unsupported)
    );
    assert_eq!(
        write(&[]).unwrap(),
        write_pocketed_orthogonal(&stock, &boundary, &[blind], &place).unwrap()
    );
    let missing = OrthogonalThroughBore {
        faces: [bore.faces[0], boundary.faces.len()],
        ..bore
    };
    assert_eq!(write(&[missing]), Err(StepError::Unsupported));
    assert_eq!(write(&[bore, bore]), Err(StepError::InvalidGeometry));
    assert_eq!(
        write_drilled_orthogonal(&stock, &boundary, &[blind], &[bore], &|p| {
            if p == bore.centers_mm[0] {
                [f64::NAN; 3]
            } else {
                place(p)
            }
        }),
        Err(StepError::InvalidPlacement),
    );
}

fn split_cone_seam_identity(source: &str, cone_face: &str) -> String {
    let record = |id: &str| {
        source
            .lines()
            .find(|line| line.split_once('=').unwrap_or(("", "")).0 == id)
            .unwrap()
    };
    let (_, args) = cone_face.split_once(",(").unwrap();
    let bound_id = args.split_once(')').unwrap().0;
    let loop_id = record(bound_id).split(',').nth(1).unwrap();
    let oriented_id = record(loop_id)
        .trim_end_matches("));")
        .rsplit(',')
        .next()
        .unwrap();
    let oriented = record(oriented_id);
    let seam_id = oriented.split(',').nth(3).unwrap();
    let duplicate = record(seam_id).replacen(seam_id, "#999999", 1);
    source
        .replacen(oriented, &oriented.replacen(seam_id, "#999999", 1), 1)
        .replace(
            "ENDSEC;\nEND-ISO",
            &format!("{duplicate}\nENDSEC;\nEND-ISO"),
        )
}

#[test]
fn chamfered_through_bores_share_rims_with_analytic_cones_and_caps() {
    use crate::step_read::{ImportProfile, ReadError, read_analytic_solid, read_import_solid};
    let exact = |value| Rational::new(value, 1).unwrap();
    let prism = PolygonPrism::with_circular_holes(
        vec![
            [[0, 0], [24, 0], [24, 18], [0, 18]]
                .map(|uv| uv.map(exact))
                .to_vec(),
        ],
        vec![
            Circle::new([6, 9].map(exact), exact(2)).unwrap(),
            Circle::new([18, 9].map(exact), exact(1)).unwrap(),
        ],
        exact(8),
    )
    .unwrap();
    let chamfers = [
        BoreChamfer {
            hole: 0,
            end: false,
            size_mm: 1.,
        },
        BoreChamfer {
            hole: 0,
            end: true,
            size_mm: 2.,
        },
    ];
    let bores = ChamferedBores::new(&prism, &chamfers, 1e-7, 1e-7).unwrap();
    let u = [0.8, 0.6, 0.];
    let v = [-0.36, 0.48, 0.8];
    for reverse in [false, true] {
        let source =
            write_chamfered_polygon_prism(&prism, &chamfers, [900., -200., 500.], u, v, reverse)
                .unwrap();
        let readback = read_analytic_solid(&source, 1e-7).unwrap();
        assert_eq!(readback.face_count, 10);
        assert_eq!(readback.circles.len(), 6);
        assert_eq!(readback.edges_mm.len(), 12);
        assert!((readback.volume_mm3 - bores.volume_mm3()).abs() < 1e-7);
        assert_eq!(source.matches("=CONICAL_SURFACE(").count(), 2);
        assert_eq!(source.matches("=CYLINDRICAL_SURFACE(").count(), 2);
        let (_, imported) = read_import_solid(&source, 1e-7).unwrap();
        let ImportProfile::ChamferedPolygon {
            rings,
            circular_holes,
            chamfers,
        } = imported.profile
        else {
            panic!("expected a polygon with countersunk bores");
        };
        assert_eq!(rings.len(), 1);
        assert_eq!(circular_holes.len(), 2);
        assert_eq!(chamfers.len(), 2);
        assert!((imported.height_mm - 8.).abs() < 1e-7);
        let sizes = if reverse { [1., 2.] } else { [2., 1.] };
        for (i, chamfer) in chamfers.iter().enumerate() {
            assert_eq!(chamfer.end, i == 1);
            assert!((chamfer.size_mm - sizes[i]).abs() < 1e-7);
            assert!((circular_holes[chamfer.hole].1 - 2.).abs() < 1e-7);
        }

        let cone = source
            .lines()
            .find(|line| line.contains("=CONICAL_SURFACE("))
            .unwrap();
        let (parameters, _) = cone.rsplit_once(',').unwrap();
        let bad_angle = source.replacen(cone, &format!("{parameters},0.3);"), 1);
        let (id, _) = cone.split_once('=').unwrap();
        let cone_face = source
            .lines()
            .find(|line| line.ends_with(&format!(",{id},.F.);")))
            .unwrap();
        let bad_sense = source.replacen(cone_face, &cone_face.replace(".F.", ".T."), 1);
        let missing_angle = source.replacen(cone, &format!("{parameters});"), 1);
        let zero_angle = source.replacen(cone, &format!("{parameters},0.);"), 1);
        let split_seam = split_cone_seam_identity(&source, cone_face);
        for invalid in [bad_angle, bad_sense, missing_angle, zero_angle, split_seam] {
            assert_eq!(read_analytic_solid(&invalid, 1e-7), Err(ReadError::Invalid));
        }
        let bad_units = source.replace("SI_UNIT($,.RADIAN.)", "SI_UNIT($,.DEGREE.)");
        assert_eq!(
            read_analytic_solid(&bad_units, 1e-7),
            Err(ReadError::Unsupported)
        );
    }

    // The stock is well-defined at its placement, but the nearly coincident
    // cylinder rim vertices collapse after adding the large world offset.
    let nearly_merged = [
        BoreChamfer {
            size_mm: 3.,
            ..chamfers[0]
        },
        BoreChamfer {
            size_mm: 2.999999,
            ..chamfers[1]
        },
    ];
    let thinner = PolygonPrism::with_circular_holes(
        prism.rings().to_vec(),
        prism.circular_holes().to_vec(),
        exact(6),
    )
    .unwrap();
    assert_eq!(
        write_chamfered_polygon_prism(&thinner, &nearly_merged, [0., 0., 1e15], X, Y, false),
        Err(StepError::InvalidGeometry),
    );

    let stadium = Stadium::new([[0., 0.], [4., 0.]], 1.).unwrap();
    let source = write_stadium_prism(&stadium, 8., ORIGIN, X, Y, false).unwrap();
    let cylinder = source
        .lines()
        .find(|line| line.contains("=CYLINDRICAL_SURFACE("))
        .unwrap();
    let cone = cylinder
        .trim_end_matches(");")
        .replace("CYLINDRICAL_SURFACE(", "CONICAL_SURFACE(");
    let source = source.replacen(cylinder, &format!("{cone},0.5);"), 1);
    assert_eq!(
        read_analytic_solid(&source, 1e-7),
        Err(ReadError::Unsupported)
    );
}

#[test]
fn rejected_member_rolls_back_geometry_and_entity_ids() {
    let mut writer = Writer::default();
    assert_eq!(writer.finish(), Err(StepError::InvalidGeometry));

    writer = Writer::new();
    writer.append_box(size(), ORIGIN, X, Y).unwrap();
    assert_eq!(
        writer.append_box(size(), [1e100; 3], X, Y),
        Err(StepError::InvalidGeometry)
    );
    let single = write_box(size(), ORIGIN, X, Y).unwrap();
    assert_eq!(writer.finish().unwrap(), single);

    let mut writer = Writer::new();
    writer.append_box(size(), ORIGIN, X, Y).unwrap();
    assert!(writer.append_box(size(), [1e100; 3], X, Y).is_err());
    writer.append_box(size(), [14., 0., 0.], X, Y).unwrap();
    let actual = writer.finish().unwrap();
    let mut writer = Writer::new();
    writer.append_box(size(), ORIGIN, X, Y).unwrap();
    writer.append_box(size(), [14., 0., 0.], X, Y).unwrap();
    assert_eq!(actual, writer.finish().unwrap());
}

#[test]
fn compound_respects_member_and_part21_budgets() {
    let mut writer = Writer::new();
    for index in 0..MAX_COMPOUND_SOLIDS {
        writer
            .append_placed_circular_sweep(1., None, 2., [3. * index as f64, 0., 0.], X, Y)
            .unwrap();
    }
    assert_eq!(
        writer.append_box(size(), ORIGIN, X, Y),
        Err(StepError::Limit)
    );
    assert_eq!(
        writer
            .finish()
            .unwrap()
            .matches("MANIFOLD_SOLID_BREP(")
            .count(),
        MAX_COMPOUND_SOLIDS
    );

    let mut writer = Writer::new();
    writer.append_box(size(), ORIGIN, X, Y).unwrap();
    writer.file.next = MAX_STEP_ENTITIES;
    let before = writer.file.data.clone();
    assert_eq!(
        writer.append_box(size(), [10., 0., 0.], X, Y),
        Err(StepError::Limit)
    );
    assert_eq!(writer.file.data, before);
    assert_eq!(writer.file.next, MAX_STEP_ENTITIES);
    writer
        .file
        .data
        .push_str(&" ".repeat(MAX_PLANAR_STEP_BYTES));
    assert_eq!(
        writer.append_box(size(), [10., 0., 0.], X, Y),
        Err(StepError::Limit)
    );

    let mut writer = Writer::new();
    writer.append_box(size(), ORIGIN, X, Y).unwrap();
    writer.append_box(size(), [10., 0., 0.], X, Y).unwrap();
    writer.file.next = MAX_STEP_ENTITIES;
    assert_eq!(writer.finish(), Err(StepError::Limit));
}

#[test]
fn mixed_compound_keeps_placed_stadiums_and_machined_stock_as_separate_shells() {
    use crate::{
        orthogonal::{Box3, Operation, boolean},
        orthogonal_boundary::build,
        step_read::{ReadError, read_analytic_compound, read_analytic_solid},
    };
    let stock = boolean(
        &[Box3 {
            min: ORIGIN,
            max: [20., 16., 10.],
        }],
        &[],
        Operation::Union,
        1e-7,
    )
    .unwrap()
    .unwrap();
    let boundary = build(stock.quads()).unwrap();
    let face = boundary
        .faces
        .iter()
        .position(|face| face.normal == [0., 0., 1.])
        .unwrap();
    let pocket = OrthogonalPocket {
        face,
        center_mm: [16., 7., 10.],
        radius_mm: 1.,
        depth_mm: 2.,
    };
    let slot = OrthogonalSlot {
        face,
        centers_mm: [[4., 7., 10.], [9., 7., 10.]],
        radius_mm: 1.5,
        depth_mm: 3.,
    };
    let profile = Stadium::new([[2., 3.], [8., 3.]], 1.5).unwrap();
    let place = |[x, y, z]: Point| [y + 7., -x + 2., z + 9.];
    let round = |writer: &mut Writer| {
        writer.append_placed_circular_sweep(4., Some(1.), 3., [0., 0., 30.], X, Y)
    };
    let mut writer = Writer::new();
    writer
        .append_stadium_prism(&profile, 4., ORIGIN, X, Y, false)
        .unwrap();
    writer
        .append_slotted_orthogonal(&stock, &boundary, &[pocket], &[slot], &place)
        .unwrap();
    writer
        .append_stadium_prism(&profile, 4., [35., 0., 0.], Y, [0., 0., 1.], true)
        .unwrap();
    round(&mut writer).unwrap();
    let encoded = writer.finish().unwrap();
    assert_eq!(
        encoded
            .matches("ADVANCED_BREP_SHAPE_REPRESENTATION(")
            .count(),
        1
    );
    assert_eq!(encoded.matches("CLOSED_SHELL(").count(), 4);
    assert_eq!(encoded.matches("MANIFOLD_SOLID_BREP(").count(), 4);
    assert_eq!(
        read_analytic_solid(&encoded, 1e-7),
        Err(ReadError::Unsupported)
    );

    let readback = read_analytic_compound(&encoded, 1e-7).unwrap();
    let expected = [
        write_stadium_prism(&profile, 4., ORIGIN, X, Y, false).unwrap(),
        write_slotted_orthogonal(&stock, &boundary, &[pocket], &[slot], &place).unwrap(),
        write_stadium_prism(&profile, 4., [35., 0., 0.], Y, [0., 0., 1.], true).unwrap(),
        write_placed_circular_sweep(4., Some(1.), 3., [0., 0., 30.], X, Y).unwrap(),
    ];
    assert_eq!(
        readback,
        expected.map(|member| read_analytic_solid(&member, 1e-7).unwrap())
    );
}
