//! Bit-packed JIS X 0410 hierarchical grid (標準地域メッシュ) & Hilbert key.
//!
//! The Japanese Standard Regional Grid (JIS X 0410:2002) divides the region
//! `lat ∈ [0°, 66.66…°)`, `lon ∈ [100°E, 200°E)` into the following
//! hierarchy (all cell edges follow meridians and parallels — every cell is
//! a loxodromic rectangle):
//!
//! | Level | Name                  | Size (lat × lon) | Digits | Subdivision indices |
//! |-------|-----------------------|------------------|--------|---------------------|
//! | 1     | 1次メッシュ (~80 km)   | 40′ × 1°         | 4      | `p1` = floor(lat·1.5), `u1` = floor(lon) − 100 |
//! | 2     | 2次メッシュ (~10 km)  | 5′ × 7.5′        | 6      | `q1 q2` ∈ 0..=7 (south→north, west→east) |
//! | 3     | 3次メッシュ (~1 km)   | 30″ × 45″        | 8      | `r1 r2` ∈ 0..=9 (south→north, west→east) |
//! | 4     | 4次メッシュ (~500 m)  | 15″ × 22.5″      | 9      | `s` ∈ 1..=4 (SW=1, SE=2, NW=3, NE=4) |
//! | 5     | 5次メッシュ (~250 m)  | 7.5″ × 11.25″    | 10     | `t` ∈ 1..=4 |
//! | 6     | 6次メッシュ (~125 m)  | 3.75″ × 5.625″   | 11     | `u` ∈ 1..=4 |
//!
//! Per JIS X 0410:2002 the 2nd/3rd-mesh subdivision digits are **0-based**
//! (0–7 and 0–9, numbered south→north and west→east), while the 4th–6th
//! quarter digits are **1-based** with SW = 1, SE = 2, NW = 3, NE = 4. A
//! cell includes its south and west edges and excludes its north and east
//! edges (half-open `[south, north) × [west, east)`).
//!
//! [`JisMeshCode`] packs all fields into a single `u64` (37 bits used) with
//! `#[repr(transparent)]`: conversions are `O(1)`, allocation-free bitwise
//! operations. The conversion from latitude/longitude uses **exact integer
//! micro-arcsecond arithmetic** (`1 µas = 10⁻⁶″`), which eliminates the
//! `f64` boundary rounding bugs of naive implementations (e.g. a point at
//! exactly `35°40′00.000000″` lands in the northern adjacent cell, per the
//! half-open convention). [`HilbertKey`] maps each cell to a position on a
//! 2-D Hilbert space-filling curve over the level-6 base grid for
//! cache-local spatial indexing.

use crate::ellipsoid::{BoundingBox, Coord};
use core::fmt;

/// Error type for mesh-code construction and parsing.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MeshError {
    /// Latitude outside the JIS X 0410 domain `[0°, 66.66…°)`.
    LatitudeOutOfRange(f64),
    /// Longitude outside the JIS X 0410 domain `[100°E, 200°E)` (wrapped).
    LongitudeOutOfRange(f64),
    /// A code string or field combination is not structurally valid.
    InvalidCode,
    /// A subdivision digit is out of range for the requested level.
    InvalidSubdivision,
}

impl core::error::Error for MeshError {}

impl fmt::Display for MeshError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MeshError::LatitudeOutOfRange(v) => write!(f, "latitude {v}° outside [0, 66.66…)"),
            MeshError::LongitudeOutOfRange(v) => write!(f, "longitude {v}° outside [100, 200)"),
            MeshError::InvalidCode => write!(f, "invalid mesh code structure"),
            MeshError::InvalidSubdivision => write!(f, "subdivision digit out of range"),
        }
    }
}

/// The six standard JIS X 0410 mesh levels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(u8)]
pub enum MeshLevel {
    /// Level 1 — 1次メッシュ, 40′ × 1° (~80 km).
    Level1 = 1,
    /// Level 2 — 2次メッシュ, 5′ × 7.5′ (~10 km).
    Level2 = 2,
    /// Level 3 — 3次メッシュ / basic grid, 30″ × 45″ (~1 km).
    Level3 = 3,
    /// Level 4 — 4次メッシュ / half grid, 15″ × 22.5″ (~500 m).
    Level4 = 4,
    /// Level 5 — 5次メッシュ / quarter grid, 7.5″ × 11.25″ (~250 m).
    Level5 = 5,
    /// Level 6 — 6次メッシュ / eighth grid, 3.75″ × 5.625″ (~125 m).
    Level6 = 6,
}

impl MeshLevel {
    /// Level ordinal as `u8` (1..=6).
    #[inline]
    pub const fn number(self) -> u8 {
        self as u8
    }

