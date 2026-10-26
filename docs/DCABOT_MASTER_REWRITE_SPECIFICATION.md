# DCABOT MASTER SYSTEM REWRITE BLUEPRINT & ARCHITECTURAL SPECIFICATION
**Version:** 3.0-CANONICAL-UNIFIED  
**Target Architecture:** Rust High-Performance Quantitative Engine (`dca-core`) + Next.js 15 App Router Quantitative Frontend (`dca-ui`)  
**Core Principles:** Zero-Float Rational Arithmetic (M01–M20), 8 Fail-Closed Risk Gates, Zero-Execution AI, Zero-Withdrawal Security Vault, Single Process / Low-Resource Lean Deployment.

---

## 1. EXECUTIVE ARCHITECTURAL BLUEPRINT

### 1.1 Architectural Vision & Dual-Core Decoupling
The DCABOT system is an enterprise-grade quantitative algorithmic trading platform designed for Spot and Linear Futures markets. To eliminate the runtime baggage, garbage-collection pauses, and floating-point non-determinism of interpreted languages while maintaining a world-class user experience, the system is decoupled into two clean tiers:

```
┌────────────────────────────────────────────────────────────────────────────────┐
│                  NEXT.JS 15+ QUANTITATIVE TRADING TERMINAL                     │
│  (App Router, React Server Components, Tailwind CSS, TanStack Query, Canvas)   │
│  - Kokpit / Dashboard (NAV, Live PnL, Macro Radar, Allocation Matrix)           │
│  - 5-Step Bot Creator Wizard (Strategy -> Asset -> Params -> Capital -> Guard) │
│  - 7-Tab Bot Deep Inspection Studio & Realtime Fleet Management                │
│  - Interactive DCA / Grid Mathematical Simulator & Liquidation Thermometer     │
└──────────────────────────────────────┬─────────────────────────────────────────┘
                                       │ High-Throughput Local IPC / SSE / JSON-RPC
                                       ▼
┌────────────────────────────────────────────────────────────────────────────────┐
│                       RUST QUANTITATIVE CORE ENGINE                            │
│           (Safe Rust, Tokio Async Event Loop, SQLite WAL Persistence)          │
│                                                                                │
│  ┌────────────────────────┐ ┌────────────────────────┐ ┌────────────────────┐  │
│  │     dca_math Crate     │ │    dca_kernel Crate    │ │ dca_strategy Crate │  │
│  │ - 4096-bit Rational    │ │ - 8 Fail-Closed Gates  │ │ - DCA Long / Short │  │
│  │   Fraction Arithmetic  │ │ - Single Event Kernel  │ │ - Spot/Fut Grid    │  │
│  │ - Half-Even Rounding   │ │   (Backtest/Paper/Live)│ │ - Hedge Groups     │  │
│  │ - WAC Accounting       │ │ - M16 Order Reconciler │ │ - Combo & Infinity │  │
│  │ - M01-M20 Normative    │ │ - Liquidation Monitor  │ │ - Sizing Engine    │  │
│  └────────────────────────┘ └────────────────────────┘ └────────────────────┘  │
│  ┌──────────────────────────────────────────────────────────────────────────┐  │
│  │                   dca_exchange & dca_risk Crates                         │  │
│  │ - Binance & Bybit Adapters (REST & WebSocket Feeds)                      │  │
│  │ - Zero-Withdrawal Hard Gate (HTTP 422) & AES-256-GCM Vault               │  │
│  │ - Market Session Engine (NYSE Easter Rules, 23/5 Commodities, 24/7)      │  │
│  └──────────────────────────────────────────────────────────────────────────┘  │
└────────────────────────────────────────────────────────────────────────────────┘
```

### 1.2 Immutable System Invariants
1. **Mathematical Core Invariant (Zero-Float Rule):** Floating-point IEEE-754 arithmetic (`f32`, `f64`, JavaScript `number`) is strictly forbidden for all financial calculations, price derivations, order sizing, fees, funding, and PnL. All calculations must use canonical 4096-bit rational fractions ($\frac{\text{Numerator}}{\text{Denominator}}$) with exact integer arithmetic, or exact decimal parsing.
2. **Deterministic Fail-Closed Risk Gates (ADR-006):** The kernel enforces 8 immutable risk gates. If any gate condition is violated, the engine halts the cycle, rejects the order intent, or triggers an emergency state. No trade is executed under ambiguous or missing state.
3. **Single-Process Lean Footprint:** The entire architecture must run locally or in low-cost single-instance environments (e.g. Bun/Rust binary + Next.js, zero-cost SQLite with Write-Ahead Logging `PRAGMA journal_mode=WAL`).
4. **Zero-Execution AI Principle (López de Prado 2018):** AI diagnostic engines, copilot assistants, and sentiment analyzers are strictly read-only observers. They are mathematically isolated from the order dispatch pipeline and possess zero order execution or cancellation authority.
5. **Zero-Withdrawal Hard Gate:** API vault connections must verify permissions via exchange metadata. If `canWithdraw` is true, the key is instantly rejected with an unbypassable HTTP 422 Hard-Fail.
6. **Unified Engine Triad (ADR-001):** The identical kernel and strategy logic executes Backtest, Paper Replay, and Live Execution. Strategies cannot distinguish between simulated and real environments except through explicit port injection.

---

## 2. BOT TAXONOMY & FUNCTIONAL CAPABILITIES

### 2.1 Bot Strategy Portfolio

| Bot Strategy | Market Venue | Description & Primary Mathematical Engine |
| :--- | :--- | :--- |
| **DCA Long** | Spot & Linear Futures | Averaging-down martingales/geometric ladders. Expands safety orders below entry VWAP ($P_0$); dynamic net take-profit targeting (M05); trailing exit; M20 SL conflict protection. |
| **DCA Short** | Linear Futures | Averaging-up ladder above entry VWAP ($P_0$); inverted level generation; positive/negative funding rate bridge; liquidation price tracker. |
| **Spot Grid** | Spot | Geometric/arithmetic limit order mesh; fee breaker guard ($g_{\min}$); inventory turnover tracker; reinvestment and auto-rebalance. |
| **Futures Grid** | Linear Futures | Long, Short, or Neutral grid with leverage brackets, liquidation monitoring, and margin headroom enforcement. |
| **Infinity Grid** | Spot | Unbounded upper range grid; holds constant base currency value while distributing quote profit on upward volatility. |
| **Hedge Groups (Protective)** | Linear Futures | Primary DCA Long bot coupled with a Secondary Short Hedge bot. Triggers when primary position reaches a defined drawdown threshold; delta-hedges inventory; coordinated exits. |
| **Hedge Groups (Dual DCA)** | Linear Futures | Simultaneous Long DCA and Short DCA bots on the same symbol with a unified Combined Net Profit target exit (`combinedExit`). |
| **Combo DCA + Grid** | Spot & Futures | Executes a tight grid trading mesh within an equilibrium channel; transitions to DCA ladder when market breaks out of the grid bounds. |

---

### 2.2 Detailed Bot Specifications

#### A. DCA Bot (Long & Short)
1. **Entry Triggers:**
   - Instant (Market on Deal Start).
   - Technical Indicators (RSI threshold, Bollinger Band touch, MACD crossover, Moving Average filter).
   - External Webhooks (TradingView alert signals with JSON payload verification).
   - Cooldown Timer: Minimum elapsed bars/time between deals to prevent immediate re-entry after stop/TP.
2. **Order Sizing Modes:**
   - `BASE_QTY`: Base order and safety orders specify quantities in base asset units (e.g. $0.05 \text{ BTC}$).
   - `QUOTE_NOTIONAL`: Base order and safety orders specify sizes in quote currency (e.g. $100 \text{ USDT}$). Safety order quantities are calculated as $q_i = \lfloor N_i / P_i \rfloor_{\text{step}}$.
3. **Safety Order Ladder Engine:**
   - Anchor: $P_0 = \text{VWAP of filled base order}$.
   - Number of Safety Orders: $n \ge 0$.
   - Price Deviation ($d$): Percentage deviation for first safety level ($d > 0$, e.g. $0.02 = 2\%$).
   - Step Scale ($a$): Geometric multiplier for level intervals ($a \ge 1.0$).
   - Volume Scale ($v$): Geometric multiplier for safety order sizes ($v \ge 1.0$, typically $1.0 \le v \le 1.5$; $v \ge 2.0$ flagged as high risk).
   - Single Active Order Invariant: At any time, only ONE risk-increasing safety order is placed as a resting limit order on the exchange. When safety order $k$ fills, safety order $k+1$ is evaluated and placed.
