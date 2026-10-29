//! dca_pine — Pine Script v5 UYUMLU ALT KÜME yorumlayıcısı (sıfır float).
//!
// Faz 21 (CI): evaluator'ın buy/sell aynaları aynı gövdeyi paylaşır (bilinçli);
// prev_close alanları parser'ın future-diagnostics defterinde ayrılmıştır
// (şimdilik okunmuyor — P2 borcu olarak belgeli).
#![allow(
    clippy::if_same_then_else,
    clippy::ptr_arg,
    dead_code,
    // Faz 21 (CI): pine değerlendiricide sayaçlı while-loop ayrıştırıcı doğası;
    // iç tip imzaları parser AST sözleşmesidir.
    clippy::explicit_counter_loop,
    clippy::type_complexity,
    clippy::doc_lazy_continuation
)]

//! ## Bu nedir, ne değildir?
//! TradingView'ın kapalı Pine Script motoru üçüncü taraf ortamda çalıştırılamaz.
//! Bu crate, DCABOT'un kendi motorunda çalışan **Pine sözdizimli bir strateji
//! alt kümesidir**: stratejinizi Pine benzeri yazarsınız, motor aynı sözdizimini
//! ayrıştırıp 4096-bit kesirli aritmetikle (M01) çalıştırır. Tam PineScript
//! DEĞİLDİR — desteklenen fonksiyon tablosu `is_known_function` ile tanımlıdır
//! ve derleyici bilinmeyen her fonksiyon/sembolü NET olarak reddeder.
//!
//! ## Desteklenen alt küme
//! - Bildirim: `//@version=5` (yutulur), `strategy("ad")`, `indicator("ad")`
//! - Atama: `x = ifade`, `var x = ifade` (kalıcı), `x := ifade`
//! - Blok: `if koşul` / `else if koşul` / `else` — girinti ile
//! - İfadeler: + - * / %, < <= > >= == !=, and/or/not, `? :`, `[n]` geçmişi
//! - Seriler: open/high/low/close/volume/time/hl2/hlc3/ohlc4, her değişken
//! - ta.*: sma, ema, rma, rsi, tr, atr, highest, lowest, change, mom,
//!   crossover, crossunder, cross, sum, cum, barssince
//! - math.*: abs, max, min, round, floor, ceil, pow (tam sayı üs)
//! - input.*: float, int, bool, string (defval ilk argüman; bot
//!   pine_inputs'u değişken adıyla override eder)
//! - strategy.*: entry(id, strategy.long|short[, when=koşul]),
//!   close(id[, when=koşul]), close_all([when=koşul]),
//!   exit(id[, stop=, limit=][, when=koşul]) — exit seviyeleri pozisyon açıkken
//!   bar OHLC kesişiminde exit sinyali üretir; when=false/na olan barda komut
//!   hiç çalışmaz (fail-closed)
//! - nz(x[, y]), plot("başlık") veya plot("başlık", seri) — seri argümanı
//!   verildiğinde değerler RunResult.plots'ta bar-diziniyle döner
//!
//! ## Bilinçli sınırlar (dokümante)
//! - Pyramiding yok: açık pozisyon varken entry yoksayılır (uyarı kaydılır);
//!   strategy(pyramiding=N>1) derlemede dürüst uyarı üretir (sessiz yutma yok).
//! - ta.* koşullu blok içinde seriyi kaydırabilir — TradingView davranışıyla
//!   birebir; derleyici bu durumda "series consistency" uyarısı (notes) üretir.
//! - ta.* çağrısı üzerinde [n] DESTEKLENİR (düğüm başına çıkış geçmişi);
//!   koşullu kapsamda çağrı atlanan barlarda geçmiş uzamaz (aynı kural).
//! - strategy.exit'ten üretilen exit, bot tarafında "PINE_EXIT" kapanışıdır.
//! - when=false çağrıyı atlar ama ÖNCEDEN yerleşmiş stop/limit emrini İPTAL
//!   ETMEZ (TV emir/iptal ayrımı; iptal için strategy.cancel* yoktur — bilerek
//!   desteklenmez, derleme reddeder). O barın tetikleme kontrolü de atlanır;
//!   eski stop sonraki barda yeniden değerlendirilir.
//! - strategy.close(id) id eşleşmesi ARAMAZ — tek pozisyon modeli:
//!   pyramiding olmadığı için açık pozisyon neyse o kapanır (TV'de eşleşmeyen
//!   id hiçbir şeyi kapatmaz; bu fark dokümante, pratikte tek pozisyonda aynı
//!   sonuç).
//! - strategy.order yoktur — derleme NET reddeder (sessiz yutma yok).
//! - Kesirli üs (math.pow(x, 0.5)) yoktur.
//! - ta.* periyodu 600'ü (ring kapasitesi) aşamaz — aşan periyot kalıcı na
//!   üretirdi; yerine dürüst çalışma-anı hatası verilir.

