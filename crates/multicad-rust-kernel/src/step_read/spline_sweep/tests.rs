use super::*;
use crate::{
    Rational,
    spur_gear::{Profile, Spec},
    step::{Writer, write_spur_gear},
};
use std::f64::consts::{FRAC_1_SQRT_2, PI};

const ORIGIN: Point = [11., -7., 23.];
const U: Point = [FRAC_1_SQRT_2, 0., FRAC_1_SQRT_2];
const V: Point = [0., 1., 0.];
const TOL: f64 = 1e-7;

fn profile(teeth: u32, bore_radius_mm: f64) -> Profile {
    let pressure = 20_f64.to_radians();
    let pitch = f64::from(teeth);
    Profile::new(
        Spec {
            teeth,
            pitch_radius_mm: pitch,
            base_radius_mm: pitch * pressure.cos(),
            tip_radius_mm: pitch + 2.,
            root_radius_mm: pitch - 2.5,
            root_fillet_radius_mm: 0.4,
            half_base_angle_rad: PI / (2. * pitch) + pressure.tan()
                - pressure
                - 0.05 / (2. * pitch),
            face_width_mm: 8.,
            bore_radius_mm,
        },
        1e-5,
        1e7,
    )
    .unwrap()
}

#[test]
fn independent_reader_verifies_placed_spur_shells_and_mixed_compounds() {
    let n = cross(U, V);
    for (teeth, bore) in [(20, 2.), (64, 0.)] {
        let profile = profile(teeth, bore);
        let source = write_spur_gear(&profile, ORIGIN, U, V, TOL).unwrap();
        let shape = read_analytic_solid(&source, TOL).unwrap();
        assert_eq!(shape.solid_count, 1);
        assert_eq!(
            shape.face_count,
            2 + profile.outline().len() + usize::from(bore != 0.)
        );
        assert_eq!(shape.splines.len(), 4 * teeth as usize);
        assert_eq!(shape.circles.len(), 2 * usize::from(bore != 0.));
        assert!((shape.volume_mm3 - profile.volume_mm3()).abs() < 1e-5);
        let expected: [Point; 2] = [false, true].map(|upper| {
            std::array::from_fn(|i| {
                let s = if upper { 1. } else { -1. };
                ORIGIN[i] + s * profile.support([s * U[i], s * V[i], s * n[i]])
            })
        });
        for (actual, expected) in shape
            .bounds_mm
            .into_iter()
            .flatten()
            .zip(expected.into_iter().flatten())
        {
            assert!((actual - expected).abs() < TOL, "{actual} != {expected}");
        }
        assert_eq!(
            read_import_solid(&source, TOL).unwrap_err(),
            ReadError::Unsupported
        );
    }

    let mut writer = Writer::new();
    writer
        .append_box([Rational::new(2, 1).unwrap(); 3], ORIGIN, U, V)
        .unwrap();
    writer
        .append_spur_gear(&profile(20, 2.), ORIGIN, U, V, TOL)
        .unwrap();
    let compound = writer.finish().unwrap();
    let members = read_analytic_compound(&compound, TOL).unwrap();
    assert_eq!(members.len(), 2);
    assert!(members[0].splines.is_empty());
    assert_eq!(members[1].splines.len(), 80);
}

type Records<'a> = BTreeMap<u32, Vec<Entity<'a>>>;

struct Fixture<'a> {
    records: Records<'a>,
    faces: Vec<u32>,
    extrusions: Vec<u32>,
    cylinders: Vec<u32>,
    splines: Vec<u32>,
}