    /// Cell height in micro-arcseconds of latitude.
    #[inline]
    pub const fn lat_micros(self) -> i64 {
        match self {
            MeshLevel::Level1 => LAT1_MICROS,
            MeshLevel::Level2 => LAT1_MICROS / 8,
            MeshLevel::Level3 => LAT1_MICROS / 80,
            MeshLevel::Level4 => LAT1_MICROS / 160,
            MeshLevel::Level5 => LAT1_MICROS / 320,
            MeshLevel::Level6 => LAT1_MICROS / 640,
        }
    }

    /// Cell width in micro-arcseconds of longitude.
    #[inline]
    pub const fn lon_micros(self) -> i64 {
        match self {
            MeshLevel::Level1 => LON1_MICROS,
            MeshLevel::Level2 => LON1_MICROS / 8,
            MeshLevel::Level3 => LON1_MICROS / 80,
            MeshLevel::Level4 => LON1_MICROS / 160,
            MeshLevel::Level5 => LON1_MICROS / 320,
            MeshLevel::Level6 => LON1_MICROS / 640,
        }
    }

    /// Approximate cell diagonal in meters at the given latitude (degrees).
    pub fn approx_size_meters(self, lat_deg: f64) -> f64 {
        const M_PER_DEG: f64 = 111_320.0;
        let lat_h = self.lat_micros() as f64 / MICRO_PER_ARCSEC as f64 / 3600.0;
        let lon_h = self.lon_micros() as f64 / MICRO_PER_ARCSEC as f64 / 3600.0;
        let h_north = lat_h * M_PER_DEG;
        let h_east = lon_h * M_PER_DEG * lat_deg.to_radians().cos();
        (h_north * h_north + h_east * h_east).sqrt()
    }

    /// Parses a level from its ordinal (1..=6).
    pub fn from_number(n: u8) -> Result<MeshLevel, MeshError> {
        match n {
            1 => Ok(MeshLevel::Level1),
            2 => Ok(MeshLevel::Level2),
            3 => Ok(MeshLevel::Level3),
            4 => Ok(MeshLevel::Level4),
            5 => Ok(MeshLevel::Level5),
            6 => Ok(MeshLevel::Level6),
            _ => Err(MeshError::InvalidCode),
        }
    }
}

/// Micro-arcseconds per arcsecond.
pub const MICRO_PER_ARCSEC: i64 = 1_000_000;
/// 40′ (level-1 cell height) in micro-arcseconds: 2 400 000 000.
const LAT1_MICROS: i64 = 40 * 60 * MICRO_PER_ARCSEC;
/// 1° (level-1 cell width) in micro-arcseconds: 3 600 000 000.
const LON1_MICROS: i64 = 3_600 * MICRO_PER_ARCSEC;
/// Longitude anchor of the JIS grid: 100°E in micro-arcseconds.
const LON_ANCHOR_MICROS: i64 = 100 * LON1_MICROS;
/// Northern domain limit: p1 ≤ 99 ⇒ lat < 99·40′ + 40′ = 66.66…°.
const LAT_MAX_MICROS: i64 = 100 * LAT1_MICROS;
/// Eastern domain limit: u1 ≤ 99 ⇒ lon < 200°.
const LON_MAX_MICROS: i64 = 200 * LON1_MICROS;

/// A bit-packed JIS X 0410 mesh code.
///
/// Layout (little-endian bit fields over a `u64`, 37 bits used):
///
/// | bits   | field | range |
/// |--------|-------|-------|
/// | 0–2    | level | 1..=6 |
/// | 3–9    | `p1`  | 0..=99 (latitude band index, floor(lat·1.5)) |
/// | 10–16  | `u1`  | 0..=99 (longitude band index, floor(lon) − 100) |
/// | 17–19  | `q1`  | 0..=7 |
/// | 20–22  | `q2`  | 0..=7 |
/// | 23–26  | `r1`  | 0..=9 |
/// | 27–30  | `r2`  | 0..=9 |
/// | 31–32  | `s`   | 0..=3 (quarter index; printed digit = +1: SW=1, SE=2, NW=3, NE=4) |
/// | 33–34  | `t`   | 0..=3 |
/// | 35–36  | `u`   | 0..=3 |
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(transparent)]
pub struct JisMeshCode(u64);

impl JisMeshCode {
    // Bit layout constants.
    const LEVEL_SHIFT: u32 = 0;
    const P1_SHIFT: u32 = 3;
    const U1_SHIFT: u32 = 10;
    const Q1_SHIFT: u32 = 17;
    const Q2_SHIFT: u32 = 20;
    const R1_SHIFT: u32 = 23;
    const R2_SHIFT: u32 = 27;
    const S_SHIFT: u32 = 31;
    const T_SHIFT: u32 = 33;
    const U_SHIFT: u32 = 35;