pub mod eval;
pub mod lexer;
pub mod parser;

pub use eval::{run, run_all, BarVerdict, PineInputs, RunResult};
pub use parser::{compile, DeclKind, Program, ScriptDecl};

/// Bot oluşturma/doğrulama akışı için: kodu derle; hata mesajını aynen döndür.
pub fn validate_source(src: &str) -> Result<Program, String> {
    let src_t = src.trim();
    if src_t.is_empty() {
        return Err("pine_script boş olamaz".into());
    }
    if src_t.lines().count() > 200 {
        return Err("pine_script en fazla 200 satır olabilir".into());
    }
    compile(src_t)
}

#[cfg(test)]
mod tests {
    use super::*;
    use dca_kernel::Bar;

    fn bar(t: i64, c: &str) -> Bar {
        Bar { t, o: c.into(), h: c.into(), l: c.into(), c: c.into(), v: "1000".into() }
    }

    /// Deterministik test serisi: 40 bar, 100 → 101 → ... (her bar +1).
    fn rising(n: usize) -> Vec<Bar> {
        let mut v = Vec::new();
        let mut px = 100i64;
        let mut t = 1_700_000_000_000i64;
        for _ in 0..n {
            v.push(bar(t, &px.to_string()));
            px += 1;
            t += 3_600_000;
        }
        v
    }

    /// Zigzag serisi: 2 bar yukarı, 2 bar aşağı — crossover'lar üretir.
    fn zigzag(n: usize) -> Vec<Bar> {
        let mut v = Vec::new();
        let mut px = 100f64;
        let mut dir = 1.0;
        let mut t = 1_700_000_000_000i64;
        for i in 0..n {
            px += dir;
            if (i + 1) % 2 == 0 {
                dir = -dir;
            }
            let c = format!("{px:.0}");
            v.push(bar(t, &c));
            t += 3_600_000;
        }
        v
    }

    const EMA_CROSS: &str = r#"//@version=5
strategy("EMA Cross")
fastLen = input.int(3, "Fast EMA")
slowLen = input.int(5, "Slow EMA")
fast = ta.ema(close, fastLen)
slow = ta.ema(close, slowLen)
if ta.crossover(fast, slow)
    strategy.entry("Long", strategy.long)
if ta.crossunder(fast, slow)
    strategy.close_all()
"#;

    #[test]
    fn ema_cross_produces_entries_and_exits() {
        let prog = validate_source(EMA_CROSS).unwrap();
        let bars = zigzag(60);
        let res = run(&prog, &bars, &PineInputs::new()).unwrap();
        let entries = res.verdicts.iter().filter(|v| v.entry).count();
        let exits = res.verdicts.iter().filter(|v| v.exit).count();
        assert!(entries >= 2, "zigzag'ta en az 2 crossover olmalı, entries={entries}");
        assert!(exits >= 2, "zigzag'ta en az 2 crossunder olmalı, exits={exits}");
        // ilk barlar warmup (slow len=5 → bar 5 öncesi na) → crossover olamaz
        assert!(!res.verdicts[..4].iter().any(|v| v.entry), "warmup'ta giriş olamaz");
    }

