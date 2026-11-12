//! # ellipgrid
//!
//! **Exact GRS80/WGS84 ellipsoidal geodesics, authalic polygon manifolds, and
//! topologically exact JIS X 0410 / Hilbert spatial covering in Rust.**
//!
//! A production-grade, `#![forbid(unsafe_code)]`, dependency-minimal crate:
//!
//! * [`ellipsoid`] — GRS80/WGS84 constants, curvature radii `M(φ)`, `N(φ)`,
//!   auxiliary latitudes (parametric, authalic via a 6th-order series in `n`
//!   plus the exact `q`-definition, isometric), meridian arc and inverse.
//! * [`geodesic`] — exact Direct & Inverse ellipsoidal geodesics: a
//!   Helmert/Vincenty fixed-point iteration refined by 6th-order Karney
//!   series evaluation, with a guaranteed Newton–Raphson fallback for the
//!   antipodal and near-antipodal regime. Returns distance, forward/reverse
//!   azimuths, and the reduced length `m12` to sub-millimeter precision for
//!   *any* pair of coordinates on Earth.
//! * [`predicates`] — Dekker/Shewchuk error-free floating-point
//!   transformations (`two_sum`, `fast_two_sum`, `split`, `two_product`),
//!   expansion arithmetic, and the adaptive exact predicates `orient2d` and
//!   `incircle` with zero topological false positives/negatives.
//! * [`polygon`] — ellipsoidal polygon area via Karney's Gauss–Bonnet
//!   theorem with `C4` series coefficients (geodesic edges) and exact
//!   Green's/ authalic-manifold integration (rhumb edges); perimeter;
//!   exact point-in-polygon for both loxodromic and geodesic edges.
//! * [`jismesh`] — the complete JIS X 0410 (標準地域メッシュ) hierarchical
//!   grid, levels 1–6, packed into a zero-allocation
//!   `#[repr(transparent)]` [`JisMeshCode`](jismesh::JisMeshCode) using exact
//!   integer micro-arcsecond arithmetic; `O(1)` parent/children/exact Moore
//!   neighborhoods; cache-local Hilbert keys.
//! * [`cover`] — exact geodesic-disk and polygon-to-mesh covering engines.
//!
//! ## Conventions
//!
//! * Angles are radians unless a name ends in `_degrees`.
//! * Longitudes: east positive; accepted range for the JIS grid is
//!   `[100°E, 200°E)` (wrapped), latitudes `[0°, 66.66…°)`.
//! * Azimuths: clockwise from true north, returned in `(−π, π]`.
//! * The crate computes on GRS80 by default (Japan's official ellipsoid);
//!   pass [`Ellipsoid::WGS84`](ellipsoid::Ellipsoid::WGS84) for GPS work.
//!
//! ## Example
//!
//! ```
//! use ellipgrid::{Ellipsoid, Coord};
//!
//! let e = Ellipsoid::GRS80;
//! let tokyo = Coord::from_degrees(35.6812, 139.7671);
//! let osaka = Coord::from_degrees(34.7024, 135.4959);
//! let inv = ellipgrid::geodesic::inverse(&e, tokyo, osaka);
//! assert!((inv.s12 - 403_830.7).abs() < 1.0); // ≈ 403.8 km (GRS80)
//!
//! use ellipgrid::jismesh::{JisMeshCode, MeshLevel};
//! let code = JisMeshCode::from_lat_lon(35.658581, 139.745433, MeshLevel::Level3).unwrap();
//! assert_eq!(code.to_string(), "53393599"); // Tokyo Tower 3rd mesh
//! ```

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod ellipsoid;
pub mod geodesic;
pub mod jismesh;
pub mod predicates;
pub mod cover;
pub mod polygon;

pub use ellipsoid::{BoundingBox, Coord, Ellipsoid, EllipsoidKind};
pub use geodesic::{GeodesicDirectResult, GeodesicInverseResult};
pub use jismesh::{HilbertKey, JisMeshCode, MeshError, MeshLevel};
pub use polygon::{EdgeKind, EllipsoidalPolygon, PolygonError, PolygonMeasure};