    /// Constructs a code from packed fields. `s`, `t`, `u` are the internal
    /// 0-based quarter indices (row-major from the south-west: 0 = SW,
    /// 1 = SE, 2 = NW, 3 = NE); the printed JIS digits are these + 1.
    #[inline]
    #[allow(clippy::too_many_arguments)]
    const fn pack(
        level: u64,
        p1: u64,
        u1: u64,
        q1: u64,
        q2: u64,
        r1: u64,
        r2: u64,
        s: u64,
        t: u64,
        u: u64,
    ) -> JisMeshCode {
        JisMeshCode(
            (level << Self::LEVEL_SHIFT)
                | (p1 << Self::P1_SHIFT)
                | (u1 << Self::U1_SHIFT)
                | (q1 << Self::Q1_SHIFT)
                | (q2 << Self::Q2_SHIFT)
                | (r1 << Self::R1_SHIFT)
                | (r2 << Self::R2_SHIFT)
                | (s << Self::S_SHIFT)
                | (t << Self::T_SHIFT)
                | (u << Self::U_SHIFT),
        )
    }

    #[inline]
    const fn level_ordinal(&self) -> u64 {
        (self.0 >> Self::LEVEL_SHIFT) & 0b111
    }

    /// The mesh level of this code.
    #[inline]
    pub const fn level(&self) -> MeshLevel {
        // Only levels 1..=6 are ever packed; the fallback arm is unreachable.
        match self.level_ordinal() {
            1 => MeshLevel::Level1,
            2 => MeshLevel::Level2,
            3 => MeshLevel::Level3,
            4 => MeshLevel::Level4,
            5 => MeshLevel::Level5,
            _ => MeshLevel::Level6,
        }
    }

    #[inline]
    const fn p1(&self) -> u64 {
        (self.0 >> Self::P1_SHIFT) & 0x7F
    }
    #[inline]
    const fn u1(&self) -> u64 {
        (self.0 >> Self::U1_SHIFT) & 0x7F
    }
    #[inline]
    const fn q1(&self) -> u64 {
        (self.0 >> Self::Q1_SHIFT) & 0b111
    }
    #[inline]
    const fn q2(&self) -> u64 {
        (self.0 >> Self::Q2_SHIFT) & 0b111
    }
    #[inline]
    const fn r1(&self) -> u64 {
        (self.0 >> Self::R1_SHIFT) & 0b1111
    }
    #[inline]
    const fn r2(&self) -> u64 {
        (self.0 >> Self::R2_SHIFT) & 0b1111
    }
    #[inline]
    const fn s(&self) -> u64 {
        (self.0 >> Self::S_SHIFT) & 0b11
    }
    #[inline]
    const fn t(&self) -> u64 {
        (self.0 >> Self::T_SHIFT) & 0b11
    }
    #[inline]
    const fn u(&self) -> u64 {
        (self.0 >> Self::U_SHIFT) & 0b11
    }

    /// Converts a latitude/longitude (degrees) to the mesh code at `level`.
    ///
    /// The computation uses exact integer micro-arcsecond arithmetic: the
    /// inputs are rounded to the nearest micro-arcsecond and all subsequent
    /// divisions are exact integer floors, eliminating `f64` boundary
    /// rounding bugs (a point at exactly `35°40′00.000000″` lands in the
    /// northern cell, per the half-open `[south, north)` convention).
    pub fn from_lat_lon(
        lat_deg: f64,
        lon_deg: f64,
        level: MeshLevel,
    ) -> Result<JisMeshCode, MeshError> {
        // Normalize longitude into [100, 200) by wrapping.
        let mut lon = lon_deg % 360.0;
        if lon < 0.0 {
            lon += 360.0;
        }
        while lon < 100.0 {
            lon += 360.0;
        }
        let lat_us = (lat_deg * 3600.0 * MICRO_PER_ARCSEC as f64).round() as i64;
        let lon_us = (lon * 3600.0 * MICRO_PER_ARCSEC as f64).round() as i64;
        Self::from_micros(lat_us, lon_us, level)
    }

