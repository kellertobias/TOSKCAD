//! Cylindrical seats retain an actual planar annular shoulder.
use super::*;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Counterbore {
    pub hole: usize,
    pub end: bool,
    pub radius_mm: f64,
    pub depth_mm: f64,
}

impl ChamferedBores {
    /// Cut seats in an already proved constant-section stock. The maximum
    /// complete disk of each cut must clear every other boundary; intersecting
    /// seats require a different Boolean provider. Opposite seats must leave
    /// a finite length of the original through-bore.
    pub fn counterbored(
        prism: &PolygonPrism,
        cuts: &[Counterbore],
        clearance_mm: f64,
        minimum_mm: f64,
    ) -> Result<Self, PocketError> {
        use PocketError::{BudgetExceeded, InvalidInput, UnresolvedBoundary};
        if cuts.is_empty()
            || !clearance_mm.is_finite()
            || clearance_mm < 0.
            || !minimum_mm.is_finite()
            || minimum_mm <= 0.
        {
            return Err(InvalidInput);
        }
        let holes = prism.circular_holes();
        if cuts.len() > 2 * holes.len() {
            return Err(BudgetExceeded);
        }
        let mut cuts = cuts.to_vec();
        cuts.sort_unstable_by_key(|cut| (cut.hole, cut.end));
        let mut seats = vec![[None; 2]; holes.len()];
        let height = prism.height().as_f64();
        let scale = prism
            .rings()
            .iter()
            .flatten()
            .flatten()
            .map(|x| x.as_f64().abs())
            .fold(height, f64::max);
        let guard = clearance_mm.max(minimum_mm) + 64. * f64::EPSILON * scale;
        for &cut in &cuts {
            let pair = seats.get_mut(cut.hole).ok_or(InvalidInput)?;
            let radius = holes[cut.hole].radius().as_f64();
            if pair[usize::from(cut.end)].is_some()
                || !cut.radius_mm.is_finite()
                || !cut.depth_mm.is_finite()
                || cut.radius_mm - radius < minimum_mm
                || cut.depth_mm < minimum_mm
            {
                return Err(InvalidInput);
            }
            pair[usize::from(cut.end)] = Some(cut);
        }
        let first = 2 + prism.rings().iter().map(Vec::len).sum::<usize>();
        // PolygonPrism limits straight vertices plus holes to 1024.
        // Four additional seat faces per hole keep every ordinal below 5122.
        let mut next = (first + holes.len()) as u32;
        let mut new_face = || {
            let index = next;
            next += 1;
            index
        };
        let centroid = prism.centroid();
        let mut cap_loss = [0.; 2];
        let mut cap_moment = [[0.; 2]; 2];
        let mut lost = 0.;
        let mut stacks = Vec::with_capacity(holes.len());
        for (i, (hole, pair)) in holes.iter().zip(seats).enumerate() {
            let radius = hole.radius().as_f64();
            let center = hole.center().map(Rational::as_f64);
            let depths = pair.map(|c| c.map_or(0., |c| c.depth_mm));
            if height - depths[0] - depths[1] <= guard {
                return Err(UnresolvedBoundary);
            }
            for (cap, cut) in pair.iter().enumerate() {
                if let Some(cut) = cut {
                    let area = PI * (cut.radius_mm - radius) * (cut.radius_mm + radius);
                    cap_loss[cap] += area;
                    lost += area * cut.depth_mm;
                    for axis in 0..2 {
                        cap_moment[cap][axis] += area * (center[axis] - centroid[axis]);
                    }
                }
            }
            let mut layers = vec![BoreLayer {
                z_mm: 0.,
                radius_mm: pair[0].map_or(radius, |c| c.radius_mm),
            }];
            let mut faces = Vec::new();
            if let Some(cut) = pair[0] {
                layers.push(BoreLayer {
                    z_mm: cut.depth_mm,
                    radius_mm: cut.radius_mm,
                });
                faces.push(new_face());
                layers.push(BoreLayer {
                    z_mm: cut.depth_mm,
                    radius_mm: radius,
                });
                faces.push(new_face());
            }
            layers.push(BoreLayer {
                z_mm: height - depths[1],
                radius_mm: radius,
            });
            faces.push((first + i) as u32);
            if let Some(cut) = pair[1] {
                layers.push(BoreLayer {
                    z_mm: height - cut.depth_mm,
                    radius_mm: cut.radius_mm,
                });
                faces.push(new_face());
                layers.push(BoreLayer {
                    z_mm: height,
                    radius_mm: cut.radius_mm,
                });
                faces.push(new_face());
            }
            stacks.push(BoreStack {
                center_mm: center,
                layers,
                face_indices: faces,
            });
        }
        // Do not use cap-only pair separation here: two different deep seats
        // can overlap in the interior while their enlarged mouths are on
        // opposite ends of the stock.
        let rings: Vec<Vec<_>> = prism
            .rings()
            .iter()
            .map(|ring| ring.iter().map(|p| p.map(Rational::as_f64)).collect())
            .collect();
        let max_radius = |s: &BoreStack| s.layers.iter().map(|l| l.radius_mm).fold(0., f64::max);
        for (i, stack) in stacks.iter().enumerate() {
            let radius = max_radius(stack);
            for ring in &rings {
                ring_contains_clear_disk(ring, stack.center_mm, radius + guard)?;
            }
            for other in &stacks[..i] {
                let d = std::array::from_fn::<_, 2, _>(|k| stack.center_mm[k] - other.center_mm[k]);
                if d[0].hypot(d[1]) <= radius + max_radius(other) + guard {
                    return Err(UnresolvedBoundary);
                }
            }
        }
        let volume_mm3 = prism.volume().as_f64() - lost;
        let caps = std::array::from_fn(|cap| {
            let area = prism.area().as_f64() - cap_loss[cap];
            (
                area,
                std::array::from_fn(|axis| centroid[axis] - cap_moment[cap][axis] / area),
            )
        });
        validate_measures(volume_mm3, &caps)?;
        Ok(Self {
            stock: Arc::new(prism.clone()),
            chamfers: Vec::new(),
            counterbores: cuts,
            stacks,
            caps,
            volume_mm3,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        Circle,
        display::{Tessellation, chamfered_polygon_prism},
        manufacturing::certify,
        step::Writer as StepWriter,
        step_read::read_analytic_solid,
    };
    fn q(n: i64) -> Rational {
        Rational::new(n, 1).unwrap()
    }
    fn stock(centers: &[[i64; 2]]) -> PolygonPrism {
        PolygonPrism::with_circular_holes(
            vec![
                vec![[0, 0], [20, 0], [20, 16], [0, 16]]
                    .into_iter()
                    .map(|p| p.map(q))
                    .collect(),
            ],
            centers
                .iter()
                .map(|p| Circle::new(p.map(q), q(1)).unwrap())
                .collect(),
            q(6),
        )
        .unwrap()
    }
    fn cut(hole: usize, end: bool, r: f64, d: f64) -> Counterbore {
        Counterbore {
            hole,
            end,
            radius_mm: r,
            depth_mm: d,
        }
    }
    fn build(p: &PolygonPrism, c: &[Counterbore]) -> Result<ChamferedBores, PocketError> {
        ChamferedBores::counterbored(p, c, 1e-7, 1e-5)
    }
    #[test]
    fn actual_flat_seats_keep_shoulders_volume_caps_and_closed_manufacturing_shells() {
        let p = stock(&[[10, 8]]);
        for cuts in [
            vec![cut(0, false, 3., 1.)],
            vec![cut(0, true, 2., 1.5)],
            vec![cut(0, false, 3., 1.), cut(0, true, 2., 1.5)],
        ] {
            let b = build(&p, &cuts).unwrap();
            assert_eq!(b.counterbores(), cuts);
            assert!(b.chamfers().is_empty());
            let loss: f64 = cuts
                .iter()
                .map(|c| std::f64::consts::PI * (c.radius_mm * c.radius_mm - 1.) * c.depth_mm)
                .sum();
            assert!((b.volume_mm3() - (p.volume().as_f64() - loss)).abs() < 1e-10);
            let s = &b.stacks()[0];
            assert_eq!(s.face_indices.len(), s.layers.len() - 1);
            assert_eq!(
                s.layers
                    .windows(2)
                    .filter(|w| w[0].z_mm == w[1].z_mm)
                    .count(),
                cuts.len()
            );
            for end in [false, true] {
                let radius = cuts
                    .iter()
                    .find(|c| c.end == end)
                    .map_or(1., |c| c.radius_mm);
                let (a, c) = b.cap_properties(end);
                assert!((a - (320. - std::f64::consts::PI * radius * radius)).abs() < 1e-10);
                assert_eq!(c, [10., 8.]);
            }
            let mesh = chamfered_polygon_prism(
                &p,
                &b,
                Tessellation {
                    deflection: 0.001,
                    angle: 0.08,
                    triangle_limit: 100000,
                },
            )
            .unwrap();
            let receipt = certify(&mesh).unwrap();
            assert!((receipt.signed_volume_mm3 - b.volume_mm3()).abs() < 0.1);
            for reverse in [false, true] {
                let mut writer = StepWriter::new();
                writer
                    .append_treated_polygon_prism(
                        &p,
                        &b,
                        [40., -8., 12.],
                        [0.8, 0.6, 0.],
                        [-0.36, 0.48, 0.8],
                        reverse,
                    )
                    .unwrap();
                let bytes = writer.finish().unwrap();
                assert!(!bytes.contains("CONICAL_SURFACE"));
                let read = read_analytic_solid(&bytes, 1e-7).unwrap();
                assert!((read.volume_mm3 - b.volume_mm3()).abs() < 1e-7);
            }
        }
    }
    #[test]
    fn opposite_deep_seats_on_different_holes_must_not_intersect_inside_stock() {
        let p = stock(&[[7, 8], [12, 8]]);
        assert_eq!(
            build(&p, &[cut(0, false, 3., 4.), cut(1, true, 3., 4.)]),
            Err(PocketError::UnresolvedBoundary)
        );
        assert!(build(&p, &[cut(0, false, 2., 1.), cut(1, true, 2., 1.)]).is_ok());
    }
    #[test]
    fn asymmetric_caps_and_untouched_bores_preserve_original_source_faces() {
        let p = stock(&[[5, 8], [14, 8]]);
        let b = build(&p, &[cut(0, false, 3., 1.)]).unwrap();
        assert_eq!(
            b.stacks()[1].layers,
            [
                BoreLayer {
                    z_mm: 0.,
                    radius_mm: 1.
                },
                BoreLayer {
                    z_mm: 6.,
                    radius_mm: 1.
                }
            ]
        );
        assert_eq!(b.stacks()[1].face_indices, [7]);
        assert_eq!(b.stacks()[0].face_indices, [8, 9, 6]);
        let loss = 8. * PI;
        let original = p.centroid();
        let original_area = p.area().as_f64();
        let (area, center) = b.cap_properties(false);
        for axis in 0..2 {
            let expected =
                (original_area * original[axis] - loss * [5., 8.][axis]) / (original_area - loss);
            assert!((center[axis] - expected).abs() < 1e-12);
        }
        assert!((area - (original_area - loss)).abs() < 1e-12);
        assert_eq!(b.cap_properties(true), (original_area, original));
        let mesh = chamfered_polygon_prism(
            &p,
            &b,
            Tessellation {
                deflection: 0.001,
                angle: 0.08,
                triangle_limit: 100000,
            },
        )
        .unwrap();
        certify(&mesh).unwrap();
        let faces: std::collections::BTreeSet<_> = mesh.face_indices.iter().copied().collect();
        assert_eq!(faces, (0..10).collect());
    }
    #[test]
    fn scalar_measure_validation_refuses_overflow_and_degenerate_material() {
        let caps = [(1., [0., 0.]); 2];
        for volume in [0., -1., f64::MAX * 2., f64::NAN] {
            assert_eq!(
                validate_measures(volume, &caps),
                Err(PocketError::UnresolvedBoundary)
            );
        }
        for area in [0., -1., f64::MAX * 2.] {
            assert_eq!(
                validate_measures(1., &[(area, [0., 0.]); 2]),
                Err(PocketError::UnresolvedBoundary)
            );
        }
        assert_eq!(
            validate_measures(1., &[(1., [f64::INFINITY, 0.]); 2]),
            Err(PocketError::UnresolvedBoundary)
        );
        let p = stock(&[[10, 8]]);
        assert_eq!(
            build(&p, &[cut(0, false, 2., 1.); 3]),
            Err(PocketError::BudgetExceeded)
        );
        for (clearance, minimum) in [(f64::NAN, 1e-5), (-1., 1e-5), (0., 0.), (0., f64::NAN)] {
            assert_eq!(
                ChamferedBores::counterbored(&p, &[cut(0, false, 2., 1.)], clearance, minimum),
                Err(PocketError::InvalidInput)
            );
        }
    }
    #[test]
    fn invalid_seats_leave_the_stock_and_writer_reusable() {
        let p = stock(&[[10, 8]]);
        let before = p.clone();
        for cuts in [
            vec![],
            vec![cut(9, false, 3., 1.)],
            vec![cut(0, false, f64::NAN, 1.)],
            vec![cut(0, false, 3., f64::INFINITY)],
            vec![cut(0, false, 1., 1.)],
            vec![cut(0, false, 3., 0.)],
            vec![cut(0, false, 3., 1.), cut(0, false, 3., 2.)],
            vec![cut(0, false, 3., 4.), cut(0, true, 2., 2.)],
            vec![cut(0, false, 20., 1.)],
        ] {
            assert!(build(&p, &cuts).is_err());
            assert_eq!(p, before);
        }
        let b = build(&p, &[cut(0, false, 3., 1.)]).unwrap();
        let mut writer = StepWriter::new();
        assert!(
            writer
                .append_treated_polygon_prism(
                    &stock(&[[9, 8]]),
                    &b,
                    [0.; 3],
                    [1., 0., 0.],
                    [0., 1., 0.],
                    false
                )
                .is_err()
        );
        writer
            .append_treated_polygon_prism(&p, &b, [0.; 3], [1., 0., 0.], [0., 1., 0.], false)
            .unwrap();
        assert!(read_analytic_solid(&writer.finish().unwrap(), 1e-7).is_ok());
    }
}
