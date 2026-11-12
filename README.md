English | [日本語](README.ja.md)

# ellipgrid

**Exact GRS80/WGS84 ellipsoidal geodesics, authalic polygon manifolds, and topologically exact JIS X 0410 / Hilbert spatial covering in Rust.**

A production-grade, `#![forbid(unsafe_code)]`, dependency-minimal (zero dependencies) Rust crate providing:

* exact Direct & Inverse ellipsoidal geodesics (distance, forward/reverse azimuths, reduced length),
* ellipsoidal polygon area via Karney's `C4` Gauss–Bonnet series and an independent authalic-manifold Green engine,
* exact Shewchuk-style robust geometric predicates,
* the complete bit-packed JIS X 0410 (標準地域メッシュ) grid with exact integer micro-arcsecond arithmetic, `O(1)` topology queries, and Hilbert space-filling-curve keys,
* exact geodesic-disk and polygon-to-mesh covering engines.

```toml
[dependencies]
ellipgrid = "0.1"
```

---

## 1. Mathematical foundations

### 1.1 Reference ellipsoids (`ellipsoid`)

Both canonical ellipsoids share $a = 6\,378\,137.0$ m:

| Ellipsoid | Flattening $f$ | Users |
|---|---|---|
| GRS80 | $1/298.257222101$ | JGD2011 / JGD2024 (Japan) |
| WGS84 | $1/298.257223563$ | GPS |

Derived constants are computed exactly in const context:

$$b = a(1-f),\qquad e^2 = f(2-f),\qquad e'^2 = \frac{e^2}{1-e^2},\qquad n = \frac{f}{2-f}.$$

Curvature radii:

$$M(\varphi) = \frac{a(1-e^2)}{(1-e^2\sin^2\varphi)^{3/2}},\qquad
N(\varphi) = \frac{a}{\sqrt{1-e^2\sin^2\varphi}}.$$

Auxiliary latitudes:

* **Parametric** $\beta = \arctan\!\big((1-f)\tan\varphi\big)$ (exact, branch-stable via `atan2`),
* **Authalic** $\xi$ — two paths: the 6th-order Fourier series in $n$
  $\;\xi(\varphi) = \varphi + \sum_{j=1}^{6} n^j\sum_{k=1}^{j} C[k][j]\,\sin(2k\varphi)$
  (coefficients derived symbolically from the exact definition; agreement $\lesssim 10^{-18}$ rad in
  extended precision, dominated only by the `f64` `asin` rounding of the reference path), and the
  exact $q$-definition $\sin\xi = q(\varphi)/q(\tfrac{\pi}{2})$ with
  $q(\varphi) = (1-e^2)\left[\tfrac{\sin\varphi}{1-e^2\sin^2\varphi} + \tfrac{\mathrm{atanh}(e\sin\varphi)}{e}\right]$
  refined by Newton iteration,
* **Isometric** $\psi(\varphi) = \mathrm{atanh}(\sin\varphi) - e\,\mathrm{atanh}(e\sin\varphi)$
  (loxodromes are straight in the $(\lambda,\psi)$ plane),
* **Meridian arc** $m(\varphi)$ (Snyder 3-21, 6th order in $e^2$) with Newton inversion.

The authalic radius squared $R_A^2 = \tfrac{a^2}{2}\big(1 + \tfrac{1-e^2}{e}\mathrm{atanh} e\big)$
is the total-area scale: $4\pi R_A^2$ equals the ellipsoid surface area exactly (verified analytically
and numerically in the test suite).

### 1.2 Geodesics (`geodesic`)

**Inverse problem** — `inverse(p1, p2) -> {s12, azi1, azi2, m12}`:

1. **Helmert/Vincenty fixed-point iteration** on the auxiliary-sphere longitude (series to $O(f^4)$),
   converged to $|\Delta\lambda| < 10^{-14}$ rad (≤ 64 steps).
