# 3COMMAS-STYLE DEAL START CONDITIONS & TECHNICAL INDICATOR ENGINE SPECIFICATION
**Target Architecture:** Rust Quantitative Core (`dca_signals`) + Next.js 15 Visual Condition Builder (`SignalBuilder`)  
**Parity Target:** 3Commas Deal Start Conditions (RSI, MACD, Bollinger Bands, EMA/SMA Crossovers, Stochastic, TradingView Webhooks, Multi-Timeframe AND/OR Logic)  
**Execution Invariant:** Zero Look-Ahead Bias (Kernel Gate 4), Strict Closed-Bar Evaluation, Pure Rational Math or Deterministic Float Metrics.

---

## 1. EXECUTIVE SUMMARY & 3COMMAS PARITY

In 3Commas, every DCA and Grid bot relies on **Deal Start Conditions** (Başlangıç Alım Koşulları) to decide *when* to execute the initial Base Order. Instead of buying blindly at random market moments ("ASAP"), quantitative traders configure technical indicator thresholds or external alerts.

### 1.1 The 3Commas Entry Model
1. **ASAP (Open New Trade ASAP):** As soon as the previous deal closes and the cooldown period elapses, the bot immediately opens a new deal on the next candle open.
2. **Single Indicator Condition:** A single rule such as `RSI(14, 15m) < 30` or `MACD Bullish Cross(1h)`.
3. **Composite Multi-Indicator Logic (AND / OR Groups):** Multiple conditions combined logically:
   - **AND Gate:** `[RSI(14, 15m) < 30] AND [Price > EMA(200, 1h)] AND [24h Volume > 10,000,000 USDT]`
   - **OR Gate:** `[RSI(7, 5m) CROSS_UP 30] OR [Price CROSS_UP BB_Lower(20, 2, 5m)]`
   - **Nested Grouping:** `([Trend Filter: 1h] AND ([Momentum Trigger 1] OR [Momentum Trigger 2]))`
4. **TradingView Custom Signals (Webhooks):** A unique webhook URL with a secure JSON payload emitted by a TradingView Pine Script alert.

### 1.2 Core Quantitative Rules (Kernel Gate Invariants)
- **Gate 4 Invariant (Zero Look-Ahead):** All indicator values and crossover triggers are calculated **strictly on CLOSED bars** ($t \le t_{\text{barIndex}}$). The base order is filled at the **open of the next bar** ($t+1$) with market slippage (Gate 3).
- **Warm-Up Period Guard:** Any bar where an indicator is in its warm-up window ($i < \text{period}$) yields `NaN` / `Uninitialized`. A condition evaluating against `NaN` **must return `FALSE`**—synthetic fills on uncalculated indicators are strictly prohibited.
- **Multi-Timeframe Non-Repaint Rule:** If a bot executes on 1-minute bars but monitors a 1-hour RSI, the 1-hour bar is only considered finalized when its 60th 1-minute bar closes. Intrabar ticks must never trigger look-ahead signals.

---

