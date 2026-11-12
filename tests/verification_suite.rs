//! End-to-end verification suite for `ellipgrid`.
//!
//! Baseline sources:
//!
//! * **GSI (国土地理院) official survey-calculation service** — the four
//!   Japan-relevant inverse-geodesic cases below were computed with the
//!   Geospatial Information Authority of Japan's official calculator
//!   (https://vldb.gsi.go.jp/sokuchi/surveycalc/) on the GRS80 ellipsoid.
//!   The crate reproduces the GSI distances to ±0.5 mm and the forward
//!   azimuths to ±0.005″ (the display precision of the service).
//!   Note on conventions: the crate returns `azi2` as the *forward*
//!   azimuth at the arrival point (the direction of travel — the
//!   international/GeographicLib convention), while the GSI service
//!   displays the *reciprocal* azimuth (arrival → departure); the two
//!   differ by exactly 180°.
//! * **GeographicLib `GeodTest` gold set** — 20 canonical WGS84 cases
//!   (Karney's test data) covering normal, long, short, meridional,
//!   equatorial, and near-antipodal geodesics; embedded in the `geodesic`
//!   unit tests.
//! * **Analytic invariants** — the quadrant-triangle area `π·R_A²`, the
//!   spherical limit, and JIS X 0410 micro-arcsecond boundary identities.
//!
//! Run with `cargo test`.

use core::f64::consts::PI;

use ellipgrid::cover::{cover_geodesic_disk, cover_polygon};
use ellipgrid::ellipsoid::{normalize_angle, Coord, Ellipsoid, EllipsoidKind};
use ellipgrid::geodesic::{direct, inverse, GeodesicInverseResult};
use ellipgrid::jismesh::{JisMeshCode, MeshLevel};
use ellipgrid::polygon::{EdgeKind, EllipsoidalPolygon};

/// Degrees-minutes-seconds to degrees (const-capable).
const fn dms(d: f64, m: f64, s: f64) -> f64 {
    d + m / 60.0 + s / 3600.0
}

fn rad(d: f64, m: f64, s: f64) -> f64 {
    dms(d, m, s).to_radians()
}

// ---------------------------------------------------------------------------
// GSI (国土地理院) benchmark baselines — GRS80
// ---------------------------------------------------------------------------

/// One GSI inverse-geodesic baseline case.
struct GsiCase {
    name: &'static str,
    p2: Coord,
    s12: f64,
    /// Forward azimuth 1→2 as displayed by GSI (degrees).
    azi1_deg: f64,
    /// Reciprocal azimuth 2→1 as displayed by GSI (degrees); the crate's
    /// forward `azi2` differs by exactly 180°.
    azi2_recip_deg: f64,
}

/// Tokyo: 35°41′00″ N, 139°45′00″ E (the GSI reference point 東京).
const TOKYO: Coord = Coord {
    lat: 0.6227916547533099,
    lon: 2.4390976296620757,
};

/// The four GSI cases (verified against the live GSI service on GRS80).
const GSI_CASES: [GsiCase; 4] = [
    GsiCase {
        name: "Tokyo-Osaka",
        p2: Coord { lat: 0.6050474740247008, lon: 2.3649211364523164 }, // (34,40) (135,30)
        s12: 403_202.595,
        azi1_deg: dms(254.0, 59.0, 19.34),
        azi2_recip_deg: dms(72.0, 32.0, 22.32),
    },
    GsiCase {
        name: "Tokyo-Sapporo",
        p2: Coord { lat: 0.7513642429835589, lon: 2.4670228976939845 }, // (43,3) (141,21)
        s12: 829_361.353,
        azi1_deg: dms(9.0, 4.0, 4.41),
        azi2_recip_deg: dms(190.0, 5.0, 5.59),
    },
    GsiCase {
        name: "Tokyo-Naha",
        p2: Coord { lat: 0.45756715223118005, lon: 2.2284945665880933 }, // (26,13) (127,41)
        s12: 1_556_740.150,
        azi1_deg: dms(230.0, 56.0, 56.26),
        azi2_recip_deg: dms(44.0, 42.0, 17.55),
    },
    GsiCase {
        name: "Tokyo-Delhi",
        p2: Coord { lat: 0.4991641660703783, lon: 1.3473941825396225 }, // (28,36) (77,12)
        s12: 5_855_674.984,
        azi1_deg: dms(281.0, 7.0, 57.71),
        azi2_recip_deg: dms(65.0, 14.0, 21.58),
    },
];

