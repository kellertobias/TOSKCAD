//! Actual analytic cone evaluated on its complete decoded parameter curve.
use super::polynomial::Curve;
use super::{polynomial::*, *};
use crate::thread_entry_chamfer::trim::Jet;

pub(super) struct Cone {
    frame: frame::Frame,
    radius: I,
    sin: I,
    cos: I,
}
impl Cone {
    pub fn read(shell: &Shell<'_, '_>, id: u32) -> Result<Self> {
        let [
            Value::String(_),
            axis,
            Value::Number(radius),
            Value::Number(angle),
        ] = shell.args(id, "CONICAL_SURFACE")?
        else {
            return Err(ReadError::Invalid);
        };
        require(
            radius.is_finite()
                && *radius > 0.
                && angle.is_finite()
                && *angle > 0.
                && *angle < std::f64::consts::FRAC_PI_2,
        )?;
        let angle = I::point(*angle);
        Ok(Self {
            frame: frame::Frame::read(shell, reference(axis)?)?,
            radius: I::point(*radius),
            sin: angle.sin(),
            cos: angle.cos(),
        })
    }
    fn composition(&self, uv: [Jet; 2]) -> Result<[Jet; 3]> {
        let [sin, cos] = uv[0].sin_cos().map_err(|_| ReadError::Unsupported)?;
        let radius = Jet::constant(self.radius) + uv[1].scale(self.sin);
        let local = [radius * cos, radius * sin, uv[1].scale(self.cos)];
        let world = std::array::from_fn(|i| {
            (0..3).fold(Jet::constant(I::point(0.)), |sum, a| {
                sum + local[a].scale(self.frame.axes[a][i])
            })
        });
        for jet in world {
            jet.checked().map_err(|_| ReadError::Unsupported)?;
        }
        Ok(world)
    }
    fn error(&self, curve: &[[I; 3]], uv: &[[I; 2]], work: &mut Work) -> Result<f64> {
        // Subtract the fixed retained origin before any Taylor enclosure.
        // This is an exact interval displacement, not a reconstructed pose.
        let points = curve
            .iter()
            .map(|p| std::array::from_fn(|i| p[i] - self.frame.origin[i]))
            .collect::<Vec<_>>();
        let actual = jets(&points, I::point(0.5), work)?;
        let actual_range = jet_bounds(&points, work)?;
        let expected = self.composition(jets(uv, I::point(0.5), work)?)?;
        let expected_range = self.composition(jet_bounds(uv, work)?)?;
        taylor_error(
            actual,
            expected,
            actual_range.map(|j| j.0[4]),
            expected_range.map(|j| j.0[4]),
        )
    }
    pub fn verify(
        &self,
        curve: &Curve<3>,
        uv: &Curve<2>,
        tolerance: f64,
        work: &mut Work,
    ) -> Result<f64> {
        require(curve.domain() == uv.domain() && tolerance.is_finite() && tolerance > 0.)?;
        let mut knots = curve.knots.clone();
        knots.extend_from_slice(&uv.knots);
        knots.sort_by(f64::total_cmp);
        knots.dedup();
        let mut maximum = 0_f64;
        let mut pending = Vec::new();
        for pair in knots.windows(2) {
            pending.push((
                curve.restrict([pair[0], pair[1]], work)?,
                uv.restrict([pair[0], pair[1]], work)?,
                0u8,
            ));
            while let Some((c, p, depth)) = pending.pop() {
                work.charge(1)?;
                let error = self.error(&c, &p, work)?;
                if error <= tolerance {
                    maximum = maximum.max(error);
                    continue;
                }
                if depth >= 40 {
                    return Err(ReadError::Unsupported);
                }
                let [ca, cb] = split(&c, I::point(0.5), work)?;
                let [pa, pb] = split(&p, I::point(0.5), work)?;
                pending.push((cb, pb, depth + 1));
                pending.push((ca, pa, depth + 1));
            }
        }
        Ok(maximum)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn emitted_conical_trims_bind_to_both_actual_lifted_periodic_branches() {
        let source = crate::helical_thread::Thread::new(crate::helical_thread::Spec {
            major_radius_mm: 3.,
            pitch_mm: 1.,
            length_mm: 6.6,
            depth_mm: 0.5,
            crest_width_mm: 0.125,
            lower_flank_deg: 30.,
            upper_flank_deg: 30.,
            starts: 1,
            left_hand: false,
            root_radius_mm: None,
        })
        .unwrap();
        let body = Chamfer::new(source, 0.4, None).unwrap();
        let text = crate::step::write_thread_entry_chamfer(
            &body,
            [0.; 3],
            [1., 0., 0.],
            [0., 1., 0.],
            1e-7,
        )
        .unwrap();
        compound::read_members_by(&text, 1e-7, true, true, |shell, id, _| {
            let mut work = Work {
                remaining: 100_000_000,
                interrupt: None,
                cancelled: false,
            };
            let mut count = 0;
            for face_id in shell.member_faces(id)?.into_iter().skip(6) {
                let (surface, _, bounds) = face(shell, face_id)?;
                let actual = Cone::read(shell, surface)?;
                for (edge, _) in ring(shell, bounds[0], true)? {
                    let [_, _, _, geometry, _] =
                        <&[Value<'_>; 5]>::try_from(shell.args(edge, "EDGE_CURVE")?)
                            .expect("fixture edge");
                    let geometry = shell.simple(reference(geometry)?)?;
                    let [_, curve, pcurves, _] =
                        <&[Value<'_>; 4]>::try_from(geometry.args.as_slice())
                            .expect("fixture surface curve");
                    if shell.simple(reference(curve)?)?.name != "B_SPLINE_CURVE_WITH_KNOTS" {
                        continue;
                    }
                    let c = Curve::<3>::read(shell, reference(curve)?)?;
                    for pc in refs(pcurves)? {
                        let (support, uv) = shell.pcurve_definition(pc)?;
                        if support != surface {
                            continue;
                        }
                        let uv = Curve::<2>::read(shell, uv)?;
                        actual.verify(&c, &uv, 1e-7, &mut work)?;
                        count += 1;
                    }
                }
            }
            assert!(count >= 6);
            eprintln!(
                "bound {count} actual cone curves, remaining work {}",
                work.remaining
            );
            Ok(())
        })
        .unwrap();
    }
}

#[cfg(test)]
mod failure_tests {
    use super::*;
    fn work() -> Work<'static> {
        Work {
            remaining: 5_000_000,
            interrupt: None,
            cancelled: false,
        }
    }
    #[test]
    fn actual_cone_record_and_full_interval_departures_are_checked_before_acceptance() {
        with_shell("#1=CONICAL_SURFACE('');", |s| {
            assert!(Cone::read(s, 1).is_err())
        });
        with_shell("#1=CONICAL_SURFACE('',#2,-1.,0.);", |s| {
            assert!(Cone::read(s, 1).is_err())
        });
        with_shell(
            "#1=CARTESIAN_POINT('',(0.,0.,0.));#2=DIRECTION('',(0.,0.,1.));#3=DIRECTION('',(1.,0.,0.));#4=AXIS2_PLACEMENT_3D('',#1,#2,#3);#5=CONICAL_SURFACE('',#4,2.,0.7853981633974483);",
            |s| {
                let cone = Cone::read(s, 5).unwrap();
                let angle = std::f64::consts::FRAC_PI_4;
                let c = Curve::new(
                    1,
                    vec![0., 1.],
                    vec![2, 2],
                    vec![[2., 0., 0.], [2. + angle.sin(), 0., angle.cos()]],
                )
                .unwrap();
                let uv = Curve::new(1, vec![0., 1.], vec![2, 2], vec![[0., 0.], [0., 1.]]).unwrap();
                assert!(cone.verify(&c, &uv, 1e-10, &mut work()).unwrap() < 1e-10);
                let mut bad = c.clone();
                for p in &mut bad.poles {
                    p[2] = p[2] + I::point(0.1);
                }
                assert!(matches!(
                    cone.verify(&bad, &uv, 1e-10, &mut work()),
                    Err(ReadError::Unsupported)
                ));
            },
        );
    }
}
