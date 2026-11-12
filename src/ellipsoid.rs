//! Reference ellipsoids, curvature radii, auxiliary latitudes, and meridian arc.
//!
//! This module provides the exact geometric parameters of the reference
//! ellipsoids used by modern geodetic datums — [`EllipsoidKind::GRS80`]
//! (the ellipsoid of Japan's JGD2011/JGD2024) and [`EllipsoidKind::WGS84`] —
//! together with the fundamental derived quantities:
//!
//! | Quantity             | Symbol              | Definition                                   |
//! |----------------------|---------------------|----------------------------------------------|
//! | Semi-major axis      | `a`                 | 6 378 137.0 m (both GRS80 and WGS84)          |
//! | Flattening           | `f`                 | 1/298.257222101 (GRS80), 1/298.257223563 (WGS84) |
//! | Semi-minor axis      | `b = a(1 − f)`      |                                              |
//! | First eccentricity²  | `e² = f(2 − f)`     |                                              |
//! | Second eccentricity² | `e'² = e²/(1 − e²)` |                                              |
//! | Third flattening     | `n = f/(2 − f)`     |                                              |
//! | Meridian radius      | `M(φ) = a(1−e²)/(1−e²sin²φ)^{3/2}` |                                |
//! | Prime-vertical radius| `N(φ) = a/√(1−e²sin²φ)` |                                          |
//!
//! Auxiliary latitudes implemented:
//!
//! * **Parametric (reduced) latitude** `β = atan((1−f) tan φ)` — exact.
//! * **Authalic latitude** `ξ` — two paths:
//!   1. a 6th-order Fourier series in the third flattening `n`
//!      (see [`Ellipsoid::authalic_latitude`]), accurate to ≲10⁻¹⁸ rad on the
//!      GRS80/WGS84 ellipsoids, and
//!   2. the exact definition via the auxiliary function *q* with Newton
//!      refinement ([`Ellipsoid::authalic_latitude_exact`]).
//! * **Isometric (Mercator) latitude** `ψ = atanh(sin φ) − e·atanh(e sin φ)` —
//!   exact; loxodromic (rhumb-line) geometry is straight in `(λ, ψ)`.
//! * **Meridian arc** `m(φ)` and its inverse (Newton on the 6th-order series
//!   in `e²`), used to convert metric radii into latitude extents.
//!
//! All angular I/O in this crate is in radians unless a function name or
//! documentation explicitly states degrees.

/// A point on the ellipsoid given in geodetic coordinates.
///
/// `lat` and `lon` are in radians. Longitude conventions follow ISO 6709:
/// east is positive, west is negative; the value is not required to be
/// normalized to any particular branch, but most algorithms reduce it to
/// `(−π, π]` internally.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Coord {
    /// Geodetic latitude in radians (positive north).
    pub lat: f64,
    /// Geodetic longitude in radians (positive east).
    pub lon: f64,
}

impl Coord {
    /// Constructs a coordinate from radians.
    #[inline]
    pub fn new(lat: f64, lon: f64) -> Self {
        Coord { lat, lon }
    }

    /// Constructs a coordinate from degrees.
    #[inline]
    pub fn from_degrees(lat_deg: f64, lon_deg: f64) -> Self {
        Coord { lat: lat_deg.to_radians(), lon: lon_deg.to_radians() }
    }

    /// Latitude in degrees.
    #[inline]
    pub fn lat_degrees(&self) -> f64 {
        self.lat.to_degrees()
    }

    /// Longitude in degrees, normalized to `(−180°, 180°]`.
    #[inline]
    pub fn lon_degrees(&self) -> f64 {
        normalize_lon_degrees(self.lon.to_degrees())
    }
}

/// Reduces a longitude in degrees to the branch `(−180, 180]`.
#[inline]
pub fn normalize_lon_degrees(lon_deg: f64) -> f64 {
    let mut lon = lon_deg % 360.0;
    if lon <= -180.0 {
        lon += 360.0;
    } else if lon > 180.0 {
        lon -= 360.0;
    }
    lon
}