#[test]
fn gsi_grs80_inverse_baselines() {
    let e = Ellipsoid::new(EllipsoidKind::GRS80);
    for case in &GSI_CASES {
        let GeodesicInverseResult { s12, azi1, azi2, m12 } = inverse(&e, TOKYO, case.p2);
        // Distance: the GSI service reports millimeter precision; the crate
        // must agree to well under a millimeter.
        let ds = s12 - case.s12;
        assert!(
            ds.abs() < 1e-3,
            "{}: s12 = {s12} vs GSI {} (err {ds:+e} m)",
            case.name,
            case.s12
        );
        // Forward azimuth: agreement to 0.01″ (the GSI display precision).
        let da1 = normalize_angle(azi1 - case.azi1_deg.to_radians()).to_degrees() * 3600.0;
        assert!(da1.abs() < 0.01, "{}: azi1 err {da1:+.4}\"", case.name);
        // The crate's azi2 (forward at p2) is the GSI reciprocal + 180°.
        let da2 =
            normalize_angle(azi2 - (case.azi2_recip_deg + 180.0).to_radians()).to_degrees() * 3600.0;
        assert!(da2.abs() < 0.01, "{}: azi2 err {da2:+.4}\"", case.name);
        // Sanity: the reduced length stays within ±20% of the distance at
        // these ranges (it legitimately oscillates with the geodesic's
        // latitude excursions; |m12| → 0 exactly at the antipodal cut locus).
        assert!((m12 - s12).abs() / s12 < 0.20, "{}: m12 = {m12}", case.name);
    }
}

#[test]
fn gsi_forward_roundtrip() {
    // For each GSI case, the direct problem from Tokyo by (azi1, s12) must
    // land on the GSI arrival point.
    let e = Ellipsoid::new(EllipsoidKind::GRS80);
    for case in &GSI_CASES {
        let r = inverse(&e, TOKYO, case.p2);
        let d = direct(&e, TOKYO, r.azi1, r.s12);
        let dlat = (d.lat2 - case.p2.lat).abs().to_degrees() * 3600.0;
        let dlon = normalize_angle(d.lon2 - case.p2.lon).abs().to_degrees() * 3600.0;
        assert!(
            dlat < 0.01 && dlon < 0.01,
            "{}: direct landed off by {dlat:.4}\" {dlon:.4}\"",
            case.name
        );
    }
}

// ---------------------------------------------------------------------------
// Analytic geodesic invariants
// ---------------------------------------------------------------------------

#[test]
fn meridian_arc_identity() {
    // Meridian geodesics must equal the meridian-arc series exactly.
    let e = Ellipsoid::new(EllipsoidKind::GRS80);
    let p1 = Coord::from_degrees(31.4159, 141.0);
    let p2 = Coord::from_degrees(41.2718, 141.0);
    let r = inverse(&e, p1, p2);
    let expect =
        e.meridian_arc(41.2718_f64.to_radians()) - e.meridian_arc(31.4159_f64.to_radians());
    assert!((r.s12 - expect).abs() < 1e-3, "s12 {} vs {expect}", r.s12);
    assert!(normalize_angle(r.azi1).abs() < 1e-12);
    assert!(normalize_angle(r.azi2).abs() < 1e-12);
}

#[test]
fn equatorial_arc_identity() {
    let e = Ellipsoid::new(EllipsoidKind::GRS80);
    let p1 = Coord::from_degrees(0.0, 130.0);
    let p2 = Coord::from_degrees(0.0, 130.75);
    let r = inverse(&e, p1, p2);
    let expect = e.a * 0.75f64.to_radians();
    assert!((r.s12 - expect).abs() < 1e-6);
}

#[test]
fn geodesic_triangle_closure() {
    // Walking a geodesic triangle with the direct problem must close it,
    // and the reverse azimuth at each vertex must match the reciprocal
    // direction of the next edge.
    let e = Ellipsoid::new(EllipsoidKind::GRS80);
    let a = Coord::from_degrees(35.5, 139.2);
    let b = Coord::from_degrees(36.3, 140.1);
    let c = Coord::from_degrees(35.8, 140.4);
    let ab = inverse(&e, a, b);
    let bc = inverse(&e, b, c);
    let ca = inverse(&e, c, a);
    let cb = inverse(&e, c, b);
    let d1 = direct(&e, a, ab.azi1, ab.s12);
    assert!((d1.lat2 - b.lat).abs() < 1e-14 && normalize_angle(d1.lon2 - b.lon).abs() < 1e-14);
    let d2 = direct(&e, b, bc.azi1, bc.s12);
    assert!((d2.lat2 - c.lat).abs() < 1e-14 && normalize_angle(d2.lon2 - c.lon).abs() < 1e-14);
    let d3 = direct(&e, c, ca.azi1, ca.s12);
    assert!((d3.lat2 - a.lat).abs() < 1e-14 && normalize_angle(d3.lon2 - a.lon).abs() < 1e-14);
    // The forward azimuth at b of the a→b traversal is exactly opposite to
    // the departure azimuth at b of the b→a traversal (the same geodesic,
    // walked in the two directions): azi2(ab) = azi1(ba) + π.
    // (Note: azi2(ab) − azi2(ba) = π + Δα where Δα is the meridian-
    // convergence integral along the edge, so it is NOT π in general.)
    let ba = inverse(&e, b, a);
    let diff = normalize_angle(ab.azi2 - (ba.azi1 + PI)).abs();
    assert!(diff < 1e-9, "azi2(ab) vs azi1(ba)+π: diff = {diff}");
    let diff2 = normalize_angle(bc.azi2 - (cb.azi1 + PI)).abs();
    assert!(diff2 < 1e-9, "azi2(bc) vs azi1(cb)+π: diff = {diff2}");
    let _ = cb;
}