impl<'a> Fixture<'a> {
    fn new(source: &'a str) -> Self {
        let records = parse_entities(source, TOL).unwrap();
        let matching = |kind| {
            records
                .iter()
                .filter_map(|(&id, e)| (e[0].name == kind).then_some(id))
                .collect()
        };
        let (extrusions, cylinders, splines) = (
            matching("SURFACE_OF_LINEAR_EXTRUSION"),
            matching("CYLINDRICAL_SURFACE"),
            matching("B_SPLINE_CURVE_WITH_KNOTS"),
        );
        let reader = Shell::new(&records, TOL, true);
        let faces = reader.shell_faces().unwrap();
        Self {
            records,
            faces,
            extrusions,
            cylinders,
            splines,
        }
    }
    fn args(&self, id: u32) -> &[Value<'a>] {
        &self.records[&id][0].args
    }
    fn edit(&mut self, id: u32) -> &mut Vec<Value<'a>> {
        &mut self.records.get_mut(&id).unwrap()[0].args
    }
    fn id(&self, id: u32, field: usize) -> u32 {
        reference(&self.args(id)[field]).unwrap()
    }
    fn list(&self, id: u32, field: usize) -> Vec<u32> {
        refs(&self.args(id)[field]).unwrap()
    }
    fn cap_plane(&self, cap: usize) -> u32 {
        self.id(self.faces[cap], 2)
    }
    fn face_for(&self, surface: u32) -> u32 {
        *self
            .faces
            .iter()
            .find(|&&face| self.id(face, 2) == surface)
            .unwrap()
    }
    fn read(&self) -> Result<Readback> {
        read_member(
            &mut Shell::new(&self.records, TOL, true),
            self.faces.clone(),
        )
    }
}