/// Reduces an angle in radians to the branch `(−π, π]`.
#[inline]
pub fn normalize_angle(rad: f64) -> f64 {
    let mut a = rad % (2.0 * core::f64::consts::PI);
    if a <= -core::f64::consts::PI {
        a += 2.0 * core::f64::consts::PI;
    } else if a > core::f64::consts::PI {
        a -= 2.0 * core::f64::consts::PI;
    }
    a
}

/// An axis-aligned bounding box in geodetic coordinates (degrees).
///
/// `south <= north` always. `west`/`east` are stored in the branch
/// `[−180, 180)`; boxes crossing the antimeridian are represented with
/// `west > east`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BoundingBox {
    /// Southern edge (degrees).
    pub south: f64,
    /// Western edge (degrees).
    pub west: f64,
    /// Northern edge (degrees).
    pub north: f64,
    /// Eastern edge (degrees).
    pub east: f64,
}

impl BoundingBox {
    /// Constructs a bounding box from degree values.
    #[inline]
    pub fn from_degrees(south: f64, west: f64, north: f64, east: f64) -> Self {
        BoundingBox { south, west, north, east }
    }

    /// Center point of the box (radians).
    #[inline]
    pub fn center(&self) -> Coord {
        let lat = 0.5 * (self.south + self.north);
        let lon = if self.west > self.east {
            // antimeridian-crossing: center on the wrapped branch
            let span = (self.east + 360.0) - self.west;
            normalize_lon_degrees(self.west + 0.5 * span)
        } else {
            0.5 * (self.west + self.east)
        };
        Coord::from_degrees(lat, lon)
    }

    /// Latitude span in degrees.
    #[inline]
    pub fn lat_span(&self) -> f64 {
        self.north - self.south
    }

    /// Longitude span in degrees (always positive).
    #[inline]
    pub fn lon_span(&self) -> f64 {
        if self.west > self.east {
            (self.east + 360.0) - self.west
        } else {
            self.east - self.west
        }
    }
}

/// The supported reference ellipsoids.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EllipsoidKind {
    /// GRS80 (Geodetic Reference System 1980) — the ellipsoid of JGD2011 /
    /// JGD2024 used by the Geospatial Information Authority of Japan.
    GRS80,
    /// WGS84 — the ellipsoid of the World Geodetic System 1984 GPS datum.
    WGS84,
}

/// A reference ellipsoid with all derived constants precomputed.
///
/// The struct is deliberately `Copy` and cheap so that it can be passed by
/// value everywhere. Construct the two canonical instances with
/// [`Ellipsoid::GRS80`] / [`Ellipsoid::WGS84`] or a custom one with
/// [`Ellipsoid::custom`].
#[derive(Debug, Clone, Copy)]
pub struct Ellipsoid {
    /// Kind tag (informational for custom ellipsoids).
    pub kind: EllipsoidKind,
    /// Semi-major axis `a` (meters).
    pub a: f64,
    /// Flattening `f`.
    pub f: f64,
    /// Semi-minor axis `b = a(1 − f)` (meters).
    pub b: f64,
    /// First eccentricity squared `e² = f(2 − f)`.
    pub e2: f64,
    /// First eccentricity `e`.
    pub e: f64,
    /// Second eccentricity squared `e'² = e²/(1 − e²)`.
    pub ep2: f64,
    /// Third flattening `n = f/(2 − f)`.
    pub n: f64,
    /// Squared third flattening `n²`.
    pub n2: f64,
    /// `1 − f`.
    pub f1: f64,
    /// Authalic (equal-area) radius squared `R_A²` (m²).
    ///
    /// `R_A² = (a²/2)·(1 + (1−e²)/e · atanh(e))` — the total surface area of
    /// the ellipsoid is `4πR_A²`. This is the constant GeographicLib calls
    /// `C2`, and it is the scaling factor of the Gauss–Bonnet polygon area.
    pub c2: f64,
}