    /// Builds a code from integer micro-arcsecond coordinates by exact floor
    /// division (the cell containing the point). The domain is
    /// `lat_us ∈ [0, 66.66…°)`, `lon_us ∈ [100°, 200°)` in micro-arcseconds.
    pub fn from_micros(lat_us: i64, lon_us: i64, level: MeshLevel) -> Result<JisMeshCode, MeshError> {
        if !(0..LAT_MAX_MICROS).contains(&lat_us) {
            return Err(MeshError::LatitudeOutOfRange(
                lat_us as f64 / (3600.0 * MICRO_PER_ARCSEC as f64),
            ));
        }
        if !(LON_ANCHOR_MICROS..LON_MAX_MICROS).contains(&lon_us) {
            return Err(MeshError::LongitudeOutOfRange(
                lon_us as f64 / (3600.0 * MICRO_PER_ARCSEC as f64),
            ));
        }
        let p1 = (lat_us / LAT1_MICROS) as u64;
        let rem_lat1 = lat_us % LAT1_MICROS;
        let u1 = ((lon_us - LON_ANCHOR_MICROS) / LON1_MICROS) as u64;
        let rem_lon1 = (lon_us - LON_ANCHOR_MICROS) % LON1_MICROS;

        let qsize = LAT1_MICROS / 8; // 5′
        let qlsize = LON1_MICROS / 8; // 7.5′
        let rsize = qsize / 10; // 30″
        let rlsize = qlsize / 10; // 45″
        let q1 = (rem_lat1 / qsize) as u64;
        let q2 = (rem_lon1 / qlsize) as u64;
        let rem_lat2 = rem_lat1 % qsize;
        let rem_lon2 = rem_lon1 % qlsize;
        let r1 = (rem_lat2 / rsize) as u64;
        let r2 = (rem_lon2 / rlsize) as u64;
        let rem_lat3 = rem_lat2 % rsize;
        let rem_lon3 = rem_lon2 % rlsize;

        // Quarter decomposition (row-major from SW): row bit from latitude,
        // column bit from longitude; index = 2·row + col.
        let half4 = LAT1_MICROS / 160; // 15″
        let halfw4 = LON1_MICROS / 160; // 22.5″
        let s = 2 * ((rem_lat3 / half4) as u64) + (rem_lon3 / halfw4) as u64;
        let rem_lat4 = rem_lat3 % half4;
        let rem_lon4 = rem_lon3 % halfw4;
        let half5 = LAT1_MICROS / 320; // 7.5″
        let halfw5 = LON1_MICROS / 320; // 11.25″
        let t = 2 * ((rem_lat4 / half5) as u64) + (rem_lon4 / halfw5) as u64;
        let rem_lat5 = rem_lat4 % half5;
        let rem_lon5 = rem_lon4 % halfw5;
        let half6 = LAT1_MICROS / 640; // 3.75″
        let halfw6 = LON1_MICROS / 640; // 5.625″
        let u = 2 * ((rem_lat5 / half6) as u64) + (rem_lon5 / halfw6) as u64;

        // Fields deeper than the requested level must be zero so that codes
        // compare equal regardless of the construction path.
        let (q1, q2, r1, r2, s, t, u) = match level.number() {
            1 => (0, 0, 0, 0, 0, 0, 0),
            2 => (q1, q2, 0, 0, 0, 0, 0),
            3 => (q1, q2, r1, r2, 0, 0, 0),
            4 => (q1, q2, r1, r2, s, 0, 0),
            5 => (q1, q2, r1, r2, s, t, 0),
            _ => (q1, q2, r1, r2, s, t, u),
        };

        Ok(Self::pack(level as u64, p1, u1, q1, q2, r1, r2, s, t, u))
    }

    /// Southern edge of the cell in micro-arcseconds of latitude.
    pub fn south_lat_micros(&self) -> i64 {
        let n = self.level_ordinal();
        let mut lat = self.p1() as i64 * LAT1_MICROS;
        if n >= 2 {
            lat += self.q1() as i64 * (LAT1_MICROS / 8);
        }
        if n >= 3 {
            lat += self.r1() as i64 * (LAT1_MICROS / 80);
        }
        if n >= 4 {
            lat += ((self.s() >> 1) & 1) as i64 * (LAT1_MICROS / 160);
        }
        if n >= 5 {
            lat += ((self.t() >> 1) & 1) as i64 * (LAT1_MICROS / 320);
        }
        if n >= 6 {
            lat += ((self.u() >> 1) & 1) as i64 * (LAT1_MICROS / 640);
        }
        lat
    }

    /// Western edge of the cell in micro-arcseconds of longitude.
    pub fn west_lon_micros(&self) -> i64 {
        let n = self.level_ordinal();
        let mut lon = LON_ANCHOR_MICROS + self.u1() as i64 * LON1_MICROS;
        if n >= 2 {
            lon += self.q2() as i64 * (LON1_MICROS / 8);
        }
        if n >= 3 {
            lon += self.r2() as i64 * (LON1_MICROS / 80);
        }
        if n >= 4 {
            lon += (self.s() & 1) as i64 * (LON1_MICROS / 160);
        }
        if n >= 5 {
            lon += (self.t() & 1) as i64 * (LON1_MICROS / 320);
        }
        if n >= 6 {
            lon += (self.u() & 1) as i64 * (LON1_MICROS / 640);
        }
        lon
    }

    /// The cell's bounding box in degrees (south/west included, north/east
    /// excluded — the half-open convention).
    pub fn to_bounds(&self) -> BoundingBox {
        let level = self.level();
        let scale = 3600.0 * MICRO_PER_ARCSEC as f64;
        let south = self.south_lat_micros() as f64 / scale;
        let west = self.west_lon_micros() as f64 / scale;
        BoundingBox::from_degrees(
            south,
            west,
            south + level.lat_micros() as f64 / scale,
            west + level.lon_micros() as f64 / scale,
        )
    }

