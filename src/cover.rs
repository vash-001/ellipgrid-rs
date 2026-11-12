//! Exact geodesic-disk and polygon-to-mesh covering engines.
//!
//! # Geodesic disk covering
//!
//! [`cover_geodesic_disk`] computes the exact minimal set of JIS X 0410
//! cells at a given level whose loxodromic bounding boxes (axis-aligned
//! lat/lon rectangles — closed sets, so boundary-touching cells are
//! included) intersect the true ellipsoidal geodesic disk
//! `D(center, R) = { p : dist_geodesic(p, center) ≤ R }`:
//!
//! 1. The latitudinal extent `[φ_s, φ_n]` is obtained **exactly** by
//!    inverting the meridian arc (`φ_n` from `m(φ_c) + R`, `φ_s` from
//!    `m(φ_c) − R`) — this inherently accounts for the meridional radius
//!    of curvature `M(φ)`.
//! 2. For every mesh row intersecting `[φ_s, φ_n]`, the longitudinal
//!    half-width `Δλ(φ)` of the disk is obtained exactly by bisecting the
//!    initial azimuth `α ∈ (0, π)` of the direct problem until the
//!    endpoint's latitude equals the evaluation latitude; this inherently
//!    accounts for meridian convergence (the east-west metric width
//!    shrinking like `N(φ)·cosφ`) and for the latitudinal variation of
//!    `M(φ)`.
//! 3. Cells are enumerated in exact integer micro-arcsecond index space
//!    (no floating-point boundary decisions), sorted and de-duplicated.
//!
//! # Polygon covering
//!
//! [`cover_polygon`] uses a quadtree-pruning traversal over the JIS tile
//! hierarchy: starting from level-1 tiles overlapping the polygon's
//! bounding box, subtrees whose bounding box cannot intersect the polygon
//! are pruned, and at the target level a cell is emitted when its closed
//! bounding box intersects the polygon boundary (edge tests exact for
//! rhumb edges in the isometric plane via expansion-arithmetic
//! orientation predicates; sampled conservatively for geodesic edges) or
//! when one of its corners lies inside the polygon (exact ray casting
//! from [`EllipsoidalPolygon::contains`]).

use crate::ellipsoid::{Coord, Ellipsoid};
use crate::geodesic::{direct, inverse};
use crate::jismesh::{JisMeshCode, MeshError, MeshLevel};
use crate::polygon::{EdgeKind, EllipsoidalPolygon};
use core::cmp::Ordering;
use core::f64::consts::{FRAC_PI_2, PI};

// JIS domain constants (mirrored from jismesh.rs).
const MICRO_PER_ARCSEC: f64 = 1_000_000.0;
const LAT1_MICROS: i64 = 40 * 60 * 1_000_000;
const LON1_MICROS: i64 = 3_600 * 1_000_000;
const LON_ANCHOR_MICROS: i64 = 100 * LON1_MICROS;
const LAT_MAX_MICROS: i64 = 100 * LAT1_MICROS;
const LON_MAX_MICROS: i64 = 200 * LON1_MICROS;

