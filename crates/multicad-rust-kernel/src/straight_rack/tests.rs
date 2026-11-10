use super::*;
use crate::{
    display::{Tessellation, normal_sweep},
    manufacturing::certify,
    step::write_normal_sweep,
    step_read::{ImportProfile, read_import_solid},
};
use std::f64::consts::PI;

const MINIMUM: f64 = 1e-5;
const TOLERANCE: f64 = 1e-7;
const LIMIT: f64 = 1e7;

fn specimen(teeth: u32, module: f64) -> Spec {
    Spec {
        teeth,
        pitch_mm: PI * module,
        pitch_thickness_mm: (PI / 2. - 0.07) * module,
        pressure_angle_rad: 20_f64.to_radians(),
        addendum_mm: module,
        dedendum_mm: 1.25 * module,
        root_fillet_radius_mm: 0.3 * module,
        backing_thickness_mm: 2. * module,
        face_width_mm: 6.,
    }
}

fn close(actual: f64, expected: f64, tolerance: f64) {
    assert!(
        (actual - expected).abs() <= tolerance,
        "{actual} != {expected} within {tolerance}"
    );
}

fn area(spec: Spec) -> f64 {
    let alpha = spec.pressure_angle_rad;
    let sharp_tooth = (spec.addendum_mm + spec.dedendum_mm)
        * (spec.pitch_thickness_mm + (spec.dedendum_mm - spec.addendum_mm) * alpha.tan());
    // The tangent quadrilateral area minus its circular sector.
    let root_corner = spec.root_fillet_radius_mm.powi(2)
        * (1. / alpha.cos() - alpha.tan() - (FRAC_PI_2 - alpha) / 2.);
    f64::from(spec.teeth)
        * (spec.pitch_mm * spec.backing_thickness_mm + sharp_tooth + 2. * root_corner)
}

fn tangent(edge: Edge, fraction: f64, flank: Edge) {
    let [x, y] = edge.point(fraction);
    let Edge::Arc { center, .. } = edge else {
        panic!("expected tangent circular transition");
    };
    let [a, b] = [flank.start(), flank.end()];
    close(
        (x - center[0]) * (b[0] - a[0]) + (y - center[1]) * (b[1] - a[1]),
        0.,
        1e-12,
    );
}

fn pitch_x(edge: Edge, alpha: f64) -> f64 {
    let Edge::Line { a, b } = edge else {
        panic!("expected straight tooth flank");
    };
    let slope = (b[0] - a[0]) / (b[1] - a[1]);
    close(slope.abs().atan(), alpha, 1e-12);
    assert!(a[1].min(b[1]) < 0. && a[1].max(b[1]) > 0.);
    a[0] - a[1] * slope
}

#[test]
fn tooth_dimensions_exact_area_and_provenance() {
    for spec in [specimen(4, 2.), specimen(8, 1.), specimen(64, 1.)] {
        let (sweep, roles) = build(spec, TOLERANCE, MINIMUM, LIMIT).unwrap();
        let edges = &sweep.rings()[0];
        let half_length = f64::from(spec.teeth) * spec.pitch_mm / 2.;
        assert_eq!(edges.len(), spec.teeth as usize * 6 + 4);
        assert_eq!(roles.len(), edges.len());
        assert_eq!(roles[0], "rack/backing/bottom");
        assert_eq!(roles[1], "rack/end/right");
        assert_eq!(roles[roles.len() - 2], "rack/space/left_end");
        assert_eq!(roles[roles.len() - 1], "rack/end/left");

        for (step, ordinal) in (0..spec.teeth).rev().enumerate() {
            let start = 2 + step * 6;
            let [space, right_arc, right_flank, tip, left_flank, left_arc] =
                <[Edge; 6]>::try_from(&edges[start..start + 6]).unwrap();
            assert_eq!(
                roles[start..start + 6],
                [
                    format!("rack/space/{ordinal}"),
                    format!("rack/tooth/{ordinal}/fillet/right"),
                    format!("rack/tooth/{ordinal}/flank/right"),
                    format!("rack/tooth/{ordinal}/tip"),
                    format!("rack/tooth/{ordinal}/flank/left"),
                    format!("rack/tooth/{ordinal}/fillet/left"),
                ]
            );
            for arc in [right_arc, left_arc] {
                close(arc.radius_mm().unwrap(), spec.root_fillet_radius_mm, 1e-12);
                close(
                    arc.signed_sweep_rad().unwrap(),
                    spec.pressure_angle_rad - FRAC_PI_2,
                    1e-12,
                );
            }
            tangent(right_arc, 0., space);
            tangent(right_arc, 1., right_flank);
            tangent(left_arc, 0., left_flank);
            tangent(left_arc, 1., edges[start + 6]);
            close(tip.start()[1], spec.addendum_mm, 1e-12);
            close(tip.end()[1], spec.addendum_mm, 1e-12);
            let right = pitch_x(right_flank, spec.pressure_angle_rad);
            let left = pitch_x(left_flank, spec.pressure_angle_rad);
            close(right - left, spec.pitch_thickness_mm, 1e-12);
            close(
                (right + left) / 2.,
                (f64::from(ordinal) + 0.5) * spec.pitch_mm - half_length,
                1e-12,
            );
        }
        close(sweep.area_mm2(), area(spec), 1e-10);
        close(sweep.volume_mm3(), area(spec) * spec.face_width_mm, 1e-9);
        close(sweep.centroid()[0], 0., 1e-12);
        for (axis, extent) in [
            ([1., 0., 0.], half_length),
            ([-1., 0., 0.], half_length),
            ([0., 1., 0.], spec.addendum_mm),
            ([0., -1., 0.], spec.dedendum_mm + spec.backing_thickness_mm),
            ([0., 0., 1.], spec.face_width_mm),
            ([0., 0., -1.], 0.),
        ] {
            close(sweep.support(axis), extent, 1e-12);
        }
    }
}