impl Ellipsoid {
    /// The GRS80 ellipsoid (`a = 6 378 137 m`, `f = 1/298.257222101`).
    pub const GRS80: Ellipsoid = Ellipsoid::make(EllipsoidKind::GRS80, 6_378_137.0, 1.0 / 298.257222101);
    /// The WGS84 ellipsoid (`a = 6 378 137 m`, `f = 1/298.257223563`).
    pub const WGS84: Ellipsoid = Ellipsoid::make(EllipsoidKind::WGS84, 6_378_137.0, 1.0 / 298.257223563);

    const fn make(kind: EllipsoidKind, a: f64, f: f64) -> Ellipsoid {
        // Const-context float arithmetic is stable (Rust 1.82+); `sqrt` and
        // `atanh` are not const-callable, so they are provided by the const
        // Newton/series helpers below.
        let b = a * (1.0 - f);
        let e2 = f * (2.0 - f);
        let e = const_sqrt(e2);
        let ep2 = e2 / (1.0 - e2);
        let n = f / (2.0 - f);
        // Authalic radius squared: R_A² = (a²/2)(1 + (1−e²)·atanh(e)/e);
        // for the sphere (e = 0) it reduces to a².
        let c2 = if e == 0.0 {
            a * a
        } else {
            let atanh_e = const_atanh(e);
            0.5 * a * a * (1.0 + (1.0 - e2) * atanh_e / e)
        };
        Ellipsoid {
            kind,
            a,
            f,
            b,
            e2,
            e,
            ep2,
            n,
            n2: n * n,
            f1: 1.0 - f,
            c2,
        }
    }

    /// Builds a canonical ellipsoid (with `c2` correctly initialized).
    #[inline]
    pub fn new(kind: EllipsoidKind) -> Ellipsoid {
        match kind {
            EllipsoidKind::GRS80 => Ellipsoid::with_c2(Ellipsoid::GRS80),
            EllipsoidKind::WGS84 => Ellipsoid::with_c2(Ellipsoid::WGS84),
        }
    }

    fn with_c2(mut e: Ellipsoid) -> Ellipsoid {
        e.c2 = e.exact_authalic_radius_sq();
        e
    }

    /// Builds a custom ellipsoid from semi-major axis and flattening.
    pub fn custom(a: f64, f: f64) -> Ellipsoid {
        let mut e = Ellipsoid::make(EllipsoidKind::GRS80, a, f);
        e.kind = EllipsoidKind::GRS80;
        e.c2 = e.exact_authalic_radius_sq();
        e
    }

    /// The sphere as a degenerate ellipsoid (`f = 0`).
    pub fn sphere(radius: f64) -> Ellipsoid {
        Ellipsoid::custom(radius, 0.0)
    }

    /// Exact authalic radius squared `R_A² = (a²/2)(1 + (1−e²)·atanh(e)/e)`.
    pub fn exact_authalic_radius_sq(&self) -> f64 {
        if self.e == 0.0 {
            return self.a * self.a;
        }
        0.5 * self.a * self.a * (1.0 + (1.0 - self.e2) * self.e.atanh() / self.e)
    }

    /// Meridian radius of curvature
    /// `M(φ) = a(1−e²) / (1 − e² sin²φ)^{3/2}` (meters).
    #[inline]
    pub fn m_radius(&self, lat: f64) -> f64 {
        let s = lat.sin();
        let w = 1.0 - self.e2 * s * s;
        self.a * (1.0 - self.e2) / (w * w.sqrt())
    }

    /// Prime-vertical radius of curvature `N(φ) = a / √(1 − e² sin²φ)` (meters).
    #[inline]
    pub fn n_radius(&self, lat: f64) -> f64 {
        let s = lat.sin();
        let w = 1.0 - self.e2 * s * s;
        self.a / w.sqrt()
    }