4. **Exit Strategies:**
   - **Gross Take Profit:** Simple percentage above average price ($A \times (1 + t)$).
   - **Net Take Profit (M05):** Exact price calculated to yield net profit $T = t \times C_{\text{ref}}$ after covering all round-trip exchange fees ($f_{\text{in}}, f_{\text{out}}$) and cumulative funding costs ($F$).
   - **Trailing Take Profit:** Triggered when mark price reaches activation threshold. Tracks peak price $P_{\text{peak}}$; closes position when price drops by trailing deviation $\delta_{\text{trail}}$ from peak.
   - **Stop Loss:** Hard price trigger ($A \times (1 - \text{sl})$ for Long; $A \times (1 + \text{sl})$ for Short).
   - **Break-Even Stop:** Adjusts SL to entry price once a predetermined profit threshold is crossed.
   - **Multi-Level Partial Take Profit (M10):** Splits exit into up to 5 tranches (e.g., $30\%$ at $+1.5\%$, $30\%$ at $+3.0\%$, $40\%$ at $+5.0\%$).
5. **Operator Controls:**
   - Manual Add Funds (injects extra fill and updates WAC).
   - Market Close All (immediate reduction).
   - Dynamic SL/TP patching on active deals.
   - Detach deal from bot.

#### B. Grid Bot (Spot & Futures)
1. **Grid Spacing Modes:**
   - **Arithmetic Grid:** Constant price step between adjacent levels:
     $$P_j = P_{\text{lower}} + j \cdot \Delta P, \quad \Delta P = \frac{P_{\text{upper}} - P_{\text{lower}}}{n}$$
   - **Geometric Grid:** Constant percentage step between adjacent levels:
     $$P_j = P_{\text{lower}} \cdot \left(\frac{P_{\text{upper}}}{P_{\text{lower}}}\right)^{j/n}, \quad j = 0, \dots, n$$
2. **Fee-Breaker Rule:**
   - The engine validates that the step percentage exceeds the round-trip fee threshold:
     $$g_{\min} = \frac{f_{\text{buy}} + f_{\text{sell}}}{1 - f_{\text{sell}}} \cdot 100\%$$
   - If $\Delta P_{\%} \le g_{\min}$, grid formation is rejected to prevent fee erosion.
3. **Inventory Management & Order Placement:**
   - Lower levels below market price: Resting Buy Limit orders.
   - Upper levels above market price: Resting Sell Limit orders.
   - When a buy limit fills at $P_j$, an offsetting sell limit is immediately dispatched at $P_{j+1}$ (and vice versa).
   - Total inventory is capped to prevent exceeding allocated equity: $\sum q_i \cdot P_i \le \text{Allocated Capital}$.
4. **Range Expansion & Trailing:**
   - Trailing Up: When price rises above $P_{\text{upper}}$, the entire grid ladder shifts upwards by $k$ levels, canceling lowest buys and creating higher sells.
   - Trailing Down: When price drops below $P_{\text{lower}}$, the grid ladder shifts downwards.
   - Stop-Loss / Stop-Trigger when price exits configured safety bounds.

#### C. Hedge Group Engine
1. **Protective Hedge:**
   - Monitored: Main Long DCA deal.
   - Trigger: Drawdown of main deal reaches threshold $DD_{\text{hedge}}$ (e.g. $-6\%$).
   - Execution: Spawns a Short Futures position sized by delta hedge ratio ($q_{\text{hedge}} = \rho \cdot q_{\text{main}}$).
   - Exit: Short leg takes independent profit at target, or closes simultaneously when main DCA deal achieves net break-even.
2. **Dual Leg (Long + Short DCA):**
   - Both legs operate simultaneously on the same futures symbol.
   - Position Mode: Dual-Position / Hedge Mode required on exchange (Binance/Bybit).
   - Combined Exit Engine: Calculates joint unrealized + realized PnL:
     $$\Pi_{\text{combined}} = \Pi_{\text{long}} + \Pi_{\text{short}}$$
   - When $\Pi_{\text{combined}} \ge T_{\text{combined}}$, both positions are closed simultaneously via market orders.

---

## 3. CANONICAL MATHEMATICAL & FINANCIAL SPECIFICATIONS (M01 – M20)

### M01 — Exact Numeric Representation & Units
- **Core Requirement:** Lossless exact arithmetic. No IEEE-754 binary floating-point numbers.
- **Internal Type:** Rational Fraction $\frac{N}{D}$, where $N \in \mathbb{Z}$, $D \in \mathbb{N}^+$, $\gcd(|N|, D) = 1$.
- **Precision Bounds:** Numerator and denominator bit length must not exceed 4096 bits:
  $$\max(\text{bit\_length}(|N|), \text{bit\_length}(D)) \le 4096$$
  If exceeded, the calculation aborts with `Overflow4096Bit`.