    /// Center point of the cell (radians).
    pub fn to_center(&self) -> Coord {
        let b = self.to_bounds();
        Coord::from_degrees(0.5 * (b.south + b.north), 0.5 * (b.west + b.east))
    }

    /// The parent code (one level up); `None` at level 1.
    pub fn parent(&self) -> Option<JisMeshCode> {
        let n = self.level_ordinal();
        let code = match n {
            2 => Self::pack(1, self.p1(), self.u1(), 0, 0, 0, 0, 0, 0, 0),
            3 => Self::pack(2, self.p1(), self.u1(), self.q1(), self.q2(), 0, 0, 0, 0, 0),
            4 => Self::pack(
                3,
                self.p1(),
                self.u1(),
                self.q1(),
                self.q2(),
                self.r1(),
                self.r2(),
                0,
                0,
                0,
            ),
            5 => Self::pack(
                4,
                self.p1(),
                self.u1(),
                self.q1(),
                self.q2(),
                self.r1(),
                self.r2(),
                self.s(),
                0,
                0,
            ),
            6 => Self::pack(
                5,
                self.p1(),
                self.u1(),
                self.q1(),
                self.q2(),
                self.r1(),
                self.r2(),
                self.s(),
                self.t(),
                0,
            ),
            _ => return None,
        };
        Some(code)
    }

    /// The children of this code at the next level down.
    ///
    /// * Level 1 → 64 children (8 × 8),
    /// * Level 2 → 100 children (10 × 10),
    /// * Levels 3–5 → 4 quadrant children each,
    /// * Level 6 → empty.
    pub fn children(&self) -> Vec<JisMeshCode> {
        let n = self.level_ordinal();
        let mut out = Vec::new();
        match n {
            1 => {
                for q1 in 0..8u64 {
                    for q2 in 0..8u64 {
                        out.push(Self::pack(2, self.p1(), self.u1(), q1, q2, 0, 0, 0, 0, 0));
                    }
                }
            }
            2 => {
                for r1 in 0..10u64 {
                    for r2 in 0..10u64 {
                        out.push(Self::pack(
                            3,
                            self.p1(),
                            self.u1(),
                            self.q1(),
                            self.q2(),
                            r1,
                            r2,
                            0,
                            0,
                            0,
                        ));
                    }
                }
            }
            3 => {
                for s in 0..4u64 {
                    out.push(Self::pack(
                        4,
                        self.p1(),
                        self.u1(),
                        self.q1(),
                        self.q2(),
                        self.r1(),
                        self.r2(),
                        s,
                        0,
                        0,
                    ));
                }
            }
            4 => {
                for t in 0..4u64 {
                    out.push(Self::pack(
                        5,
                        self.p1(),
                        self.u1(),
                        self.q1(),
                        self.q2(),
                        self.r1(),
                        self.r2(),
                        self.s(),
                        t,
                        0,
                    ));
                }
            }
            5 => {
                for u in 0..4u64 {
                    out.push(Self::pack(
                        6,
                        self.p1(),
                        self.u1(),
                        self.q1(),
                        self.q2(),
                        self.r1(),
                        self.r2(),
                        self.s(),
                        self.t(),
                        u,
                    ));
                }
            }
            _ => {}
        }
        out
    }

    /// The 8 surrounding cells (Moore neighborhood) at the same level.
    ///
    /// The computation is performed in a flat integer index space (row/col
    /// of micro-arcsecond cells anchored at 100°E / 0°N), so the neighborhood
    /// is **exact across 1st/2nd/3rd-mesh digit boundaries** — e.g. the
    /// north neighbor of `q1 = 7` carries into the next `p1` band correctly.
    /// Neighbor cells that fall outside the JIS domain are skipped.
    pub fn neighbors_8(&self) -> Vec<JisMeshCode> {
        let level = self.level();
        let row = self.south_lat_micros() / level.lat_micros();
        let col = (self.west_lon_micros() - LON_ANCHOR_MICROS) / level.lon_micros();
        let mut out = Vec::with_capacity(8);
        for drow in -1i64..=1 {
            for dcol in -1i64..=1 {
                if drow == 0 && dcol == 0 {
                    continue;
                }
                let nrow = row + drow;
                let ncol = col + dcol;
                if nrow < 0 || ncol < 0 {
                    continue;
                }
                let nlat = nrow * level.lat_micros();
                let nlon = LON_ANCHOR_MICROS + ncol * level.lon_micros();
                if nlat >= LAT_MAX_MICROS || nlon >= LON_MAX_MICROS {
                    continue;
                }
                if let Ok(code) = Self::from_micros(nlat, nlon, level) {
                    out.push(code);
                }
            }
        }
        out
    }

