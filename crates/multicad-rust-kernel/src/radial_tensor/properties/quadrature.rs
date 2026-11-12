//! Tensor-product Gaussian estimates with whole-source eighth-derivative bounds.
use super::*;
use measure::{Density, QUANTITIES, Values, evaluate};
use source::Source;
pub(super) fn enclosure(
    source: &Source,
    density: Density,
    chart: &axial::Chart,
    center: f64,
    work: &mut Work<'_>,
) -> Result<Values, Error> {
    // A fixed degree-nine source cell, all derivative coefficients through
    // order nine, and sixteen Gaussian nodes have bounded arithmetic cost.
    work.charge(if source.dimensions() == 2 {
        32768
    } else {
        1024
    })?;
    let mut remainder = [I::point(0.); QUANTITIES];
    for axis in 0..source.dimensions() {
        let derivatives = evaluate(density, source.taylor(axis == 1)?, chart, center)?;
        for i in 0..QUANTITIES {
            // Gauss4 on [0,1]: the remainder coefficient is
            // (4!)^4/(9*(8!)^2) = 1/44100 times sup|c8|.
            remainder[i] =
                remainder[i] + I::point(derivatives[i].c[8].magnitude()) / I::point(44100.);
        }
    }
    let root = (I::point(6.) / I::point(5.)).sqrt();
    let outer = ((I::point(3.) + I::point(2.) * root) / I::point(7.)).sqrt() / I::point(2.);
    let inner = ((I::point(3.) - I::point(2.) * root) / I::point(7.)).sqrt() / I::point(2.);
    let weight_outer = (I::point(18.) - I::point(30.).sqrt()) / I::point(72.);
    let weight_inner = (I::point(18.) + I::point(30.).sqrt()) / I::point(72.);
    let nodes = [
        (I::point(0.5) - outer, weight_outer),
        (I::point(0.5) - inner, weight_inner),
        (I::point(0.5) + inner, weight_inner),
        (I::point(0.5) + outer, weight_outer),
    ];
    let mut value = [I::point(0.); QUANTITIES];
    for (u, wu) in nodes {
        let second = if source.dimensions() == 2 {
            nodes.to_vec()
        } else {
            vec![(I::point(0.), I::point(1.))]
        };
        for (v, wv) in second {
            let jet = source.at([u, v]);
            let at = evaluate(density, jet, chart, center)?;
            for i in 0..QUANTITIES {
                value[i] = value[i] + wu * wv * at[i].c[0];
            }
        }
    }
    for i in 0..QUANTITIES {
        value[i] = (value[i] + I::new(-remainder[i].hi, remainder[i].hi)).checked()?;
    }
    Ok(value)
}