- **String Parsing:** Input parsing rejects `NaN`, `Infinity`, and scientific notation without explicit exponents.
- **Display & Formatting:** Display strings are converted using IEEE 854 / Python Decimal style `HALF_EVEN` (Banker's rounding) to a maximum of 12 decimal places. Rounding occurs *only* at the UI/reporting boundary; intermediate calculation state is never rounded.

---

### M02 — DCA Fixed-Level Plan & Cumulative Deviation
- **Behavior:** `AVERAGING_DOWN` (Long) / `AVERAGING_UP` (Short).
- **Anchor Mode:** `BASE_ORDER_FINAL_VWAP`. The anchor price $P_0$ is locked to the volume-weighted average fill price of the base order once it is filled.
- **Cumulative Deviation:** For safety level $i \in \{1, \dots, n\}$:
  $$D_i = d \cdot \sum_{k=0}^{i-1} a^k = d \cdot \begin{cases} i & \text{if } a = 1 \\ \frac{a^i - 1}{a - 1} & \text{if } a \ne 1 \end{cases}$$
- **Execution Target Prices:**
  - **Long:** $P_i = P_0 \cdot (1 - D_i)$, rounded **down** to tick size (`tickDown`).
  - **Short:** $P_i = P_0 \cdot (1 + D_i)$, rounded **up** to tick size (`tickUp`).
- **Domain Constraints:** $d > 0$, $a > 0$, $n \ge 0$, $D_i < 1.0$ (for Long), $P_i > 0$. Any configuration yielding $P_i \le 0$ or overlapping tick levels is strictly rejected.

---

### M03 — Plan Notional, Sizing & Stress Collateral
- **Sizing Formulations:**
  - **BASE_QTY Mode:**
    $$q_i = S_q \cdot v^{i-1}, \quad N_{\text{plan}} = B_q \cdot P_0 + \sum_{i=1}^n q_i \cdot P_i$$
  - **QUOTE_NOTIONAL Mode:**
    $$N_i = S_N \cdot v^{i-1}, \quad q_i = \lfloor N_i / P_i \rfloor_{\text{step}}, \quad N_{\text{plan}} = B_N + \sum_{i=1}^n N_i$$
- **Capital & Collateral Requirements:**
  - `plannedGrossNotional`: Total quote value of all planned orders.
  - `estimatedInitialMargin`:
    $$\text{IM} = \frac{N_{\text{plan}}}{L} \quad (\text{for Futures with leverage } L)$$
  - `policyRequiredCollateral`: Stressed worst-case capital requirement across stress paths $\Omega$:
    $$R_{\text{open}}(z) = \text{IM}_{\text{pos}}(z) + \text{IM}_{\text{pending}}(z) + R_{\text{extra}}$$
    $$R_{\text{survive}}(z) = \text{MM}(z) + R_{\text{close}} + R_{\text{funding}} + R_{\text{gap}}$$
    $$C_{\text{required}} = \max\left(0, \sup_{z \in \Omega} (\max(R_{\text{open}}(z), R_{\text{survive}}(z)) - \Pi(z))\right)$$
  - **Spot Mode:** Price shocks do not create additional collateral calls. Required capital is strictly:
    $$C_{\text{spot}} = N_{\text{plan}} + \text{Estimated Fees}$$

---

### M04 — Weighted Average Cost (WAC), Fills & Partial Exits
- **Position State:** Signed direction $s \in \{+1 \text{ (Long)}, -1 \text{ (Short)}\}$, quantity $q \ge 0$, cost basis $C \ge 0$, average entry $A = \frac{C}{q}$, realized gross PnL $G$.
- **Inventory Expansion ($a > 0$ filled at price $p$):**
  $$q' = q + a, \quad C' = C + a \cdot p, \quad A' = \frac{C'}{q'}, \quad \Delta G = 0$$
- **Inventory Reduction ($r > 0$ filled at price $p$, $r \le q$):**
  $$\Delta G = s \cdot r \cdot (p - A), \quad q' = q - r, \quad C' = C - r \cdot A$$
  *Key Rule:* On partial close, average price $A$ remains **unchanged**.
- **Complete Close ($r = q$):**
  $$q' = 0, \quad C' = 0, \quad A' = \text{None}$$
- **Idempotency:** Every fill has an explicit identifier `fill_key`. If a fill with an existing `fill_key` is received, it is ignored without modifying state. Overfill events are recorded, reported as violations, and handled without state truncation.

---

### M05 — Fees, Funding & Net Target Pricing
- **Fee Accounting:** Expense is positive; rebates are negative ($K_{\text{fee}} > 0$). Maker and taker fees are differentiated.
- **Estimated Fee:**
  $$\text{Fee}_{\text{est}} = |q \cdot p| \cdot \text{Rate}$$
- **Exact Net Take Profit Price ($P_{\text{TP}}$):**
  Given current open quantity $q$, average cost $A$, cumulative funding fees incurred $F$, planned exit fee rate $f_{\text{out}}$, and target net quote profit $T$:
  - **Long:**
    $$\text{NetProfit}(p) = q \cdot (p - A) - F - f_{\text{out}} \cdot q \cdot p \ge T$$
    $$P_{\text{TP, long}} = \frac{q \cdot A + F + T}{q \cdot (1 - f_{\text{out}})} \quad (f_{\text{out}} < 1)$$
    Aligned **upward** to tick size (`tickUp`) to guarantee net return.
  - **Short:**
    $$P_{\text{TP, short}} = \frac{q \cdot A - F - T}{q \cdot (1 + f_{\text{out}})}$$
    Aligned **downward** to tick size (`tickDown`).
- **Funding Sign Bridge:**
  - Protocol cashflow: Long pays when funding rate is positive (cash outflow).
  - Internal ledger: Funding expense is expense-positive.
  $$\text{Expense}_{\text{funding}} = -1 \cdot \text{Cashflow}_{\text{funding}}$$

---

### M06 — Equity, Unrealized PnL & Drawdown Tracking
- **Unrealized Gross PnL ($U$):**
  $$U = s \cdot q \cdot (m - A) \quad (m = \text{Mark Price})$$
- **Total Equity ($E_t$):**
  $$E_t = \text{Cash Wallet} + U$$
- **High-Water Mark ($H_t$) & Drawdown ($DD_t$):**
  $$H_t = \max_{u \le t} E_u, \quad DD_t = \frac{H_t - E_t}{H_t}, \quad \text{MDD} = \max_t DD_t$$
- **Time-Weighted Returns (TWR):** External deposits or withdrawals $X_t$ break the performance period, and sub-period returns are chained:
  $$R_{\text{period}} = \frac{E_t - X_t}{E_{t-1}} - 1, \quad \text{TWR} = \prod (1 + R_i) - 1$$

---

### M07 — Exact Liquidation Boundary & Bracket Maintenance
- **Isolated Linear Futures Balance Equation:**
  At liquidation mark price $m^*$, wallet balance plus unrealized PnL equals maintenance margin requirement:
  $$W + s \cdot q \cdot (m^* - A) = \text{MM}(m^*) = r \cdot q \cdot m^* - d$$
  Where $W$ is margin balance, $r$ is maintenance margin rate (MMR), and $d$ is maintenance amount deduction for the notional bracket.
- **Exact Roots:**
  - **Long ($s = +1$):**
    $$m^* = \frac{q \cdot A - W - d}{q \cdot (1 - r)}$$
  - **Short ($s = -1$):**
    $$m^* = \frac{q \cdot A + W + d}{q \cdot (1 + r)}$$
- **Validation:** The calculated $m^*$ must fall within the notional bracket $[N_{\min}, N_{\max})$ where $r$ and $d$ apply. If $m^*$ falls in another tier, the root must be solved iteratively across bracket boundaries.

---

### M08 — Tick, Step & Intrabar Quantization
- **Lattice Alignment:**
  Given base lattice origin $o$ and tick size $h > 0$:
  $$\text{tickDown}(x) = o + h \cdot \left\lfloor \frac{x - o}{h} \right\rfloor$$
  $$\text{tickUp}(x) = o + h \cdot \left\lceil \frac{x - o}{h} \right\rceil$$
- **Quantity Alignment:** Order quantities are always quantized **down** using step size $h_q$:
  $$\text{qtyDown}(q) = \left\lfloor \frac{q}{h_q} \right\rfloor \cdot h_q$$
- **Filter Checks:** Orders must satisfy $q \ge q_{\min}$ and $q \cdot p \ge \text{minNotional}$. If violated, the order intent is rejected.
- **Time Separation Rule:** Decisions are made strictly at bar close ($t_{\text{close}}$). Market fills occur at the next bar open ($t_{\text{open}} + \text{slippage}$). Limit fills require explicit price penetration:
  - Buy Limit at $P_{\text{limit}}$ fills only if Bar Low $\le P_{\text{limit}}$.
  - Sell Limit at $P_{\text{limit}}$ fills only if Bar High $\ge P_{\text{limit}}$.

---

### M09 — Deal Actions & State Machine
- **Deal Lifecycle States:** `PENDING_OPEN` $\to$ `OPEN` $\to$ `CLOSING` $\to$ `CLOSED` (or `CANCELED`).
- **Allowed Dynamic Mutations:**
  - `MANUAL_ADD`: Injects cash/notional into active deal; recalculates $q, C, A$ via M04; shifts remaining safety levels and updates TP target.
  - `MANUAL_CLOSE`: Generates reduce-only market order for $q$; transitions deal to `CLOSED`.
  - `UPDATE_TP_SL`: Dynamically adjusts target profit percentage or stop loss.
  - `CANCEL_REPLACE`: Replaces resting limit order with updated price/quantity without state loss.

---

### M10 — Advanced Exit Suite (Trailing & Multi-Tranche)
- **Trailing Take Profit State Machine:**
  - State: `IDLE` $\to$ `ARMED` $\to$ `TRIGGERED`.
  - Condition: Mark price reaches $A \cdot (1 + t_{\text{trigger}})$.
  - Tracking: Peak price $P_{\text{peak}} = \max(P_{\text{peak}}, P_{\text{current}})$.
  - Exit Trigger: Price falls below $P_{\text{peak}} \cdot (1 - \delta_{\text{trail}})$.
- **Break-Even Stop:**
  When price reaches $A \cdot (1 + \text{be\_trigger})$, SL is moved to $A \cdot (1 + \text{be\_offset})$ (protecting fees).
- **Multi-Level Partial Take Profit (Ladder TP):**
  A set of tranches $\{(\text{pct}_k, \text{target}_k)\}_{k=1}^m$ where $\sum \text{pct}_k = 100\%$. As each target price is reached, quantity $q \cdot \text{pct}_k$ is closed via reduce-only limit/market order.

---

### M11 — Position Sizing Engine
- **Base Order Calculation:**
  Resolves sizing specification:
  $$q_{\text{base}} = \begin{cases} \text{specified\_qty} & \text{if BASE\_QTY} \\ \lfloor \text{notional} / P_0 \rfloor_{\text{step}} & \text{if QUOTE\_NOTIONAL} \end{cases}$$
- **Capital Allocation Guard:**
  $$\text{Allocated Capital} \le \text{Free Collateral} \times \text{Max Allocation Ratio}$$

---

### M12 — Ladder Risk & Depth Verification
- **Liquidity Stress Check:** Compares total planned safety order notional against average order book depth within the ladder price range.
- **Maintenance Bracket Resolution:** Resolves $(r, d)$ dynamically as position notional expands:
  $$\text{Bracket} = \operatorname{lookupBracket}(N_{\text{pos}})$$

---

### M13 — Multi-Deal Portfolio Margin
- Cross-deal exposure tracking across $K$ active bots.
- Shared quote budget allocation: Sum of active initial margins must never exceed account equity:
  $$\sum_{k=1}^K \text{IM}_k \le E_{\text{account}} \times \text{Max Leverage Utilization}$$

---

### M14 — Futures Balance & Unrealized Reconciliation
- Continuous reconciliation between internal ledger and exchange snapshot:
  $$|\text{Position}_{\text{internal}} - \text{Position}_{\text{exchange}}| < \epsilon_{\text{step}}$$
  $$|\text{Cash}_{\text{internal}} - \text{Cash}_{\text{exchange}}| < \epsilon_{\text{quote}}$$
- Discrepancy triggers an audit alert and halts new deal entries.

---

### M15 — Grid Mathematical Suite
- **Arithmetic Mesh:** $P_j = P_{\text{lower}} + j \cdot \Delta P$.
- **Geometric Mesh:** $P_j = P_{\text{lower}} \cdot r^j$, where $r = \left(\frac{P_{\text{upper}}}{P_{\text{lower}}}\right)^{1/n}$.
- **Neutral Grid Splitting:**
  - $P_j < P_{\text{current}}$: Allocated to Long sub-grid (Buy low, Sell mid).
  - $P_j > P_{\text{current}}$: Allocated to Short sub-grid (Sell high, Buy mid).

---

### M16 — Order Reconciliation State Machine
- Computes minimal diff between Desired Orders $\{D\}$ and Venue Active Orders $\{V\}$:
  $$\Delta_{\text{cancel}} = \{v \in V \mid v \notin D \lor \text{price/qty mismatch}\}$$
  $$\Delta_{\text{place}} = \{d \in D \mid d \notin V\}$$
  $$\Delta_{\text{keep}} = \{v \in V \mid v \in D \text{ with exact match}\}$$
- Dispatches cancellations first, then placements, guaranteeing zero duplicate fills.

---

### M17 — Combo & Infinity Grid Mathematics
- **Infinity Grid:** Maintains a fixed total value of the base asset in quote terms:
  $$V_{\text{base}} = q \cdot p = \text{Constant}$$
  As price increases by $\Delta p$, the excess quantity $\Delta q = q - \frac{V_{\text{base}}}{p + \Delta p}$ is sold to lock in profit.

---

### M18 — Hedge Group Coupling & Delta Neutrality
- **Target Delta:** $\Delta_{\text{net}} = q_{\text{main}} - \rho \cdot q_{\text{hedge}}$.
- **Combined Exit Condition:**
  $$\Pi_{\text{main}}(p) + \Pi_{\text{hedge}}(p) \ge \text{Target Combined Profit}$$

---

### M19 — Live Grid Inventory Derivation
- Pure event-driven state derivation. Inventory $q(t)$ and average cost $A(t)$ are computed directly from the sorted sequence of authenticated execution fills, eliminating state desynchronization caused by missed WebSocket packets.

---

### M20 — Stop-Loss to Safety Ladder Conflict Detector
- **Problem Statement:** In DCA trading, if the Stop Loss percentage is set too tight relative to the ladder deviation, the SL trigger price will be hit *before* lower safety orders are filled. This leaves allocated capital dead and stops the deal prematurely.
- **Formal Invariant:**
  For every safety order level $k \in \{1, \dots, n\}$ with execution price $P_k$ and resulting post-fill average price $A_k$:
  $$\text{SL Trigger}(A_k) = A_k \cdot (1 - \text{sl}) \quad (\text{for Long})$$
  The next safety level $P_{k+1}$ is **unreachable** if:
  $$\text{SL Trigger}(A_k) \ge P_{k+1}$$
  If this condition holds, safety orders from $k+1$ to $n$ can never be reached under normal market progression. The configuration analyzer flags this as an unviable strategy.

---

### M21 — TradFi Market Sessions & NYSE Calendar Engine
- **Session Types:**
  - `US_EQUITY`: Monday–Friday 09:30–16:00 EST. Observes US Eastern Time with full Daylight Saving Time (EDT/EST) transitions (Second Sunday in March, First Sunday in November).
  - `COMMODITY_23_5`: Opens Sunday 23:00 UTC, Closes Friday 22:00 UTC. Daily maintenance pause 22:00–23:00 UTC.
  - `24_7_CRYPTO`: Uninterrupted continuous execution.
- **NYSE Holiday Calculation (Meeus/Jones/Butcher Gregorian Easter Algorithm):**
  Holidays are calculated dynamically without hardcoded tables (New Year's Day, MLK Day, Washington's Birthday, Good Friday, Memorial Day, Juneteenth, Independence Day, Labor Day, Thanksgiving, Christmas). Weekend holidays roll to closest weekday (Saturday $\to$ Friday, Sunday $\to$ Monday).
- **TradFi Financing Engine:** Daily borrow interest:
  $$\text{Interest} = N_{\text{position}} \cdot \frac{\text{Annual Borrow Rate}}{360}$$

---

## 4. THE 8 FAIL-CLOSED RISK GATES & EXECUTION KERNEL

```
  Bar Feed / Ticks
         │
         ▼
┌─────────────────┐   FAIL
│ Gate 1: Data    ├──────────► [ HALT ENGINE / THROW ]
└────────┬────────┘
         ▼ PASS
┌─────────────────┐   MARKET CLOSED
│ Gate 7: Session ├──────────────────────────────┐
└────────┬────────┘                              │
         ▼ OPEN                                  │
┌─────────────────┐   09:30-09:35                │
│ Gate 8: Gap Buf ├──────────────────┐           │
└────────┬────────┘                  │           │
         ▼ CLEAR                     ▼           ▼
┌─────────────────┐            [ DEFER/SKIP ] [ REJECT ADD ]
│ Gate 4: Zero LA │            (Safety Orders) (Preserve Exits)
└────────┬────────┘
         ▼ PASS
┌─────────────────┐   INSUFFICIENT
│ Gate 6: Capital ├──────────────────────────────► [ SKIP / WAIT ]
└────────┬────────┘
         ▼ PASS
┌─────────────────┐
│ Gate 5: Limit   │ ──► [ Label: MARKETABLE_TAKER / RESTING_TOUCH ]
└────────┬────────┘
         ▼
┌─────────────────┐
│ Gate 2: Fee     │ ──► [ Deduct 100% Commissions (Maker/Taker) ]
└────────┬────────┘
         ▼
┌─────────────────┐
│ Gate 3: Slip    │ ──► [ Apply Market Slippage Penalty ]
└────────┬────────┘
         ▼
[ DISPATCH / SIMULATE FILL ]
```

### 4.1 Gate Definitions

1. **Gate 1: Strict Data Integrity & Monotonicity (Data Guard)**
   - Bars must arrive in strictly ascending chronological order: $t_i > t_{i-1}$.
   - Monotonic interval: $t_i - t_{i-1} = \Delta t$.
   - Structural price validity: $\text{Low} \le \text{Open} \le \text{High}$, $\text{Low} \le \text{Close} \le \text{High}$, $\text{Volume} \ge 0$, $\text{Prices} > 0$.
   - Violation: Motor halts immediately. Zero execution on corrupted data.

2. **Gate 2: Universal Fee Deduction (Fee Guard)**
   - Every single fill (simulated or real) must calculate and deduct fees. Zero-fee fills are rejected unless explicitly running on a verified zero-fee promotion pair.
   - Maker vs Taker classification must reflect execution reality (`LIMIT` does not automatically imply Maker).

3. **Gate 3: Market Order Slippage Penalty (Slippage Guard)**
   - Market orders cannot execute at the triggering price.
   - Slippage penalty:
     $$P_{\text{fill, buy}} = P_{\text{open}} \cdot (1 + \text{slip}), \quad P_{\text{fill, sell}} = P_{\text{open}} \cdot (1 - \text{slip})$$

4. **Gate 4: Zero Look-Ahead Bias (Look-Ahead Guard)**
   - Decision generation is strictly confined to closed historical bars ($t \le t_{\text{bar}}$).
   - Orders generated at bar close $t$ can only execute at bar $t+1$ or later.

5. **Gate 5: Limit Realism & Penetration Tagging (Optimism Guard)**
   - Limit orders are labeled by execution circumstance:
     - `MARKETABLE_TAKER`: Limit price crossed market at placement time $\to$ executed as taker.
     - `RESTING_TOUCH`: Price barely touched limit $\to$ requires penetration check.
     - `OPEN_GAP_BETTER`: Market gapped past limit $\to$ executed at better open price.
   - Max Bar Volume Share: A single bar cannot fill more than $10\%$ of its total volume for the strategy's orders.

6. **Gate 6: Pre-Trade Capital & Margin Guard (Cash Guard)**
   - Before any order intent is submitted or filled, the engine evaluates available wallet cash and maintenance margin:
     $$\text{Cash Required} = \begin{cases} q \cdot p + \text{fee} & \text{Spot} \\ \frac{q \cdot p}{L} + \text{fee} & \text{Futures} \end{cases}$$
   - If available balance is insufficient:
     - In `SKIP` mode: Order is permanently skipped.
     - In `WAIT` mode: Order is held until balance is restored.
     - *Under no circumstances does the engine permit negative cash or unbacked leverage.*

7. **Gate 7: Market Session & Macro Blackout Gate (Session Guard)**
   - When market is closed or during high-impact macro blackout windows (e.g. CPI/FOMC releases):
     - New risk intents (`kind = ADD`) are **rejected** with `FAIL_CLOSED`.
     - Risk-reducing exits (`kind = REDUCE`, TP, SL) remain **active and protected**.

8. **Gate 8: Gap Protection & Opening Salvo Buffer (Gap Guard)**
   - During the first 5 minutes of market open (09:30–09:35 EST for equities):
     - Safety orders are deferred to prevent filling on abnormal opening spreads.
   - If opening price gaps down through multiple safety levels, they are consolidated into a single synthetic VWAP fill:
     $$Q_{\text{cons}} = \sum_{k=1}^m q_k, \quad P_{\text{cons}} = P_{\text{open}}$$

---

## 5. RUST IMPLEMENTATION BLUEPRINT (FOR LLM RUST REWRITE)

This section provides the complete Rust architecture, crate organization, structs, traits, and algorithm implementations. An LLM tasked with generating the Rust codebase must follow these exact definitions.

### 5.1 Cargo Workspace Layout
```toml
# Cargo.toml
[workspace]
members = [
    "crates/dca_math",
    "crates/dca_kernel",
    "crates/dca_strategy",
    "crates/dca_risk",
    "crates/dca_exchange",
    "crates/dca_server"
]
resolver = "2"

[workspace.dependencies]
num-bigint = "0.4"
num-integer = "0.1"
num-traits = "0.2"
tokio = { version = "1.38", features = ["full"] }
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
thiserror = "1.0"
tracing = "0.1"
chrono = { version = "0.4", features = ["serde"] }
rusqlite = { version = "0.31", features = ["bundled"] }
axum = { version = "0.7", features = ["ws"] }
reqwest = { version = "0.12", features = ["json"] }
```

---

### 5.2 Crate `dca_math`: Rational Representation & Core Math

```rust
// crates/dca_math/src/fraction.rs
use num_bigint::{BigInt, Sign};
use num_integer::Integer;
use num_traits::{One, Zero, Signed};
use std::fmt;
use std::str::FromStr;
use thiserror::Error;

#[derive(Error, Debug, PartialEq, Eq)]
pub enum MathError {
    #[error("M01: Division by zero")]
    DivisionByZero,
    #[error("M01: 4096-bit integer limit exceeded")]
    BitLimitExceeded,
    #[error("M01: Invalid decimal string: {0}")]
    InvalidDecimalString(String),
    #[error("M01: Negative or fractional exponent not allowed")]
    InvalidExponent,
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Fraction {
    num: BigInt,
    den: BigInt, // Strictly > 0
}

const MAX_BITS: u64 = 4096;

impl Fraction {
    pub fn new(mut num: BigInt, mut den: BigInt) -> Result<Self, MathError> {
        if den.is_zero() {
            return Err(MathError::DivisionByZero);
        }
        if den.is_negative() {
            num = -num;
            den = -den;
        }
        let gcd = num.gcd(&den);
        let norm_num = if gcd.is_zero() { BigInt::zero() } else { num / &gcd };
        let norm_den = if gcd.is_zero() { BigInt::one() } else { den / &gcd };

        if norm_num.bits() > MAX_BITS || norm_den.bits() > MAX_BITS {
            return Err(MathError::BitLimitExceeded);
        }

        Ok(Self { num: norm_num, den: norm_den })
    }

    pub fn zero() -> Self {
        Self { num: BigInt::zero(), den: BigInt::one() }
    }

    pub fn one() -> Self {
        Self { num: BigInt::one(), den: BigInt::one() }
    }

    pub fn from_i64(v: i64) -> Self {
        Self { num: BigInt::from(v), den: BigInt::one() }
    }

    pub fn from_decimal_str(s: &str) -> Result<Self, MathError> {
        let trimmed = s.trim();
        if trimmed.is_empty() {
            return Err(MathError::InvalidDecimalString(s.to_string()));
        }

        let parts: Vec<&str> = trimmed.split(['e', 'E']).collect();
        let (mantissa_str, exp_val) = match parts.len() {
            1 => (parts[0], 0i64),
            2 => {
                let exp = parts[1].parse::<i64>().map_err(|_| MathError::InvalidDecimalString(s.to_string()))?;
                (parts[0], exp)
            }
            _ => return Err(MathError::InvalidDecimalString(s.to_string())),
        };

        let is_negative = mantissa_str.starts_with('-');
        let clean_mantissa = mantissa_str.trim_start_matches('+').trim_start_matches('-');
        let dec_parts: Vec<&str> = clean_mantissa.split('.').collect();

        let (int_part, frac_part) = match dec_parts.len() {
            1 => (dec_parts[0], ""),
            2 => (dec_parts[0], dec_parts[1]),
            _ => return Err(MathError::InvalidDecimalString(s.to_string())),
        };

        let digits = format!("{}{}", int_part, frac_part);
        if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_digit()) {
            return Err(MathError::InvalidDecimalString(s.to_string()));
        }

        let mut num = BigInt::from_str(&digits).map_err(|_| MathError::InvalidDecimalString(s.to_string()))?;
        let frac_len = frac_part.len() as i64;
        let mut den = BigInt::from(10).pow((frac_len.max(0)) as u32);

        if exp_val > 0 {
            num *= BigInt::from(10).pow(exp_val as u32);
        } else if exp_val < 0 {
            den *= BigInt::from(10).pow((-exp_val) as u32);
        }

        if is_negative {
            num = -num;
        }

        Self::new(num, den)
    }

    pub fn add(&self, other: &Self) -> Result<Self, MathError> {
        let num = &self.num * &other.den + &other.num * &self.den;
        let den = &self.den * &other.den;
        Self::new(num, den)
    }

    pub fn sub(&self, other: &Self) -> Result<Self, MathError> {
        let num = &self.num * &other.den - &other.num * &self.den;
        let den = &self.den * &other.den;
        Self::new(num, den)
    }

    pub fn mul(&self, other: &Self) -> Result<Self, MathError> {
        let num = &self.num * &other.num;
        let den = &self.den * &other.den;
        Self::new(num, den)
    }

    pub fn div(&self, other: &Self) -> Result<Self, MathError> {
        if other.num.is_zero() {
            return Err(MathError::DivisionByZero);
        }
        let num = &self.num * &other.den;
        let den = &self.den * &other.num;
        Self::new(num, den)
    }

    pub fn is_positive(&self) -> bool {
        self.num.is_positive()
    }

    pub fn is_zero(&self) -> bool {
        self.num.is_zero()
    }

    pub fn to_half_even_display(&self, decimals: usize) -> String {
        // High-precision banker's rounding implementation to exact string representation
        let multiplier = BigInt::from(10).pow((decimals + 1) as u32);
        let scaled = (&self.num * &multiplier) / &self.den;
        let sign = if scaled.is_negative() { "-" } else { "" };
        let abs_scaled = scaled.abs();
        let last_digit = (&abs_scaled % 10u32).to_u32_digits().1.first().copied().unwrap_or(0);
        let mut base = abs_scaled / 10u32;
        let rem = (&self.num * &multiplier) % &self.den;

        if last_digit > 5 || (last_digit == 5 && (!rem.is_zero() || base.is_odd())) {
            base += 1u32;
        }

        let base_str = format!("{:0>width$}", base, width = decimals + 1);
        let split_pos = base_str.len() - decimals;
        let int_slice = &base_str[..split_pos];
        let frac_slice = &base_str[split_pos..];

        if decimals == 0 {
            format!("{}{}", sign, int_slice)
        } else {
            format!("{}{}.{}", sign, int_slice, frac_slice)
        }
    }
}
```

---

### 5.3 M02, M04, M05 Implementation in Rust

```rust
// crates/dca_math/src/m02.rs
use crate::fraction::{Fraction, MathError};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SizingMode {
    BaseQty,
    QuoteNotional,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Long,
    Short,
}

#[derive(Debug, Clone)]
pub struct DcaLevel {
    pub level_index: usize,
    pub price: Fraction,
    pub qty: Fraction,
    pub notional: Fraction,
    pub cumulative_deviation: Fraction,
}

pub fn build_dca_plan(
    anchor_p0: &Fraction,
    base_qty: &Fraction,
    deviation_pct: &Fraction,
    step_scale: &Fraction,
    volume_scale: &Fraction,
    safety_count: usize,
    direction: Direction,
    tick_size: &Fraction,
    step_size: &Fraction,
    sizing_mode: SizingMode,
) -> Result<Vec<DcaLevel>, MathError> {
    let mut levels = Vec::with_capacity(safety_count);
    let mut current_step_dev = deviation_pct.clone();
    let mut cum_dev = Fraction::zero();
    let mut current_qty = base_qty.clone();

    for i in 1..=safety_count {
        cum_dev = cum_dev.add(&current_step_dev)?;
        if direction == Direction::Long && cum_dev >= Fraction::one() {
            return Err(MathError::InvalidDecimalString("Cumulative deviation >= 100% on Long".into()));
        }

        let raw_price = match direction {
            Direction::Long => {
                let factor = Fraction::one().sub(&cum_dev)?;
                anchor_p0.mul(&factor)?
            }
            Direction::Short => {
                let factor = Fraction::one().add(&cum_dev)?;
                anchor_p0.mul(&factor)?
            }
        };

        // Align price to tick
        let price = match direction {
            Direction::Long => tick_down(&raw_price, tick_size)?,
            Direction::Short => tick_up(&raw_price, tick_size)?,
        };

        current_qty = current_qty.mul(volume_scale)?;
        let qty = qty_down(&current_qty, step_size)?;
        let notional = price.mul(&qty)?;

        levels.push(DcaLevel {
            level_index: i,
            price,
            qty,
            notional,
            cumulative_deviation: cum_dev.clone(),
        });

        current_step_dev = current_step_dev.mul(step_scale)?;
    }

    Ok(levels)
}

pub fn tick_down(price: &Fraction, tick: &Fraction) -> Result<Fraction, MathError> {
    let div = price.div(tick)?;
    let int_part = Fraction::from_i64((div.num / div.den).to_string().parse::<i64>().unwrap_or(0));
    int_part.mul(tick)
}

pub fn tick_up(price: &Fraction, tick: &Fraction) -> Result<Fraction, MathError> {
    let div = price.div(tick)?;
    let mut int_num = div.num / &div.den;
    if !(&div.num % &div.den).is_zero() {
        int_num += 1;
    }
    let int_part = Fraction::from_i64(int_num.to_string().parse::<i64>().unwrap_or(0));
    int_part.mul(tick)
}

pub fn qty_down(qty: &Fraction, step: &Fraction) -> Result<Fraction, MathError> {
    tick_down(qty, step)
}
```

```rust
// crates/dca_math/src/m04.rs
use crate::fraction::{Fraction, MathError};
use std::collections::HashSet;

#[derive(Debug, Clone)]
pub struct PositionState {
    pub direction: i8, // +1 Long, -1 Short
    pub qty: Fraction,
    pub cost: Fraction,
    pub avg_price: Option<Fraction>,
    pub realized_gross: Fraction,
    pub seen_fill_keys: HashSet<String>,
}

impl PositionState {
    pub fn new(direction: i8) -> Self {
        Self {
            direction,
            qty: Fraction::zero(),
            cost: Fraction::zero(),
            avg_price: None,
            realized_gross: Fraction::zero(),
            seen_fill_keys: HashSet::new(),
        }
    }

    pub fn apply_fill(
        &mut self,
        fill_key: &str,
        price: &Fraction,
        qty: &Fraction,
        is_add: bool,
    ) -> Result<bool, MathError> {
        if self.seen_fill_keys.contains(fill_key) {
            return Ok(false); // Idempotent skip
        }

        if is_add {
            // Inventory Addition
            self.qty = self.qty.add(qty)?;
            let added_cost = qty.mul(price)?;
            self.cost = self.cost.add(&added_cost)?;
            self.avg_price = Some(self.cost.div(&self.qty)?);
        } else {
            // Inventory Reduction (WAC: Avg Price stays UNCHANGED)
            let avg = self.avg_price.as_ref().ok_or(MathError::DivisionByZero)?;
            let diff = if self.direction == 1 {
                price.sub(avg)?
            } else {
                avg.sub(price)?
            };
            let delta_realized = qty.mul(&diff)?;
            self.realized_gross = self.realized_gross.add(&delta_realized)?;

            self.qty = self.qty.sub(qty)?;
            let cost_reduction = qty.mul(avg)?;
            self.cost = self.cost.sub(&cost_reduction)?;

            if self.qty.is_zero() {
                self.cost = Fraction::zero();
                self.avg_price = None;
            }
        }

        self.seen_fill_keys.insert(fill_key.to_string());
        Ok(true)
    }
}
```

```rust
// crates/dca_math/src/m05.rs
use crate::fraction::{Fraction, MathError};

pub fn calculate_net_take_profit(
    direction: i8,
    qty: &Fraction,
    avg_price: &Fraction,
    cumulative_funding: &Fraction,
    exit_fee_rate: &Fraction,
    target_net_profit: &Fraction,
) -> Result<Fraction, MathError> {
    if direction == 1 {
        // Long: P_TP = (q * A + F + T) / (q * (1 - f_out))
        let q_a = qty.mul(avg_price)?;
        let numerator = q_a.add(cumulative_funding)?.add(target_net_profit)?;
        let one_minus_fee = Fraction::one().sub(exit_fee_rate)?;
        let denominator = qty.mul(&one_minus_fee)?;
        numerator.div(&denominator)
    } else {
        // Short: P_TP = (q * A - F - T) / (q * (1 + f_out))
        let q_a = qty.mul(avg_price)?;
        let numerator = q_a.sub(cumulative_funding)?.sub(target_net_profit)?;
        let one_plus_fee = Fraction::one().add(exit_fee_rate)?;
        let denominator = qty.mul(&one_plus_fee)?;
        numerator.div(&denominator)
    }
}
```

---

### 5.4 Crate `dca_kernel`: Execution Stepper & Gate Enforcement

```rust
// crates/dca_kernel/src/stepper.rs
use dca_math::fraction::Fraction;
use dca_math::m04::PositionState;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Bar {
    pub t: i64,
    pub o: String,
    pub h: String,
    pub l: String,
    pub c: String,
    pub v: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GateStatus {
    pub gate1_data_valid: bool,
    pub gate2_fee_deducted: bool,
    pub gate3_slippage_applied: bool,
    pub gate4_no_lookahead: bool,
    pub gate5_realism_labeled: bool,
    pub gate6_capital_guarded: bool,
    pub gate7_session_checked: bool,
    pub gate8_gap_protected: bool,
}

pub struct SimulationStepper {
    pub position: PositionState,
    pub cash: Fraction,
    pub equity: Fraction,
    pub current_bar_idx: usize,
    pub gates: GateStatus,
    pub pending_orders: Vec<OrderIntent>,
}

#[derive(Debug, Clone)]
pub struct OrderIntent {
    pub id: String,
    pub side: OrderSide,
    pub kind: OrderKind,
    pub price: Option<Fraction>,
    pub qty: Fraction,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OrderSide { Buy, Sell }
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OrderKind { Add, Reduce }

impl SimulationStepper {
    pub fn step(&mut self, bar: &Bar, is_market_open: bool) -> Result<(), String> {
        // Gate 1: Check Data Integrity
        self.verify_bar_integrity(bar)?;

        // Gate 7: Check Session Gate
        if !is_market_open {
            self.pending_orders.retain(|o| o.kind == OrderKind::Reduce);
        }

        // Intrabar fill evaluations for resting limit orders
        self.evaluate_resting_orders(bar)?;

        // Stale tracking
        self.current_bar_idx += 1;
        Ok(())
    }

    fn verify_bar_integrity(&self, bar: &Bar) -> Result<(), String> {
        // High >= Low, Open, Close check
        let h = Fraction::from_decimal_str(&bar.h).map_err(|e| e.to_string())?;
        let l = Fraction::from_decimal_str(&bar.l).map_err(|e| e.to_string())?;
        let o = Fraction::from_decimal_str(&bar.o).map_err(|e| e.to_string())?;
        let c = Fraction::from_decimal_str(&bar.c).map_err(|e| e.to_string())?;

        if h < l || o < l || c < l || o > h || c > h {
            return Err("Gate 1: OHLC integrity violation".into());
        }
        Ok(())
    }

    fn evaluate_resting_orders(&mut self, bar: &Bar) -> Result<(), String> {
        let low = Fraction::from_decimal_str(&bar.l).unwrap();
        let high = Fraction::from_decimal_str(&bar.h).unwrap();

        let mut remaining_orders = Vec::new();
        for order in self.pending_orders.drain(..) {
            if let Some(target_price) = &order.price {
                let filled = match order.side {
                    OrderSide::Buy => target_price >= &low,
                    OrderSide::Sell => target_price <= &high,
                };

                if filled {
                    // Gate 6: Pre-trade margin/cash check
                    let cost = order.qty.mul(target_price).unwrap();
                    if order.kind == OrderKind::Add && cost > self.cash {
                        // Skip order, do not fill
                        continue;
                    }

                    // Apply fill via M04
                    self.position.apply_fill(&order.id, target_price, &order.qty, order.kind == OrderKind::Add).unwrap();
                    if order.kind == OrderKind::Add {
                        self.cash = self.cash.sub(&cost).unwrap();
                    } else {
                        self.cash = self.cash.add(&cost).unwrap();
                    }
                } else {
                    remaining_orders.push(order);
                }
            }
        }
        self.pending_orders = remaining_orders;
        Ok(())
    }
}
```

---

## 6. NEXT.JS FRONTEND ARCHITECTURE (FOR LLM NEXT.JS REWRITE)

This section specifies the modern Next.js 15 App Router architecture, directory layout, UI design system, state stores, and page blueprints.

### 6.1 Directory Layout & App Structure
```
dca-ui/
├── app/
│   ├── (auth)/
│   │   └── login/page.tsx
│   ├── (dashboard)/
│   │   ├── layout.tsx             # Main layout with AppHeader, Sidebar, FleetTicker
│   │   ├── page.tsx               # Cockpit / Executive Hub (NAV, Allocation, Pulse)
│   │   ├── bots/
│   │   │   ├── page.tsx           # Fleet Overview (Card grid & table view)
│   │   │   ├── new/page.tsx       # 5-Step Bot Creator Wizard
│   │   │   └── [id]/page.tsx      # 7-Tab Bot Deep Inspection Studio
│   │   ├── hedge-groups/
│   │   │   ├── page.tsx           # Hedge Group List View
│   │   │   └── [id]/page.tsx      # Dual-Leg Inspection & Combined Exit Monitor
│   │   ├── markets/
│   │   │   └── page.tsx           # Live Charts, Order Book, Volatility Calibration
│   │   ├── lab/
│   │   │   └── backtest/page.tsx  # Backtest Studio, Monte Carlo, WFE/DSR Certifier
│   │   ├── hesaplayici/
│   │   │   └── page.tsx           # Interactive DCA & Liquidation Calculator
│   │   └── settings/
│   │       └── page.tsx           # Venue Keys (AES-256-GCM Vault), Account Modes
│   └── api/                       # Lightweight Next.js proxy to Rust Core Engine
├── components/
│   ├── ui/                        # Radix UI + Tailwind primitives
│   ├── charts/                    # Interactive Canvas Candlestick & Ladder Chart
│   ├── wizard/                    # 5-Step Creation Steps
│   └── cockpit/                   # Dashboard widgets
├── stores/
│   ├── fleet-store.ts             # Zustand store for live bot statuses
│   └── stream-store.ts            # SSE / WebSocket subscriber
└── lib/
    ├── api-client.ts              # Typed fetch client connecting to Rust Core
    └── validations/               # Zod validation schemas matching M01-M08
```

---

### 6.2 UI & UX Design System Specifications
- **Theme:** Ultra-clean Dark Financial Terminal (`#0B0E14` Background, `#151A24` Cards, `#1F2637` Borders).
- **Accents:** 
  - Emerald Green (`#10B981` / `#059669`): Long positions, profit, positive delta.
  - Rose Red (`#F43F5E` / `#E11D48`): Short positions, drawdowns, stop loss.
  - Cyan / Cobalt (`#06B6D4` / `#2563EB`): Grids, pending safety levels, active indicators.
  - Amber (`#F59E0B`): Warnings, M20 ladder conflict alerts, high margin utilization.
- **Typography:** `Inter` for interfaces; `JetBrains Mono` for numeric price displays, fractional ledger amounts, and order tables.

---

### 6.3 Page Blueprints

#### 1. Kokpit / Dashboard (`app/(dashboard)/page.tsx`)
- **Top Row - Net Asset Value & Exposure:**
  - Total Equity (USDT), 24h PnL (\$, \%), Daily Net Cashflow, Margin Utilization Bar.
- **Middle Row - Fleet Performance & Allocation Matrix:**
  - Active Bots vs Total Bots; Active Deals count; Long/Short Delta Exposure.
  - Allocation Donut Chart (Capital by Pair: BTC/ETH/SOL/TradFi).
- **Bottom Row - Real-time Stream & Alerts:**
  - Unified Event Log (SSE feed: Fills, Funding debits, Gate checks, Deal state changes).
  - Emergency Global Killswitch (One-click halt button with dual-confirmation modal).

#### 2. The 5-Step Bot Creator Wizard (`app/(dashboard)/bots/new/page.tsx`)
- **Step 1: Strategy Selection**
  - Card selector: DCA Long, DCA Short, Spot Grid, Futures Grid, Infinity Grid, Protective Hedge.
- **Step 2: Instrument & Venue Selection**
  - Venue picker (Binance / Bybit / Paper Lab).
  - Pair search with realtime ticker, tick size, step size, and minimum notional badges.
- **Step 3: Algorithmic Parameters**
  - Base Order Size & Sizing Mode toggle (`BASE_QTY` vs `QUOTE_NOTIONAL`).
  - Safety Order Parameters: Deviation ($d$), Step Scale ($a$), Volume Scale ($v$), Safety Count ($n$).
  - Target Profit Percentage & Fee Mode toggle (Gross vs M05 Net Profit).
  - Trailing TP toggle (Activation % and Callback Deviation %).
  - Stop Loss % with real-time M20 Conflict Alert indicator.
- **Step 4: Capital & Stress Collateral Card (M03 Integration)**
  - Interactive table showing all $n$ safety order prices, order sizes, cumulative capital required, and average price after each fill.
  - Stress Collateral Thermometer: Displays estimated liquidation price (M07) and required maintenance cushion under a 3-day volatility shock.
- **Step 5: Pre-Flight Review & Launch**
  - Verification summary card confirming all 8 risk gates pass pre-validation.
  - "Launch Paper Bot" or "Launch Live Bot" button.

#### 3. Bot Deep Inspection Studio (`app/(dashboard)/bots/[id]/page.tsx`)
- 7 Independent Tab Panes:
  1. **Genel (Overview):** Deal performance chart, current position size, entry VWAP ($A$), mark price, unrealized PnL thermometer.
  2. **Deal'ler (Deals):** Historical deal list with duration, total safety orders triggered, gross PnL, funding paid, net realized PnL.
  3. **Fill'ler (Fills):** Realtime execution table displaying Fill Key, Timestamp, Price, Qty, Role (Base/Safety/TP), Fee Paid, and Gate 5 Realism tag.
  4. **Ayarlar (Config):** Read-only configuration view with live config-patching interface (dynamically edit TP, SL, trailing without stopping deal).
  5. **Sürümler (Checkpoints):** Serialized state snapshots; revert or fork bot state.
  6. **Sertifika (Backtest/Cert):** Walk-Forward Efficiency (WFE), Deflated Sharpe Ratio (DSR), Probability of Backtest Overfitting (PBO) badge.
  7. **Olaylar (Logs):** Internal kernel events, gate logs, skip reasons, and exchange latency metrics.

#### 4. Interactive DCA & Grid Calculator (`app/(dashboard)/hesaplayici/page.tsx`)
- Standalone zero-risk sandbox.
- Users input custom ladder parameters and observe instant recalculation of:
  - Exact ladder levels, tick alignment, and WAC trajectory.
  - M20 Stop-Loss conflict threshold (visually highlighting which safety levels are rendered unreachable).
  - Spot vs Futures liquidation boundaries across various leverage choices ($1\times$ to $50\times$).

---

### 6.4 Client-Side Zod Validation Schemas

```typescript
// lib/validations/dca-schema.ts
import { z } from 'zod';

export const dcaConfigSchema = z.object({
  name: z.string().min(3).max(50),
  pair: z.string().regex(/^[A-Z0-9]+USDT$/, "Must be a valid USDT pair"),
  direction: z.enum(['LONG', 'SHORT']),
  sizingMode: z.enum(['BASE_QTY', 'QUOTE_NOTIONAL']),
  baseOrderSize: z.string().refine((v) => !isNaN(Number(v)) && Number(v) > 0, "Must be positive decimal"),
  safetyOrderSize: z.string().refine((v) => !isNaN(Number(v)) && Number(v) > 0, "Must be positive decimal"),
  deviationPct: z.string().refine((v) => Number(v) > 0 && Number(v) < 1, "Must be between 0 and 1 (e.g. 0.02 for 2%)"),
  stepScale: z.string().refine((v) => Number(v) >= 1.0, "Step scale must be >= 1.0"),
  safetyVolumeScale: z.string().refine((v) => Number(v) >= 1.0, "Volume scale must be >= 1.0"),
  safetyOrderCount: z.number().int().min(0).max(30),
  targetProfitPct: z.string().refine((v) => Number(v) > 0, "Target profit must be positive"),
  stopLossPct: z.string().optional().refine((v) => !v || (Number(v) > 0 && Number(v) < 1), "Stop loss must be between 0 and 1"),
  trailingEnabled: z.boolean(),
  trailingDeviationPct: z.string().optional(),
}).refine((data) => {
  // M20 Conflict Check: Validate that SL is not narrower than first safety step
  if (data.stopLossPct && Number(data.stopLossPct) <= Number(data.deviationPct)) {
    return false;
  }
  return true;
}, {
  message: "M20 Conflict: Stop Loss is tighter than or equal to first safety order deviation!",
  path: ["stopLossPct"],
});
```

---

## 7. HIGH-PERFORMANCE IPC & SSE EVENT CONTRACTS

The Next.js frontend communicates with the Rust engine via high-speed local HTTP/SSE or JSON-RPC.

### 7.1 Server-Sent Events (SSE) Stream Specification (`GET /api/stream`)
The Rust engine emits structured JSON events over an unbroken SSE channel:

```json
// Example: Realtime Fill Event
{
  "event": "FILL",
  "data": {
    "dealId": "d-8f92b1c4",
    "botId": "b-01",
    "orderId": "ord-safety-2",
    "role": "SAFETY",
    "side": "BUY",
    "price": "62450.50",
    "qty": "0.0125",
    "fee": "0.585",
    "feeRole": "MAKER",
    "fillModel": "RESTING_TOUCH",
    "ts": 1776672000000
  }
}

// Example: Gate Skip Event
{
  "event": "GATE_SKIP",
  "data": {
    "botId": "b-02",
    "gate": "GATE6_CASH_GUARD",
    "reason": "Insufficient free margin for safety order 4",
    "required": "150.00",
    "available": "42.10",
    "ts": 1776672001500
  }
}
```

---

## 8. MASTER PHASE-BY-PHASE IMPLEMENTATION ROADMAP

```
Horizon 1: Pure Math & Kernel (Weeks 1-4)
├── Crate dca_math: Fraction 4096-bit, M01-M08, M15, M20
├── Proptests & Canonical TS Parity Tests
└── Crate dca_kernel: 8 Risk Gates, Simulation Stepper

Horizon 2: Strategies & Backtesting (Weeks 5-8)
├── Crate dca_strategy: DCA Long/Short, Grid Suite, Hedge Legs
├── Backtest Runner & Walk-Forward Optimizer
└── Checkpoint Snapshot Serialization (KK2)

Horizon 3: Live Connectors & Reconciliation (Weeks 9-12)
├── Crate dca_exchange: Binance & Bybit REST/WebSocket
├── M16 Order Reconciliation Engine
├── AES-256-GCM Vault & Zero-Withdrawal 422 Hard Gate
└── SQLite WAL State Persistence

Horizon 4: Next.js Terminal Frontend (Weeks 13-16)
├── Modern App Router Layout & Design System
├── 5-Step Creation Wizard & 7-Tab Studio
├── Interactive Canvas Charts & Realtime SSE Integration
└── Standalone DCA/Grid Math Sandbox Calculator

Horizon 5: Parity Verification & 6-Month Testnet Ramp (Weeks 17+)
├── Dual-Run Parity Audit (TS vs Rust side-by-side)
├── 20/20 Testnet Validation Run
└── 6-Month Real-World Paper & Live Incubation
```

### Phase Details

#### Phase 1: Pure Rational Math Core & Invariant Testing (Rust)
- Implement `dca_math` with `Fraction` 4096-bit exact integer arithmetic.
- Implement canonical M01 through M08 formulas.
- Implement M15 (Grid equations) and M20 (SL conflict detector).
- Execute property-based tests (`proptest`) mirroring `tests/unit/math-contract.test.ts` and `finance-invariants.test.ts`.

#### Phase 2: Execution Kernel & The 8 Risk Gates (Rust)
- Implement `dca_kernel` with `SimulationStepper`.
- Implement Gate 1 (OHLCV validity) through Gate 8 (Gap protection).
- Add support for TradFi market sessions (Meeus/Jones/Butcher Easter calendar).
- Verify zero look-ahead bias and intrabar realism models (`MARKETABLE_TAKER` vs `RESTING_TOUCH`).

#### Phase 3: Bot Strategy Suite & Backtest Certifier (Rust)
- Port DCA Long, DCA Short, Grid, and Hedge strategies into `dca_strategy`.
- Implement WAC accounting (M04) and fee/funding net profit adjustments (M05).
- Build the backtest engine with Monte Carlo permutations, Walk-Forward Efficiency, and Deflated Sharpe Ratio outputs.

#### Phase 4: Exchange Adapters & Live Driver (Rust)
- Implement Binance and Bybit REST/WebSocket clients using `tokio` and `reqwest`.
- Implement M16 Minimal Order Reconciliation state machine.
- Integrate the Zero-Withdrawal Hard Gate (reject keys with `canWithdraw: true`).
- Configure SQLite WAL persistence with checkpoint truncate hygiene.

#### Phase 5: Next.js Modern Frontend Terminal
- Construct Next.js 15 App Router scaffold with Tailwind CSS and Radix UI.
- Build the Cockpit Dashboard, 5-Step Bot Creator Wizard, and 7-Tab Bot Deep Inspection Studio.
- Implement interactive Canvas-based Candlestick and DCA Ladder charts.
- Build the real-time SSE stream subscriber.

#### Phase 6: System Verification, Parity Audit & Testnet Deployment
- Run Rust engine and legacy TypeScript engine in parallel lockstep replay.
- Confirm byte-for-byte equity curve, fill sequence, and gate event parity.
- Execute 20/20 Testnet verification checklist.
- Enter the lean 6-month low-cost test phase under SQLite WAL.

---

## 9. CONCLUSION & LLM INSTRUCTIONS

This document serves as the absolute, normative specification for DCABOT.
- When generating **Rust** code: adhere strictly to the structs, traits, rational fraction representations, error types, and mathematical formulas specified in Section 3, 4, and 5.
- When generating **Next.js** code: adhere strictly to the routes, Zod schemas, design system rules, wizard steps, and tab structures defined in Section 6.
- Under no circumstances may floating-point numbers be used in quantitative logic, nor may any of the 8 risk gates be bypassed.