2. **Karney-series evaluation** (6th-order Fourier series in $\varepsilon$ where
   $\varepsilon = k^2/(2(1+\sqrt{1+k^2})+k^2)$, $k^2 = \cos^2\alpha_0\,e'^2$):

$$s_{12} = b\,A_1\big(\sigma_{12} + \Delta B_1\big),\qquad
m_{12} = b\Big[\mathrm{dn}_2(\cos\sigma_1\sin\sigma_2) - \mathrm{dn}_1(\sin\sigma_1\cos\sigma_2) - \cos\sigma_1\cos\sigma_2\,J_{12}\Big]$$

   with $J_{12} = (A_1-A_2)\sigma_{12} + A_1\Delta B_1 - A_2\Delta B_2$,
   $\mathrm{dn}_i = \sqrt{1+e'^2\sin^2\beta_i}$, and the node-relative arcs
   $\sin\sigma_i = \sin\beta_i/\cos\alpha_0$, $\cos\sigma_i = \cos\alpha_i\cos\beta_i/\cos\alpha_0$.
   This raises the accuracy from Vincenty's ≈0.5 mm to ≲10⁻⁸ m.

3. **Newton–Raphson antipodal fallback** — when $|\lambda_{12}| > \pi - 10^{-3}$ or Vincenty fails
   to converge: a damped 2-D Newton–Raphson on $(\alpha_1, s_{12})$ driven by the direct problem
   (numeric Jacobian, Levenberg backtracking) from eight initial guesses (the spherical solution,
   Vincenty's last iterate, and the four cardinal azimuths with meridional/equatorial half-perimeter
   arcs). Among converging solutions the **shortest arc** is selected — the shooting equations have
   additional longer solutions (full loops past exactly antipodal targets).

**Direct problem** — `direct(p1, azi1, s12)`: the auxiliary arc is recovered from the metric length
through the same 6th-order series fixed point $\sigma_{12} = \tfrac{s_{12}}{b A_1} - \Delta B_1(\sigma_{12})$,
then mapped through the auxiliary sphere with the Vincenty longitude correction at the arc midpoint.

**Equatorial special case** (both endpoints on the equator): $s_{12} = a|\lambda_{12}|$ exactly,
$m_{12} = b\sin(|\lambda_{12}|\,a/b)$.

#### Validation

| Baseline | Result |
|---|---|
| GeographicLib `GeodTest` gold set (20 WGS84 cases incl. near-antipodal) | $s_{12}$, $m_{12}$: $<10^{-4}$ m; azimuths $<10^{-8}$° |
| **GSI (国土地理院) official calculator**, GRS80, live-verified | 4 cases (Tokyo–Osaka, –Sapporo, –Naha, –Delhi): distances **±0.5 mm**, forward azimuths **±0.005″** |
| Exactly antipodal pairs (GeographicLib-verified) | shortest-arc selection correct, round-trip closure $<10^{-12}$ rad |

> **Convention.** `azi2` is the *forward* azimuth at the arrival point (direction of travel — the
> international/GeographicLib convention). The GSI service displays the *reciprocal* azimuth
> (arrival → departure); the two differ by exactly 180°.

### 1.3 Robust predicates (`predicates`)

Error-free transformations: `two_sum` (Knuth), `fast_two_sum`, Dekker `split`, `two_product`
(FMA-based, with `p + e == a·b` exactly). Expansion arithmetic (ascending non-overlapping
components; `grow_expansion`, `scale_expansion`, exact sums) drives the adaptive predicates:

* `orient2d` / `orient2d_exact` — Shewchuk filter bound $(3+16\varepsilon)\varepsilon\cdot\text{perm}$,
* `incircle` / `incircle_exact` — filter bound $(10+96\varepsilon)\varepsilon^2\cdot\text{perm}$.

Adversarial tests include the translated-nearly-degenerate configurations where the naive `f64`
determinant is exactly 0 while the true determinant is non-zero (zero false positives/negatives).

### 1.4 Polygon area, perimeter, containment (`polygon`)

Closed rings with **geodesic** or **loxodromic (rhumb)** edges.

**Area — Karney `C4` Gauss–Bonnet engine** (geodesic edges): the per-edge area swathe

$$S_{12} = A_4\big(B_4(\sigma_2) - B_4(\sigma_1)\big) + R_A^2\,(\alpha_2 - \alpha_1),\qquad
A_4 = a^2\cos\alpha_0\sin\alpha_0\,e^2,$$

with the 6th-order `C4` Fourier cosine series (coefficients per GeographicLib, polynomials in $n$
evaluated at $\varepsilon$). Summing $S_{12}$ over a closed ring yields the enclosed area
(clockwise-positive raw convention; negated for this crate's CCW-positive API). Verified against
GeographicLib v2.1 to **1e-10 relative** on mid-latitude polygons. Rings touching a pole are
degenerate in the plain sum (returning 0, exactly as GeographicLib) and are routed automatically to
the Green engine.

**Area — authalic-manifold Green engine** (any edge kind): the exact area element in authalic
coordinates is $dA = R_A^2\cos\xi\,d\xi\,d\lambda$, so

$$A = -R_A^2\oint \sin\xi(\varphi)\,d\lambda$$

evaluated by 32-node Gauss–Legendre quadrature per edge (rhumb edges parametrized exactly in the
isometric latitude; geodesic edges with the analytic integrand
$d\lambda/dt = s_{12}\sin\alpha(t)/(N(\varphi)\cos\varphi)$, $\sin\alpha(t) = \sin\alpha_0/\cos\beta(t)$
from Clairaut). Pole vertices contribute the exact longitude-jump term
$\sin\xi(\pm\tfrac{\pi}{2})\,\Delta\lambda$, reproducing the analytic quadrant value
$A = \pi R_A^2$ to 1e-9 relative.

**Perimeter** — geodesic edges via the inverse problem; rhumb edges exactly as
$\Delta m/\cos\alpha$ with $\tan\alpha = \Delta\lambda/\Delta\psi$ (parallels: $N\cos\varphi\,\Delta\lambda$).

**Containment** — ray casting toward +λ with longitudes unwrapped relative to the query point
(antimeridian-safe): rhumb edges are tested exactly in the isometric plane with
`orient2d_exact` (boundary-inclusive, zero topological false positives/negatives); geodesic
edges by recursive bisection of the true arc (robust to ~1e-12 rad) plus an exact geodesic
triangle-inequality boundary test.

### 1.5 JIS X 0410 mesh codes (`jismesh`)

| Level | Name | Size (lat × lon) | Digits | Subdivision |
|---|---|---|---|---|
| 1 | 1次メッシュ (~80 km) | 40′ × 1° | 4 | `p1` = ⌊lat·1.5⌋, `u1` = ⌊lon⌋ − 100 |
| 2 | 2次メッシュ (~10 km) | 5′ × 7.5′ | 6 | `q1 q2` ∈ 0..8 (S→N, W→E) |
| 3 | 3次メッシュ (~1 km) | 30″ × 45″ | 8 | `r1 r2` ∈ 0..10 (S→N, W→E) |
| 4 | 4次メッシュ (~500 m) | 15″ × 22.5″ | 9 | `s` ∈ 1..5 (SW=1, SE=2, NW=3, NE=4) |
| 5 | 5次メッシュ (~250 m) | 7.5″ × 11.25″ | 10 | `t` ∈ 1..5 |
| 6 | 6次メッシュ (~125 m) | 3.75″ × 5.625″ | 11 | `u` ∈ 1..5 |

Per JIS X 0410:2002, 2nd/3rd-mesh digits are 0-based and 4th–6th quarter digits are 1-based;
cells are half-open `[south, north) × [west, east)`.

`JisMeshCode(u64)` is `#[repr(transparent)]`, 37 bits packed, with all conversions `O(1)` and
allocation-free. `from_lat_lon` rounds to the nearest **micro-arcsecond** and performs exact
integer floor divisions — eliminating `f64` boundary rounding (a point at exactly
$35°40′00.000000″$ lands in the northern cell, provably, at every level).

Topology: `parent`, `children` (64/100/4), `neighbors_8` — computed in a flat micro-arcsecond index
space so the Moore neighborhood is **exact across 1st/2nd/3rd-mesh digit carries**; `to_hilbert64`
maps each cell to a 32-bit Hilbert-curve key over the level-6 base grid for cache-local indexing.

### 1.6 Covering engines (`cover`)

**`cover_geodesic_disk(center, radius, level)`** — the exact minimal set of cells whose closed
bounding boxes intersect the true geodesic disk:

1. exact latitudinal extent $[\varphi_s, \varphi_n]$ by meridian-arc inversion (handles $M(\varphi)$),
2. per-row longitudinal half-width $\Delta\lambda(\varphi)$ by bisection of the initial azimuth of
   the direct problem (handles meridian convergence and the $\cos\varphi$ east-west shrinking),
3. enumeration in exact integer micro-arcsecond index space; sorted, de-duplicated output.

**`cover_polygon(poly, level)`** — bounding-box row/column sweep with edge-bbox fast rejection,
exact interior-cell detection (a connected cell rectangle missing all boundary segments lies
entirely inside or outside — settled by one containment query), corner-containment tests, and
exact rhumb segment-vs-rectangle intersection via expansion-arithmetic orientation predicates
(geodesic edges conservatively sampled into short chords).

---

## 2. Complexity

| Operation | Complexity | Notes |
|---|---|---|
| `from_lat_lon` / `to_bounds` / `to_center` | $O(1)$ | integer micro-arcsecond math, no allocation |
| `parent` / `children` / `to_hilbert64` | $O(1)$ | bitwise |
| `neighbors_8` | $O(1)$ | 8 exact index-space constructions |
| inverse (Vincenty path) | $\le 64$ fixed-point steps × $O(1)$ | ≈ 20 flops/step |
| inverse (antipodal fallback) | $\le 8$ starts × 60 Newton steps × direct | only near-antipodal |
| direct | ≤ 24 fixed-point steps | series evaluation |
| polygon area (C4) | $O(n)$ edges × series | closed form per edge |
| polygon area (Green) | $O(n)$ edges × 32-node GL | machine-precision quadrature |
| `contains` (rhumb) | $O(n)$ exact predicate evaluations | no allocation |
| disk cover | rows × bisection(80 direct) + cells | exact per-row extents |
| polygon cover | bbox cells × edge tests | bbox fast-reject first |

---

## 3. Usage

```rust
use ellipgrid::{Ellipsoid, Coord};
use ellipgrid::geodesic::{inverse, direct};

let e = Ellipsoid::GRS80;
let tokyo  = Coord::from_degrees(35.6812, 139.7671);
let osaka  = Coord::from_degrees(34.7024, 135.4959);

let inv = inverse(&e, tokyo, osaka);
println!("distance = {} m", inv.s12);          // ±0.5 mm vs GSI
println!("azi1 = {}°, azi2 = {}°", inv.azi1.to_degrees(), inv.azi2.to_degrees());
println!("reduced length = {} m", inv.m12);

let d = direct(&e, tokyo, inv.azi1, 100_000.0);
```

```rust
use ellipgrid::jismesh::{JisMeshCode, MeshLevel};

let code = JisMeshCode::from_lat_lon(35.658581, 139.745433, MeshLevel::Level3)?;
assert_eq!(code.to_string(), "53393599");      // Tokyo Tower, 3rd mesh

let neighbors = code.neighbors_8();            // exact across mesh boundaries
let key = code.to_hilbert64();                 // cache-local spatial index
```

```rust
use ellipgrid::polygon::{EllipsoidalPolygon, EdgeKind};

let japan_poly = EllipsoidalPolygon::new(vec![/* ring */], EdgeKind::Rhumb)?;
let area = japan_poly.area(&e);                // signed m², CCW-positive
assert!(japan_poly.contains(&e, Coord::from_degrees(36.0, 138.0)));
```

```rust
use ellipgrid::cover::cover_geodesic_disk;

let cells = cover_geodesic_disk(&e, tokyo, 5000.0, MeshLevel::Level6)?;
// exact minimal set of ~125 m cells intersecting the 5 km geodesic disk
```

---

## 4. Verification summary

| Property | Verified against | Agreement |
|---|---|---|
| Inverse distance (WGS84) | GeographicLib GeodTest (20 cases) | < 0.1 mm |
| Inverse distance (GRS80) | **GSI official calculator** (4 cases) | **±0.5 mm** |
| Azimuths | GSI + GeodTest | < 0.005″ / < 10⁻⁸° |
| Reduced length `m12` | GeodTest | < 0.1 mm |
| Antipodal shortest arcs | GeographicLib | < 0.2 mm |
| Edge area swathe `S12` | GeographicLib C4 | 1e-11 relative |
| Polygon area (C4 vs Green) | cross-engine | 1e-8 relative |
| Quadrant triangle area | analytic $\pi R_A^2$ | 1e-9 relative |
| Authalic latitude | exact $q$-definition | < 5e-14 rad |
| JIS codes | jismesh reference library | exact |
| JIS boundary semantics | JIS X 0410:2002 half-open rule | exact (integer µas) |
| Disk covering | brute-force sampled reference | exact set equality |
| Predicates | Shewchuk-adversarial configurations | zero false signs |

69 tests + 1 doc-test, `#![forbid(unsafe_code)]`, zero dependencies.

## 5. License

MIT — see [LICENSE-MIT](LICENSE-MIT).