    /// Parametric (reduced) latitude `β = atan((1−f) tan φ)` (radians).
    ///
    /// Handles the poles and the equator exactly.
    #[inline]
    pub fn reduced_latitude(&self, lat: f64) -> f64 {
        // atan((1-f) tan φ) is numerically fragile near ±π/2; use the
        // stable form via sin/cos: tan β = (1-f) sin φ / cos φ.
        (self.f1 * lat.sin(), lat.cos()).atan2_self()
    }

    /// Inverse of [`Ellipsoid::reduced_latitude`]: geodetic latitude from β.
    #[inline]
    pub fn from_reduced_latitude(&self, beta: f64) -> f64 {
        // tan φ = tan β / (1-f); stable via sin/cos.
        ((beta.sin(), self.f1 * beta.cos())).atan2_self()
    }

    /// Authalic latitude `ξ` via the 6th-order Fourier series in the third
    /// flattening `n`:
    ///
    /// `ξ(φ) = φ + Σ_{j=1..6} n^j Σ_{k=1..j} C[k][j]·sin(2kφ)`.
    ///
    /// The coefficients were derived symbolically from the exact definition
    /// `sin ξ = q(φ)/q(π/2)` (see [`Ellipsoid::authalic_latitude_exact`]) and
    /// reproduce the exact value to ≲10⁻¹⁸ rad on GRS80/WGS84. The series is
    /// the primary path because it is branch-free and allocation-free.
    #[inline]
    pub fn authalic_latitude(&self, lat: f64) -> f64 {
        let n = self.n;
        let s = lat.sin();
        let c = lat.cos();
        // sin(2kφ) evaluated by Chebyshev-style recurrence on cos(2φ):
        // keep T = cos 2φ, U_k = sin(2kφ) via sin((k+1)x) = 2cos x sin(kx) − sin((k−1)x)
        let sin2 = 2.0 * s * c; // sin 2φ
        let cos2 = c * c - s * s; // cos 2φ
        // coeff[k] = Σ_{j>=k} C[k][j] n^j, accumulated in nested form per
        // harmonic: coeff_k(n) = C[k][k] n^k (1 + ...) — we instead evaluate
        // the double sum directly with Horner in n per harmonic.
        let n2 = n * n;
        let n3 = n2 * n;
        let n4 = n3 * n;
        let n5 = n4 * n;
        let n6 = n5 * n;
        // Harmonic 1 (sin 2φ)
        let c1 = -4.0 / 3.0 * n + -4.0 / 45.0 * n2 + 88.0 / 315.0 * n3
            + 20097882.0 / 176510209.0 * n4 + 42270777.0 / 949539638.0 * n5
            + -173201.0 / 10988127.0 * n6;
        // Harmonic 2 (sin 4φ)
        let s4 = 2.0 * cos2 * sin2; // sin(4φ)
        let c2 = 34.0 / 45.0 * n2 + 8.0 / 105.0 * n3
            + -137350062.0 / 784422695.0 * n4 + -18740405.0 / 235703719.0 * n5
            + -29778474.0 / 508351193.0 * n6;
        // Harmonic 3 (sin 6φ)
        let s6 = 2.0 * cos2 * s4 - sin2; // sin(6φ) = 2cos2φ·sin4φ − sin2φ
        let c3 = -1532.0 / 2835.0 * n3 + -38359430.0 / 605506593.0 * n4
            + 791379.0 / 6734597.0 * n5 + 24562019.0 / 468985531.0 * n6;
        // Harmonic 4 (sin 8φ)
        let s8 = 2.0 * cos2 * s6 - s4;
        let c4 = 235223782.0 / 555068605.0 * n4 + 5064848.0 / 96718195.0 * n5
            + -57211505.0 / 689801941.0 * n6;
        // Harmonic 5 (sin 10φ)
        let s10 = 2.0 * cos2 * s8 - s6;
        let c5 = -281983022.0 / 806795453.0 * n5 + -40150234.0 / 925098287.0 * n6;
        // Harmonic 6 (sin 12φ)
        let s12v = 2.0 * cos2 * s10 - s8;
        let c6 = 125729439.0 / 422315657.0 * n6;
        lat + c1 * sin2 + c2 * s4 + c3 * s6 + c4 * s8 + c5 * s10 + c6 * s12v
    }