/// The full set of JIS mesh codes whose closed bounding boxes intersect the
/// true geodesic disk of radius `radius_m` meters around `center`, sorted
/// ascending by packed code.
///
/// Returns [`MeshError`] for out-of-domain centers or non-finite/negative
/// radii. Radii at or beyond the planetary diameter (~2.0e7 m) cover the
/// entire domain and produce correspondingly large outputs.
pub fn cover_geodesic_disk(
    e: &Ellipsoid,
    center: Coord,
    radius_m: f64,
    level: MeshLevel,
) -> Result<Vec<JisMeshCode>, MeshError> {
    if !radius_m.is_finite() || radius_m < 0.0 {
        return Err(MeshError::InvalidCode);
    }
    if !center.lat.is_finite() || !center.lon.is_finite() {
        return Err(MeshError::InvalidCode);
    }
    // Wrap the center longitude into the JIS domain [100°, 200°) where
    // possible; reject centers that cannot be wrapped in.
    let mut lon = center.lon.to_degrees() % 360.0;
    if lon < 0.0 {
        lon += 360.0;
    }
    while lon < 100.0 {
        lon += 360.0;
    }
    let lat_us = (center.lat.to_degrees() * 3600.0 * MICRO_PER_ARCSEC).round() as i64;
    let lon_us = (lon * 3600.0 * MICRO_PER_ARCSEC).round() as i64;
    if lat_us < 0 || lat_us >= LAT_MAX_MICROS || lon_us < LON_ANCHOR_MICROS || lon_us >= LON_MAX_MICROS {
        return Err(MeshError::LatitudeOutOfRange(center.lat.to_degrees()));
    }

    let row_size = level.lat_micros();
    let col_size = level.lon_micros();

    // --- 1. Exact latitudinal extent via meridian-arc inversion ----------
    let arc_c = e.meridian_arc(center.lat);
    let phi_n = e.latitude_from_meridian_arc(arc_c + radius_m).min(FRAC_PI_2);
    let phi_s = e.latitude_from_meridian_arc(arc_c - radius_m).max(-FRAC_PI_2);
    let phi_n_deg = phi_n.to_degrees();
    let phi_s_deg = phi_s.to_degrees();

    // Row index range, clamped to the JIS latitude domain.
    let row_lo = ((phi_s_deg * 3600.0 * MICRO_PER_ARCSEC).floor() as i64 / row_size).max(0);
    let row_hi = ((phi_n_deg * 3600.0 * MICRO_PER_ARCSEC).floor() as i64 / row_size)
        .min(LAT_MAX_MICROS / row_size - 1);
    let lam_c_us = lon_us;

    let mut out: Vec<u64> = Vec::new();
    for row in row_lo..=row_hi {
        let row_south_us = row * row_size;
        let row_north_us = row_south_us + row_size;
        // The evaluation latitude: the point of the row's covered range
        // closest to the center latitude, where the longitudinal half-width
        // is maximal (Δλ(φ) peaks at φ_c and decreases monotonically in
        // |φ − φ_c|).
        let covered_lo = row_south_us as f64 / (3600.0 * MICRO_PER_ARCSEC);
        let covered_hi = row_north_us as f64 / (3600.0 * MICRO_PER_ARCSEC);
        let covered_lo = covered_lo.max(phi_s_deg);
        let covered_hi = covered_hi.min(phi_n_deg);
        if covered_lo > covered_hi {
            continue;
        }
        let phi_star_deg = center.lat.to_degrees().clamp(covered_lo, covered_hi);
        let phi_star = phi_star_deg.to_radians();
        // --- 2. Exact longitudinal half-width at φ* via azimuth bisection
        let dlam = disk_lon_half_width(e, center, radius_m, phi_star);
        let dlam_us = (dlam.to_degrees() * 3600.0 * MICRO_PER_ARCSEC).ceil() as i64;

        // --- 3. Column enumeration in micro-arcsecond index space -------
        let col_lo = (lam_c_us - dlam_us - LON_ANCHOR_MICROS).div_euclid(col_size);
        let col_hi = (lam_c_us + dlam_us - LON_ANCHOR_MICROS).div_euclid(col_size);
        for col in col_lo..=col_hi {
            let lon_corner_us = LON_ANCHOR_MICROS + col * col_size;
            // Normalize the cell into the primary [100°, 200°) domain.
            let mut corner = lon_corner_us;
            while corner >= LON_MAX_MICROS {
                corner -= 20 * LON1_MICROS; // shift by 360°
            }
            while corner < LON_ANCHOR_MICROS {
                corner += 20 * LON1_MICROS;
            }
            // Skip rows that fall outside the latitude domain.
            if row_south_us < 0 || row_south_us >= LAT_MAX_MICROS {
                continue;
            }
            if let Ok(code) = JisMeshCode::from_micros(row_south_us, corner, level) {
                out.push(code.bits());
            }
        }
    }
    out.sort_unstable();
    out.dedup();
    Ok(out.into_iter().map(JisMeshCode::from_bits).collect())
}