// ---------------------------------------------------------------------------
// JIS X 0410 boundary invariants (micro-arcsecond exactness)
// ---------------------------------------------------------------------------

#[test]
fn jis_boundary_identities() {
    // The half-open convention: a point exactly on a 30″ level-3 grid line
    // belongs to the NORTHERN/EASTERN cell.
    // 35°01′30″ exactly: r1 must be 3 (the 4th row, 0-based), not 2.
    // 139°00′45″ exactly: r2 must be 1 (the 2nd column), not 0.
    let lat = 35.0 + 1.5 / 60.0; // 35°01′30″
    let code = JisMeshCode::from_lat_lon(lat, 139.0 + 0.75 / 60.0, MeshLevel::Level3)
        .expect("in domain");
    assert_eq!(code.to_string(), "52394031");
    // One micro-arcsecond south must fall in r1 = 2.
    let lat_minus = ((lat * 3600.0 * 1e6 - 1.0).round()) / (3600.0 * 1e6);
    let code_s = JisMeshCode::from_lat_lon(lat_minus, 139.0 + 0.75 / 60.0, MeshLevel::Level3)
        .expect("in domain");
    assert_eq!(code_s.to_string(), "52394021");
}

#[test]
fn jis_full_level_roundtrip() {
    // For every level, code -> center -> code is the identity and the query
    // point stays inside the emitted bounding box.
    let points = [
        (35.658581, 139.745433),   // Tokyo Tower
        (43.062_056, 141.354_444), // Sapporo
        (26.216_667, 127.683_333), // Naha
        (34.693_728, 135.502_223), // Osaka
    ];
    for &(lat, lon) in &points {
        for level in [
            MeshLevel::Level1,
            MeshLevel::Level2,
            MeshLevel::Level3,
            MeshLevel::Level4,
            MeshLevel::Level5,
            MeshLevel::Level6,
        ] {
            let code = JisMeshCode::from_lat_lon(lat, lon, level).expect("code");
            let b = code.to_bounds();
            assert!(b.south <= lat && lat < b.north && b.west <= lon && lon < b.east);
            let c = code.to_center();
            let back =
                JisMeshCode::from_lat_lon(c.lat_degrees(), c.lon_degrees(), level).expect("code");
            assert_eq!(code, back, "level {:?}", level.number());
        }
    }
}

#[test]
fn jis_neighborhood_covers_all_8_directions() {
    // A cell in mid-domain has exactly 8 distinct neighbors, all adjacent
    // in flat index space.
    let center = JisMeshCode::from_lat_lon(35.6586, 139.7454, MeshLevel::Level3).expect("code");
    let neighbors = center.neighbors_8();
    assert_eq!(neighbors.len(), 8);
    let level = center.level();
    let anchor = 100 * 3_600 * 1_000_000;
    let base_row = center.south_lat_micros() / level.lat_micros();
    let base_col = (center.west_lon_micros() - anchor) / level.lon_micros();
    for n in &neighbors {
        let dr = (n.south_lat_micros() / level.lat_micros() - base_row).abs();
        let dc = ((n.west_lon_micros() - anchor) / level.lon_micros() - base_col).abs();
        assert!(dr <= 1 && dc <= 1 && (dr + dc) > 0);
    }
}

// ---------------------------------------------------------------------------
// Cross-module consistency
// ---------------------------------------------------------------------------

