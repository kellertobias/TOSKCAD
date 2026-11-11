use super::*;
use std::f64::consts::TAU;

// Symmetric 16-point Gauss-Legendre quadrature for the variable-speed
// radial connector and off-axis fillet area density. Centered arcs and
// involutes have constant density relative to planar arc length.
const ABSCISSA: [f64; 8] = [
    0.09501250983763744,
    0.2816035507792589,
    0.4580167776572274,
    0.6178762444026438,
    0.755404408355003,
    0.8656312023878318,
    0.9445750230732326,
    0.9894009349916499,
];
const WEIGHT: [f64; 8] = [
    0.1894506104550685,
    0.1826034150449236,
    0.16915651939500254,
    0.14959598881657673,
    0.12462897125553387,
    0.09515851168249278,
    0.062253523938647894,
    0.027152459411754095,
];

fn transverse_metrics(edge: Edge, rate: f64) -> ([f64; 2], f64) {
    let length = edge.length_mm();
    match edge {
        Edge::Involute(flank) => {
            return (
                edge.centroid(),
                length * (rate * flank.base_radius_mm()).hypot(1.),
            );
        }
        Edge::Planar(normal_sweep::Edge::Arc {
            center: [0., 0.], ..
        }) => {
            return (edge.centroid(), length);
        }
        _ => {}
    }
    let mut measure = 0.;
    let mut moment = [0.; 2];
    for (abscissa, weight) in ABSCISSA.into_iter().zip(WEIGHT) {
        for fraction in [(1. - abscissa) / 2., (1. + abscissa) / 2.] {
            let [x, y] = edge.point(fraction);
            let [tx, ty] = edge.tangent(fraction);
            let weight = weight * (rate * (x * tx + y * ty)).hypot(1.) / 2.;
            measure += weight;
            moment[0] += weight * x;
            moment[1] += weight * y;
        }
    }
    (moment.map(|m| m / measure), length * measure)
}

pub(super) fn local_faces(gear: &Herringbone, topology: &Topology) -> Vec<Face> {
    let height = gear.profile().height_mm();
    let rate = gear.twist_rad_per_mm();
    let wall = gear
        .profile()
        .outline()
        .iter()
        .map(|entry| {
            let ([x, y], length) = transverse_metrics(entry.edge, rate);
            ([x, y], length * height / 2.)
        })
        .collect::<Vec<_>>();
    topology
        .faces
        .iter()
        .enumerate()
        .map(|(i, face)| {
            let (plane, center_mm, area_mm2) = match face {
                GearFace::Cap { end } => {
                    let origin = [0., 0., if *end { height } else { 0. }];
                    (
                        Some((origin, [0., 0., if *end { 1. } else { -1. }])),
                        origin,
                        gear.profile().area_mm2()
                            - gear.hub_extensions()[usize::from(*end)].map_or(0., |hub| {
                                let bore = gear
                                    .profile()
                                    .bore()
                                    .and_then(normal_sweep::Edge::radius_mm)
                                    .unwrap_or(0.);
                                std::f64::consts::PI * (hub.radius_mm.powi(2) - bore.powi(2))
                            }),
                    )
                }
                GearFace::Tooth { band, .. } => {
                    let [lo, hi] = band.axial_interval_mm(height);
                    let ([x, y], area) = wall[(i - 2) % wall.len()];
                    let phases = [lo, hi].map(|z| gear.phase_at(z));
                    let half = (phases[1] - phases[0]) / 2.;
                    let mean = half.sin() / half;
                    let (sin, cos) = phases[0].midpoint(phases[1]).sin_cos();
                    (
                        None,
                        [
                            mean * (cos * x - sin * y),
                            mean * (sin * x + cos * y),
                            lo.midpoint(hi),
                        ],
                        area,
                    )
                }
                GearFace::Bore => {
                    let radius = gear
                        .profile()
                        .bore()
                        .and_then(normal_sweep::Edge::radius_mm)
                        .expect("named bore");
                    let [low, high] = gear.axial_bounds();
                    (
                        None,
                        [0., 0., low.midpoint(high)],
                        TAU * radius * (high - low),
                    )
                }
                GearFace::HubWall { extension } => {
                    let length = (extension.end_z_mm - extension.gear_z_mm).abs();
                    (
                        None,
                        [0., 0., extension.end_z_mm.midpoint(extension.gear_z_mm)],
                        TAU * extension.radius_mm * length,
                    )
                }
                GearFace::HubCap { extension } => {
                    let bore = gear
                        .profile()
                        .bore()
                        .and_then(normal_sweep::Edge::radius_mm)
                        .unwrap_or(0.);
                    let origin = [0., 0., extension.end_z_mm];
                    let outward = (extension.end_z_mm - extension.gear_z_mm).signum();
                    (
                        Some((origin, [0., 0., outward])),
                        origin,
                        std::f64::consts::PI * (extension.radius_mm.powi(2) - bore.powi(2)),
                    )
                }
            };
            Face {
                index: i as u32,
                plane,
                center_mm,
                area_mm2,
            }
        })
        .collect()
}
