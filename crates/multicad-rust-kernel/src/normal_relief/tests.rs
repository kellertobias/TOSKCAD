use super::*;
use crate::{display::Tessellation, manufacturing::certify};
use std::f64::consts::PI;
const TOL: f64 = 1e-7;
const MIN: f64 = 1e-5;
fn rectangle(min: [f64; 2], max: [f64; 2]) -> Vec<Edge> {
    let p = [min, [max[0], min[1]], max, [min[0], max[1]]];
    (0..4)
        .map(|i| Edge::Line {
            a: p[i],
            b: p[(i + 1) % 4],
        })
        .collect()
}
fn circle(center: [f64; 2], radius_mm: f64, clockwise: bool) -> Vec<Edge> {
    vec![Edge::Circle {
        center,
        radius_mm,
        clockwise,
    }]
}
fn base() -> Sweep {
    Sweep::new(
        vec![rectangle([0., 0.], [20., 16.]), circle([10., 8.], 1., true)],
        4.,
        TOL,
        MIN,
    )
    .unwrap()
}
fn motifs() -> Vec<Sweep> {
    vec![
        Sweep::new(vec![circle([3., 3.], 0.75, false)], 1.5, TOL, MIN).unwrap(),
        Sweep::new(
            vec![vec![
                Edge::Line {
                    a: [12., 2.],
                    b: [14., 2.],
                },
                Edge::Arc {
                    center: [14., 3.],
                    a: [14., 2.],
                    b: [14., 4.],
                    clockwise: false,
                },
                Edge::Line {
                    a: [14., 4.],
                    b: [12., 4.],
                },
                Edge::Line {
                    a: [12., 4.],
                    b: [12., 2.],
                },
            ]],
            0.6,
            TOL,
            MIN,
        )
        .unwrap(),
    ]
}
fn options() -> Tessellation {
    Tessellation {
        deflection: 0.001,
        angle: 0.08,
        triangle_limit: 100_000,
    }
}
#[test]
fn exact_mixed_relief_metrics_and_one_shared_closed_mesh() {
    for kind in [Kind::Raised, Kind::Recessed] {
        let (base, motifs) = (base(), motifs());
        let before = (base.clone(), motifs.clone());
        let relief = Relief::new(base, motifs, kind, TOL, MIN).unwrap();
        let added = 0.75_f64.powi(2) * PI * 1.5 + (4. + PI / 2.) * 0.6;
        let expected = (320. - PI) * 4. + if kind == Kind::Raised { added } else { -added };
        assert!((relief.volume_mm3() - expected).abs() < 1e-10);
        assert_eq!(relief.boundary().faces.len(), 14);
        assert_eq!(
            (relief.base(), relief.motifs()),
            (&before.0, before.1.as_slice())
        );
        let graph = relief.boundary();
        let mut uses = vec![vec![]; graph.edges.len()];
        for (face, node) in graph.faces.iter().enumerate() {
            for edge in node.loops.iter().flatten() {
                uses[edge.edge].push((face, edge.forward));
            }
        }
        assert!(
            uses.iter()
                .all(|edge| edge.len() == 2 && edge[0].1 != edge[1].1)
        );
        let mesh = relief.mesh(options()).unwrap();
        let proof = certify(&mesh).unwrap();
        assert!((proof.signed_volume_mm3 - relief.volume_mm3()).abs() < 0.05);
        assert_eq!(
            mesh.face_indices
                .iter()
                .copied()
                .collect::<std::collections::BTreeSet<_>>(),
            (0..14).collect()
        );
        assert!(relief.contains([3., 3., if kind == Kind::Raised { 5. } else { 2. }], TOL));
        assert!(!relief.contains([10., 8., 2.], TOL));
        assert!(!relief.contains([3., 3., if kind == Kind::Raised { 6. } else { 3. }], TOL));
        assert_eq!(
            relief.bounds()[1][2],
            if kind == Kind::Raised { 5.5 } else { 4. }
        );
    }
}
#[test]
fn all_192_motifs_are_certified_without_iterated_solid_booleans() {
    let base = Sweep::new(vec![rectangle([0., 0.], [100., 80.])], 4., TOL, MIN).unwrap();
    let motifs = (0..192)
        .map(|i| {
            Sweep::new(
                vec![circle(
                    [3. + (i % 16) as f64 * 6., 3. + (i / 16) as f64 * 6.],
                    0.5,
                    false,
                )],
                0.5 + (i % 3) as f64 * 0.1,
                TOL,
                MIN,
            )
            .unwrap()
        })
        .collect::<Vec<_>>();
    let started = std::time::Instant::now();
    let relief = Relief::new(base, motifs, Kind::Raised, TOL, MIN).unwrap();
    eprintln!("192 relief analytic motifs: {:?}", started.elapsed());
    assert_eq!(relief.motifs().len(), 192);
    assert_eq!(relief.boundary().faces.len(), 390);
    certify(&relief.mesh(options()).unwrap()).unwrap();
}
#[test]
fn bad_contact_overlap_holes_floor_and_cancellation_leave_inputs_unchanged() {
    let base = base();
    let retained = base.clone();
    for motif in [
        Sweep::new(vec![circle([0., 3.], 1., false)], 1., TOL, MIN).unwrap(),
        Sweep::new(vec![circle([1., 3.], 1., false)], 1., TOL, MIN).unwrap(),
        Sweep::new(vec![circle([10., 8.], 0.5, false)], 1., TOL, MIN).unwrap(),
    ] {
        assert!(Relief::new(base.clone(), vec![motif], Kind::Raised, TOL, MIN).is_err());
    }
    let motif = motifs().remove(0);
    assert!(
        Relief::new(
            base.clone(),
            vec![motif.clone(), motif.clone()],
            Kind::Raised,
            TOL,
            MIN
        )
        .is_err()
    );
    let full = Sweep::new(motif.rings().to_vec(), 4., TOL, MIN).unwrap();
    assert_eq!(
        Relief::new(base.clone(), vec![full], Kind::Recessed, TOL, MIN).err(),
        Some(Error::Invalid)
    );
    assert_eq!(
        Relief::new(
            base.clone(),
            vec![motif.clone(); 193],
            Kind::Raised,
            TOL,
            MIN
        )
        .err(),
        Some(Error::Limit)
    );
    let calls = std::cell::Cell::new(0);
    let stop = || {
        calls.set(calls.get() + 1);
        calls.get() > 20
    };
    assert_eq!(
        Relief::new_with_interrupt(
            base.clone(),
            vec![motif.clone()],
            Kind::Raised,
            TOL,
            MIN,
            Some(&stop)
        )
        .err(),
        Some(Error::Cancelled)
    );
    assert_eq!(base, retained);
    Relief::new(base, vec![motif], Kind::Raised, TOL, MIN).unwrap();
}
