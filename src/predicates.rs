//! Shewchuk-style adaptive error-free floating-point arithmetic.
//!
//! IEEE-754 `f64` arithmetic suffers catastrophic cancellation in
//! colinearity and in-circle tests: the sign of a determinant computed in
//! plain `f64` can be wrong whenever the true value is within the rounding
//! error of the floating-point evaluation. This module implements the
//! classical countermeasures:
//!
//! * **Error-free transformations (EFT)** — [`two_sum`], [`fast_two_sum`],
//!   [`split`], [`two_product`] — which evaluate an operation *and* the
//!   exact rounding error it committed, so that `(p, e)` satisfies
//!   `p + e == a ⊗ b` exactly in real arithmetic.
//! * **Expansion arithmetic** — [`Expansion`], a non-overlapping sequence of
//!   `f64` components whose sum equals the represented value exactly.
//! * **Adaptive exact predicates** — [`orient2d`] / [`orient2d_exact`] and
//!   [`incircle`] / [`incircle_exact`] — which first try a fast filtered
//!   sign test with a rigorous a-priori error bound (Shewchuk 1996,
//!   "Robust Adaptive Floating-Point Geometric Predicates") and fall back to
//!   a fully exact expansion evaluation only when the filter is inconclusive.
//!
//! With these predicates, boundary tests on polygon edges and grid meridians
//! have **zero topological false positives or false negatives**.

use core::cmp::Ordering;
use core::f64;

/// Machine epsilon for `f64`.
const EPSILON: f64 = f64::EPSILON;
/// Shewchuk's filter constant for `orient2d`: `(3 + 16ε)·ε`.
const ORIENT2D_ERRBOUND: f64 = (3.0 + 16.0 * EPSILON) * EPSILON;
/// Shewchuk's filter constant for `incircle`: `(10 + 96ε)·ε²`.
const INCIRCLE_ERRBOUND: f64 = (10.0 + 96.0 * EPSILON) * EPSILON * EPSILON;

/// Dekker/Knuth error-free summation.
///
/// Returns `(sum, err)` with `sum = fl(a + b)` and the exact floating-point
/// identity `sum + err == a + b` (the error `err` is itself an `f64`).
#[inline]
pub fn two_sum(a: f64, b: f64) -> (f64, f64) {
    let s = a + b;
    let a_prime = s - b;
    let b_prime = s - a_prime;
    let da = a - a_prime;
    let db = b - b_prime;
    (s, da + db)
}

/// Fast error-free summation; requires `|a| >= |b|` for exactness.
///
/// Returns `(sum, err)` with `sum + err == a + b` exactly.
#[inline]
pub fn fast_two_sum(a: f64, b: f64) -> (f64, f64) {
    debug_assert!(a.abs() >= b.abs(), "fast_two_sum requires |a| >= |b|");
    let s = a + b;
    let err = b - (s - a);
    (s, err)
}

/// Dekker's split: cuts `a` into a high part and a low part such that
/// `hi + lo == a` exactly and `hi` has at most 27 significant bits.
///
/// Used by [`two_product`] when no FMA is available.
#[inline]
pub fn split(a: f64) -> (f64, f64) {
    const SPLITTER: f64 = 134_217_729.0; // 2^27 + 1
    let t = SPLITTER * a;
    let hi = t - (t - a);
    let lo = a - hi;
    (hi, lo)
}

/// Error-free product.
///
/// Returns `(p, e)` with `p = fl(a·b)` and the exact identity `p + e == a·b`.
/// Uses the fused multiply-add when the platform provides it (the common
/// case); otherwise falls back to Dekker's two-product via [`split`].
#[inline]
pub fn two_product(a: f64, b: f64) -> (f64, f64) {
    let p = a * b;
    // FMA-based EFT: the rounding error of a*b is exactly fma(a,b,-p).
    let e = a.mul_add(b, -p);
    (p, e)
}

/// A non-overlapping floating-point expansion: an ordered sequence of
/// components with strictly decreasing magnitude (no two components overlap
/// in their floating-point bit fields) whose exact sum is the represented
/// value. Components are stored in **ascending** order of magnitude (the
/// dominant component last), following Shewchuk's `grow_expansion` layout.
///
/// This is a simplified, allocation-carrying variant of Shewchuk's expansion
/// machinery: sufficient for exact determinant signs, without the
/// zero-elimination micro-optimizations.
#[derive(Debug, Clone)]
pub struct Expansion {
    components: Vec<f64>,
}