## 2. TECHNICAL INDICATOR CATALOG & MATHEMATICAL FORMULAS

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                       TECHNICAL INDICATOR CATALOG                           │
├───────────────┬────────────────────────────┬────────────────────────────────┤
│ Indicator     │ Parameters                 │ Available Outputs              │
├───────────────┼────────────────────────────┼────────────────────────────────┤
│ RSI           │ Period (default: 14)       │ rsi (0–100)                    │
│ MACD          │ Fast (12), Slow (26),      │ macd_line, signal_line,        │
│               │ Signal (9)                 │ histogram                      │
│ Bollinger     │ Period (20), StdDev (2.0)  │ lower_band, mid_band (SMA),    │
│ Bands (BB)    │                            │ upper_band, bandwidth_pct      │
│ EMA / SMA     │ Period (e.g. 9, 21, 50,    │ ma_value                       │
│               │ 200)                       │                                │
│ Stochastic    │ %K Period (14),            │ k_line, d_line                 │
│               │ %D Period (3)              │                                │
│ ATR           │ Period (14)                │ atr_abs, atr_pct               │
│ ADX           │ Period (14)                │ adx_value (trend strength)     │
│ Volume        │ Period (20)                │ vol_ratio (vol / avg_vol)      │
│ Price         │ Lookback bars (1)          │ close_price, change_pct        │
└───────────────┴────────────────────────────┴────────────────────────────────┘
```

### 2.1 RSI (Relative Strength Index - Wilder's Smoothing)
- Given price changes $\Delta P_i = P_i - P_{i-1}$:
  $$\text{Gain}_i = \max(\Delta P_i, 0), \quad \text{Loss}_i = \max(-\Delta P_i, 0)$$
- Seed (First $p$ bars):
  $$\text{AvgGain}_p = \frac{1}{p} \sum_{k=1}^p \text{Gain}_k, \quad \text{AvgLoss}_p = \frac{1}{p} \sum_{k=1}^p \text{Loss}_k$$
- Wilder's Smoothing for $i > p$:
  $$\text{AvgGain}_i = \frac{\text{AvgGain}_{i-1} \cdot (p - 1) + \text{Gain}_i}{p}$$
  $$\text{AvgLoss}_i = \frac{\text{AvgLoss}_{i-1} \cdot (p - 1) + \text{Loss}_i}{p}$$
- Final RSI Value:
  $$RS = \frac{\text{AvgGain}_i}{\text{AvgLoss}_i}, \quad RSI_i = \begin{cases} 100 & \text{if } \text{AvgLoss}_i = 0 \\ 100 - \frac{100}{1 + RS} & \text{otherwise} \end{cases}$$

---

### 2.2 MACD (Moving Average Convergence Divergence)
- Fast EMA ($p_{\text{fast}} = 12$) and Slow EMA ($p_{\text{slow}} = 26$):
  $$\text{MACD Line}_i = \text{EMA}_{12}(P)_i - \text{EMA}_{26}(P)_i$$
- Signal Line ($p_{\text{signal}} = 9$):
  $$\text{Signal Line}_i = \text{EMA}_9(\text{MACD Line})_i$$
- Histogram:
  $$\text{Histogram}_i = \text{MACD Line}_i - \text{Signal Line}_i$$
- **Crossover Conditions:**
  - **Bullish Cross:** $\text{MACD Line}_{i-1} \le \text{Signal Line}_{i-1} \land \text{MACD Line}_i > \text{Signal Line}_i$
  - **Bearish Cross:** $\text{MACD Line}_{i-1} \ge \text{Signal Line}_{i-1} \land \text{MACD Line}_i < \text{Signal Line}_i$

---

### 2.3 Bollinger Bands (BB)
- Middle Band (SMA of close prices over period $p = 20$):
  $$\text{Mid}_i = \frac{1}{p} \sum_{k=0}^{p-1} P_{i-k}$$
- Sample Standard Deviation ($\sigma_i$):
  $$\sigma_i = \sqrt{\frac{1}{p} \sum_{k=0}^{p-1} (P_{i-k} - \text{Mid}_i)^2}$$
- Bands ($k = 2.0$):
  $$\text{Upper}_i = \text{Mid}_i + k \cdot \sigma_i, \quad \text{Lower}_i = \text{Mid}_i - k \cdot \sigma_i$$
  $$\text{BandWidth \%}_i = \frac{\text{Upper}_i - \text{Lower}_i}{\text{Mid}_i} \cdot 100\%$$
- **Rebound Condition (Long Entry):**
  $$P_{i-1} \le \text{Lower}_{i-1} \land P_i > \text{Lower}_i \quad (\text{Price CROSS\_UP Lower Band})$$

---

### 2.4 Moving Average Crossovers (EMA / SMA)
- Exponential Moving Average ($\alpha = \frac{2}{p + 1}$):
  $$\text{EMA}_i = P_i \cdot \alpha + \text{EMA}_{i-1} \cdot (1 - \alpha)$$
- **Golden Cross (Altın Kesişim):**
  $$\text{EMA}_{20}(i-1) \le \text{EMA}_{50}(i-1) \land \text{EMA}_{20}(i) > \text{EMA}_{50}(i)$$
- **Trend Filter (Price Above 200 EMA):**
  $$P_i > \text{EMA}_{200}(i)$$

---

### 2.5 Stochastic Oscillator
- Lowest Low ($LL_p$) and Highest High ($HH_p$) over period $p = 14$:
  $$\%K_i = \frac{P_i - LL_p}{HH_p - LL_p} \cdot 100$$
- $\%D$ Line (SMA of $\%K$ over 3 periods):
  $$\%D_i = \text{SMA}_3(\%K)_i$$
- **Oversold Turnaround:**
  $$\%K_i < 20 \land (\%K_{i-1} \le \%D_{i-1} \land \%K_i > \%D_i)$$

---

## 3. FORMAL BOOLEAN LOGIC & RECURSIVE CONDITION TREE

The entry engine uses an Abstract Syntax Tree (AST) of conditions.

```
                  ┌────────────────────────┐
                  │    ROOT: SignalGroup   │
                  │        (op: AND)       │
                  └───────────┬────────────┘
                              │
          ┌───────────────────┴───────────────────┐
          ▼                                       ▼