    #[test]
    fn zero_lookahead_prefix_stability() {
        // Kritik regresyon: bar i'nin kararı, gelecekteki barlar eklenince DEĞİŞMEMELİ.
        let bars = zigzag(50);
        let prog = validate_source(EMA_CROSS).unwrap();
        let full = run(&prog, &bars, &PineInputs::new()).unwrap();
        for cut in [10usize, 25, 40] {
            let part = run(&prog, &bars[..cut], &PineInputs::new()).unwrap();
            for (i, v) in part.verdicts.iter().enumerate() {
                assert_eq!(v.entry, full.verdicts[i].entry, "bar {cut}/{i} entry look-ahead sızıntısı!");
                assert_eq!(v.exit, full.verdicts[i].exit, "bar {cut}/{i} exit look-ahead sızıntısı!");
            }
        }
    }

    #[test]
    fn input_override_changes_behavior() {
        let prog = validate_source(EMA_CROSS).unwrap();
        let bars = zigzag(60);
        let first_entry = |res: &RunResult| -> Option<usize> {
            res.verdicts.iter().position(|v| v.entry)
        };
        let plain = run(&prog, &bars, &PineInputs::new()).unwrap();
        // kısa periyotlar (1/2): ema(1)=close → crossover en kısa warmup'ta
        let mut inp = PineInputs::new();
        inp.insert("fastLen".into(), "1".into());
        inp.insert("slowLen".into(), "2".into());
        let over = run(&prog, &bars, &inp).unwrap();
        let f_over = first_entry(&over).expect("override ilk giriş üretmeli");
        let f_plain = first_entry(&plain).expect("default ilk giriş üretmeli");
        assert!(f_over < f_plain, "kısa periyot daha erken sinyal üretmeli: {f_over} vs {f_plain}");
    }

    #[test]
    fn rsi_below_strategy_entry() {
        let src = r#"//@version=5
strategy("RSI Dip")
rsiVal = ta.rsi(close, 5)
if rsiVal < 40
    strategy.entry("Dip", strategy.long)
if rsiVal > 60
    strategy.close_all()
"#;
        let prog = validate_source(src).unwrap();
        // düşen sonra yükselen seri → RSI önce düşer sonra yükselir
        let mut bars = Vec::new();
        let mut px = 100i64;
        let mut t = 1_700_000_000_000i64;
        for i in 0..40 {
            if i < 20 {
                px -= 1;
            } else {
                px += 1;
            }
            bars.push(bar(t, &px.to_string()));
            t += 3_600_000;
        }
        let res = run(&prog, &bars, &PineInputs::new()).unwrap();
        let entries = res.verdicts.iter().filter(|v| v.entry).count();
        let exits = res.verdicts.iter().filter(|v| v.exit).count();
        assert!(entries >= 1, "RSI<40 en az bir kez tetiklenmeli");
        assert!(exits >= 1, "RSI>60 en az bir kez tetiklenmeli");
    }

    #[test]
    fn strategy_exit_stop_hits_on_cross() {
        // SABİT mutlak stop (input ile): strategy.exit stop seviyesi bar OHLC
        // kesişiminde exit üretir (iz süren stop: her bar close*0.97 gibi ifade
        // verilirse seviye fiyatı takip eder — dokümante davranış).
        let src = r#"//@version=5
strategy("Stop Test")
stopPx = input.float(97.0, "Stop")
strategy.entry("L", strategy.long)
strategy.exit("X", stop=stopPx)
"#;
        let prog = validate_source(src).unwrap();
        // bar 0'da giriş (100); bar 3'te low=95 < 97 → stop tetiklenir
        let mut bars = rising(10);
        bars[3].l = "95".into();
        bars[3].c = "96".into();
        let res = run(&prog, &bars, &PineInputs::new()).unwrap();
        assert!(res.verdicts[0].entry, "bar 0'da giriş");
        assert!(!res.verdicts[1].exit && !res.verdicts[2].exit, "düşmeden önce exit yok");
        assert!(res.verdicts[3].exit, "sabit 97 stop bar 3'te tetiklenmeli (low=95 < 97)");
    }