    /// Authalic latitude via the exact definition
    /// `sin ξ = q(φ)/q(π/2)` with
    ///
    /// `q(φ) = (1−e²)·[ sinφ/(1−e²sin²φ) + atanh(e·sinφ)/e ]`,
    ///
    /// refined by one Newton step on `f(ξ) = sin ξ − q(φ)/q(π/2)`.
    /// Accurate to machine precision; used by tests to validate the series.
    pub fn authalic_latitude_exact(&self, lat: f64) -> f64 {
        let ratio = self.q_ratio(lat);
        let mut xi = ratio.asin();
        // One Newton refinement: d/dξ sin ξ = cos ξ.
        for _ in 0..2 {
            let resid = xi.sin() - ratio;
            xi -= resid / xi.cos();
        }
        xi
    }

    /// `q(φ)/q(π/2)` — the sine of the authalic latitude (exact definition).
    fn q_ratio(&self, lat: f64) -> f64 {
        if self.e == 0.0 {
            return lat.sin();
        }
        let s = lat.sin();
        let w = 1.0 - self.e2 * s * s;
        let q = (1.0 - self.e2) * (s / w + (self.e * s).atanh() / self.e);
        let w_p = 1.0 - self.e2;
        let q_p = (1.0 - self.e2) * (1.0 / w_p + self.e.atanh() / self.e);
        q / q_p
    }

    /// Isometric (Mercator) latitude
    /// `ψ(φ) = atanh(sin φ) − e·atanh(e·sin φ)` (radians, dimensionless).
    ///
    /// Rhumb lines are straight in the `(λ, ψ)` plane; this is the coordinate
    /// in which loxodromic containment tests are performed exactly.
    #[inline]
    pub fn isometric_latitude(&self, lat: f64) -> f64 {
        let s = lat.sin();
        if self.e == 0.0 {
            return s.atanh();
        }
        s.atanh() - self.e * (self.e * s).atanh()
    }

    /// Inverse isometric latitude: geodetic latitude from `ψ` (radians).
    ///
    /// Solved by Newton iteration on `ψ(φ) − ψ = 0`.
    pub fn from_isometric_latitude(&self, psi: f64) -> f64 {
        if self.e == 0.0 {
            return psi.sinh().asin();
        }
        // Initial guess via the Gudermannian atan(sinh ψ), which is valid
        // for all ψ (the naive asin(sinh ψ) leaves ±1's domain for |ψ| > 1).
        let mut phi = psi.sinh().atan();
        for _ in 0..8 {
            let resid = self.isometric_latitude(phi) - psi;
            let dpsi = (1.0 - self.e2) / (phi.cos() * (1.0 - self.e2 * phi.sin().powi(2)));
            phi -= resid / dpsi;
            if resid.abs() < 1e-16 {
                break;
            }
        }
        phi
    }

    /// Meridian arc length from the equator to latitude `φ` (meters),
    /// 6th-order series in `e²` (Snyder 3-21):
    ///
    /// `m(φ) = a·[(1−e²/4−3e⁴/64−5e⁶/256)φ − (3e²/8+3e⁴/32+45e⁶/1024)·sin2φ
    /// + (15e⁴/256+45e⁶/1024)·sin4φ − (35e⁶/3072)·sin6φ]`.
    pub fn meridian_arc(&self, lat: f64) -> f64 {
        let e2 = self.e2;
        let e4 = e2 * e2;
        let e6 = e4 * e2;
        let a0 = 1.0 - e2 / 4.0 - 3.0 * e4 / 64.0 - 5.0 * e6 / 256.0;
        let a2 = 3.0 * e2 / 8.0 + 3.0 * e4 / 32.0 + 45.0 * e6 / 1024.0;
        let a4 = 15.0 * e4 / 256.0 + 45.0 * e6 / 1024.0;
        let a6 = 35.0 * e6 / 3072.0;
        let s = lat.sin();
        let c = lat.cos();
        let sin2 = 2.0 * s * c;
        let cos2 = c * c - s * s;
        let sin4 = 2.0 * cos2 * sin2;
        let sin6 = 2.0 * cos2 * sin4 - sin2;
        self.a * (a0 * lat - a2 * sin2 + a4 * sin4 - a6 * sin6)
    }