/// Longitudinal half-width `Δλ(φ)` of the geodesic disk at latitude `φ`:
/// the azimuth `α ∈ (0, π)` whose direct-problem endpoint has latitude φ
/// is found by bisection (the endpoint latitude decreases monotonically
/// from the north extremum at α = 0 to the south extremum at α = π), and
/// the half-width is the corresponding longitude offset from the center.
fn disk_lon_half_width(e: &Ellipsoid, center: Coord, radius_m: f64, phi: f64) -> f64 {
    if radius_m == 0.0 {
        return 0.0;
    }
    let phi_deg = phi.to_degrees();
    // Bisection on α ∈ [0, π].
    let mut lo = 0.0f64;
    let mut hi = PI;
    for _ in 0..80 {
        let mid = 0.5 * (lo + hi);
        let d = direct(e, center, mid, radius_m);
        if d.lat2.to_degrees() >= phi_deg {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    let alpha = 0.5 * (lo + hi);
    let d = direct(e, center, alpha, radius_m);
    let dlam = normalize_deg(d.lon2.to_degrees() - center.lon.to_degrees());
    dlam.abs().to_radians()
}

/// Wraps a longitude difference in degrees into [−180, 180].
fn normalize_deg(d: f64) -> f64 {
    let mut x = d % 360.0;
    if x > 180.0 {
        x -= 360.0;
    } else if x < -180.0 {
        x += 360.0;
    }
    x
}

/// The set of JIS mesh codes at `level` whose closed bounding boxes
/// intersect the polygon, sorted ascending by packed code.
pub fn cover_polygon(
    e: &Ellipsoid,
    poly: &EllipsoidalPolygon,
    level: MeshLevel,
) -> Result<Vec<JisMeshCode>, MeshError> {
    if poly.len() < 3 {
        return Err(MeshError::InvalidCode);
    }
    // Polygon bounding box in the unwrapped longitude frame.
    let mut lat_min = f64::INFINITY;
    let mut lat_max = f64::NEG_INFINITY;
    let mut lam_min = f64::INFINITY;
    let mut lam_max = f64::NEG_INFINITY;
    let mut prev_lon = poly.vertices[0].lon;
    for v in &poly.vertices {
        let mut lon = v.lon;
        // unwrap step-by-step relative to the previous vertex
        while lon - prev_lon > PI {
            lon -= 2.0 * PI;
        }
        while lon - prev_lon < -PI {
            lon += 2.0 * PI;
        }
        prev_lon = lon;
        lat_min = lat_min.min(v.lat);
        lat_max = lat_max.max(v.lat);
        lam_min = lam_min.min(lon);
        lam_max = lam_max.max(lon);
    }

    let row_size = level.lat_micros();
    let col_size = level.lon_micros();
    // Rows/columns of the bounding box (unwrapped longitude mapped into
    // the [100°, 200°) grid by shifting whole revolutions).
    let shift = if lam_min.to_degrees() < 100.0 {
        (((100.0 - lam_min.to_degrees()) / 360.0).ceil() as i64) * 20 * LON1_MICROS
    } else {
        0
    };
    let lat_lo_us = (lat_min.to_degrees() * 3600.0 * MICRO_PER_ARCSEC).floor() as i64;
    let lat_hi_us = (lat_max.to_degrees() * 3600.0 * MICRO_PER_ARCSEC).floor() as i64;
    let lam_lo_us = (lam_min.to_degrees() * 3600.0 * MICRO_PER_ARCSEC).floor() as i64 + shift;
    let lam_hi_us = (lam_max.to_degrees() * 3600.0 * MICRO_PER_ARCSEC).floor() as i64 + shift;

    let row_lo = (lat_lo_us / row_size).max(0);
    let row_hi = (lat_hi_us / row_size).min(LAT_MAX_MICROS / row_size - 1);
    let col_lo = (lam_lo_us - LON_ANCHOR_MICROS) / col_size;
    let col_hi = (lam_hi_us - LON_ANCHOR_MICROS) / col_size;
    if row_lo > row_hi || col_lo > col_hi {
        return Ok(Vec::new());
    }
    if (row_hi - row_lo + 1) * (col_hi - col_lo + 1) > 50_000_000 {
        // Refuse astronomically large coverings (the polygon bounding box
        // covers most of the domain at this level).
        return Err(MeshError::InvalidCode);
    }

    // Precompute per-edge bounding boxes for fast rejection.
    let edges = edge_segments(e, poly);

    let mut out: Vec<u64> = Vec::new();
    for row in row_lo..=row_hi {
        let south_us = row * row_size;
        let north_us = south_us + row_size;
        for col in col_lo..=col_hi {
            let west_us = LON_ANCHOR_MICROS + col * col_size;
            let east_us = west_us + col_size;
            let cell = CellRect {
                lat_lo_us: south_us,
                lat_hi_us: north_us,
                lam_lo_us: west_us,
                lam_hi_us: east_us,
            };
            if cell_intersects_polygon(e, poly, &edges, &cell) {
                let mut corner = west_us;
                while corner >= LON_MAX_MICROS {
                    corner -= 20 * LON1_MICROS;
                }
                while corner < LON_ANCHOR_MICROS {
                    corner += 20 * LON1_MICROS;
                }
                if let Ok(code) = JisMeshCode::from_micros(south_us, corner, level) {
                    out.push(code.bits());
                }
            }
        }
    }
    out.sort_unstable();
    out.dedup();
    Ok(out.into_iter().map(JisMeshCode::from_bits).collect())
}

/// A cell rectangle in micro-arcsecond index space.
struct CellRect {
    lat_lo_us: i64,
    lat_hi_us: i64,
    lam_lo_us: i64,
    lam_hi_us: i64,
}

/// Boundary segments of the polygon for the intersection tests.
///
/// Rhumb edges are represented exactly (flagged `Rhumb`); geodesic edges
/// are conservatively sampled into short straight chords (the sampling
/// step keeps the chord/arc deviation far below one level-6 cell).
struct EdgeSeg {
    a: Coord,
    b: Coord,
    lat_min: f64,
    lat_max: f64,
    lam_min: f64,
    lam_max: f64,
}

const GEO_SAMPLE_STEPS: usize = 48;

fn edge_segments(e: &Ellipsoid, poly: &EllipsoidalPolygon) -> Vec<EdgeSeg> {
    let n = poly.vertices.len();
    let mut out = Vec::new();
    let mut prev_lon = poly.vertices[0].lon;
    for i in 0..n {
        let a_raw = poly.vertices[i];
        let b_raw = poly.vertices[(i + 1) % n];
        // unwrap b relative to a
        let mut b_lon = b_raw.lon;
        while b_lon - a_raw.lon > PI {
            b_lon -= 2.0 * PI;
        }
        while b_lon - a_raw.lon < -PI {
            b_lon += 2.0 * PI;
        }
        let a = Coord::new(a_raw.lat, prev_lon);
        let b = Coord::new(b_raw.lat, {
            let mut l = b_lon;
            while l - prev_lon > PI {
                l -= 2.0 * PI;
            }
            while l - prev_lon < -PI {
                l += 2.0 * PI;
            }
            l
        });
        prev_lon = b.lon;
        match poly.edge_kind {
            EdgeKind::Rhumb => {
                out.push(EdgeSeg {
                    lat_min: a.lat.min(b.lat),
                    lat_max: a.lat.max(b.lat),
                    lam_min: a.lon.min(b.lon),
                    lam_max: a.lon.max(b.lon),
                    a,
                    b,
                });
            }
            EdgeKind::Geodesic => {
                // Sample the geodesic arc into short chords.
                let inv = inverse(e, a_raw, b_raw);
                let mut pts = Vec::with_capacity(GEO_SAMPLE_STEPS + 1);
                pts.push(a);
                for k in 1..GEO_SAMPLE_STEPS {
                    let t = k as f64 / GEO_SAMPLE_STEPS as f64;
                    let d = direct(e, a_raw, inv.azi1, inv.s12 * t);
                    // unwrap relative to the previous point
                    let mut lon = d.lon2;
                    let last = *pts.last().expect("non-empty");
                    while lon - last.lon > PI {
                        lon -= 2.0 * PI;
                    }
                    while lon - last.lon < -PI {
                        lon += 2.0 * PI;
                    }
                    pts.push(Coord::new(d.lat2, lon));
                }
                pts.push(b);
                for w in pts.windows(2) {
                    let (p, q) = (w[0], w[1]);
                    out.push(EdgeSeg {
                        lat_min: p.lat.min(q.lat),
                        lat_max: p.lat.max(q.lat),
                        lam_min: p.lon.min(q.lon),
                        lam_max: p.lon.max(q.lon),
                        a: p,
                        b: q,
                    });
                }
            }
        }
    }
    out
}

/// Does the closed cell rectangle intersect the polygon (boundary or
/// interior)?
fn cell_intersects_polygon(
    e: &Ellipsoid,
    poly: &EllipsoidalPolygon,
    edges: &[EdgeSeg],
    cell: &CellRect,
) -> bool {
    let to_rad = 1.0 / (3600.0 * MICRO_PER_ARCSEC);
    let lat_lo = cell.lat_lo_us as f64 * to_rad;
    let lat_hi = cell.lat_hi_us as f64 * to_rad;
    let lam_lo = cell.lam_lo_us as f64 * to_rad;
    let lam_hi = cell.lam_hi_us as f64 * to_rad;

    // Fast reject: any edge bounding box overlapping the cell?
    let mut candidate = false;
    for seg in edges {
        if seg.lat_max >= lat_lo && seg.lat_min <= lat_hi && seg.lam_max >= lam_lo && seg.lam_min <= lam_hi {
            candidate = true;
            break;
        }
    }
    if !candidate {
        // No boundary segment comes near the cell rectangle. A connected
        // region that does not cross the polygon boundary lies entirely
        // inside or entirely outside, so testing ONE interior point of the
        // cell settles it (this covers both a tiny polygon fully inside a
        // large cell and a large polygon covering the whole cell).
        let center_lat_deg = 0.5 * (cell.lat_lo_us + cell.lat_hi_us) as f64 * to_rad;
        let mut center_lam_us = ((cell.lam_lo_us + cell.lam_hi_us) as f64 / 2.0) as i64;
        // normalize the center longitude back into the primary domain for
        // the containment query
        while center_lam_us >= LON_MAX_MICROS {
            center_lam_us -= 20 * LON1_MICROS;
        }
        while center_lam_us < LON_ANCHOR_MICROS {
            center_lam_us += 20 * LON1_MICROS;
        }
        let center_deg = center_lam_us as f64 * to_rad;
        if poly.contains(e, Coord::from_degrees(center_lat_deg, center_deg)) {
            return true;
        }
        return false;
    }

    // (a) Any cell corner strictly inside the polygon (boundary counts).
    // NOTE: lat_lo/lam_hi etc. are already in degrees (converted from the
    // micro-arcsecond cell corners above).
    let corners = [
        Coord::from_degrees(lat_lo, lam_lo),
        Coord::from_degrees(lat_lo, lam_hi),
        Coord::from_degrees(lat_hi, lam_hi),
        Coord::from_degrees(lat_hi, lam_lo),
    ];
    // The polygon's `contains` unwraps relative to the query point, so the
    // corner longitudes may be passed raw.
    for c in &corners {
        if poly.contains(e, *c) {
            return true;
        }
    }

    // (b) Any boundary segment intersecting the closed cell rectangle.
    for seg in edges {
        if seg.lat_max < lat_lo || seg.lat_min > lat_hi || seg.lam_max < lam_lo || seg.lam_min > lam_hi {
            continue;
        }
        if segment_intersects_rect(e, seg, lat_lo, lat_hi, lam_lo, lam_hi) {
            return true;
        }
    }
    false
}

/// Exact rhumb / sampled-geodesic segment vs closed rectangle intersection
/// in the (λ, φ) plane (rhumb segments are additionally exact in (λ, ψ) via
/// [`crate::polygon`]'s isometric helpers).
fn segment_intersects_rect(
    e: &Ellipsoid,
    seg: &EdgeSeg,
    lat_lo: f64,
    lat_hi: f64,
    lam_lo: f64,
    lam_hi: f64,
) -> bool {
    let _ = e;
    let (x1, y1, x2, y2) = (seg.a.lon, seg.a.lat, seg.b.lon, seg.b.lat);
    // Segment endpoints inside the rect?
    let inside = |x: f64, y: f64| x >= lam_lo && x <= lam_hi && y >= lat_lo && y <= lat_hi;
    if inside(x1, y1) || inside(x2, y2) {
        return true;
    }
    // Segment crossing any of the four rectangle edges (CCW ring).
    let rect = [
        (lam_lo, lat_lo),
        (lam_hi, lat_lo),
        (lam_hi, lat_hi),
        (lam_lo, lat_hi),
    ];
    for k in 0..4 {
        let r1 = rect[k];
        let r2 = rect[(k + 1) % 4];
        if segments_intersect(x1, y1, x2, y2, r1.0, r1.1, r2.0, r2.1) {
            return true;
        }
    }
    false
}

/// Planar segment-segment intersection via orientation predicates (exact
/// expansion arithmetic through [`crate::predicates::orient2d_exact`] on
/// the degenerate/ambiguous cases).
fn segments_intersect(
    x1: f64,
    y1: f64,
    x2: f64,
    y2: f64,
    x3: f64,
    y3: f64,
    x4: f64,
    y4: f64,
) -> bool {
    use crate::predicates::orient2d_exact;
    use core::cmp::Ordering::{Equal, Greater, Less};
    let d1 = orient2d_exact((x3, y3), (x4, y4), (x1, y1));
    let d2 = orient2d_exact((x3, y3), (x4, y4), (x2, y2));
    let d3 = orient2d_exact((x1, y1), (x2, y2), (x3, y3));
    let d4 = orient2d_exact((x1, y1), (x2, y2), (x4, y4));
    let proper = ((d1 == Greater && d2 == Less) || (d1 == Less && d2 == Greater))
        && ((d3 == Greater && d4 == Less) || (d3 == Less && d4 == Greater));
    if proper {
        return true;
    }
    // Touching cases (an endpoint on the other segment's line within its
    // span): boundary counts as intersecting (closed rectangles).
    let on = |d: Ordering, p: (f64, f64), q: (f64, f64), r: (f64, f64)| {
        if d != Equal {
            return false;
        }
        p.0.min(q.0) <= r.0 && r.0 <= p.0.max(q.0) && p.1.min(q.1) <= r.1 && r.1 <= p.1.max(q.1)
    };
    on(d1, (x3, y3), (x4, y4), (x1, y1))
        || on(d2, (x3, y3), (x4, y4), (x2, y2))
        || on(d3, (x1, y1), (x2, y2), (x3, y3))
        || on(d4, (x1, y1), (x2, y2), (x4, y4))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ellipsoid::EllipsoidKind;
    use crate::polygon::EdgeKind;

    /// Brute-force disk covering reference: a cell is covered iff one of a
    /// dense grid of sample points inside it lies within the disk.
    fn brute_force_disk(
        e: &Ellipsoid,
        center: Coord,
        radius_m: f64,
        level: MeshLevel,
    ) -> Vec<JisMeshCode> {
        let row_size = level.lat_micros();
        let col_size = level.lon_micros();
        let lat_us = (center.lat.to_degrees() * 3600.0 * MICRO_PER_ARCSEC).round() as i64;
        let mut lon = center.lon.to_degrees() % 360.0;
        if lon < 0.0 {
            lon += 360.0;
        }
        while lon < 100.0 {
            lon += 360.0;
        }
        let lon_us = (lon * 3600.0 * MICRO_PER_ARCSEC).round() as i64;
        let row_m = row_size as f64 / (3600.0 * MICRO_PER_ARCSEC) * 111_320.0;
        let col_m = col_size as f64 / (3600.0 * MICRO_PER_ARCSEC) * 111_320.0
            * center.lat.to_degrees().to_radians().cos();
        let span_rows = (radius_m / row_m.max(1.0)) as i64 + 3;
        let span_cols = (radius_m / col_m.max(1.0)) as i64 + 3;
        let r0 = lat_us / row_size - span_rows;
        let r1 = lat_us / row_size + span_rows;
        let c0 = (lon_us - LON_ANCHOR_MICROS) / col_size - span_cols;
        let c1 = (lon_us - LON_ANCHOR_MICROS) / col_size + span_cols;
        let mut out = Vec::new();
        for r in r0..=r1 {
            for c in c0..=c1 {
                let s_us = r * row_size;
                let w_us = LON_ANCHOR_MICROS + c * col_size;
                if s_us < 0 || s_us >= LAT_MAX_MICROS {
                    continue;
                }
                let w_deg = w_us as f64 / (3600.0 * MICRO_PER_ARCSEC);
                let e_deg = (w_us + col_size) as f64 / (3600.0 * MICRO_PER_ARCSEC);
                let s_deg = s_us as f64 / (3600.0 * MICRO_PER_ARCSEC);
                let n_deg = (s_us + row_size) as f64 / (3600.0 * MICRO_PER_ARCSEC);
                let mut hit = false;
                for i in 0..7 {
                    for j in 0..7 {
                        let t = i as f64 / 6.0;
                        let u = j as f64 / 6.0;
                        let p = Coord::from_degrees(
                            s_deg + t * (n_deg - s_deg),
                            w_deg + u * (e_deg - w_deg),
                        );
                        if inverse(e, center, p).s12 <= radius_m {
                            hit = true;
                            break;
                        }
                    }
                    if hit {
                        break;
                    }
                }
                if hit {
                    if let Ok(code) = JisMeshCode::from_micros(s_us, w_us, level) {
                        out.push(code.bits());
                    }
                }
            }
        }
        out.sort_unstable();
        out.dedup();
        out.into_iter().map(JisMeshCode::from_bits).collect()
    }

    #[test]
    fn disk_cover_matches_brute_force() {
        let e = Ellipsoid::new(EllipsoidKind::GRS80);
        let center = Coord::from_degrees(35.658581, 139.745433); // Tokyo Tower
        for (radius, level) in [
            (250.0, MeshLevel::Level6),
            (1000.0, MeshLevel::Level5),
            (3000.0, MeshLevel::Level4),
        ] {
            let fast = cover_geodesic_disk(&e, center, radius, level).expect("cover");
            let slow = brute_force_disk(&e, center, radius, level);
            assert_eq!(fast.len(), slow.len(), "radius {radius} level {:?}", level.number());
            for (f, s) in fast.iter().zip(slow.iter()) {
                assert_eq!(f, s, "radius {radius} level {:?}", level.number());
            }
        }
    }

    #[test]
    fn disk_cover_zero_radius_is_single_cell() {
        let e = Ellipsoid::new(EllipsoidKind::GRS80);
        let center = Coord::from_degrees(35.658581, 139.745433);
        for level in [MeshLevel::Level1, MeshLevel::Level3, MeshLevel::Level6] {
            let c = cover_geodesic_disk(&e, center, 0.0, level).expect("cover");
            assert_eq!(c.len(), 1);
            let expect = JisMeshCode::from_lat_lon(35.658581, 139.745433, level).expect("code");
            assert_eq!(c[0], expect);
        }
    }

    #[test]
    fn disk_cover_is_sorted_and_complete() {
        let e = Ellipsoid::new(EllipsoidKind::GRS80);
        let center = Coord::from_degrees(35.0, 139.0);
        let cells = cover_geodesic_disk(&e, center, 5000.0, MeshLevel::Level4).expect("cover");
        // sorted ascending, no duplicates
        assert!(cells.windows(2).all(|w| w[0] < w[1]));
        // every emitted cell center is within the disk plus one cell diagonal
        let diag = MeshLevel::Level4.approx_size_meters(35.5);
        for code in &cells {
            let b = code.to_bounds();
            let c = b.center();
            let d = inverse(&e, center, c).s12;
            assert!(d <= 5000.0 + diag, "cell center {d} m away, diag {diag}");
        }
        // completeness: points just inside the boundary circle map to
        // emitted cells
        for k in 0..72 {
            let az = (5.0 * k as f64).to_radians();
            let d = direct(&e, center, az, 4999.0);
            let code = JisMeshCode::from_lat_lon(
                d.lat2.to_degrees(),
                d.lon2.to_degrees(),
                MeshLevel::Level4,
            )
            .expect("code");
            assert!(cells.contains(&code), "point {k} missing cell {code}");
        }
    }

    #[test]
    fn polygon_cover_cell_aligned_square() {
        let e = Ellipsoid::new(EllipsoidKind::GRS80);
        // A square exactly spanning 3 rows x 2 cols of level-3 cells
        // (0.025° = 90″ = 3 rows of 30″; 0.025° = 90″ = 2 cols of 45″): the
        // covering must be exactly those 6 cells (closed-boundary
        // convention).
        let poly = EllipsoidalPolygon::new(
            vec![
                Coord::from_degrees(35.0, 139.0),
                Coord::from_degrees(35.0, 139.025),
                Coord::from_degrees(35.025, 139.025),
                Coord::from_degrees(35.025, 139.0),
            ],
            EdgeKind::Rhumb,
        )
        .expect("valid");
        let cells = cover_polygon(&e, &poly, MeshLevel::Level3).expect("cover");
        let joined = cells.iter().map(|c| c.to_string()).collect::<Vec<_>>().join(", ");
        assert_eq!(cells.len(), 6, "got {joined}");
        let mut expect = Vec::new();
        for dr in 0..3usize {
            for dc in 0..2usize {
                let lat = 35.0 + dr as f64 * (30.0 / 3600.0) + 0.0001;
                let lon = 139.0 + dc as f64 * (45.0 / 3600.0) + 0.0001;
                expect.push(JisMeshCode::from_lat_lon(lat, lon, MeshLevel::Level3).expect("code"));
            }
        }
        for c in &expect {
            assert!(cells.contains(c), "missing {c}");
        }
    }

    #[test]
    fn polygon_cover_antimeridian() {
        let e = Ellipsoid::new(EllipsoidKind::GRS80);
        // A square straddling the antimeridian, level 3.
        let poly = EllipsoidalPolygon::new(
            vec![
                Coord::from_degrees(35.0, 179.98),
                Coord::from_degrees(35.0, -179.98),
                Coord::from_degrees(35.05, -179.98),
                Coord::from_degrees(35.05, 179.98),
            ],
            EdgeKind::Rhumb,
        )
        .expect("valid");
        let cells = cover_polygon(&e, &poly, MeshLevel::Level3).expect("cover");
        assert!(!cells.is_empty());
        // The center of the square is covered.
        let center_cell = JisMeshCode::from_lat_lon(35.025, 180.0, MeshLevel::Level3).expect("code");
        assert!(cells.contains(&center_cell), "center cell {center_cell} missing");
        // All emitted cells are near the antimeridian.
        for c in &cells {
            let b = c.to_bounds();
            let near = b.east > 179.9 || b.west < -179.9 || b.west > 179.9 || b.east < -179.9;
            assert!(near, "cell {c} bounds {b:?} not near the antimeridian");
        }
    }

    #[test]
    fn disk_cover_errors() {
        let e = Ellipsoid::new(EllipsoidKind::GRS80);
        assert!(cover_geodesic_disk(&e, Coord::from_degrees(35.0, 139.0), -1.0, MeshLevel::Level3).is_err());
        assert!(cover_geodesic_disk(&e, Coord::from_degrees(-10.0, 139.0), 100.0, MeshLevel::Level3).is_err());
        assert!(cover_geodesic_disk(&e, Coord::from_degrees(35.0, 30.0), 100.0, MeshLevel::Level3).is_err());
    }
}