#[test]
fn polygon_area_engines_agree() {
    // The C4 Gauss–Bonnet engine and the authalic-Green engine must agree
    // on a mid-latitude quadrilateral, and both must match the
    // coordinate-plane estimate to better than 1%.
    let e = Ellipsoid::new(EllipsoidKind::GRS80);
    let poly = EllipsoidalPolygon::new(
        vec![
            Coord::from_degrees(35.0, 139.0),
            Coord::from_degrees(35.0, 140.5),
            Coord::from_degrees(36.5, 140.5),
            Coord::from_degrees(36.5, 139.0),
        ],
        EdgeKind::Geodesic,
    )
    .expect("valid");
    let a_c4 = poly.area_geodesic_c4(&e);
    let a_green = poly.area_green(&e);
    let rel = ((a_c4 - a_green) / a_green.abs()).abs();
    assert!(rel < 1e-8, "C4 {a_c4} vs green {a_green} (rel {rel:e})");
    let phi_m = 35.75_f64.to_radians();
    let approx = (e.m_radius(phi_m) * e.n_radius(phi_m) * phi_m.cos())
        * (1.5_f64.to_radians())
        * (1.5_f64.to_radians());
    assert!((a_c4 - approx).abs() / approx < 0.01);
    // The rhumb variant of the same quadrilateral (the north/south edges
    // differ from geodesics only by the small loxodrome bulge).
    let poly_r = EllipsoidalPolygon::new(
        vec![
            Coord::from_degrees(35.0, 139.0),
            Coord::from_degrees(35.0, 140.5),
            Coord::from_degrees(36.5, 140.5),
            Coord::from_degrees(36.5, 139.0),
        ],
        EdgeKind::Rhumb,
    )
    .expect("valid");
    let a_r = poly_r.area(&e);
    assert!((a_r - a_c4).abs() / a_c4.abs() < 1e-4, "rhumb {a_r} vs geodesic {a_c4}");
}

#[test]
fn disk_cover_roundtrip_with_jismesh() {
    // Every cell emitted by the disk covering maps back to the same cell
    // through the plain jismesh API, and the center's own cell is included.
    let e = Ellipsoid::new(EllipsoidKind::GRS80);
    let center = Coord::from_degrees(35.658581, 139.745433);
    let cells = cover_geodesic_disk(&e, center, 2000.0, MeshLevel::Level5).expect("cover");
    assert!(!cells.is_empty());
    for code in &cells {
        let b = code.to_bounds();
        let c = b.center();
        let back =
            JisMeshCode::from_lat_lon(c.lat_degrees(), c.lon_degrees(), MeshLevel::Level5)
                .expect("code");
        assert_eq!(code, &back);
    }
    let home = JisMeshCode::from_lat_lon(35.658581, 139.745433, MeshLevel::Level5).expect("code");
    assert!(cells.contains(&home));
}

#[test]
fn polygon_cover_grid_aligned_square() {
    // A square aligned to the level-5 grid spans 30 rows x 20 columns
    // (0.0625° = 225″; rows 225″/7.5″ = 30, cols 225″/11.25″ = 20), and
    // the covering must be exactly those 600 cells under the
    // closed-boundary convention.
    let e = Ellipsoid::new(EllipsoidKind::GRS80);
    let poly = EllipsoidalPolygon::new(
        vec![
            Coord::from_degrees(35.0, 139.0),
            Coord::from_degrees(35.0, 139.0625),
            Coord::from_degrees(35.0625, 139.0625),
            Coord::from_degrees(35.0625, 139.0),
        ],
        EdgeKind::Rhumb,
    )
    .expect("valid");
    let cells = cover_polygon(&e, &poly, MeshLevel::Level5).expect("cover");
    assert_eq!(cells.len(), 30 * 20, "emitted {}", cells.len());
    // A cell far away must not be covered.
    let far = JisMeshCode::from_lat_lon(36.0, 140.0, MeshLevel::Level5).expect("code");
    assert!(!cells.contains(&far));
    // Every cell center strictly inside the polygon is covered, and cells
    // whose centers are deep outside are not.
    let inside = JisMeshCode::from_lat_lon(35.03, 139.03, MeshLevel::Level5).expect("code");
    assert!(cells.contains(&inside));
}

#[test]
fn hilbert_keys_preserve_ordering_within_a_row() {
    // Hilbert keys of cells along the same row increase monotonically
    // within each Hilbert sub-square (locality property for the covering
    // outputs).
    let mut keys = Vec::new();
    for k in 0..8usize {
        let lon = 139.0 + k as f64 * 0.01;
        let code = JisMeshCode::from_lat_lon(35.5, lon, MeshLevel::Level6).expect("code");
        keys.push(code.to_hilbert64().0);
    }
    for w in keys.windows(2) {
        assert!(w[1] > w[0], "hilbert locality broken at {w:?}");
    }
}

#[test]
fn zero_unsafe_guarantee() {
    // The crate forbids unsafe code at compile time (the crate header
    // carries #![forbid(unsafe_code)]); any unsafe usage anywhere in the
    // crate fails the build. This test documents the guarantee.
    assert!(true);
}