fn damaged(edit: impl FnOnce(&mut Fixture<'_>), error: ReadError) {
    let source = write_spur_gear(&profile(20, 2.), ORIGIN, U, V, TOL).unwrap();
    let mut f = Fixture::new(&source);
    edit(&mut f);
    assert_eq!(f.read().err(), Some(error));
}

#[test]
fn rejects_missing_and_duplicate_topology_or_invalid_cap_geometry() {
    damaged(|f| f.faces.truncate(4), ReadError::Limit);
    damaged(|f| f.faces.push(f.faces[0]), ReadError::Invalid);
    damaged(
        |f| {
            f.edit(f.faces[0]).pop();
        },
        ReadError::Invalid,
    );
    damaged(
        |f| {
            f.edit(f.faces[0])[3] = Value::Enum("F");
        },
        ReadError::Invalid,
    );
    damaged(
        |f| {
            f.faces.pop();
        },
        ReadError::Invalid,
    );
    damaged(
        |f| {
            let vector = f.id(f.extrusions[0], 2);
            let dir = f.id(vector, 1);
            f.edit(dir)[1] = Value::List([0.3, 0.4, 0.5].map(Value::Number).into());
        },
        ReadError::Invalid,
    );
    damaged(
        |f| {
            let p = f.id(f.id(f.cap_plane(1), 1), 1);
            f.edit(p)[1] = Value::List([22., 0., 40.].map(Value::Number).into());
        },
        ReadError::Invalid,
    );
    damaged(
        |f| {
            let point = f.list(f.splines[0], 2)[2];
            f.edit(point)[1] = Value::List([23., 0., 44.].map(Value::Number).into());
        },
        ReadError::Invalid,
    );
    damaged(
        |f| {
            let point = f.list(f.splines[0], 2)[2];
            let Value::List(v) = &mut f.edit(point)[1] else {
                panic!()
            };
            let Value::Number(y) = &mut v[1] else {
                panic!()
            };
            *y += 0.001;
        },
        ReadError::Invalid,
    );
    damaged(
        |f| {
            let plane = f.cap_plane(0);
            f.edit(plane).pop();
        },
        ReadError::Invalid,
    );
}

#[test]
fn rejects_invalid_face_edge_and_extrusion_records() {
    damaged(
        |f| {
            f.edit(f.faces[0])[1] = Value::List(vec![]);
        },
        ReadError::Unsupported,
    );
    damaged(
        |f| {
            let bound = f.list(f.faces[0], 1)[0];
            f.edit(f.faces[0])[1] =
                Value::List(vec![Value::Reference(bound), Value::Reference(bound)]);
        },
        ReadError::Invalid,
    );
    damaged(
        |f| {
            let bound = f.list(f.faces[0], 1)[0];
            let loop_ = f.id(bound, 1);
            f.edit(loop_).pop();
        },
        ReadError::Invalid,
    );
    damaged(
        |f| {
            let bound = f.list(f.faces[0], 1)[0];
            let loop_ = f.id(bound, 1);
            f.edit(loop_)[1] = Value::List(vec![]);
        },
        ReadError::Limit,
    );
    damaged(
        |f| {
            let bound = f.list(f.faces[0], 1)[0];
            f.edit(bound)[2] = Value::Enum("F");
        },
        ReadError::Invalid,
    );
    damaged(
        |f| {
            let bound = f.list(f.faces[0], 1)[0];
            let loop_ = f.id(bound, 1);
            let edge = f.list(loop_, 1)[0];
            f.edit(edge).pop();
        },
        ReadError::Invalid,
    );
    damaged(
        |f| {
            let bound = f.list(f.faces[0], 1)[0];
            let loop_ = f.id(bound, 1);
            let edges = f.list(loop_, 1);
            f.edit(loop_)[1] =
                Value::List(vec![Value::Reference(edges[0]), Value::Reference(edges[0])]);
        },
        ReadError::Invalid,
    );
    damaged(
        |f| {
            let bound = f.list(f.faces[0], 1)[0];
            let loop_ = f.id(bound, 1);
            let Value::List(edges) = &mut f.edit(loop_)[1] else {
                panic!()
            };
            edges.swap(0, 1);
        },
        ReadError::Invalid,
    );
    damaged(
        |f| {
            let bound = f.list(f.faces[0], 1)[0];
            let loop_ = f.id(bound, 1);
            let edge = f.id(f.list(loop_, 1)[0], 3);
            f.edit(edge).pop();
        },
        ReadError::Invalid,
    );
    damaged(
        |f| {
            let spline = f.splines[0];
            let edge = f
                .records
                .iter()
                .find_map(|(&id, e)| {
                    (e[0].name == "EDGE_CURVE" && f.id(id, 3) == spline).then_some(id)
                })
                .unwrap();
            f.edit(edge)[4] = Value::Enum("F");
        },
        ReadError::Unsupported,
    );
    damaged(
        |f| {
            let spline = f.splines[0];
            let replacement = f.list(f.splines[1], 2)[2];
            let Value::List(poles) = &mut f.edit(spline)[2] else {
                panic!()
            };
            poles[0] = Value::Reference(replacement);
        },
        ReadError::Invalid,
    );
    damaged(
        |f| {
            let first = f.extrusions[0];
            f.edit(first)[1] = Value::Reference(f.splines[1]);
        },
        ReadError::Invalid,
    );
    damaged(
        |f| {
            let first = f.cylinders[0];
            f.edit(first)[2] = Value::Number(100.);
        },
        ReadError::Invalid,
    );
    damaged(
        |f| {
            let first = f.cylinders[0];
            f.edit(first).pop();
        },
        ReadError::Invalid,
    );
    damaged(
        |f| {
            let face = f.face_for(f.cylinders[0]);
            let before = sense(&f.args(face)[3]).unwrap();
            f.edit(face)[3] = Value::Enum(if before { "F" } else { "T" });
        },
        ReadError::Invalid,
    );
    damaged(
        |f| {
            let wall = f
                .faces
                .iter()
                .copied()
                .skip(2)
                .find(|&face| f.records[&f.id(face, 2)][0].name == "PLANE")
                .unwrap();
            f.edit(wall)[3] = Value::Enum("F");
        },
        ReadError::Invalid,
    );
}

#[test]
fn rejects_duplicate_outer_bounds_and_unequal_cap_areas() {
    damaged(
        |f| {
            let inner = f.list(f.faces[0], 1)[1];
            f.records.get_mut(&inner).unwrap()[0].name = "FACE_OUTER_BOUND";
        },
        ReadError::Invalid,
    );
    damaged(
        |f| {
            let poles = f.list(f.splines[0], 2);
            let point = poles[poles.len() / 2];
            let Value::List(v) = &mut f.edit(point)[1] else {
                panic!()
            };
            let Value::Number(y) = &mut v[1] else {
                panic!()
            };
            *y += 0.005;
        },
        ReadError::Invalid,
    );
}

fn cap_and_wall<'a, 'input>(
    f: &'a Fixture<'input>,
    surface: u32,
) -> (Shell<'a, 'input>, Edges, FaceWire, FaceWire, FaceWire) {
    let mut reader = Shell::new(&f.records, TOL, true);
    let mut edges = Edges::new();
    let [low, high] = [0, 1].map(|i| read_face(&mut reader, f.faces[i], &mut edges).unwrap());
    let wall = read_face(&mut reader, f.face_for(surface), &mut edges).unwrap();
    (reader, edges, low, high, wall)
}

#[test]
fn side_check_rejects_inconsistent_supports_and_trim_incidence() {
    let source = write_spur_gear(&profile(20, 2.), ORIGIN, U, V, TOL).unwrap();
    let f = Fixture::new(&source);
    let (reader, mut edges, low, high, mut wall) = cap_and_wall(&f, f.cylinders[0]);
    let normal = cross(U, V);
    let delta = normal.map(|n| n * 8.);
    let verify = |edges: &Edges, wall: &FaceWire, low_uses| {
        verify_wall(
            &reader,
            edges,
            wall,
            &mut rim_uses(&low),
            &mut rim_uses(&high),
            &mut BTreeSet::new(),
            low_uses,
            normal,
        )
    };
    assert_eq!(verify(&edges, &wall, delta), Ok(()));
    assert_eq!(verify(&edges, &wall, [0.; 3]), Err(ReadError::Invalid));

    let extra = wall.outer.pop().unwrap();
    assert_eq!(verify(&edges, &wall, delta), Err(ReadError::Unsupported));
    wall.outer.push(extra);
    wall.hole = Some(vec![]);
    assert_eq!(verify(&edges, &wall, delta), Err(ReadError::Unsupported));
    wall.hole = None;

    let down = wall.outer[3].edge;
    wall.outer[3].edge = wall.outer[1].edge;
    assert_eq!(verify(&edges, &wall, delta), Err(ReadError::Invalid));
    wall.outer[3].edge = down;

    let up = wall.outer[2].edge;
    edges.get_mut(&up).unwrap().geometry = Geometry::Analytic(Curve::Line);
    assert_eq!(verify(&edges, &wall, delta), Err(ReadError::Invalid));
    let spline = reader.spline_curve(f.splines[0]).unwrap();
    edges.get_mut(&up).unwrap().geometry = Geometry::Cubic {
        id: f.splines[0],
        curve: spline,
    };
    assert_eq!(verify(&edges, &wall, delta), Err(ReadError::Invalid));
}

#[test]
fn cap_check_rejects_bad_winding_nonplanarity_and_hole_escapes() {
    let source = write_spur_gear(&profile(20, 2.), ORIGIN, U, V, TOL).unwrap();
    let f = Fixture::new(&source);
    let (mut reader, mut edges, mut low, _, _) = cap_and_wall(&f, f.cylinders[0]);
    let plane = face_plane(&reader, &low).unwrap();
    let hole = low.hole.as_mut().unwrap();
    let bore = hole[0].edge;
    let Geometry::Analytic(Curve::Circle(mut circle)) = edges[&bore].geometry else {
        panic!()
    };
    let original = circle;
    hole.push(Traversal {
        edge: bore,
        start: hole[0].start,
        end: hole[0].end,
        forward: hole[0].forward,
    });
    assert_eq!(
        section::verify(&reader, &edges, &plane, &low),
        Err(ReadError::Unsupported)
    );
    low.hole.as_mut().unwrap().pop();
    edges.get_mut(&bore).unwrap().geometry = Geometry::Analytic(Curve::Line);
    assert_eq!(
        section::verify(&reader, &edges, &plane, &low),
        Err(ReadError::Unsupported)
    );
    edges.get_mut(&bore).unwrap().geometry = Geometry::Analytic(Curve::Circle(circle));
    low.hole.as_mut().unwrap()[0].forward ^= true;
    assert_eq!(
        section::verify(&reader, &edges, &plane, &low),
        Err(ReadError::Invalid)
    );
    low.hole.as_mut().unwrap()[0].forward ^= true;
    circle.center_mm = std::array::from_fn(|i| original.center_mm[i] + 100. * plane.u[i]);
    edges.get_mut(&bore).unwrap().geometry = Geometry::Analytic(Curve::Circle(circle));
    assert_eq!(
        section::verify(&reader, &edges, &plane, &low),
        Err(ReadError::Invalid)
    );
    edges.get_mut(&bore).unwrap().geometry = Geometry::Analytic(Curve::Circle(original));

    let vertex = low.outer[0].start;
    let position = reader.points[&vertex];
    reader.points.insert(
        vertex,
        std::array::from_fn(|i| position[i] + plane.normal[i]),
    );
    assert_eq!(
        section::verify(&reader, &edges, &plane, &low),
        Err(ReadError::Invalid)
    );
    reader.points.insert(vertex, position);

    low.hole = None;
    let reverse = Plane {
        normal: plane.normal.map(|v| -v),
        v: plane.v.map(|v| -v),
        ..plane
    };
    assert_eq!(
        section::verify(&reader, &edges, &reverse, &low),
        Err(ReadError::Invalid)
    );
    let plane = face_plane(&reader, &low).unwrap();
    let edge = low.outer[0].edge;
    edges.get_mut(&edge).unwrap().geometry = Geometry::Analytic(Curve::Circle(original));
    assert_eq!(
        section::verify(&reader, &edges, &plane, &low),
        Err(ReadError::Invalid)
    );
    low.outer.truncate(2);
    assert_eq!(
        section::verify(&reader, &edges, &plane, &low),
        Err(ReadError::Unsupported)
    );
}

#[test]
fn semicircle_metrics_preserve_trim_extents_and_refuse_invalid_volume() {
    let source = write_spur_gear(&profile(20, 2.), ORIGIN, U, V, TOL).unwrap();
    let f = Fixture::new(&source);
    let (reader, _, low, _, _) = cap_and_wall(&f, f.cylinders[0]);
    let arc = SemicircleEdge {
        center_mm: [0.; 3],
        normal: [0., 0., 1.],
        radius_mm: 100.,
        radial_mid: [0., 1., 0.],
    };
    assert_eq!(round_geometry(Curve::Semicircle(arc)).unwrap().sweep, PI);
    let edges = Edges::from([(
        0,
        EdgeData {
            vertices: [low.outer[0].start, low.outer[0].end],
            geometry: Geometry::Analytic(Curve::Semicircle(arc)),
            uses: [1; 2],
        },
    )]);
    let shape = metrics(&reader, edges, &BTreeSet::new(), 1, 1.).unwrap();
    assert_eq!(shape.semicircles, [arc]);
    assert_eq!(shape.bounds_mm[1][1], 100.);
    assert_eq!(
        metrics(&reader, Edges::new(), &BTreeSet::new(), 1, 0.).err(),
        Some(ReadError::Invalid)
    );
}

#[test]
fn refuses_a_nonmonotone_cubic_even_when_its_endpoints_still_match() {
    damaged(
        |f| {
            let points = f.list(f.splines[0], 2);
            let point = points[2];
            let start = points[0];
            let end = *points.last().unwrap();
            let point_at = |id| numbers(&f.args(id)[1]).unwrap();
            let [a, b] = [point_at(start), point_at(end)];
            f.edit(point)[1] = Value::List(
                std::array::from_fn::<_, 3, _>(|i| Value::Number(a[i] - (b[i] - a[i]))).into(),
            );
        },
        ReadError::Unsupported,
    );
}