#[test]
fn rack_manufacturing_mesh_and_step_retain_circular_roots() {
    let spec = specimen(8, 1.);
    let (sweep, _) = build(spec, TOLERANCE, MINIMUM, LIMIT).unwrap();
    let options = Tessellation {
        deflection: 1e-4,
        angle: 0.1,
        triangle_limit: 100_000,
    };
    let mesh = normal_sweep(&sweep, options).unwrap();
    let proof = certify(&mesh).unwrap();
    let curve_length = 2.
        * f64::from(spec.teeth)
        * spec.root_fillet_radius_mm
        * (FRAC_PI_2 - spec.pressure_angle_rad);
    close(
        proof.signed_volume_mm3,
        sweep.volume_mm3(),
        options.deflection * curve_length * spec.face_width_mm,
    );
    let step = write_normal_sweep(&sweep, [0.; 3], [1., 0., 0.], [0., 1., 0.], false).unwrap();
    let (verified, imported) = read_import_solid(&step, TOLERANCE).unwrap();
    let ImportProfile::NormalSweep { sweep: reopened } = imported.profile else {
        panic!("expected an analytic line and arc sweep");
    };
    close(verified.volume_mm3, sweep.volume_mm3(), 1e-9);
    close(reopened.area_mm2(), sweep.area_mm2(), 1e-9);
    assert_eq!(verified.solid_count, 1);
    assert_eq!(verified.face_count, spec.teeth as usize * 6 + 6);
    assert_eq!(verified.arcs.len(), spec.teeth as usize * 4);
    for arc in verified.arcs {
        close(arc.radius_mm, spec.root_fillet_radius_mm, 1e-12);
        close(arc.sweep_rad, FRAC_PI_2 - spec.pressure_angle_rad, 1e-12);
    }
}

#[test]
fn rejects_out_of_budget_or_consumed_tooth_profiles() {
    let base = specimen(8, 1.);
    for teeth in [3, 65, u32::MAX] {
        assert_eq!(
            outline(Spec { teeth, ..base }, MINIMUM, LIMIT),
            Err(Error::Limit)
        );
    }
    assert_eq!(outline(base, MINIMUM, 10.), Err(Error::Limit));
    assert_eq!(
        outline(
            Spec {
                backing_thickness_mm: 16.,
                ..base
            },
            MINIMUM,
            16.,
        ),
        Err(Error::Limit)
    );
    for (minimum, limit) in [
        (0., LIMIT),
        (f64::NAN, LIMIT),
        (1., 1.),
        (1., f64::INFINITY),
    ] {
        assert_eq!(outline(base, minimum, limit), Err(Error::InvalidDimensions));
    }
    for spec in [
        Spec {
            pitch_mm: f64::INFINITY,
            ..base
        },
        Spec {
            face_width_mm: 0.,
            ..base
        },
        Spec {
            backing_thickness_mm: LIMIT + 1.,
            ..base
        },
        Spec {
            pressure_angle_rad: f64::NAN,
            ..base
        },
        Spec {
            pressure_angle_rad: 0.,
            ..base
        },
        Spec {
            pressure_angle_rad: FRAC_PI_2,
            ..base
        },
    ] {
        assert_eq!(outline(spec, MINIMUM, LIMIT), Err(Error::InvalidDimensions));
    }
    for spec in [
        Spec {
            pitch_mm: 1.,
            ..base
        },
        Spec {
            addendum_mm: 10.,
            ..base
        },
        Spec {
            dedendum_mm: 0.1,
            ..base
        },
        Spec {
            pitch_mm: 8.,
            pressure_angle_rad: PI / 4.,
            addendum_mm: 0.5,
            root_fillet_radius_mm: MINIMUM,
            ..base
        },
    ] {
        assert_eq!(
            build(spec, TOLERANCE, MINIMUM, LIMIT),
            Err(Error::InvalidProfile)
        );
    }
    assert_eq!(
        build(base, MINIMUM, MINIMUM, LIMIT),
        Err(Error::Sweep(crate::normal_sweep::Error::InvalidProfile))
    );
}