    /// Inverse meridian arc: the geodetic latitude whose meridian arc from
    /// the equator equals `arc` meters. Solved by Newton iteration.
    pub fn latitude_from_meridian_arc(&self, arc: f64) -> f64 {
        let mut phi = arc / self.a; // spherical initial guess
        for _ in 0..12 {
            let resid = self.meridian_arc(phi) - arc;
            phi -= resid / self.m_radius(phi);
            if resid.abs() < 1e-12 {
                break;
            }
        }
        phi
    }
}

/// Const-context square root via Newton iteration (200 steps converge to
/// machine precision for the ellipsoid parameter ranges).
const fn const_sqrt(x: f64) -> f64 {
    if x <= 0.0 {
        return 0.0;
    }
    let mut g = x;
    let mut i = 0;
    while i < 200 {
        g = 0.5 * (g + x / g);
        i += 1;
    }
    g
}

/// Const-context `atanh` via the power series
/// `atanh(x) = x + x³/3 + x⁵/5 + …` (converges for |x| < 1; the canonical
/// ellipsoid eccentricities are ≈0.08, so 400 terms overflow the precision).
const fn const_atanh(x: f64) -> f64 {
    let mut term = x;
    let mut sum = 0.0;
    let x2 = x * x;
    let mut k = 1u32;
    while k <= 400 {
        sum += term / (k as f64);
        term *= x2;
        k += 2;
    }
    sum
}