    /// The Hilbert curve key over the level-6 base grid.
    ///
    /// The cell's south-west corner is mapped to the level-6 cell indices
    /// `(col6, row6)`, which are interleaved along a 15-bit × 15-bit Hilbert
    /// curve (30-bit key). Codes that are spatially close receive nearby
    /// keys, improving cache locality in spatial indexes.
    pub fn to_hilbert64(&self) -> HilbertKey {
        let row6 = (self.south_lat_micros() / (LAT1_MICROS / 640)) as u32;
        let col6 = ((self.west_lon_micros() - LON_ANCHOR_MICROS) / (LON1_MICROS / 640)) as u32;
        // The level-6 longitude grid spans 64 000 columns (100° / 5.625″),
        // so the Hilbert square must be 2^16 × 2^16 (32-bit key).
        HilbertKey(hilbert_xy_to_d(1 << 16, col6, row6))
    }

    /// Parses the standard digit string (e.g. `"53393599"`, `"533935991"`).
    ///
    /// Accepts 4, 6, 8, 9, 10, or 11 digits. For levels 4–6 the final digit
    /// is 1-based (1..=4) per JIS X 0410.
    pub fn from_code(code: &str) -> Result<JisMeshCode, MeshError> {
        let digits: Vec<u8> = code
            .chars()
            .map(|c| c.to_digit(10).map(|d| d as u8).ok_or(MeshError::InvalidCode))
            .collect::<Result<_, _>>()?;
        let n = digits.len();
        if !matches!(n, 4 | 6 | 8 | 9 | 10 | 11) {
            return Err(MeshError::InvalidCode);
        }
        let d = |i: usize| -> Result<u64, MeshError> {
            digits
                .get(i)
                .map(|&v| v as u64)
                .ok_or(MeshError::InvalidCode)
        };
        let p1 = d(0)? * 10 + d(1)?;
        let u1 = d(2)? * 10 + d(3)?;
        if p1 > 99 || u1 > 99 {
            return Err(MeshError::InvalidCode);
        }
        let level = match n {
            4 => MeshLevel::Level1,
            6 => MeshLevel::Level2,
            8 => MeshLevel::Level3,
            9 => MeshLevel::Level4,
            10 => MeshLevel::Level5,
            _ => MeshLevel::Level6,
        };
        let sub = |i: usize| -> Result<u64, MeshError> {
            d(i)?.checked_sub(1).ok_or(MeshError::InvalidSubdivision)
        };
        let (q1, q2, r1, r2, s, t, u) = match n {
            4 => (0, 0, 0, 0, 0, 0, 0),
            6 => (d(4)?, d(5)?, 0, 0, 0, 0, 0),
            8 => (d(4)?, d(5)?, d(6)?, d(7)?, 0, 0, 0),
            9 => (d(4)?, d(5)?, d(6)?, d(7)?, sub(8)?, 0, 0),
            10 => (d(4)?, d(5)?, d(6)?, d(7)?, sub(8)?, sub(9)?, 0),
            _ => (d(4)?, d(5)?, d(6)?, d(7)?, sub(8)?, sub(9)?, sub(10)?),
        };
        if q1 > 7 || q2 > 7 || r1 > 9 || r2 > 9 || s > 3 || t > 3 || u > 3 {
            return Err(MeshError::InvalidSubdivision);
        }
        Ok(Self::pack(level as u64, p1, u1, q1, q2, r1, r2, s, t, u))
    }

    /// The raw packed `u64` (for serialization).
    #[inline]
    pub const fn bits(self) -> u64 {
        self.0
    }

    /// Rebuilds a code from [`JisMeshCode::bits`].
    #[inline]
    pub const fn from_bits(bits: u64) -> JisMeshCode {
        JisMeshCode(bits)
    }
}

impl fmt::Display for JisMeshCode {
    /// Standard digit representation. Levels 1–3 print the 0-based
    /// subdivision digits verbatim; levels 4–6 print their quarter digit as
    /// the 1-based JIS value (1..=4).
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:02}{:02}", self.p1(), self.u1())?;
        let n = self.level_ordinal();
        if n >= 2 {
            write!(f, "{:02}", self.q1() * 10 + self.q2())?;
        }
        if n >= 3 {
            write!(f, "{:02}", self.r1() * 10 + self.r2())?;
        }
        if n >= 4 {
            write!(f, "{}", self.s() + 1)?;
        }
        if n >= 5 {
            write!(f, "{}", self.t() + 1)?;
        }
        if n >= 6 {
            write!(f, "{}", self.u() + 1)?;
        }
        Ok(())
    }
}

impl fmt::Debug for JisMeshCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "JisMeshCode({}, L{})", self, self.level_ordinal())
    }
}

/// Position of a mesh cell on the 30-bit Hilbert space-filling curve over the
/// level-6 base grid.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(transparent)]
pub struct HilbertKey(pub u64);

