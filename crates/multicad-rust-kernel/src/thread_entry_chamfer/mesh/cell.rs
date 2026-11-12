//! Linear clipping proposes mesh connectivity; source certificates decide admission.
use super::*;
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum Kind {
    Entry,
    Seam,
    Top,
}
#[derive(Clone, Copy, Debug)]
pub(super) struct Corner {
    pub u: f64,
    pub w: f64,
    pub row: Option<i64>,
    pub boundary: Option<Kind>,
}
#[derive(Clone, Copy)]
pub(super) struct Bound {
    pub ends: [f64; 2],
    pub kind: Kind,
}
impl Bound {
    fn at(self, u: f64) -> f64 {
        (self.ends[1] - self.ends[0]).mul_add(u, self.ends[0])
    }
}
pub(super) fn polygon(
    row: i64,
    step: f64,
    lower: Bound,
    upper: Bound,
) -> Result<Vec<Corner>, Error> {
    let w = [row as f64 * step, (row + 1) as f64 * step];
    let mut p = vec![
        Corner {
            u: 0.,
            w: w[0],
            row: Some(row),
            boundary: None,
        },
        Corner {
            u: 1.,
            w: w[0],
            row: Some(row),
            boundary: None,
        },
        Corner {
            u: 1.,
            w: w[1],
            row: Some(row + 1),
            boundary: None,
        },
        Corner {
            u: 0.,
            w: w[1],
            row: Some(row + 1),
            boundary: None,
        },
    ];
    for (bound, sign) in [(lower, 1.), (upper, -1.)] {
        if p.is_empty() {
            break;
        }
        let mut out = Vec::with_capacity(6);
        let mut a = *p.last().unwrap();
        let mut da = sign * (a.w - bound.at(a.u));
        for &b in &p {
            let db = sign * (b.w - bound.at(b.u));
            if !da.is_finite() || !db.is_finite() {
                return Err(Error::Unresolved);
            }
            if (da < 0.) != (db < 0.) {
                out.push(intersection(a, b, da, db, bound)?);
            }
            if db >= 0. {
                out.push(b);
            }
            a = b;
            da = db;
        }
        p = out;
    }
    Ok(p)
}

fn intersection(a: Corner, b: Corner, da: f64, db: f64, bound: Bound) -> Result<Corner, Error> {
    let denominator = da - db;
    if !denominator.is_finite() || denominator == 0. {
        return Err(Error::Unresolved);
    }
    let t = da / denominator;
    let (mut u, mut height) = ((b.u - a.u).mul_add(t, a.u), (b.w - a.w).mul_add(t, a.w));
    let grid = if a.row == b.row { a.row } else { None };
    if a.u == b.u {
        u = a.u;
        height = bound.at(u);
    } else if a.w == b.w {
        height = a.w;
        u = (height - bound.ends[0]) / (bound.ends[1] - bound.ends[0]);
    }
    if !u.is_finite() || !height.is_finite() {
        return Err(Error::Unresolved);
    }
    Ok(Corner {
        u: u.clamp(0., 1.),
        w: height,
        row: grid,
        boundary: Some(bound.kind),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn area(p: &[Corner]) -> f64 {
        p.iter()
            .zip(p.iter().cycle().skip(1))
            .map(|(a, b)| a.u * b.w - a.w * b.u)
            .sum::<f64>()
            * 0.5
    }
    #[test]
    fn clipped_cell_proposals_preserve_boundary_ownership_and_empty_regions() {
        let lower = Bound {
            ends: [0.25, 0.5],
            kind: Kind::Entry,
        };
        let upper = Bound {
            ends: [0.75, 0.875],
            kind: Kind::Top,
        };
        let p = polygon(0, 1., lower, upper).unwrap();
        assert_eq!(p.len(), 4);
        assert_eq!(area(&p), 0.4375);
        assert!(p.iter().all(|v| v.boundary.is_some()));
        let lower = Bound {
            ends: [-0.25, 0.75],
            kind: Kind::Seam,
        };
        let upper = Bound {
            ends: [0.5, 1.25],
            kind: Kind::Top,
        };
        let p = polygon(0, 1., lower, upper).unwrap();
        assert!(area(&p) > 0.);
        assert!(p.iter().any(|v| v.row.is_some() && v.boundary.is_some()));
        assert!(
            polygon(
                0,
                1.,
                Bound {
                    ends: [2., 3.],
                    kind: Kind::Entry
                },
                Bound {
                    ends: [4., 5.],
                    kind: Kind::Top
                }
            )
            .unwrap()
            .is_empty()
        );
        assert!(
            polygon(
                0,
                1.,
                Bound {
                    ends: [f64::NAN, 0.],
                    kind: Kind::Entry
                },
                upper
            )
            .is_err()
        );
    }
    #[test]
    fn finite_edge_intersections_refuse_overflow_and_nonunique_supports() {
        let bound = Bound {
            ends: [0., 0.],
            kind: Kind::Entry,
        };
        let a = Corner {
            u: f64::MAX,
            w: 1.,
            row: None,
            boundary: None,
        };
        let b = Corner {
            u: -f64::MAX,
            w: -1.,
            row: None,
            boundary: None,
        };
        assert!(intersection(a, b, 1., -1., bound).is_err());
        assert!(
            polygon(
                0,
                f64::MAX,
                Bound {
                    ends: [0., f64::MAX],
                    kind: Kind::Entry
                },
                Bound {
                    ends: [f64::MAX, 0.],
                    kind: Kind::Top
                }
            )
            .is_err()
        );
        assert!(intersection(a, b, 0., 0., bound).is_err());
    }
}
