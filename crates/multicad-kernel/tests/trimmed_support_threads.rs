#![cfg(feature = "occt")]
use multicad_kernel::{
    GeometryPolicy, PlaneFrame, Solid,
    rust_kernel::{
        helical_thread::{Spec, Thread},
        step::write_helical_thread,
    },
};

#[test]
fn external_support_uses_physical_faces_without_solid_containment() {
    let directory = tempfile::tempdir().unwrap();
    for frame in [
        PlaneFrame::default(),
        PlaneFrame {
            origin: [17., -8., 13.],
            u: [0.8, 0.6, 0.],
            v: [-0.36, 0.48, 0.8],
        },
    ] {
        for left_hand in [false, true] {
            for stock in [None, Some(8.)] {
                let thread = Thread::new(Spec {
                    major_radius_mm: 5.,
                    pitch_mm: 2.,
                    length_mm: 6.74,
                    depth_mm: 1.25,
                    crest_width_mm: 1. - 15_f64.to_radians().tan(),
                    lower_flank_deg: 15.,
                    upper_flank_deg: 15.,
                    starts: 1,
                    left_hand,
                    root_radius_mm: None,
                })
                .unwrap();
                let text =
                    write_helical_thread(&thread, stock, frame.origin, frame.u, frame.v, 1e-7)
                        .unwrap();
                let path = directory.path().join("thread.step");
                std::fs::write(&path, text).unwrap();
                let read = Solid::read_step(&path, GeometryPolicy::default()).unwrap();
                let bounds = read.bounds().unwrap();
                for sign in [-1., 1.] {
                    for axis in 0..3 {
                        let normal = frame.normal();
                        let direction = [
                            sign * frame.u[axis],
                            sign * frame.v[axis],
                            sign * normal[axis],
                        ];
                        let support = stock.map_or_else(
                            || thread.support(direction),
                            |radius| {
                                radius * direction[0].hypot(direction[1])
                                    + (thread.spec().length_mm * direction[2]).max(0.)
                            },
                        );
                        let expected = frame.origin[axis] + sign * support;
                        let actual = if sign < 0. {
                            bounds.min[axis]
                        } else {
                            bounds.max[axis]
                        };
                        assert!(
                            (actual - expected).abs() < 2e-6,
                            "{actual} != {expected}; {frame:?}, left={left_hand},stock={stock:?}"
                        );
                    }
                }
                let expected = stock.map_or_else(
                    || thread.volume_mm3(),
                    |radius| {
                        std::f64::consts::PI * radius * radius * thread.spec().length_mm
                            - thread.volume_mm3()
                    },
                );
                assert!((read.volume_mm3().unwrap() - expected).abs() < 1e-5);
                assert_eq!(read.solid_count().unwrap(), 1);
            }
        }
    }
}

#[test]
fn support_keeps_loose_curve_and_point_geometry_in_legacy_compounds() {
    let thread = Thread::new(Spec {
        major_radius_mm: 5.,
        pitch_mm: 2.,
        length_mm: 6.74,
        depth_mm: 1.25,
        crest_width_mm: 1. - 15_f64.to_radians().tan(),
        lower_flank_deg: 15.,
        upper_flank_deg: 15.,
        starts: 1,
        left_hand: false,
        root_radius_mm: None,
    })
    .unwrap();
    let mut text =
        write_helical_thread(&thread, None, [0.; 3], [1., 0., 0.], [0., 1., 0.], 1e-7).unwrap();
    let entity_id = |name: &str| {
        text.lines()
            .find(|line| line.contains(name))
            .unwrap()
            .split('=')
            .next()
            .unwrap()
            .to_owned()
    };
    let product_context = entity_id("=PRODUCT_CONTEXT(");
    let definition_context = entity_id("=PRODUCT_DEFINITION_CONTEXT(");
    let spatial = entity_id("GEOMETRIC_REPRESENTATION_CONTEXT(");
    let mut next = text
        .lines()
        .filter_map(|line| {
            line.strip_prefix('#')?
                .split('=')
                .next()?
                .parse::<usize>()
                .ok()
        })
        .max()
        .unwrap()
        + 1;
    let mut records = String::new();
    let mut add = |record: String| {
        let id = format!("#{next}");
        next += 1;
        records.push_str(&format!("{id}={record};\n"));
        id
    };
    let a = add("CARTESIAN_POINT('',(100.,0.,2.))".into());
    let b = add("CARTESIAN_POINT('',(101.,0.,2.))".into());
    let vertex = add("CARTESIAN_POINT('',(0.,0.,50.))".into());
    let line = add(format!("POLYLINE('',({a},{b}))"));
    let curves = add(format!("GEOMETRIC_CURVE_SET('',({line},{vertex}))"));
    let representation = add(format!(
        "GEOMETRICALLY_BOUNDED_WIREFRAME_SHAPE_REPRESENTATION('',({curves}),{spatial})"
    ));
    let product = add(format!("PRODUCT('loose','loose','',({product_context}))"));
    let formation = add(format!("PRODUCT_DEFINITION_FORMATION('','',{product})"));
    let definition = add(format!(
        "PRODUCT_DEFINITION('design','',{formation},{definition_context})"
    ));
    let shape = add(format!("PRODUCT_DEFINITION_SHAPE('','',{definition})"));
    add(format!(
        "SHAPE_DEFINITION_REPRESENTATION({shape},{representation})"
    ));
    let end = text.rfind("ENDSEC;").unwrap();
    text.insert_str(end, &records);
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("compound.step");
    std::fs::write(&path, text).unwrap();
    let read = Solid::read_step(&path, GeometryPolicy::default()).unwrap();
    assert_eq!(read.solid_count().unwrap(), 1);
    assert!((read.volume_mm3().unwrap() - thread.volume_mm3()).abs() < 1e-5);
    let bounds = read.bounds().unwrap();
    assert!((bounds.max[0] - 101.).abs() < 2e-6);
    assert!((bounds.max[2] - 50.).abs() < 2e-6);
}