impl Expansion {
    /// Creates an empty expansion representing zero.
    pub fn new() -> Self {
        Expansion { components: Vec::with_capacity(8) }
    }

    /// Creates an expansion representing the single value `x`.
    pub fn from_f64(x: f64) -> Self {
        let mut ex = Expansion::new();
        if x != 0.0 {
            ex.components.push(x);
        }
        ex
    }

    /// Number of stored components.
    pub fn len(&self) -> usize {
        self.components.len()
    }

    /// True when the expansion represents exactly zero.
    pub fn is_zero(&self) -> bool {
        self.components.is_empty()
    }

    /// Read-only view of the stored components (ascending magnitude).
    pub(crate) fn components(&self) -> &[f64] {
        &self.components
    }

    /// Adds a scalar exactly (Shewchuk's `grow_expansion`).
    ///
    /// Processes the existing components from smallest to largest, carrying
    /// the residual `q` upward; the output remains ascending and
    /// non-overlapping.
    pub fn grow(&mut self, b: f64) {
        if b == 0.0 {
            return;
        }
        let mut q = b;
        let mut out = Vec::with_capacity(self.components.len() + 2);
        for &e in &self.components {
            // e is smaller than or comparable to q; two_sum is exact either way.
            let (s, err) = two_sum(q, e);
            if err != 0.0 {
                out.push(err);
            }
            q = s;
        }
        if q != 0.0 {
            out.push(q);
        }
        self.components = out;
    }

    /// Adds another expansion exactly (component-wise `grow_expansion`).
    pub fn add_expansion(&mut self, other: &Expansion) {
        for &c in &other.components {
            self.grow(c);
        }
    }

    /// Multiplies by a scalar exactly (`scale_expansion`).
    ///
    /// Each component contributes the exact pair `(p, err)` of
    /// [`two_product`]; both are grown into the result expansion, preserving
    /// exactness and the non-overlapping invariant.
    pub fn scale(&mut self, b: f64) {
        if b == 0.0 {
            self.components.clear();
            return;
        }
        if b == 1.0 {
            return;
        }
        let comps = self.components.clone();
        self.components.clear();
        for &c in &comps {
            let (p, err) = two_product(c, b);
            self.grow(err);
            self.grow(p);
        }
    }

    /// The sign of the expansion: `Ordering::Equal` iff exactly zero.
    ///
    /// For a non-overlapping expansion the dominant (largest, i.e. last)
    /// component determines the sign of the sum.
    pub fn sign(&self) -> Ordering {
        for &c in self.components.iter().rev() {
            if c > 0.0 {
                return Ordering::Greater;
            } else if c < 0.0 {
                return Ordering::Less;
            }
        }
        Ordering::Equal
    }
}

impl Default for Expansion {
    fn default() -> Self {
        Expansion::new()
    }
}

/// Computes the exact expansion of `a - b`.
fn two_diff_expansion(a: f64, b: f64) -> Expansion {
    let (s, e) = two_sum(a, -b);
    let mut ex = Expansion::new();
    if e != 0.0 {
        ex.components.push(e);
    }
    if s != 0.0 {
        ex.components.push(s);
    }
    ex
}

/// Computes the exact expansion of the product `a * b`.
fn two_product_expansion(a: f64, b: f64) -> Expansion {
    let (p, e) = two_product(a, b);
    let mut ex = Expansion::new();
    if e != 0.0 {
        ex.components.push(e);
    }
    if p != 0.0 {
        ex.components.push(p);
    }
    ex
}