    #[test]
    fn compile_rejects_unknown_function_and_var() {
        assert!(validate_source("x = ta.unknown(close, 5)").is_err());
        assert!(validate_source("y = close + zzz").is_err(), "tanımlanmamış değişken reddedilmeli");
        assert!(validate_source("").is_err());
    }

    #[test]
    fn na_warmup_blocks_entry() {
        // sma(50) 50 bar warmup ister: 30 barda hep na → giriş olamaz
        let src = "s = ta.sma(close, 50)\nif close > s\n    strategy.entry(\"L\", strategy.long)\n";
        let prog = validate_source(src).unwrap();
        let res = run(&prog, &rising(30), &PineInputs::new()).unwrap();
        assert!(res.verdicts.iter().all(|v| !v.entry), "warmup'ta na → karşılaştırma false → giriş olamaz");
    }

    #[test]
    fn var_persists_and_history_works() {
        let src = r#"//@version=5
strategy("Hist")
var peak = close
if high > peak
    peak := high
if close < peak * 0.95
    strategy.entry("L", strategy.long)
prevClose = close[1]
"#;
        let prog = validate_source(src).unwrap();
        let mut bars = rising(20);
        bars[15].c = "80".into();
        bars[15].o = "80".into();
        bars[15].h = "80".into();
        bars[15].l = "80".into();
        let res = run(&prog, &bars, &PineInputs::new()).unwrap();
        assert!(res.verdicts[15].entry, "peak'ten %5+ düşüş giriş üretmeli");
    }

    #[test]
    fn ternary_and_math_builtins() {
        let src = r#"//@version=5
strategy("T")
d = math.abs(close - close[1])
m = math.max(high, low)
rng = m - m ? d : 0
if rng >= 0
    strategy.entry("L", strategy.long)
"#;
        let prog = validate_source(src).unwrap();
        let res = run(&prog, &rising(5), &PineInputs::new()).unwrap();
        assert!(res.verdicts[0].entry, "koşul bar 0'da zaten doğru — ilk bar girişi");
    }

    #[test]
    fn fraction_math_no_float_drift() {
        // 0.1 × 3 tam kesir olmalı: 3/10; Pine'ta float iken bizde kesir.
        let src = r#"//@version=5
strategy("F")
x = close * 0.1 * 3
if x == close * 0.3
    strategy.entry("L", strategy.long)
"#;
        let prog = validate_source(src).unwrap();
        let res = run(&prog, &rising(3), &PineInputs::new()).unwrap();
        assert!(res.verdicts[0].entry, "kesirli aritmetikte 0.1*3 == 0.3 birebir olmalı");
    }

    #[test]
    fn short_entry_reported() {
        // RSI aşırı alımda short sinyali — yükselen seride RSI=100
        let src = r#"//@version=5
strategy("S")
if ta.rsi(close, 3) > 90
    strategy.entry("S", strategy.short)
"#;
        let prog = validate_source(src).unwrap();
        let bars = rising(10);
        let res = run(&prog, &bars, &PineInputs::new()).unwrap();
        let has_short = res.verdicts.iter().any(|v| v.entry && v.entry_long == Some(false));
        assert!(has_short, "short giriş sinyali raporlanmalı");
    }