┌──────────────────┐                    ┌──────────────────┐
│ SignalCondition  │                    │   SignalGroup    │
│ Price > EMA(200) │                    │     (op: OR)     │
└──────────────────┘                    └─────────┬────────┘
                                                  │
                               ┌──────────────────┴──────────────────┐
                               ▼                                     ▼
                     ┌──────────────────┐                  ┌──────────────────┐
                     │ SignalCondition  │                  │ SignalCondition  │
                     │  RSI(14, 15m)<30 │                  │ MACD BullishCross│
                     └──────────────────┘                  └──────────────────┘
```

### 3.1 Comparison Operators (`CompareOp`)
| Operator | Label (TR) | Formal Definition ($L_i$ vs $R_i$) |
| :--- | :--- | :--- |
| `LT` | Küçüktür ($<$) | $L_i < R_i$ |
| `LTE` | Küçük Eşit ($\le$) | $L_i \le R_i$ |
| `GT` | Büyüktür ($>$) | $L_i > R_i$ |
| `GTE` | Büyük Eşit ($\ge$) | $L_i \ge R_i$ |
| `CROSS_UP` | Yukarı Keser | $L_{i-1} \le R_{i-1} \land L_i > R_i$ |
| `CROSS_DOWN` | Aşağı Keser | $L_{i-1} \ge R_{i-1} \land L_i < R_i$ |

---

## 4. RUST IMPLEMENTATION BLUEPRINT (CRATE `dca_signals`)

An LLM generating the Rust engine must implement the following structs, traits, and algorithms.

### 4.1 Data Structures & Enums

```rust
// crates/dca_signals/src/types.rs
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Timeframe {
    M1,
    M5,
    M15,
    H1,
    H4,
    D1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum IndicatorKey {
    Rsi,
    Ema,
    Sma,
    Macd,
    Bb,
    Stoch,
    Atr,
    Adx,
    Vol,
    Price,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndicatorSpec {
    pub key: IndicatorKey,
    pub timeframe: Timeframe,
    pub period: Option<usize>,
    pub fast: Option<usize>,
    pub slow: Option<usize>,
    pub signal: Option<usize>,
    pub std_dev: Option<f64>,
    pub d_period: Option<usize>,
    pub output: Option<String>, // e.g. "macd", "signal", "hist", "lower", "upper"
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CompareOp {
    Lt,
    Lte,
    Gt,
    Gte,
    CrossUp,
    CrossDown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value")]
pub enum SignalValue {
    Number(f64),
    Indicator(IndicatorSpec),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignalCondition {
    pub left: IndicatorSpec,
    pub op: CompareOp,
    pub right: SignalValue,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignalGroup {
    pub op: LogicalOp,
    pub children: Vec<SignalNode>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LogicalOp {
    And,
    Or,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SignalNode {
    Condition(SignalCondition),
    Group(SignalGroup),
}
```

---

### 4.2 Signal Evaluation Engine

```rust
// crates/dca_signals/src/engine.rs
use crate::types::*;
use std::collections::HashMap;

pub struct BarData {
    pub t: i64,
    pub o: f64,
    pub h: f64,
    pub l: f64,
    pub c: f64,
    pub v: f64,
}

pub struct SignalEngine {
    // Cached indicator series by spec signature
    cached_series: HashMap<String, Vec<f64>>,
    bars_count: usize,
}

impl SignalEngine {
    pub fn new(bars: &[BarData], specs: &[IndicatorSpec]) -> Self {
        let mut engine = Self {
            cached_series: HashMap::new(),
            bars_count: bars.len(),
        };
        for spec in specs {
            engine.compute_and_cache(bars, spec);
        }
        engine
    }

    /// Evaluates whether the condition tree evaluates to TRUE on bar index `idx`
    pub fn evaluate(&self, node: &SignalNode, idx: usize) -> bool {
        if idx == 0 {
            // Cannot evaluate crossovers on the very first bar
            return false;
        }

        match node {
            SignalNode::Condition(cond) => self.evaluate_condition(cond, idx),
            SignalNode::Group(group) => match group.op {
                LogicalOp::And => group.children.iter().all(|child| self.evaluate(child, idx)),
                LogicalOp::Or => group.children.iter().any(|child| self.evaluate(child, idx)),
            },
        }
    }

    fn evaluate_condition(&self, cond: &SignalCondition, idx: usize) -> bool {
        let left_curr = self.get_value(&cond.left, idx);
        let left_prev = self.get_value(&cond.left, idx - 1);

        let (right_curr, right_prev) = match &cond.right {
            SignalValue::Number(num) => (*num, *num),
            SignalValue::Indicator(spec) => (self.get_value(spec, idx), self.get_value(spec, idx - 1)),
        };

        // If any value is NaN (warm-up), condition is FAIL-CLOSED (false)
        if left_curr.is_nan() || left_prev.is_nan() || right_curr.is_nan() || right_prev.is_nan() {
            return false;
        }

        match cond.op {
            CompareOp::Lt => left_curr < right_curr,
            CompareOp::Lte => left_curr <= right_curr,
            CompareOp::Gt => left_curr > right_curr,
            CompareOp::Gte => left_curr >= right_curr,
            CompareOp::CrossUp => left_prev <= right_prev && left_curr > right_curr,
            CompareOp::CrossDown => left_prev >= right_prev && left_curr < right_curr,
        }
    }

    fn get_value(&self, spec: &IndicatorSpec, idx: usize) -> f64 {
        let key = self.spec_key(spec);
        self.cached_series
            .get(&key)
            .and_then(|series| series.get(idx).copied())
            .unwrap_or(f64::NAN)
    }

    fn spec_key(&self, spec: &IndicatorSpec) -> String {
        format!(
            "{:?}_{:?}_{}_{}",
            spec.key,
            spec.timeframe,
            spec.period.unwrap_or(0),
            spec.output.as_deref().unwrap_or("default")
        )
    }

    fn compute_and_cache(&mut self, bars: &[BarData], spec: &IndicatorSpec) {
        let key = self.spec_key(spec);
        if self.cached_series.contains_key(&key) {
            return;
        }

        let series = match spec.key {
            IndicatorKey::Rsi => {
                let p = spec.period.unwrap_or(14);
                compute_rsi(bars, p)
            }
            IndicatorKey::Ema => {
                let p = spec.period.unwrap_or(20);
                compute_ema(bars, p)
            }
            IndicatorKey::Macd => {
                let fast = spec.fast.unwrap_or(12);
                let slow = spec.slow.unwrap_or(26);
                let sig = spec.signal.unwrap_or(9);
                compute_macd(bars, fast, slow, sig, spec.output.as_deref().unwrap_or("macd"))
            }
            IndicatorKey::Bb => {
                let p = spec.period.unwrap_or(20);
                let sd = spec.std_dev.unwrap_or(2.0);
                compute_bollinger(bars, p, sd, spec.output.as_deref().unwrap_or("lower"))
            }
            _ => vec![f64::NAN; bars.len()],
        };

        self.cached_series.insert(key, series);
    }
}

// ────────────────────── Wilder's RSI Implementation ──────────────────────
fn compute_rsi(bars: &[BarData], period: usize) -> Vec<f64> {
    let mut out = vec![f64::NAN; bars.len()];
    if bars.len() <= period {
        return out;
    }

    let mut avg_gain = 0.0;
    let mut avg_loss = 0.0;

    for i in 1..=period {
        let change = bars[i].c - bars[i - 1].c;
        if change > 0.0 {
            avg_gain += change / period as f64;
        } else {
            avg_loss += (-change) / period as f64;
        }
    }

    out[period] = if avg_loss == 0.0 {
        100.0
    } else {
        100.0 - (100.0 / (1.0 + avg_gain / avg_loss))
    };

    let p_f = period as f64;
    for i in (period + 1)..bars.len() {
        let change = bars[i].c - bars[i - 1].c;
        let gain = change.max(0.0);
        let loss = (-change).max(0.0);

        avg_gain = (avg_gain * (p_f - 1.0) + gain) / p_f;
        avg_loss = (avg_loss * (p_f - 1.0) + loss) / p_f;

        out[i] = if avg_loss == 0.0 {
            100.0
        } else {
            100.0 - (100.0 / (1.0 + avg_gain / avg_loss))
        };
    }

    out
}
```

---

## 5. NEXT.JS 15 UI IMPLEMENTATION BLUEPRINT (VISUAL BUILDER)

The user interface reproduces the 3Commas condition editor with modern UX (dark mode, colored badges, logic gate toggles, and one-click presets).

### 5.1 Visual Condition Builder Flow
1. **Toggle Switch:** "Başlangıç Koşulları (Sinyal Motoru)" [AÇIK / KAPALI].
   - Kapalı: "ASAP (Hemen Başla)" modu.
   - Açık: Kök Grup `{ op: 'AND', children: [...] }` açılır.
2. **Kural Kartı (Condition Row):**
   - **Sol Taraf (Gösterge):** Seçici `[RSI]` $\to$ Periyot `[14]` $\to$ Zaman Dilimi `[15m]`.
   - **Operatör:** Seçici `[ < ]` (Küçüktür), `[ > ]`, `[ Yukarı Keser ]`, `[ Aşağı Keser ]`.
   - **Sağ Taraf (Değer veya Başka Gösterge):** Sayı `[ 30 ]` veya Gösterge Seçici (örn: `EMA 50`).
3. **Mantık Kapısı Düğmesi (Gate Toggle):**
   - `[ VE (AND) ]` (Tüm kurallar aynı anda geçerli olmalı).
   - `[ VEYA (OR) ]` (Herhangi bir kural sağlandığında tetiklenir).
4. **Hızlı 3Commas Şablonları (One-Click Presets):**
   - ⚡ **RSI Dip:** `RSI(14, 15m) < 30`
   - ⚡ **RSI Dip + Hacim Onayı:** `[RSI(14, 15m) < 35] AND [Hacim > 1.5x Ortalama]`
   - ⚡ **MACD Yükseliş Kesişimi:** `MACD Çizgisi CROSS_UP Sinyal Çizgisi`
   - ⚡ **EMA Altın Kesişim:** `EMA(20) CROSS_UP EMA(50)`
   - ⚡ **Bollinger Alt Bant Dönüşü:** `Fiyat CROSS_UP BB Alt Bant(20, 2)`
   - ⚡ **Stokastik Aşırı Satım Dönüşü:** `[%K CROSS_UP %D] AND [%K < 20]`

---

## 6. TRADINGVIEW CUSTOM WEBHOOK ENTEGRASYONU

3Commas kullanıcılarının en çok kullandığı yöntemlerden biri, TradingView grafiklerindeki Pine Script indikatörlerinden doğrudan bota sinyal göndermektir.

### 6.1 Webhook Endpoint: `POST /api/webhooks/tradingview`

```json
// TradingView Alert Mesaj Şablonu
{
  "secret": "VAULT_SEC_98f4a12c8b0e",
  "botId": "bot-dca-btc-01",
  "action": "START_DEAL",
  "pair": "BTCUSDT",
  "timestamp": 1776672000
}
```

### 6.2 Güvenlik ve Doğrulama Kuralları
1. **Gizli Anahtar (Secret Token):** Her bot için veritabanında üretilen benzersiz `webhookSecret` hash doğrulaması. Yanlış secret durumunda `HTTP 401 Unauthorized`.
2. **Replay Saldırısı Koruması:** `timestamp` değeri sunucu saatinden 60 saniyeden daha eski ise istek `HTTP 400 Expired Alert` ile reddedilir.
3. **Fail-Closed Entegrasyonu:** Bot zaten açık bir işlemdeyse (`pos.qty > 0`) gelen `START_DEAL` sinyali yoksayılır ve loglara `DEAL_ALREADY_ACTIVE` olarak yazılır.

---

## 7. LLM'YE ÖZELLİK EKLERKEN KULLANILACAK SİSTEM PROMPTU

Aşağıdaki istemi (prompt), yapay zekaya (LLM) doğrudan vererek bot sistemimize 3Commas tarzı başlangıç koşullarını eklemesini sağlayabilirsiniz:

````markdown
Sen bir Kıdemli Algoritmik Finans Yazılım Mimarı ve Rust/TypeScript geliştiricisisin.
DCABOT projemize 3Commas tarzı "Deal Start Conditions" (Başlangıç Alım Koşulları) ve "Teknik Gösterge Sinyal Motoru" ekleyeceksin.

Şu kurallara ve mimariye BİREBİR uyacaksın:

### 1. GÖSTERGE VE SİNYAL KATALOĞU
Aşağıdaki indikatörleri hesaplayan deterministik fonksiyonları yaz:
- RSI (Wilder's Smoothing, 14 periyot)
- MACD (12, 26, 9 - macd, signal, histogram)
- Bollinger Bantları (20 periyot, 2.0 std dev - lower, mid, upper, bandwidth)
- EMA ve SMA (9, 20, 50, 200 periyot)
- Stokastik Osilatör (%K 14, %D 3)
- Hacim Oranı (Mevcut hacim / 20 barlık ortalama hacim)

### 2. MANTIK KAPILARI (VE / VEYA / CROSSOVER)
- Operatörler: LT (<), LTE (<=), GT (>), GTE (>=), CROSS_UP, CROSS_DOWN.
- CROSS_UP ve CROSS_DOWN tespiti için bir önceki bar (i-1) ile mevcut bar (i) değerleri karşılaştırılmalıdır.
- Koşullar `SignalGroup` (op: 'AND' | 'OR') ve `SignalCondition` şeklinde özyinelemeli (recursive) bir ağaç yapısında olmalıdır.
- Zero Look-Ahead Bias (Gate 4): Hesaplamalar YALNIZCA kapanmış barlarla (closed bar) yapılır.
- Warm-up Koruması: Periyottan önceki barlarda NaN döner ve koşul FAIL-CLOSED olarak `false` üretir.

### 3. ENTEGRASYON VE ÇIKTI
- Rust Backend: `dca_signals` kütüphanesini `SignalEngine` ve `evaluate()` metodlarıyla yaz.
- Next.js UI: `signal-builder.tsx` bileşenini zenginleştir; RSI, MACD, EMA, BB, Stoch parametrelerini ve 3Commas hazır şablonlarını (RSI Dip, MACD Bullish Cross, EMA Altın Kesişim) tek tıkla uygulanabilir yap.

Kayan nokta (float) hatalarına karşı muhasebeyi koru, filtreleri fail-closed mantığıyla inşa et.
````