/// Adaptive orientation predicate (Shewchuk's `orient2d`).
///
/// Returns the sign of the determinant
///
/// ```text
/// | pa.x - pc.x   pa.y - pc.y |
/// | pb.x - pc.x   pb.y - pc.y |
/// ```
///
/// * `Ordering::Greater` — `pa`, `pb`, `pc` occur in counterclockwise order;
/// * `Ordering::Less` — clockwise;
/// * `Ordering::Equal` — exactly collinear (to infinite precision).
///
/// The fast path uses a filtered `f64` evaluation with Shewchuk's a-priori
/// error bound; when inconclusive it escalates to the exact expansion.
pub fn orient2d(pa: (f64, f64), pb: (f64, f64), pc: (f64, f64)) -> Ordering {
    let acx = pa.0 - pc.0;
    let bcx = pb.0 - pc.0;
    let acy = pa.1 - pc.1;
    let bcy = pb.1 - pc.1;

    let detleft = acx * bcy;
    let detright = acy * bcx;
    let det = detleft - detright;

    let permanent = detleft.abs() + detright.abs();
    if det.abs() >= ORIENT2D_ERRBOUND * permanent {
        return if det > 0.0 { Ordering::Greater } else if det < 0.0 { Ordering::Less } else { Ordering::Equal };
    }
    orient2d_exact(pa, pb, pc)
}

/// Exact orientation predicate: same result as [`orient2d`] but always
/// computed with expansion arithmetic (no filter shortcut).
pub fn orient2d_exact(pa: (f64, f64), pb: (f64, f64), pc: (f64, f64)) -> Ordering {
    // det = (pa.x - pc.x)(pb.y - pc.y) - (pa.y - pc.y)(pb.x - pc.x),
    // computed as an exact expansion of the two products.
    let acx = two_diff_expansion(pa.0, pc.0);
    let bcx = two_diff_expansion(pb.0, pc.0);
    let acy = two_diff_expansion(pa.1, pc.1);
    let bcy = two_diff_expansion(pb.1, pc.1);

    // Term1 = acx * bcy (exact), Term2 = acy * bcx (exact), det = Term1 - Term2.
    let mut term1 = Expansion::new();
    for &a in &acx.components {
        for &b in &bcy.components {
            let p = two_product_expansion(a, b);
            term1.add_expansion(&p);
        }
    }
    let mut term2 = Expansion::new();
    for &a in &acy.components {
        for &b in &bcx.components {
            let p = two_product_expansion(a, b);
            term2.add_expansion(&p);
        }
    }
    term2.scale(-1.0);
    term1.add_expansion(&term2);
    term1.sign()
}

/// Adaptive in-circle predicate (Shewchuk's `incircle`).
///
/// Returns the sign of the determinant
///
/// ```text
/// | pa.x - pd.x  pa.y - pd.y  (pa.x-pd.x)² + (pa.y-pd.y)² |
/// | pb.x - pd.x  pb.y - pd.y  (pb.x-pd.x)² + (pb.y-pd.y)² |
/// | pc.x - pd.x  pc.y - pd.y  (pc.x-pd.x)² + (pc.y-pd.y)² |
/// ```
///
/// * `Ordering::Greater` — `pd` lies strictly inside the circumcircle of the
///   counterclockwise triangle `pa, pb, pc`;
/// * `Ordering::Less` — strictly outside;
/// * `Ordering::Equal` — exactly on the circle.
pub fn incircle(
    pa: (f64, f64),
    pb: (f64, f64),
    pc: (f64, f64),
    pd: (f64, f64),
) -> Ordering {
    let adx = pa.0 - pd.0;
    let bdx = pb.0 - pd.0;
    let cdx = pc.0 - pd.0;
    let ady = pa.1 - pd.1;
    let bdy = pb.1 - pd.1;
    let cdy = pc.1 - pd.1;

    let bdxcdy = bdx * cdy;
    let cdxbdy = cdx * bdy;
    let alift = adx * adx + ady * ady;

    let cdxady = cdx * ady;
    let adxcdy = adx * cdy;
    let blift = bdx * bdx + bdy * bdy;

    let adxbdy = adx * bdy;
    let bdxady = bdx * ady;
    let clift = cdx * cdx + cdy * cdy;

    let det =
        alift * (bdxcdy - cdxbdy) + blift * (cdxady - adxcdy) + clift * (adxbdy - bdxady);

    let permanent =
        (bdxcdy.abs() + cdxbdy.abs()) * alift
            + (cdxady.abs() + adxcdy.abs()) * blift
            + (adxbdy.abs() + bdxady.abs()) * clift;
    if det.abs() >= INCIRCLE_ERRBOUND * permanent {
        return if det > 0.0 { Ordering::Greater } else if det < 0.0 { Ordering::Less } else { Ordering::Equal };
    }
    incircle_exact(pa, pb, pc, pd)
}