/// Standard iterative Hilbert `xy → d` mapping on a `2^k × 2^k` grid
/// (n = 2^k). Based on the canonical algorithm (Skilling / Wikipedia).
fn hilbert_xy_to_d(n: u32, mut x: u32, mut y: u32) -> u64 {
    let mut d = 0u64;
    let mut s = n / 2;
    while s > 0 {
        let rx = ((x & s) != 0) as u64;
        let ry = ((y & s) != 0) as u64;
        d += (s as u64) * (s as u64) * ((3 * rx) ^ ry);
        // rot(n, &x, &y, rx, ry): reflect and swap the lower quadrant.
        if ry == 0 {
            if rx == 1 {
                x = n - 1 - x;
                y = n - 1 - y;
            }
            core::mem::swap(&mut x, &mut y);
        }
        s /= 2;
    }
    d
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn r(lat: f64, lon: f64, level: MeshLevel) -> JisMeshCode {
        JisMeshCode::from_lat_lon(lat, lon, level).expect("valid coords")
    }

    #[test]
    fn known_codes() {
        // Tokyo Tower (35.658581, 139.745433) — reference value from the
        // widely-used jismesh library (JIS X 0410:2002 compliant).
        assert_eq!(r(35.658581, 139.745433, MeshLevel::Level3).to_string(), "53393599");
        // Tokyo 1st mesh = 5339.
        assert_eq!(r(35.6812, 139.7671, MeshLevel::Level1).to_string(), "5339");
        // Tokyo Station 2nd mesh = 533946.
        assert_eq!(r(35.6812, 139.7671, MeshLevel::Level2).to_string(), "533946");
    }

    #[test]
    fn boundary_35_40_exact() {
        // A point at exactly 35°40′00.000000″ must fall in the *northern*
        // cell of the 40′-band sub-boundary (half-open [south, north)):
        // 35°40′ = 53·40′ + 4·5′ exactly ⇒ q1 = 4 (0-based).
        let lat = 35.0 + 40.0 / 60.0;
        let code = r(lat, 139.5, MeshLevel::Level2);
        assert_eq!(code.p1(), 53);
        assert_eq!(code.q1(), 4);
        assert_eq!(code.to_string(), "533944");
    }

    #[test]
    fn micro_arcsecond_exactness() {
        // 35°40′ in micro-arcseconds is exactly 128 400 000 000.
        let lat = 35.0 + 40.0 / 60.0;
        let us = (lat * 3600.0 * 1_000_000.0f64).round() as i64;
        assert_eq!(us, 128_400_000_000);
        // Level-3 cell height = 30″ = 30 000 000 µas: exact integer result.
        // The point sits at q1 = 4 (4·5′ above the 40′ band base) and r1 = 0.
        let q1 = (us % (40 * 60 * MICRO_PER_ARCSEC)) / (5 * 60 * MICRO_PER_ARCSEC);
        let r1 = ((us % (40 * 60 * MICRO_PER_ARCSEC)) % (5 * 60 * MICRO_PER_ARCSEC))
            / 30_000_000;
        assert_eq!(q1, 4);
        assert_eq!(r1, 0);
    }

    #[test]
    fn bounds_center_roundtrip() {
        for &(lat, lon) in &[
            (35.658581, 139.745433),
            (34.7, 135.5),
            (43.06, 141.35),
            (26.2, 127.7),
        ] {
            for level in [
                MeshLevel::Level1,
                MeshLevel::Level2,
                MeshLevel::Level3,
                MeshLevel::Level4,
                MeshLevel::Level5,
                MeshLevel::Level6,
            ] {
                let code = r(lat, lon, level);
                let b = code.to_bounds();
                // the query point lies in the half-open box
                assert!(b.south <= lat && lat < b.north, "{lat} not in {b:?}");
                assert!(b.west <= lon && lon < b.east, "{lon} not in {b:?}");
                let c = code.to_center();
                let code2 = r(c.lat_degrees(), c.lon_degrees(), level);
                assert_eq!(code, code2, "center roundtrip failed at {level:?}");
            }
        }
    }

    #[test]
    fn parent_children_consistency() {
        let code = r(35.658581, 139.745433, MeshLevel::Level6);
        let p5 = code.parent().unwrap();
        assert_eq!(p5.level(), MeshLevel::Level5);
        let p4 = p5.parent().unwrap();
        let p3 = p4.parent().unwrap();
        assert_eq!(p3.to_string(), "53393599");
        let p2 = p3.parent().unwrap();
        assert_eq!(p2.level(), MeshLevel::Level2);
        let p1 = p2.parent().unwrap();
        assert_eq!(p1.to_string(), "5339");
        assert!(p1.parent().is_none());
        // children of the level-3 ancestor: 4 level-4 children containing us
        let kids = p3.children();
        assert_eq!(kids.len(), 4);
        assert!(kids.contains(&p4));
        // level 1 → 64 children; level 2 → 100 children
        assert_eq!(p1.children().len(), 64);
        assert_eq!(p2.children().len(), 100);
        assert!(r(35.0, 139.0, MeshLevel::Level6).children().is_empty());
    }

    #[test]
    fn neighbors_across_mesh_boundaries() {
        // Cell at q1 = 7 (northernmost 2nd-mesh band within p1 = 53): its
        // north neighbor must carry into the next 40′ band (p1 = 54, q1 = 0).
        let lat = 53.0 * (2.0 / 3.0) + 7.0 * (5.0 / 60.0) + 0.01;
        let code = r(lat, 139.5, MeshLevel::Level2);
        assert_eq!(code.q1(), 7);
        let north = code
            .neighbors_8()
            .into_iter()
            .find(|c| c.south_lat_micros() > code.south_lat_micros() + code.level().lat_micros() / 2)
            .expect("north neighbor");
        assert_eq!(north.p1(), 54);
        assert_eq!(north.q1(), 0);
        // Same for the 3rd-mesh boundary (r1 = 9 → q1 advances, r1 = 0).
        let lat3 = 53.0 * (2.0 / 3.0) + 7.0 * (5.0 / 60.0) + 9.0 * (30.0 / 3600.0) + 0.001;
        let code3 = r(lat3, 139.5, MeshLevel::Level3);
        assert_eq!(code3.r1(), 9);
        let north3 = code3
            .neighbors_8()
            .into_iter()
            .find(|c| c.south_lat_micros() > code3.south_lat_micros() + code3.level().lat_micros() / 2)
            .expect("north neighbor");
        // The carry crosses the q1=7 band boundary: p1 advances, q1 wraps.
        assert_eq!(north3.p1(), 54);
        assert_eq!(north3.q1(), 0);
        assert_eq!(north3.r1(), 0);
        // Longitude carry: q2 = 7 → east neighbor advances u1, q2 = 0.
        let lon_e = 139.0 + 7.0 * 0.125 + 0.01;
        let code_e = r(35.5, lon_e, MeshLevel::Level2);
        assert_eq!(code_e.q2(), 7);
        let east = code_e
            .neighbors_8()
            .into_iter()
            .find(|c| c.west_lon_micros() > code_e.west_lon_micros() + code_e.level().lon_micros() / 2)
            .expect("east neighbor");
        assert_eq!(east.u1(), code_e.u1() + 1);
        assert_eq!(east.q2(), 0);
        assert_eq!(code_e.neighbors_8().len(), 8);
    }

    #[test]
    fn code_parse_roundtrip() {
        for &(lat, lon) in &[(35.658581, 139.745433), (34.69, 135.5), (43.06, 141.35)] {
            for level in [
                MeshLevel::Level1,
                MeshLevel::Level2,
                MeshLevel::Level3,
                MeshLevel::Level4,
                MeshLevel::Level5,
                MeshLevel::Level6,
            ] {
                let code = r(lat, lon, level);
                let parsed = JisMeshCode::from_code(&code.to_string()).expect("parse");
                assert_eq!(code, parsed, "roundtrip failed for {code} at {level:?}");
            }
        }
        assert!(JisMeshCode::from_code("533").is_err());
        assert!(JisMeshCode::from_code("5339359x").is_err());
        assert!(JisMeshCode::from_code("533935995").is_err()); // s digit 5 invalid
    }

    #[test]
    fn hilbert_key_properties() {
        // Injectivity on a sample of cells and locality for adjacent cells.
        let mut keys = Vec::new();
        for lat in [35.0, 35.01, 36.0, 34.0, 43.0, 26.5] {
            for lon in [139.0, 139.01, 138.0, 140.0, 141.3, 127.8] {
                keys.push(r(lat, lon, MeshLevel::Level6).to_hilbert64().0);
            }
        }
        let mut uniq = keys.clone();
        uniq.sort_unstable();
        uniq.dedup();
        assert_eq!(uniq.len(), keys.len(), "hilbert keys must be injective on the sample");
        // Adjacent cells (north-south step ~110 m apart) have close keys.
        let base = keys[0];
        assert!((keys[1] as i64 - base as i64).abs() < 8192, "locality violated");
    }

    #[test]
    fn domain_errors() {
        assert!(JisMeshCode::from_lat_lon(67.0, 139.0, MeshLevel::Level1).is_err());
        assert!(JisMeshCode::from_lat_lon(-1.0, 139.0, MeshLevel::Level1).is_err());
        assert!(JisMeshCode::from_lat_lon(35.0, 50.0, MeshLevel::Level1).is_err());
        // −175° wraps to 185° → valid; 30° wraps to 390° → invalid.
        assert!(JisMeshCode::from_lat_lon(35.0, -175.0, MeshLevel::Level1).is_ok());
        assert!(JisMeshCode::from_lat_lon(35.0, 30.0, MeshLevel::Level1).is_err());
    }
}