    #[test]
    fn ta_call_history_access_works() {
        // ta.sma(close,1) == close olduğuna göre sma(close,1)[1] == close[1]
        // olmalı — ta-call üzerinde [n] artık destekleniyor.
        let src = r#"//@version=5
strategy("H")
if ta.sma(close, 1)[1] == close[1]
    strategy.entry("L", strategy.long)
"#;
        let prog = validate_source(src).unwrap();
        let bars = rising(5);
        // bar 1'den itibaren close[1] mevcut → bar >= 1'de giriş beklenir
        let res = run(&prog, &bars, &PineInputs::new()).unwrap();
        assert!(!res.verdicts[0].entry, "bar 0'da close[1] yok → koşul false");
        assert!(res.verdicts[1].entry, "bar 1'de sma(1)[1]==close[1] olmalı");
    }

    #[test]
    fn conditional_ta_note_emitted() {
        // Koşullu gövdedeki ta.* çağrısı derleme-anı dürüst uyarı üretir
        // (TradingView "series consistency" uyarısının karşılığı).
        let src = r#"//@version=5
strategy("C")
if close > close[1]
    s = ta.sma(close, 5)
    strategy.entry("L", strategy.long)
"#;
        let prog = validate_source(src).unwrap();
        assert!(!prog.notes.is_empty(), "koşullu ta.* uyarısı üretilmeli");
        assert!(prog.notes[0].contains("koşullu kapsam"), "got: {:?}", prog.notes);
        // koşul konumundaki ta.* koşullu DEĞİL (her barda çalışır):
        let ok = r#"//@version=5
strategy("O")
if ta.crossover(close, open)
    strategy.entry("L", strategy.long)
"#;
        let p2 = validate_source(ok).unwrap();
        assert!(p2.notes.is_empty(), "koşul konumundaki ta.* uyarı ÜRETME MELİ: {:?}", p2.notes);
    }

    #[test]
    fn pyramiding_param_honest_note() {
        let src = r#"//@version=5
strategy("P", pyramiding=3)
if close > close[1]
    strategy.entry("L", strategy.long)
"#;
        let prog = validate_source(src).unwrap();
        assert_eq!(prog.decl.as_ref().unwrap().pyramiding, Some(3));
        assert!(prog.notes.iter().any(|n| n.contains("pyramiding")), "got: {:?}", prog.notes);
        // pyramiding=1 → uyarı yok (desteklenen tek değer zaten)
        let ok = "strategy(\"P1\", pyramiding=1)\n";
        let p2 = validate_source(ok).unwrap();
        assert!(p2.notes.is_empty(), "pyramiding=1 uyarı üretmemeli: {:?}", p2.notes);
    }

    #[test]
    fn plot_series_recorded() {
        let src = r#"//@version=5
strategy("PL")
plot("Fiyat", close)
plot("Sadece Başlık")
"#;
        let prog = validate_source(src).unwrap();
        let bars = rising(4);
        let res = run(&prog, &bars, &PineInputs::new()).unwrap();
        let s = res.plots.get("Fiyat").expect("seri plot kaydedilmeli");
        assert_eq!(s.len(), 4, "bar başına bir değer");
        assert_eq!(s[0], "100");
        assert_eq!(s[3], "103");
        assert!(!res.plots.contains_key("Sadece Başlık"), "serisiz plot veri üretmez");
    }

    #[test]
    fn period_over_ring_cap_is_honest_error() {
        // 600 üstü periyot kalıcı na üretirdi — artık dürüst hata (fail-closed).
        let src = "s = ta.sma(close, 601)\nif close > s\n    strategy.entry(\"L\", strategy.long)\n";
        let prog = validate_source(src).unwrap();
        let err = run(&prog, &rising(3), &PineInputs::new()).unwrap_err();
        assert!(err.contains("ring kapasitesi"), "got: {err}");
    }