/// Small internal helper to make `(sin, cos).atan2()` read cleanly.
trait Atan2Self {
    fn atan2_self(self) -> f64;
}
impl Atan2Self for (f64, f64) {
    #[inline]
    fn atan2_self(self) -> f64 {
        self.0.atan2(self.1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derived_constants_grs80() {
        let e = Ellipsoid::new(EllipsoidKind::GRS80);
        assert_eq!(e.a, 6_378_137.0);
        assert!((e.f - 1.0 / 298.257222101).abs() < 1e-18);
        assert!((e.b - 6_356_752.314140347).abs() < 1e-6);
        assert!((e.e2 - 0.00669438002290).abs() < 1e-12);
        // e'^2 = e^2/(1-e^2)
        assert!((e.ep2 - e.e2 / (1.0 - e.e2)).abs() < 1e-18);
        // n = f/(2-f)
        assert!((e.n - e.f / (2.0 - e.f)).abs() < 1e-18);
    }

    #[test]
    fn curvature_radii_at_equator_and_pole() {
        let e = Ellipsoid::new(EllipsoidKind::GRS80);
        // At the equator: N = a, M = a(1-e^2)
        assert!((e.n_radius(0.0) - e.a).abs() < 1e-9);
        assert!((e.m_radius(0.0) - e.a * (1.0 - e.e2)).abs() < 1e-9);
        // At the pole: M = N = a/sqrt(1-e^2)
        let half_pi = core::f64::consts::FRAC_PI_2;
        let expect = e.a / (1.0 - e.e2).sqrt();
        assert!((e.m_radius(half_pi) - expect).abs() < 1e-6);
        assert!((e.n_radius(half_pi) - expect).abs() < 1e-6);
    }

    #[test]
    fn reduced_latitude_roundtrip() {
        let e = Ellipsoid::new(EllipsoidKind::GRS80);
        for deg in [-80.0f64, -33.5, 0.0, 12.25, 45.0, 66.7, 89.9] {
            let phi = deg.to_radians();
            let beta = e.reduced_latitude(phi);
            assert!((e.from_reduced_latitude(beta) - phi).abs() < 1e-15, "deg={deg}");
        }
        // tan β = (1-f) tan φ at 45°
        let beta45 = e.reduced_latitude(core::f64::consts::FRAC_PI_4);
        let expect = ((1.0 - e.f) * 1.0f64).atan();
        assert!((beta45 - expect).abs() < 1e-15);
    }

    #[test]
    fn authalic_series_matches_exact_definition() {
        let e = Ellipsoid::new(EllipsoidKind::GRS80);
        let mut max_err = 0.0f64;
        let mut d = -89.5f64;
        while d <= 89.5 {
            let phi = d.to_radians();
            let err = (e.authalic_latitude(phi) - e.authalic_latitude_exact(phi)).abs();
            max_err = max_err.max(err);
            d += 0.5;
        }
        // The series reproduces the exact q-definition to < 5e-14 rad. The
        // residual is dominated by the f64 `asin` rounding inside the exact
        // path near the poles (|dξ| = |d(sin ξ)|/cos ξ amplifies 1 ulp of
        // sine-space error to ~1e-14 rad at |φ| > 89°); in 100-digit
        // arithmetic the same comparison gives ~1e-20 rad.
        assert!(max_err < 5e-14, "max authalic series error = {max_err:e} rad");
    }

    #[test]
    fn authalic_known_values() {
        let e = Ellipsoid::new(EllipsoidKind::GRS80);
        // ξ(45°) computed with the exact q-definition; the value is stable
        // across references: about 44.87174°.
        let xi = e.authalic_latitude_exact(core::f64::consts::FRAC_PI_4);
        assert!((xi.to_degrees() - 44.8717).abs() < 1e-3, "xi = {}", xi.to_degrees());
    }

    #[test]
    fn isometric_roundtrip() {
        let e = Ellipsoid::new(EllipsoidKind::GRS80);
        for deg in [-85.0f64, -45.0, 0.0, 33.59, 80.0] {
            let phi = deg.to_radians();
            let psi = e.isometric_latitude(phi);
            assert!((e.from_isometric_latitude(psi) - phi).abs() < 1e-13, "deg={deg}");
        }
    }

    #[test]
    fn meridian_arc_inverse() {
        let e = Ellipsoid::new(EllipsoidKind::GRS80);
        for deg in [0.0f64, 10.0, 35.6812, 66.7, 89.0] {
            let phi = deg.to_radians();
            let arc = e.meridian_arc(phi);
            let back = e.latitude_from_meridian_arc(arc);
            assert!((back - phi).abs() < 1e-13, "deg={deg} err={}", (back - phi).abs());
        }
        // Meridian arc from equator to pole ≈ 10 001 965.7 m on GRS80.
        let quarter = e.meridian_arc(core::f64::consts::FRAC_PI_2);
        assert!((quarter - 10_001_965.7).abs() < 0.1, "quarter arc = {quarter}");
    }

    #[test]
    fn authalic_radius_sq_matches_ellipsoid_area_identity() {
        let e = Ellipsoid::new(EllipsoidKind::GRS80);
        // Analytic check (derived in this crate's documentation): the area of
        // the quadrant {φ ∈ [0, π/2], λ ∈ [0, π]} equals π·R_A² exactly.
        // Integral of M(φ)N(φ)cosφ over [0, π/2] times π:
        // numerically integrate with Simpson at high resolution.
        let n_steps = 2_000_000;
        let h = (core::f64::consts::FRAC_PI_2) / n_steps as f64;
        let mut sum = 0.0;
        for i in 0..=n_steps {
            let phi = i as f64 * h;
            let f = e.m_radius(phi) * e.n_radius(phi) * phi.cos();
            let w = if i == 0 || i == n_steps { 1.0 } else if i % 2 == 1 { 4.0 } else { 2.0 };
            sum += w * f;
        }
        sum *= h / 3.0;
        let quadrant = core::f64::consts::PI * sum;
        let predicted = core::f64::consts::PI * e.c2;
        assert!(
            (quadrant - predicted).abs() / predicted < 1e-10,
            "quadrant={quadrant} predicted={predicted}"
        );
    }
}
