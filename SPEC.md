# ROLE & OBJECTIVE

You are a Principal Computational Geometer, Geodesist, and Systems Rust Architect.
Build a complete, production-grade, zero-unsafe, dependency-minimal Rust crate named **`ellipgrid`** (`ellipgrid-rs`):
**"Exact GRS80/WGS84 Ellipsoidal Geodesics, Authalic Polygon Manifolds, and Topologically Exact JIS X 0410 / Hilbert Spatial Covering in Rust."**

Every module MUST contain real, mathematically complete algorithms (NO placeholders, NO `todo!()`, NO `unimplemented!()`, NO simplified Haversine approximations where ellipsoidal equations are specified). All unit tests and property invariants must compile and pass via `cargo test`.

---

# 1. MATHEMATICAL & ARCHITECTURAL SPECIFICATIONS

## Module 1: `src/ellipsoid.rs` — Reference Ellipsoids & Curvature Tensor

Implement exact geometric parameters for reference ellipsoids (`GRS80` used by Japan's JGD2011/JGD2024, and `WGS84`):
- Semi-major axis $a$ (`6_378_137.0` m), flattening $f$ (`1.0 / 298.257222101` for GRS80, `1.0 / 298.257223563` for WGS84).
- Derived exact constants: semi-minor axis $b = a(1 - f)$, first eccentricity squared $e^2 = f(2 - f)$, second eccentricity squared $e'^2 = e^2 / (1 - e^2)$, third flattening $n = f / (2 - f)$.
- Meridian radius of curvature $M(\phi) = \frac{a(1 - e^2)}{(1 - e^2 \sin^2\phi)^{3/2}}$ and prime vertical radius of curvature $N(\phi) = \frac{a}{\sqrt{1 - e^2 \sin^2\phi}}$.
- Conversion between geodetic latitude $\phi$, parametric/reduced latitude $\beta = \arctan((1 - f)\tan\phi)$, and authalic latitude $\xi$ using 6th-order series expansion in $n$.

## Module 2: `src/geodesic.rs` — Exact Ellipsoidal Direct & Inverse Geodesics (Vincenty + Karney Newton-Raphson Fallback)

Solve the **Direct** and **Inverse** geodesic problems on the oblate ellipsoid to sub-millimeter precision:
1. **Inverse Problem (`inverse(p1, p2) -> GeodesicInverseResult`):**
   - Compute geodesic distance $s_{12}$ (meters), forward azimuth $\alpha_1 \in [-\pi, \pi]$, reverse azimuth $\alpha_2 \in [-\pi, \pi]$, and reduced length $m_{12}$.
   - Use **Helmert/Vincenty iterative evaluation** with $n$-series expansion up to $O(f^4)$ AND a guaranteed **Karney/Newton-Raphson antipodal solver** when $|\lambda_{12}| > \pi - 10^{-3}$ or when standard iteration fails to converge within 64 steps, guaranteeing convergence for ANY pair of coordinates on Earth.
2. **Direct Problem (`direct(p1, azimuth, distance_m) -> GeodesicDirectResult`):**
   - Given $(\phi_1, \lambda_1)$, initial azimuth $\alpha_1$, and distance $s_{12}$, compute exact destination $(\phi_2, \lambda_2)$ and final azimuth $\alpha_2$.

## Module 3: `src/predicates.rs` — Shewchuk-Style Adaptive Error-Free Floating-Point Arithmetic

Standard IEEE-754 `f64` arithmetic suffers from catastrophic cancellation in colinearity and boundary-intersection tests:
- Implement **Error-Free Transformations (EFT)**: `two_sum(a, b) -> (f64, f64)`, `fast_two_sum(a, b)`, `split(a)`, and `two_product(a, b) -> (f64, f64)` using Dekker/FMA expansion.
- Implement `orient2d_exact(pa, pb, pc) -> Ordering` and `incircle_exact` using adaptive expansion arithmetic so boundary tests on polygon edges and grid meridians have **zero topological false positives/negatives**.

## Module 4: `src/polygon.rs` — Ellipsoidal Polygon Area (Gauss-Bonnet) & Winding Containment

- Compute the exact surface area $S$ (in $\text{m}^2$) and perimeter $P$ (in $\text{m}$) of an arbitrary closed geodesic polygon on the GRS80 ellipsoid using **Karney's spherical excess + authalic/ellipsoidal area coefficients $C_{4}$** (Gauss-Bonnet theorem over geodesic segments: $\sum \Delta \Omega_i$).
- Handle polygons crossing the antimeridian ($\pm 180^\circ$) and poles properly.
- Implement exact point-in-polygon (`contains(&self, pt: Coord)`) supporting both **Loxodromic (Rhumb-line)** edges (used by administrative/JIS grid cells) and **Geodesic** edges.

## Module 5: `src/jismesh.rs` — Bit-Packed JIS X 0410 Hierarchical Grid & Hilbert/Morton Curve

Implement the complete Japanese Standard Regional Grid (**JIS X 0410 / 標準地域メッシュ**) packed into a zero-allocation `#[repr(transparent)] pub struct JisMeshCode(u64)` and `HilbertKey(u64)`:
- Support all standard levels:
  - **Level 1 (`1次メッシュ`, ~80km)**: 4 digits (`p1p2 u1u2`).
  - **Level 2 (`2次メッシュ`, ~10km)**: 6 digits (`+ q1q2` in $8 \times 8$).
  - **Level 3 (`3次メッシュ` / Basic Grid, ~1km)**: 8 digits (`+ r1r2` in $10 \times 10$).
  - **Level 4 (`4次メッシュ` / Half Grid, ~500m)**: 9 digits (`+ s` in $\{1..4\}$).
  - **Level 5 (`5次メッシュ` / Quarter Grid, ~250m)**: 10 digits (`+ t` in $\{1..4\}$).
  - **Level 6 (`6次メッシュ` / Eighth Grid, ~125m)**: 11 digits (`+ u` in $\{1..4\}$).
- Provide $O(1)$ bitwise conversion without heap allocation (`no String formatting` in hot paths):
  - `from_lat_lon(lat, lon, level) -> Result<JisMeshCode, MeshError>` using exact rational integer micro-second arithmetic (`lat_micro_arcsec`) to eliminate `f64` boundary rounding bugs (e.g., at $35^\circ 40' 00''$).
  - `to_bounds(self) -> BoundingBox` and `to_center(self) -> Coord`.
  - `parent(self)`, `children(self)`, `neighbors_8(self)` (exact topological Moore neighborhood across 1st/2nd/3rd mesh boundaries!), and `to_hilbert64(self) -> u64` for cache-local spatial indexing.

## Module 6: `src/cover.rs` — Exact Geodesic Disk & Polygon-to-Mesh Covering Engine

- **`cover_geodesic_disk(center: Coord, radius_m: f64, level: MeshLevel) -> Vec<JisMeshCode>`**:
  Given a point $(\phi, \lambda)$ and a true ellipsoidal radius $R$ meters on GRS80, compute the exact minimal set of `JisMeshCode` cells at `level` whose loxodromic bounding box intersects the true ellipsoidal geodesic disk $D_{GRS80}(center, R)$.
  *(Note: Properly account for meridian convergence $\gamma(\phi)$ where east-west metric width shrinks as $\cos\beta$ while north-south metric height varies with $M(\phi)$.)*
- **`cover_polygon(poly: &EllipsoidalPolygon, level: MeshLevel) -> Vec<JisMeshCode>`**:
  Scanline + quadtree pruning algorithm using `orient2d_exact` to emit sorted, deduplicated mesh codes covering any polygon.

---

# 2. REPOSITORY STRUCTURE TO GENERATE

```text
ellipgrid/
├── Cargo.toml
├── README.md                 # Academic/Research-grade documentation with LaTeX math, complexity tables, and usage examples
├── LICENSE-MIT
├── src/
│   ├── lib.rs                # Public API exports and crate-level rustdoc
│   ├── ellipsoid.rs          # GRS80 / WGS84 constants, curvature radii M(φ), N(φ), auxiliary latitudes
│   ├── geodesic.rs           # Direct & Inverse ellipsoidal solvers (Vincenty + Karney Newton-Raphson antipodal)
│   ├── predicates.rs         # Dekker/Shewchuk error-free floating point transformations & exact orient2d
│   ├── polygon.rs            # Ellipsoidal Gauss-Bonnet area, perimeter, and exact containment
│   ├── jismesh.rs            # Zero-alloc bit-packed JIS X 0410 (Levels 1-6), exact integer micro-arcsecond math, 8-neighbors, Hilbert key
│   └── cover.rs              # Geodesic disk & polygon topological mesh covering
└── tests/
    └── verification_suite.rs # Geodesic GSI (国土地理院) benchmark baselines, antipodal convergence tests, JIS X 0410 boundary invariants
```