    #[test]
    fn entry_when_false_blocks_every_bar() {
        // when=false → koşulsuz entry bile hiç sinyal üretmemeli (fail-closed).
        let src = "strategy(\"W\")\nstrategy.entry(\"L\", strategy.long, when=false)\n";
        let prog = validate_source(src).unwrap();
        let res = run(&prog, &rising(10), &PineInputs::new()).unwrap();
        assert!(res.verdicts.iter().all(|v| !v.entry), "when=false iken giriş olmamalı");
    }

    #[test]
    fn entry_when_series_expression_gates_bars() {
        // when= seri ifade: true barlarda giriş, false barlarda yok.
        let src = "strategy(\"W\")\nstrategy.entry(\"L\", strategy.long, when=close > close[1])\n";
        let prog = validate_source(src).unwrap();
        let res = run(&prog, &rising(5), &PineInputs::new()).unwrap();
        assert!(!res.verdicts[0].entry, "bar 0'da close[1] yok → when false");
        assert!(res.verdicts[1].entry, "yükselen seride when true → giriş");
    }

    #[test]
    fn close_when_false_never_exits() {
        // when=false → kapanış komutu hiç çalışmaz (entry'deki sessiz-yutma
        // sınıfı close'ta da kapatıldı).
        let src = "strategy(\"W\")\nstrategy.entry(\"L\", strategy.long)\nstrategy.close(\"L\", when=false)\n";
        let prog = validate_source(src).unwrap();
        let res = run(&prog, &rising(10), &PineInputs::new()).unwrap();
        assert!(res.verdicts.iter().any(|v| v.entry), "giriş olmalı");
        assert!(res.verdicts.iter().all(|v| !v.exit), "when=false iken çıkış olmamalı");
    }

    #[test]
    fn exit_when_false_ignores_stop() {
        // when=false → stop/limit yerleşmez, pozisyon stop'a rağmen kapanmaz.
        let src = "strategy(\"W\")\nstrategy.entry(\"L\", strategy.long)\nstrategy.exit(\"X\", stop=50, when=false)\n";
        let prog = validate_source(src).unwrap();
        let mut bars = rising(10);
        bars[5].l = "10".into();
        bars[5].c = "11".into();
        let res = run(&prog, &bars, &PineInputs::new()).unwrap();
        assert!(res.verdicts.iter().all(|v| !v.exit), "when=false iken stop tetiklenmemeli");
    }

    #[test]
    fn entry_when_na_blocks() {
        // when=na → falsy sayılır, giriş yok (fail-closed üçüncü ayağı).
        let src = "strategy(\"W\")\nstrategy.entry(\"L\", strategy.long, when=na)\n";
        let prog = validate_source(src).unwrap();
        let res = run(&prog, &rising(5), &PineInputs::new()).unwrap();
        assert!(res.verdicts.iter().all(|v| !v.entry), "when=na iken giriş olmamalı");
    }

    #[test]
    fn close_when_true_still_closes() {
        // Pozitif kontrol: kapı stuck-closed olursa bu test kızarır.
        let src = "strategy(\"W\")\nstrategy.entry(\"L\", strategy.long)\nstrategy.close(\"L\", when=true)\n";
        let prog = validate_source(src).unwrap();
        let res = run(&prog, &rising(5), &PineInputs::new()).unwrap();
        assert!(res.verdicts.iter().any(|v| v.exit), "when=true iken kapanış olmalı");
    }

    #[test]
    fn exit_when_true_places_stop() {
        // Pozitif kontrol: when=true stop yerleştirir ve tetiklenir.
        let src = "strategy(\"W\")\nstrategy.entry(\"L\", strategy.long)\nstrategy.exit(\"X\", stop=50, when=true)\n";
        let prog = validate_source(src).unwrap();
        let mut bars = rising(10);
        bars[5].l = "10".into();
        bars[5].c = "11".into();
        let res = run(&prog, &bars, &PineInputs::new()).unwrap();
        assert!(res.verdicts[5].exit, "when=true iken stop bar 5'te tetiklenmeli");
    }
}
