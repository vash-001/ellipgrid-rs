[English](README.md) | 日本語

# ellipgrid

**GRS80/WGS84 楕円体上の測地線の厳密解・等積多角形多様体・JIS X 0410 / ヒルベルト曲線による位相的に厳密な空間カバーを Rust で実装**

本番品質の `#![forbid(unsafe_code)]`、依存クレートゼロの Rust クレート。以下を提供します:

* 楕円体上の測地線の Direct / Inverse 問題の厳密解（距離・順/逆方位角・縮距 m12）
* Karney の `C4` ガウス・ボネ級数による楕円体多角形の面積、および独立した等積（オーサリック）多様体 Green 積分エンジン
* Shewchuk 流れの厳密な幾何述語（誤差なし浮動小数点変換）
* JIS X 0410（標準地域メッシュ）の完全なビットパック実装 — 厳密な整数マイクロ秒（µas）演算、`O(1)` 位相クエリ、ヒルベルト空間充填曲線キー
* 測地距離円盤および多角形→メッシュの被覆エンジン

```toml
[dependencies]
ellipgrid = "0.1"
```

---

## 1. 数学的基礎

### 1.1 基準楕円体（`ellipsoid`）

両楕円体とも $a = 6\,378\,137.0$ m を共有します:

| 楕円体 | 扁平率 $f$ | 利用分野 |
|---|---|---|
| GRS80 | $1/298.257222101$ | JGD2011 / JGD2024（日本） |
| WGS84 | $1/298.257223563$ | GPS |

導出定数は const 文脈で厳密に計算されます:

$$b = a(1-f),\qquad e^2 = f(2-f),\qquad e'^2 = \frac{e^2}{1-e^2},\qquad n = \frac{f}{2-f}.$$

曲率半径:

$$M(\varphi) = \frac{a(1-e^2)}{(1-e^2\sin^2\varphi)^{3/2}},\qquad
N(\varphi) = \frac{a}{\sqrt{1-e^2\sin^2\varphi}}.$$

補助緯度:

* **離心緯度（_PARAMETRIC_）** $\beta = \arctan\!\big((1-f)\tan\varphi\big)$
  （`atan2` により分岐的に安定な厳密計算）、
* **等積緯度（_AUTHALIC_）** $\xi$ — 2 つの経路を実装:
  1. $n$ に関する 6 次フーリエ級数
     $\;\xi(\varphi) = \varphi + \sum_{j=1}^{6} n^j\sum_{k=1}^{j} C[k][j]\,\sin(2k\varphi)$
     （係数は厳密な定義から記号的に導出。拡張精度では $\lesssim 10^{-18}$ rad で一致し、
     残差は参照経路の `f64` `asin` 丸め誤差にのみ支配されます）、
  2. 厳密な $q$ 定義
     $\sin\xi = q(\varphi)/q(\tfrac{\pi}{2})$ ただし
     $q(\varphi) = (1-e^2)\left[\tfrac{\sin\varphi}{1-e^2\sin^2\varphi} + \tfrac{\mathrm{atanh}(e\sin\varphi)}{e}\right]$
     をニュートン反復で精密化、
* **等長緯度（_ISOMETRIC_）** $\psi(\varphi) = \mathrm{atanh}(\sin\varphi) - e\,\mathrm{atanh}(e\sin\varphi)$
  （等方位線＝ロクソドロムは $(\lambda,\psi)$ 平面で直線になります）、
* **子午線弧長** $m(\varphi)$（Snyder 3-21、$e^2$ の 6 次級数）とニュートン逆演算。

等積半径の 2 乗 $R_A^2 = \tfrac{a^2}{2}\big(1 + \tfrac{1-e^2}{e}\mathrm{atanh}\, e\big)$
は全表面積の尺度です: 楕円体の表面積は厳密に $4\pi R_A^2$ と等しい（テストスイートで解析的・数値的に検証済み）。