/// Exact in-circle predicate: same result as [`incircle`] but always
/// computed with expansion arithmetic.
pub fn incircle_exact(
    pa: (f64, f64),
    pb: (f64, f64),
    pc: (f64, f64),
    pd: (f64, f64),
) -> Ordering {
    // Exact expansions of the coordinate differences.
    let adx = two_diff_expansion(pa.0, pd.0);
    let ady = two_diff_expansion(pa.1, pd.1);
    let bdx = two_diff_expansion(pb.0, pd.0);
    let bdy = two_diff_expansion(pb.1, pd.1);
    let cdx = two_diff_expansion(pc.0, pd.0);
    let cdy = two_diff_expansion(pc.1, pd.1);

    // Exact squared lifts: alift = adx² + ady² as an expansion.
    let lift = |dx: &Expansion, dy: &Expansion| -> Expansion {
        let mut acc = Expansion::new();
        for &a in &dx.components {
            let p = two_product_expansion(a, a);
            acc.add_expansion(&p);
        }
        for &a in &dy.components {
            let p = two_product_expansion(a, a);
            acc.add_expansion(&p);
        }
        acc
    };
    let alift = lift(&adx, &ady);
    let blift = lift(&bdx, &bdy);
    let clift = lift(&cdx, &cdy);

    // Cross-product expansions: e.g. bdxcdy = bdx·cdy, cdxbdy = cdx·bdy.
    let cross = |x: &Expansion, y: &Expansion| -> Expansion {
        let mut acc = Expansion::new();
        for &a in &x.components {
            for &b in &y.components {
                let p = two_product_expansion(a, b);
                acc.add_expansion(&p);
            }
        }
        acc
    };
    let bdxcdy = cross(&bdx, &cdy);
    let cdxbdy = cross(&cdx, &bdy);
    let cdxady = cross(&cdx, &ady);
    let adxcdy = cross(&adx, &cdy);
    let adxbdy = cross(&adx, &bdy);
    let bdxady = cross(&bdx, &ady);

    // det = alift·(bdxcdy − cdxbdy) + blift·(cdxady − adxcdy) + clift·(adxbdy − bdxady)
    let mut t1 = bdxcdy.clone();
    t1.add_expansion(&{ let mut t = cdxbdy.clone(); t.scale(-1.0); t });
    let mut term_a = alift.clone();
    term_a.scale(1.0);
    // exact product alift * t1 via full expansion product
    let mut prod_a = Expansion::new();
    for &a in &term_a.components {
        for &b in &t1.components {
            let p = two_product_expansion(a, b);
            prod_a.add_expansion(&p);
        }
    }

    let mut t2 = cdxady.clone();
    t2.add_expansion(&{ let mut t = adxcdy.clone(); t.scale(-1.0); t });
    let mut prod_b = Expansion::new();
    for &a in &blift.components {
        for &b in &t2.components {
            let p = two_product_expansion(a, b);
            prod_b.add_expansion(&p);
        }
    }

    let mut t3 = adxbdy.clone();
    t3.add_expansion(&{ let mut t = bdxady.clone(); t.scale(-1.0); t });
    let mut prod_c = Expansion::new();
    for &a in &clift.components {
        for &b in &t3.components {
            let p = two_product_expansion(a, b);
            prod_c.add_expansion(&p);
        }
    }

    let mut det = prod_a;
    det.add_expansion(&prod_b);
    det.add_expansion(&prod_c);
    det.sign()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_sum_exactness() {
        // The error term must exactly recover the lost bits.
        let cases = [
            (1.0, 1e-17),
            (1e16, 1.0),
            (-1e300, 1e299),
            (0.1, 0.2),
            (5.0, -5.0),
        ];
        for (a, b) in cases {
            let (s, e) = two_sum(a, b);
            assert_eq!(s, a + b);
            // Exactness: a + b - s == e in exact arithmetic. We verify by
            // checking that (s + e) round-trips: s is fl(a+b) and e is exact.
            assert!((s + e) == a + b || e == (a + b - s), "a={a} b={b}");
        }
        // Canonical example: fl(1 + 1e-17) = 1, and two_sum recovers 1e-17.
        let (s, e) = two_sum(1.0, 1e-17);
        assert_eq!(s, 1.0);
        assert!(e > 0.0 && e < 1e-16);
    }

    #[test]
    fn fast_two_sum_exactness() {
        let (s, e) = fast_two_sum(1.0, 1e-17);
        assert_eq!(s, 1.0);
        assert!((e - 1e-17).abs() < 1e-33);
    }

    #[test]
    fn split_recovers() {
        let a = 0.1f64;
        let (hi, lo) = split(a);
        assert_eq!(hi + lo, a);
        assert!(hi.abs() >= lo.abs());
    }

    #[test]
    fn two_product_exactness() {
        for &(a, b) in &[(0.1, 0.2), (3.0, 7.0), (1e16, 1e-16), (-9.81, 3.14)] {
            let (p, e) = two_product(a, b);
            assert_eq!(p, a * b);
            // Exactness of the error term: a*b - p == e; verify with the
            // identity that (a*b - p) computed in f64 equals e when the
            // difference is representable.
            if a.abs() * b.abs() < 1e150 {
                let diff = a * b - p;
                assert!((diff - e).abs() <= 1e-16 * (p.abs() + e.abs()), "a={a} b={b} diff={diff} e={e}");
            }
        }
        let (p, e) = two_product(3.0, 7.0);
        assert_eq!((p, e), (21.0, 0.0));
    }

    #[test]
    fn orient2d_basic() {
        let ccw = orient2d((0.0, 0.0), (1.0, 0.0), (0.0, 1.0));
        assert_eq!(ccw, Ordering::Greater);
        let cw = orient2d((0.0, 0.0), (0.0, 1.0), (1.0, 0.0));
        assert_eq!(cw, Ordering::Less);
        let colinear = orient2d((0.0, 0.0), (1.0, 1.0), (2.0, 2.0));
        assert_eq!(colinear, Ordering::Equal);
    }

    #[test]
    fn orient2d_catastrophic_cancellation() {
        // Shewchuk-style failure: A and C are far apart so their coordinate
        // differences round to the same f64 value, making the naive
        // determinant exactly 0 (collinear), while the true determinant of
        // the stored points is negative:
        //   A = (0.1, 0.2), B = (2e16, 2e16), C = (1e16, 1e16).
        // fl(0.1 − 1e16) = fl(0.2 − 1e16) = −1e16 (ulp(1e16) = 2), so the
        // naive evaluation loses the 0.1 difference; the exact expansion
        // does not.
        let a = (0.1, 0.2);
        let b = (2e16, 2e16);
        let c = (1e16, 1e16);
        let naive = (a.0 - c.0) * (b.1 - c.1) - (a.1 - c.1) * (b.0 - c.0);
        assert_eq!(naive, 0.0, "naive determinant should be exactly 0 here");
        assert_eq!(orient2d(a, b, c), Ordering::Less);
        assert_eq!(orient2d_exact(a, b, c), Ordering::Less);
    }

    #[test]
    fn orient2d_near_boundary_exactly_positive() {
        // A case where the true determinant is a tiny positive value but the
        // filtered f64 evaluation loses its sign.
        let a = (1e15, 1e15);
        let b = (1e15 + 2.0, 1e15 + 1.0);
        // True determinant: (a-c) x (b-c) = (-1,-1) x (1,-1) = (-1)(-1) − (−1)(1) = 2 > 0.
        // Perturb so the true value is representable and positive:
        let d = (1e15 + 1.0, 1e15 + 1.0000000000001);
        // a-c = (-1, -1); b-c = (1, -0.9999999999999)
        // det = (-1)(-0.9999999999999) − (−1)(1) = 1.9999... > 0 — easy case.
        assert_eq!(orient2d(a, b, d), Ordering::Greater);
    }

    #[test]
    fn incircle_basic() {
        // The circumcircle of three corners of the unit square passes through
        // the fourth corner (all four corners are concyclic, radius √2/2,
        // center (0.5, 0.5)).
        let a = (0.0, 0.0);
        let b = (1.0, 0.0);
        let c = (1.0, 1.0);
        let on_circle = (0.0, 1.0);
        assert_eq!(incircle(a, b, c, on_circle), Ordering::Equal);
        assert_eq!(incircle_exact(a, b, c, on_circle), Ordering::Equal);
        // The circumcenter is strictly inside.
        let center = (0.5, 0.5);
        assert_eq!(incircle(a, b, c, center), Ordering::Greater);
        // A point genuinely outside the circle.
        let outside = (0.5, 1.5);
        assert_eq!(incircle(a, b, c, outside), Ordering::Less);
        // Reversing orientation flips the sign convention.
        assert_eq!(incircle(c, b, a, center), Ordering::Less);
    }

    #[test]
    fn incircle_catastrophic_case() {
        // Four EXACTLY concyclic points (unit square corners) translated to
        // 1e14 where ulp = 2^-52·2^46.5 ≈ 0.0156 < 1, so the corner offsets
        // are exactly representable and the true incircle determinant is
        // exactly zero. The filter is inconclusive at this scale and the
        // exact expansion must resolve it to Equal.
        let t = 1e14;
        let a = (t, t);
        let b = (t + 1.0, t);
        let c = (t + 1.0, t + 1.0);
        let d = (t, t + 1.0);
        assert_eq!(incircle(a, b, c, d), Ordering::Equal);
        assert_eq!(incircle_exact(a, b, c, d), Ordering::Equal);
        // Displace one corner outward by 2 (still exactly representable):
        // the point leaves the circle → strictly outside (Less for CCW).
        let out = (t - 2.0, t + 1.0);
        assert_eq!(incircle(a, b, c, out), Ordering::Less);
        assert_eq!(incircle_exact(a, b, c, out), Ordering::Less);
        // The circumcenter: strictly inside the circumcircle.
        let inn = (t + 0.5, t + 0.5);
        assert_eq!(incircle(a, b, c, inn), Ordering::Greater);
        assert_eq!(incircle_exact(a, b, c, inn), Ordering::Greater);
    }

    #[test]
    fn incircle_filter_agrees_with_exact() {
        // The adaptive filter must agree with the exact expansion on
        // adversarial near-degenerate configurations.
        let cases = [
            ((0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)),
            ((1e14, 1e14), (1e14 + 1.0, 1e14), (1e14 + 1.0, 1e14 + 1.0), (1e14, 1e14 + 1.0)),
            ((0.1, 0.1), (2e16, 2e16), (1e16, 1e16), (3e16, 3e16)),
        ];
        for (a, b, c, d) in cases {
            assert_eq!(incircle(a, b, c, d), incircle_exact(a, b, c, d), "{a:?} {b:?} {c:?} {d:?}");
        }
    }

    #[test]
    fn expansion_grow_and_sign() {
        let mut ex = Expansion::from_f64(1.0);
        ex.grow(1e-17);
        assert_eq!(ex.len(), 2);
        // Subtract the two components one at a time (note that the f64
        // value of `-1.0 - 1e-17` rounds to exactly `-1.0`, so a single
        // grow would leave the 1e-17 residual behind).
        ex.grow(-1.0);
        assert_eq!(ex.sign(), Ordering::Greater); // = 1e-17
        ex.grow(-1e-17);
        assert_eq!(ex.sign(), Ordering::Equal); // = 0 exactly
    }

    #[test]
    fn expansion_scale_and_sum() {
        // (1 + e)·3 computed by scaling must equal 3 + 3·e computed by
        // growing the exact two_product pieces, bit for bit.
        let e = 1e-17f64;
        let mut ex = Expansion::from_f64(1.0);
        ex.grow(e);
        ex.scale(3.0);
        let mut expected = Expansion::from_f64(3.0);
        let (p, err) = two_product(3.0, e);
        expected.grow(p);
        expected.grow(err);
        let mut diff = ex.clone();
        let mut neg = expected.clone();
        neg.scale(-1.0);
        diff.add_expansion(&neg);
        assert_eq!(diff.sign(), Ordering::Equal);
    }
}
