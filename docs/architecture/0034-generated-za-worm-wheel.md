# ADR-0034: Generate a bounded ZA worm wheel from its screw envelope

| | |
| --- | --- |
| Status | Accepted construction decision; product availability and verification remain separate |
| Date | 2026-11-05 |
| Decider | Codex, using delegated implementation authority |

## Context and sources

CAD-230 requires a conjugate generated wheel. An ordinary spur wheel beside a
helix cannot establish that geometry. The native adapter implements an explicit
ZA axial-straight screw and the envelope of its generating hob.

[Houser and Su, 1999, printed pages 18–19, equations 8–13](https://ik.imagekit.io/agmamedia/gt/issues/1199x/houser.pdf)
derive an explicit wheel surface from equivalent translating and screw motion.
[Hu's 1997 Newcastle thesis](https://theses.ncl.ac.uk/jspui/bitstream/10443/807/1/Hu97.pdf)
distinguishes worm forms and their generation.
[KHK's gear guide, section 4.6](https://khkgears.us/media/izojpong/gear_guide1.pdf)
provides axial-module dimensions with normal pressure angle. Its stock Type III
worm is not the ZA system selected here. The cylindrical wheel blank and sharp
hob-tip transition below are explicit construction choices. No manufacturer
stock outside-diameter formula or accuracy grade is implied.

## Decision

Support one right-hand start, diameter quotient 10, normal pressure angle 20°,
axial module `m=0.5–2 mm`, wheel count `z=32–64`, worm threaded length `16m–32m`
and wheel width `0.5m–2m`. The worm reference, tip and root radii are `5m`, `6m`
and `3.75m`. Its lead and axial pitch both equal `πm`. The wheel reference radius
is `zm/2`, blank radius is `(z/2+1)m`, and center distance is `(5+z/2)m`.
The generating hob extends to radius `6.25m`, leaving `0.25m` root clearance.

Normal pressure is 20° even though module is axial. Axial pressure is
`atan(tan(20°)*sqrt(1.01))`; lead angle is `atan(0.1)`. Physical worm pitch
thickness is `πm/2−worm_thinning`. Generating-hob pitch thickness is
`πm/2+wheel_thinning`. Each member's thinning is its design backlash plus
fabrication allowance, bounded to `0–0.1m`. These are geometric inputs, not
standardized fit classes. Bore and minimum wall checks apply to the actual
root, including the generated wheel's minimum torus radius.

### Generated surface

Use module-normalized coordinates. Let `R=z/2`, `a=5+R`, `h=1/2`,
`k=tan(20°)*sqrt(1.01)`, `H=π/4+wheel_thinning/(2m)`, and flank sign `σ=±1`.
For radial parameter `r` and wheel axial coordinate `v`, define:

```text
q = sqrt(r²−v²)
s = σ(H−k(r−5))−h asin(v/r)
n = h v/r−σ k q
w = r(q−5)/n
F(r,v) = Rz(−(s+w)/R) (a−q, w, v)
```

The retained flanks are trimmed from `F` over `r=3.75–6.25`. At the hob-tip
radius `rc=6.25`, its sharp edge sweeps the transition surface:

```text
t = (1−u) w(rc,v)
E(u,v) = Rz(−(s(rc,v)+t)/R) (a−q(rc,v), t, v)
```

The intervening generated root has radius `a−sqrt(rc²−v²)` and angle
`h/R asin(v/rc)+(2u−1)(H−1.25k)/R`. An outside closure joins the two
`r=3.75` ends beyond the retained cylindrical blank. All six patches describe
one cutting space. The adapter subtracts it from one pitch sector, replicates
the retained sector boundary, removes radial seams, sews a closed shell and
cuts the requested bore. Native Boolean and sewing history supply face origins.

### Approximation and native validity

The six analytic patches use degree-16 nonrational tensor polynomials.
Certification evaluates the actual stored polynomial and analytic source with
outward interval arithmetic at the exact degree-25 Lobatto nodes. For stored
degree at most 25, their difference from the exact degree-25 source interpolant
still has degree at most 25. A tensor interpolation norm bound of 16 therefore
multiplies the largest enclosed node residual. The source tail is bounded by
`sqrt(2)*8000m/3^25` using the reviewed common complex ellipse of radius 3.
Actual native half-knot insertion adds a separately enclosed control-point
bound. The outward sum must fit half the requested linear tolerance. Higher
degrees, rational patches, unexpected knot layouts and failed bounds refuse.

This is a source-position bound. It does not certify surface normals, native
Boolean intersection curves or manufacturing accuracy. OCCT's nonanalytic
intersection algorithm uses its own edge tolerances, including a `5e−6 mm`
floor. [ADR-0006](0006-geometry-policy.md) does not define the policy's linear
tolerance as a maximum for every native edge. Closed-shell validity, actual
surface witnesses, exchange readbacks and measured intersection residuals are
separate evidence. No tolerance metadata is presented as measured error.

The final one-solid native body primes its ordinary mass cache with the public
point Gauss–Kronrod integral. It retains the `1e−10` integration request and
`1e−7` relative-error acceptance guard. Finite positive over-limit results may
try the independently guarded basis formulation from
[ADR-0033](0033-oblique-volume-integration.md). No analytical volume oracle is
installed as a native result, and failed integration never publishes a receipt.

### Frames and limits

The worm axis is local +Z, with centered ends and phase-zero crest toward −X.
The wheel axis is local +Z, with centered width and phase-zero space toward +X.
In a pair, the wheel center is `(-a*m,0,0)` in the worm frame and its rotation
is `Rx(−π/2) Rz(drive/z)`; worm rotation is `Rz(drive)`. Drive belongs to the
worm regardless of member ordering. A sampled cycle covers a full worm turn.

ZI, ZN, ZK, multiple starts, left hand, other quotients, profile shift and
manufacturer stock blanks remain outside this construction. Native samples
do not establish continuous contact or clearance, self-locking, efficiency,
load capacity, wear, standards compliance or physical fabrication fit.