### 1.2 測地線（`geodesic`）

**Inverse 問題** — `inverse(p1, p2) -> {s12, azi1, azi2, m12}`:

1. **ヘルメルト／ヴィンセンティ固定点反復**（補助球経度、$O(f^4)$ 級数）、
   $|\Delta\lambda| < 10^{-14}$ rad に収束（≤ 64 ステップ）。
2. **Karney 級数評価**（$\varepsilon$ の 6 次フーリエ級数。ただし
   $\varepsilon = k^2/(2(1+\sqrt{1+k^2})+k^2)$、$k^2 = \cos^2\alpha_0\,e'^2$）:

$$s_{12} = b\,A_1\big(\sigma_{12} + \Delta B_1\big),\qquad
m_{12} = b\Big[\mathrm{dn}_2(\cos\sigma_1\sin\sigma_2) - \mathrm{dn}_1(\sin\sigma_1\cos\sigma_2) - \cos\sigma_1\cos\sigma_2\,J_{12}\Big]$$

   ここで $J_{12} = (A_1-A_2)\sigma_{12} + A_1\Delta B_1 - A_2\Delta B_2$、
   $\mathrm{dn}_i = \sqrt{1+e'^2\sin^2\beta_i}$、
   節点からの弧は $\sin\sigma_i = \sin\beta_i/\cos\alpha_0$、
   $\cos\sigma_i = \cos\alpha_i\cos\beta_i/\cos\alpha_0$。
   ヴィンセンティの ≈0.5 mm から ≲10⁻⁸ m へ精度を引き上げます。

3. **ニュートン・ラフソン対蹠点フォールバック** — $|\lambda_{12}| > \pi - 10^{-3}$ の場合や
   ヴィンセンティ反復が 64 ステップで収束しない場合に起動: Direct 問題をドライバとする
   $(\alpha_1, s_{12})$ の 2 次元ダンプ付きニュートン・ラフソン（数値ヤコビアン、レーヴェンバーグ
   バックトラッキング）、8 つの初期値（球面解、ヴィンセンティの最終反復値、4 方位角 × 子午線／赤道半周弧）。
   収束解の中から **最短弧** を選択します — 射撃方程式には対蹠点を 1 周超えて到達するより長い解も
   存在するためです。

**Direct 問題** — `direct(p1, azi1, s12)`: 補助弧を同じ 6 次級数の不動点
$\sigma_{12} = \tfrac{s_{12}}{b A_1} - \Delta B_1(\sigma_{12})$ で距離から復元し、弧の中点での
ヴィンセンティ経度補正を施して補助球から地図座標へ変換します。

**赤道特殊ケース**（両端点が赤道上）: $s_{12} = a|\lambda_{12}|$（厳密）、
$m_{12} = b\sin(|\lambda_{12}|\,a/b)$。

#### 検証

| 基準 | 結果 |
|---|---|
| GeographicLib `GeodTest` ゴールドセット（対蹠点近傍を含む WGS84 の 20 ケース） | $s_{12}$、$m_{12}$: $<10^{-4}$ m、方位角 $<10^{-8}$° |
| **国土地理院 測量計算サイト**、GRS80、実機検証 | 4 ケース（東京–大阪、–札幌、–那覇、–デリー）: 距離 **±0.5 mm**、順方位角 **±0.005″** |
| 厳密に対蹠点のペア（GeographicLib 検証済み） | 最短弧の選択は正しく、往復閉合 $<10^{-12}$ rad |

> **規約について。** `azi2` は到着点における *順* 方位角（進行方向 — 国際的／GeographicLib の規約）です。
> 国土地理院のサイトは *逆* 方位角（到着点 → 出発点）を表示するため、両者は厳密に 180° 異なります。

### 1.3 健全な幾何述語（`predicates`）

誤差なし変換（EFT）: `two_sum`（Knuth）、`fast_two_sum`、Dekker の `split`、`two_product`
（FMA ベース。`p + e == a·b` が厳密に成立）。展開（expansion）演算（昇順・非重畳成分、
`grow_expansion`、`scale_expansion`、厳密和）により、適応型の述語を実装:

* `orient2d` / `orient2d_exact` — Shewchuk のフィルタ限界 $(3+16\varepsilon)\varepsilon\cdot\text{perm}$、
* `incircle` / `incircle_exact` — フィルタ限界 $(10+96\varepsilon)\varepsilon^2\cdot\text{perm}$。

攻撃的なテストには、素の `f64` 行列式が厳密に 0 になるのに対し真の行列式が非ゼロとなる
「ほぼ退化した平行移動構成」が含まれます（位相的偽陽性・偽陰性ゼロ）。

### 1.4 多角形の面積・周囲長・包含判定（`polygon`）

**測地線辺** または **等方位線（ロクソドロム）辺** で閉じたリングを扱います。

**面積 — Karney `C4` ガウス・ボネ エンジン**（測地線辺）: 辺ごとの面積スウェイス

$$S_{12} = A_4\big(B_4(\sigma_2) - B_4(\sigma_1)\big) + R_A^2\,(\alpha_2 - \alpha_1),\qquad
A_4 = a^2\cos\alpha_0\sin\alpha_0\,e^2,$$

6 次の `C4` フーリエ余弦級数（係数は GeographicLib 準拠、$n$ の多項式を $\varepsilon$ で評価）。
閉リング上で $S_{12}$ を総和すると囲まれた面積になります（生の和は時計回り正の規約のため、
本クレートの CCW 正 API では符号を反転）。GeographicLib v2.1 と **1e-10 相対** で一致することを
検証済み。極に頂点を持つリングでは素の和は退化します（GeographicLib と同様に 0 を返す）ため、
自動的に Green エンジンへ切り替えます。

**面積 — 等積多様体 Green エンジン**（すべての辺種別）: 等積座標系での厳密な面積要素は
$dA = R_A^2\cos\xi\,d\xi\,d\lambda$ なので、

$$A = -R_A^2\oint \sin\xi(\varphi)\,d\lambda$$

を辺ごとに 32 節点ガウス・ルジャンドル求積で評価します（ロクソドロム辺は等長緯度で厳密に
パラメータ化、測地線辺は解析的被積分関数
$d\lambda/dt = s_{12}\sin\alpha(t)/(N(\varphi)\cos\varphi)$、
$\sin\alpha(t) = \sin\alpha_0/\cos\beta(t)$（Clairaut）を使用）。極頂点は経度ジャンプ項
$\sin\xi(\pm\tfrac{\pi}{2})\,\Delta\lambda$ を厳密に寄与させ、解析的な象限三角形の値
$A = \pi R_A^2$ を 1e-9 相対で再現します。

**周囲長** — 測地線辺は Inverse 問題によります。ロクソドロム辺は厳密に
$\Delta m/\cos\alpha$、$\tan\alpha = \Delta\lambda/\Delta\psi$（平行線上は $N\cos\varphi\,\Delta\lambda$）。

**包含判定** — クエリ点に対して経度をアンラップし（反経線対応）、+λ 方向へのレイキャスティング:
ロクソドロム辺は等長平面で `orient2d_exact` を用いて **厳密** に判定（境界を含む、
位相的偽陽性・偽陰性ゼロ）。測地線辺は真の弧の再帰的二分割（~1e-12 rad で堅牢）と、
測地距離三角不等式による厳密な境界判定を併用します。

### 1.5 JIS X 0410 メッシュコード（`jismesh`）

| 次数 | 名称 | 大きさ（緯度 × 経度） | 桁数 | 分割 |
|---|---|---|---|---|
| 1 | 1次メッシュ (~80 km) | 40′ × 1° | 4 | `p1` = ⌊lat·1.5⌋, `u1` = ⌊lon⌋ − 100 |
| 2 | 2次メッシュ (~10 km) | 5′ × 7.5′ | 6 | `q1 q2` ∈ 0..8（南→北、西→東） |
| 3 | 3次メッシュ (~1 km) | 30″ × 45″ | 8 | `r1 r2` ∈ 0..10（南→北、西→東） |
| 4 | 4次メッシュ (~500 m) | 15″ × 22.5″ | 9 | `s` ∈ 1..5（SW=1, SE=2, NW=3, NE=4） |
| 5 | 5次メッシュ (~250 m) | 7.5″ × 11.25″ | 10 | `t` ∈ 1..5 |
| 6 | 6次メッシュ (~125 m) | 3.75″ × 5.625″ | 11 | `u` ∈ 1..5 |

JIS X 0410:2002 に従い、2次・3次メッシュの等分番号は **0 起点**、4〜6次メッシュの分割番号は
**1 起点**（SW=1, SE=2, NW=3, NE=4）です。セルは半開区間 `[南, 北) × [西, 東)` として扱われます。

`JisMeshCode(u64)` は `#[repr(transparent)]` の 37 ビットパックで、すべての変換が `O(1)` かつ
アロケーションフリーです。`from_lat_lon` は最寄りの **マイクロ秒角**（1 µas = 10⁻⁶″）へ丸めた上で
厳密な整数切り捨て除算を行うため、`f64` の境界丸めバグが構造的に発生しません
（厳密に $35°40′00.000000″$ の点は、すべての次数で北側のセルに分類されます）。

位相操作: `parent`、`children`（64/100/4）、`neighbors_8` — 平坦なマイクロ秒角インデックス空間で
計算するため、ムーア近傍は **1次／2次／3次メッシュの桁の繰り上がりを跨いでも厳密** です。
`to_hilbert64` は 6次メッシュ基準グリッド上の 32 ビット・ヒルベルト曲線キーを返し、
キャッシュ局所性の高い空間インデックスを実現します。

### 1.6 被覆エンジン（`cover`）

**`cover_geodesic_disk(center, radius, level)`** — 厳密な測地距離円盤と交わる閉包矩形を持つ
セルの最小集合:

1. 緯度範囲 $[\varphi_s, \varphi_n]$ は子午線弧長の逆演算で **厳密** に求めます（$M(\varphi)$ を反映）、
2. 各メッシュ行の経度半幅 $\Delta\lambda(\varphi)$ は Direct 問題の方位角二分割で **厳密** に求めます
   （経線収束と $\cos\varphi$ による東西縮小を反映）、
3. 厳密な整数マイクロ秒角インデックス空間で列挙し、ソート・重複除去して出力します。

**`cover_polygon(poly, level)`** — バウンディングボックスの行／列スイープ、辺 bbox による高速棄却、
内部セルの厳密判定（境界セグメントと交わらない連結なセル矩形は完全に内側か外側かのいずれか —
1 回の包含判定で確定）、角点の包含判定、展開演算による向き述語を用いたロクソドロム
セグメント×矩形交差判定（測地線辺は短い弦へ保守的にサンプリング）。

---

## 2. 計算量

| 操作 | 計算量 | 備考 |
|---|---|---|
| `from_lat_lon` / `to_bounds` / `to_center` | $O(1)$ | 整数マイクロ秒角演算、アロケーションなし |
| `parent` / `children` / `to_hilbert64` | $O(1)$ | ビット演算 |
| `neighbors_8` | $O(1)$ | 8 つの厳密なインデックス空間構築 |
| inverse（ヴィンセンティ経路） | ≤ 64 不動点ステップ × $O(1)$ | ≈ 20 flops/ステップ |
| inverse（対蹠点フォールバック） | ≤ 8 開始点 × 60 ニュートンステップ × direct | 対蹠点近傍のみ |
| direct | ≤ 24 不動点ステップ | 級数評価 |
| 多角形面積（C4） | $O(n)$ 辺 × 級数 | 辺ごとの閉形式 |
| 多角形面積（Green） | $O(n)$ 辺 × 32 節点 GL | 機械精度の求積 |
| `contains`（ロクソドロム） | $O(n)$ 回の厳密述語評価 | アロケーションなし |
| 円盤被覆 | 行 × 二分割（80 direct）+ セル | 行ごとの厳密な範囲 |
| 多角形被覆 | bbox セル × 辺判定 | まず bbox 高速棄却 |

---

## 3. 使用例

```rust
use ellipgrid::{Ellipsoid, Coord};
use ellipgrid::geodesic::{inverse, direct};

let e = Ellipsoid::GRS80;
let tokyo  = Coord::from_degrees(35.6812, 139.7671);
let osaka  = Coord::from_degrees(34.7024, 135.4959);

let inv = inverse(&e, tokyo, osaka);
println!("distance = {} m", inv.s12);          // GSI に対して ±0.5 mm
println!("azi1 = {}°, azi2 = {}°", inv.azi1.to_degrees(), inv.azi2.to_degrees());
println!("reduced length = {} m", inv.m12);

let d = direct(&e, tokyo, inv.azi1, 100_000.0);
```

```rust
use ellipgrid::jismesh::{JisMeshCode, MeshLevel};

let code = JisMeshCode::from_lat_lon(35.658581, 139.745433, MeshLevel::Level3)?;
assert_eq!(code.to_string(), "53393599");      // 東京タワー、3次メッシュ

let neighbors = code.neighbors_8();            // メッシュ境界を跨ぐ近傍も厳密
let key = code.to_hilbert64();                 // キャッシュ局所性の高い空間インデックス
```

```rust
use ellipgrid::polygon::{EllipsoidalPolygon, EdgeKind};

let japan_poly = EllipsoidalPolygon::new(vec![/* リング */], EdgeKind::Rhumb)?;
let area = japan_poly.area(&e);                // 符号付き m²、CCW で正
assert!(japan_poly.contains(&e, Coord::from_degrees(36.0, 138.0)));
```

```rust
use ellipgrid::cover::cover_geodesic_disk;

let cells = cover_geodesic_disk(&e, tokyo, 5000.0, MeshLevel::Level6)?;
// 5 km の測地距離円盤と交わる ~125 m セルの厳密な最小集合
```

---

## 4. 検証サマリ

| 性質 | 検証基準 | 一致度 |
|---|---|---|
| Inverse 距離（WGS84） | GeographicLib GeodTest（20 ケース） | < 0.1 mm |
| Inverse 距離（GRS80） | **国土地理院 測量計算サイト**（4 ケース） | **±0.5 mm** |
| 方位角 | GSI + GeodTest | < 0.005″ / < 10⁻⁸° |
| 縮距 `m12` | GeodTest | < 0.1 mm |
| 対蹠点の最短弧 | GeographicLib | < 0.2 mm |
| 辺の面積スウェイス `S12` | GeographicLib C4 | 1e-11 相対 |
| 多角形面積（C4 vs Green） | 両エンジン相互 | 1e-8 相対 |
| 象限三角形の面積 | 解析値 $\pi R_A^2$ | 1e-9 相対 |
| 等積緯度 | 厳密な $q$ 定義 | < 5e-14 rad |
| JIS コード | jismesh リファレンス実装 | 厳密一致 |
| JIS 境界セマンティクス | JIS X 0410:2002 半開区間規則 | 厳密（整数 µas） |
| 円盤被覆 | 総当たりサンプリング参照 | 集合の厳密一致 |
| 述語 | Shewchuk 型攻撃的構成 | 偽符号ゼロ |

69 テスト + 1 doc テスト、`#![forbid(unsafe_code)]`、依存クレートゼロ。

## 5. ライセンス

MIT — [LICENSE-MIT](LICENSE-MIT) を参照してください。
