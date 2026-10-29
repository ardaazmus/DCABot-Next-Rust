//! Pine alt-küme evaluator: kesirli (Fraction) aritmetik, sıfır float.
//!
//! Semantik kurallar (Pine Script v5 uyumlu alt küme):
//! - Her değişken bir SERİDİR; bar başında önceki barın değeri geçmişe (na
//!   dahil) işlenir. `x[n]` geçmiş erişimi bu seriden okur.
//! - `na` yayılımı: aritmetikte na → na; karşılaştırmada na → false;
//!   koşulda na → false (Pine uyumlu).
//! - ta.* fonksiyonları çağrı düğümü (node_id) başına deterministik durum
//!   tutar; koşullu blok içindeki çağrılar o bar atlandığında seri kayar —
//!   bu TradingView davranışıyla BİREBİRDİR (Pine da koşullu bloğu atlayan
//!   ta çağrısını çalıştırmaz ve "series consistency" uyarısı verir). Parser
//!   aynı uyarıyı derleme-anı notes alanında üretir.
//! - ta.* çağrısı üzerinde [n] geçmiş erişimi desteklenir: düğüm başına çıkış
//!   geçmişi (bar başına tek itiş) tutulur.
//! - strategy.entry sinyali bar t'de üretilir; emir bar t+1 açılışında
//!   dolar (Gate 4 zero look-ahead). strategy.exit stop/limit seviyeleri
//!   pozisyon açıkken her bar yeniden değerlendirilir ve OHLC kesişiminde
//!   exit sinyali üretir.
//! - Zero look-ahead: bar i'nin yürütülmesi yalnız i'ye kadar olan barları
//!   görür (yapısal — evaluator bar dizisi üzerinde ileri akar).
//! - EMA/RMA alfa tam kesirdir: EMA alfa = 2/(len+1) yerine pay/len formu
//!   (pay=2/len) — Pine referansıyla bar-bar birebir aynı kesirli sabit.

use crate::parser::{compile, BinKind, Expr, Program, Stmt, UnKind};
use dca_kernel::Bar;
use dca_math::fraction::Fraction;
use num_bigint::BigInt;
use num_traits::One;
use std::cmp::Ordering;
use std::collections::{BTreeMap, HashMap};

/// Kullanıcı input değerleri: değişken adı → string değer.
pub type PineInputs = BTreeMap<String, String>;

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Num(Fraction),
    Bool(bool),
    Str(String),
    Na,
}

impl Value {
    fn as_num(&self) -> Option<&Fraction> {
        match self {
            Value::Num(f) => Some(f),
            _ => None,
        }
    }
    fn truthy(&self) -> bool {
        match self {
            Value::Bool(b) => *b,
            Value::Num(f) => !f.is_zero(),
            _ => false,
        }
    }
    fn type_name(&self) -> &'static str {
        match self {
            Value::Num(_) => "sayı",
            Value::Bool(_) => "bool",
            Value::Str(_) => "string",
            Value::Na => "na",
        }
    }
}

#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct BarVerdict {
    pub entry: bool,
    /// Some(true) = long giriş, Some(false) = short giriş (None = giriş yok)
    pub entry_long: Option<bool>,
    pub exit: bool,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct RunResult {
    /// Bar başına kararlar (bars ile aynı uzunlukta).
    pub verdicts: Vec<BarVerdict>,
    /// Uyarılar (pyramiding yoksayıldı, aritmetik na döndü vb.).
    pub warnings: Vec<String>,
    /// plot("başlık", seri) değerleri — bar dizinine hizalı kesirli display
    /// stringleri ("na" = o barda değer yok). Yalnız seri argümanlı plot'lar
    /// kaydedilir; koşullu kapsamdaki plot serisi kayabilir (ta.* ile aynı kural).
    pub plots: BTreeMap<String, Vec<String>>,
}

struct VarSlot {
    hist: Vec<Value>,
    cur: Option<Value>,
    var_init: bool,
}

#[derive(Default, Clone)]
struct TaState {
    ring: Vec<Value>,
    cross_prev: Option<(Value, Value)>,
    acc: Option<Fraction>,
    count: Option<i64>,
    /// Faz 6: ta.supertrend taşıyıcı durumu (öz-yineli son bantlar + yön +
    /// kendi RMA-ATR'si; düğüm başına — birden çok supertrend çağrısı izole).
    st: Option<StPrev>,
    /// Faz 12: ta.stoch pencere tamponu — (src, high, low) üçlüsü (düğüm başına).
    stoch: Vec<(Fraction, Fraction, Fraction)>,
    /// Faz 12: ta.adx taşıyıcı durumu (Wilder RMA zinciri; düğüm başına).
    adx: Option<AdxPrev>,
}

/// Faz 12: ta.adx taşıyıcı durum — Wilder'ın TR/±DM RMA düzleştirmesi + ADX.
#[derive(Clone)]
struct AdxPrev {
    prev_high: Fraction,
    prev_low: Fraction,
    prev_close: Fraction,
    s_tr: Fraction,
    s_pdm: Fraction,
    s_mdm: Fraction,
    /// RMA(DX) — ilk DX ile tohumlanır; None iken ADX Na döner (dürüst warmup).
    adx: Option<Fraction>,
}

#[derive(Clone)]
struct StPrev {
    prev_close: Fraction,
    atr: Fraction,
    final_upper: Fraction,
    final_lower: Fraction,
    dir: i8, // 1 = yukarı trend (çizgi = final_lower), -1 = aşağı (çizgi = final_upper)
}

const RING_CAP: usize = 600;

struct BarCur {
    open: Fraction,
    high: Fraction,
    low: Fraction,
    close: Fraction,
    volume: Fraction,
    time: i64,
}

pub struct Vm<'p> {
    inputs: &'p PineInputs,
    vars: HashMap<String, VarSlot>,
    ta: HashMap<usize, TaState>,
    /// ta.* düğüm başına ÇIKTI geçmişi — ta.ema(close,5)[1] gibi geçmiş erişimi
    /// için (bar başına tek itiş; RING_CAP ile sınırli)
    ta_out: HashMap<usize, Vec<Value>>,
    ta_out_bar: HashMap<usize, usize>,
    /// plot("başlık", seri) değerleri — başlık → bar dizisi (display string)
    plot_series: BTreeMap<String, Vec<String>>,
    plot_bar: HashMap<String, usize>,
    /// gömülü serilerin bar geçmişi (close[1] vb. için) — bar sonunda itilir
    bar_hist: Vec<BarCur>,
    prev_close: Option<Fraction>,
    pos_open: bool,
    pos_pending: bool,
    pos_dir: bool,
    stop: Option<Fraction>,
    limit: Option<Fraction>,
    bar: BarVerdict,
    warnings: Vec<String>,
    cur_bar: BarCur,
    /// geçerli bar indeksi (ta_out/plot tek-itiş guard'ı için)
    bar_idx: usize,
}

/// Derlenmiş programı bar dizisi üzerinden çalıştırır (zero look-ahead:
/// bar i yürütülürken yalnız i'ye kadar olan barlar görülebilir).
pub fn run(prog: &Program, bars: &[Bar], inputs: &PineInputs) -> Result<RunResult, String> {
    let mut vm = Vm {
        inputs,
        vars: HashMap::new(),
        ta: HashMap::new(),
        ta_out: HashMap::new(),
        ta_out_bar: HashMap::new(),
        plot_series: BTreeMap::new(),
        plot_bar: HashMap::new(),
        bar_hist: Vec::new(),
        prev_close: None,
        pos_open: false,
        pos_pending: false,
        pos_dir: true,
        stop: None,
        limit: None,
        bar: BarVerdict::default(),
        warnings: Vec::new(),
        cur_bar: BarCur {
            open: Fraction::zero(),
            high: Fraction::one(),
            low: Fraction::one(),
            close: Fraction::one(),
            volume: Fraction::zero(),
            time: 0,
        },
        bar_idx: 0,
    };
    let mut verdicts = Vec::with_capacity(bars.len());
    for (i, b) in bars.iter().enumerate() {
        vm.bar_idx = i;
        vm.cur_bar = BarCur {
            open: Fraction::from_decimal_str(&b.o).map_err(|e| format!("bar {i} open '{}': {e}", b.o))?,
            high: Fraction::from_decimal_str(&b.h).map_err(|e| format!("bar {i} high '{}': {e}", b.h))?,
            low: Fraction::from_decimal_str(&b.l).map_err(|e| format!("bar {i} low '{}': {e}", b.l))?,
            close: Fraction::from_decimal_str(&b.c).map_err(|e| format!("bar {i} close '{}': {e}", b.c))?,
            volume: Fraction::from_any_str(&b.v).unwrap_or_else(|_| Fraction::zero()),
            time: b.t,
        };
        vm.begin_bar();
        vm.exec_block(&prog.stmts)?;
        let v = vm.bar.clone();
        verdicts.push(v);
        // sonraki bar için pozisyon bekleyişi aktifleşir (giriş barı t → exit
        // değerlendirmesi t+1'den itibaren; emir t+1 açılışında dolar)
        if vm.pos_pending {
            vm.pos_open = true;
            vm.pos_pending = false;
        }
        vm.prev_close = Some(vm.cur_bar.close.clone());
        vm.bar_hist.push(BarCur {
            open: vm.cur_bar.open.clone(),
            high: vm.cur_bar.high.clone(),
            low: vm.cur_bar.low.clone(),
            close: vm.cur_bar.close.clone(),
            volume: vm.cur_bar.volume.clone(),
            time: vm.cur_bar.time,
        });
        if vm.bar_hist.len() > RING_CAP {
            vm.bar_hist.remove(0);
        }
        vm.bar = BarVerdict::default();
    }
    Ok(RunResult { verdicts, warnings: vm.warnings, plots: vm.plot_series })
}

/// Tek seferlik çalıştırma: derle + koş.
pub fn run_all(src: &str, bars: &[Bar], inputs: &PineInputs) -> Result<RunResult, String> {
    let prog = compile(src)?;
    run(&prog, bars, inputs)
}

impl<'p> Vm<'p> {
    fn begin_bar(&mut self) {
        // her serinin bu bar değeri geçmişe işlenir (atanmamışsa na)
        for slot in self.vars.values_mut() {
            let v = slot.cur.take().unwrap_or(Value::Na);
            slot.hist.push(v);
            if slot.hist.len() > RING_CAP {
                slot.hist.remove(0);
            }
        }
        self.bar = BarVerdict::default();
    }

    fn warn(&mut self, msg: String) {
        if self.warnings.len() < 32 && !self.warnings.contains(&msg) {
            self.warnings.push(msg);
        }
    }

    fn exec_block(&mut self, stmts: &[Stmt]) -> Result<(), String> {
        for st in stmts {
            self.exec_stmt(st)?;
        }
        Ok(())
    }

    fn exec_stmt(&mut self, st: &Stmt) -> Result<(), String> {
        match st {
            Stmt::Declare { name, is_var, via_input, expr, line } => {
                let exists_var_init = self.vars.get(name).map(|s| s.var_init).unwrap_or(false);
                if *is_var && exists_var_init {
                    return Ok(()); // var: yalnız ilk yürütmede init
                }
                let v = if *via_input {
                    self.eval_input(expr, name, *line)?
                } else {
                    self.eval(expr)?
                };
                match self.vars.get_mut(name) {
                    Some(slot) => {
                        if slot.cur.is_some() {
                            let msg = format!("satır {line}: '{name}' bu barda ikinci kez atandı — son atama geçerli");
                            self.warnings.push(msg);
                        }
                        slot.cur = Some(v);
                        slot.var_init = slot.var_init || *is_var;
                    }
                    None => {
                        self.vars.insert(
                            name.clone(),
                            VarSlot { hist: Vec::new(), cur: Some(v), var_init: *is_var },
                        );
                    }
                }
                Ok(())
            }
            Stmt::Reassign { name, expr, line } => {
                if !self.vars.contains_key(name) {
                    return Err(format!("satır {line}: ':=' ile atanacak '{name}' daha önce tanımlanmadı"));
                }
                let v = self.eval(expr)?;
                if let Some(slot) = self.vars.get_mut(name) {
                    slot.cur = Some(v);
                }
                Ok(())
            }
            Stmt::ExprStmt(e, _) => {
                self.eval(e)?;
                Ok(())
            }
            Stmt::If { cond, then, elifs, else_, line: _ } => {
                let c = self.eval(cond)?;
                if c.truthy() {
                    return self.exec_block(then);
                }
                for (ec, eb) in elifs {
                    let v = self.eval(ec)?;
                    if v.truthy() {
                        return self.exec_block(eb);
                    }
                }
                if let Some(eb) = else_ {
                    return self.exec_block(eb);
                }
                Ok(())
            }
        }
    }

    fn eval_input(&mut self, expr: &Expr, var_name: &str, line: usize) -> Result<Value, String> {
        let (fname, defval) = match expr {
            Expr::Call { name, args, .. } => {
                let d = args
                    .first()
                    .map(|a| &a.value)
                    .ok_or_else(|| format!("satır {line}: input.* için ilk argüman (defval) gerekli"))?;
                (name.as_str(), d)
            }
            _ => return self.eval(expr),
        };
        let Some(user) = self.inputs.get(var_name) else {
            return self.eval(defval);
        };
        let uv = user.trim();
        match fname {
            "input.float" => Fraction::from_any_str(uv)
                .map(Value::Num)
                .map_err(|e| format!("satır {line}: input '{var_name}' değeri '{uv}' sayı değil: {e}")),
            "input.int" => uv
                .parse::<i64>()
                .map(|v| Value::Num(Fraction::from_i64(v)))
                .map_err(|_| format!("satır {line}: input '{var_name}' değeri '{uv}' tam sayı değil")),
            "input.bool" => match uv.to_lowercase().as_str() {
                "true" | "1" | "evet" => Ok(Value::Bool(true)),
                "false" | "0" | "hayır" => Ok(Value::Bool(false)),
                _ => Err(format!("satır {line}: input '{var_name}' değeri '{uv}' bool değil")),
            },
            "input.string" | "input" => Ok(Value::Str(uv.to_string())),
            other => Err(format!("satır {line}: input türü '{other}' bu evaluator'da yok")),
        }
    }

    fn eval(&mut self, e: &Expr) -> Result<Value, String> {
        match e {
            Expr::Num(lit) => {
                let f = Fraction::from_decimal_str(lit).map_err(|e2| format!("sayı literal '{lit}': {e2}"))?;
                Ok(Value::Num(f))
            }
            Expr::Str(s) => Ok(Value::Str(s.clone())),
            Expr::Bool(b) => Ok(Value::Bool(*b)),
            Expr::Na => Ok(Value::Na),
            Expr::StrategyDir(long) => Ok(Value::Bool(*long)),
            Expr::Ident(id) => self.read_ident(id),
            Expr::Hist(inner, n) => {
                // geçmiş erişimi: adlandırılmış değişken, gömülü seri veya ta.* çağrısı
                match inner.as_ref() {
                    Expr::Ident(id) if crate::eval::is_builtin_series(id) => Ok(self.read_hist_builtin(id, *n)),
                    Expr::Ident(id) => Ok(self.read_hist_var(id, *n)),
                    Expr::Call { name, node_id, .. } if name.starts_with("ta.") => {
                        // yan etki: ta ring + çıkış geçmişi bu bar için itilir
                        let cur = self.eval(inner)?;
                        Ok(self.read_ta_hist(*node_id, *n, cur))
                    }
                    _ => Err("geçmiş erişimi [n] değişken, gömülü seri veya ta.* çağrısı üzerinde olabilir".into()),
                }
            }
            Expr::Un(kind, inner) => {
                let v = self.eval(inner)?;
                match kind {
                    UnKind::Neg => match v {
                        Value::Num(f) => f.neg().map(Value::Num).map_err(|er| er.to_string()),
                        Value::Na => Ok(Value::Na),
                        other => Err(format!("tek operand '-': {} sayı değil", other.type_name())),
                    },
                    UnKind::Not => Ok(Value::Bool(!v.truthy())),
                }
            }
            Expr::Bin(kind, a, b) => {
                let av = self.eval(a)?;
                let bv = self.eval(b)?;
                self.bin_op(*kind, av, bv)
            }
            Expr::Ternary(c, a, b) => {
                let cv = self.eval(c)?;
                if cv.truthy() {
                    self.eval(a)
                } else {
                    self.eval(b)
                }
            }
            Expr::Call { name, args, node_id } => {
                let v = self.eval_call(name, args, *node_id)?;
                if name.starts_with("ta.") {
                    self.record_ta_out(*node_id, v.clone());
                }
                Ok(v)
            }
        }
    }

    fn read_ident(&mut self, id: &str) -> Result<Value, String> {
        let b = &self.cur_bar;
        match id {
            "open" => return Ok(Value::Num(b.open.clone())),
            "high" => return Ok(Value::Num(b.high.clone())),
            "low" => return Ok(Value::Num(b.low.clone())),
            "close" => return Ok(Value::Num(b.close.clone())),
            "volume" => return Ok(Value::Num(b.volume.clone())),
            "time" => return Ok(Value::Num(Fraction::from_i64(b.time))),
            "hl2" => {
                let s = b.high.add(&b.low).map_err(|e| e.to_string())?;
                return Ok(Value::Num(s.div(&Fraction::from_u64(2)).map_err(|e| e.to_string())?));
            }
            "hlc3" => {
                let s = b.high.add(&b.low).and_then(|x| x.add(&b.close)).map_err(|e| e.to_string())?;
                return Ok(Value::Num(s.div(&Fraction::from_u64(3)).map_err(|e| e.to_string())?));
            }
            "ohlc4" => {
                let s = b.open.add(&b.high).and_then(|x| x.add(&b.low)).and_then(|x| x.add(&b.close)).map_err(|e| e.to_string())?;
                return Ok(Value::Num(s.div(&Fraction::from_u64(4)).map_err(|e| e.to_string())?));
            }
            _ => {}
        }
        if let Some(slot) = self.vars.get(id) {
            return Ok(slot.cur.clone().or_else(|| slot.hist.last().cloned()).unwrap_or(Value::Na));
        }
        Ok(Value::Na)
    }

    fn read_hist_var(&self, id: &str, n: usize) -> Value {
        match self.vars.get(id) {
            Some(slot) => {
                if n == 0 {
                    slot.cur.clone().or_else(|| slot.hist.last().cloned()).unwrap_or(Value::Na)
                } else {
                    let len = slot.hist.len();
                    if len >= n {
                        slot.hist[len - n].clone()
                    } else {
                        Value::Na
                    }
                }
            }
            None => Value::Na,
        }
    }

    /// Gömülü seri geçmişi: n=0 → bu bar; n>=1 → n bar önce (bar_hist'ten).
    fn read_hist_builtin(&self, id: &str, n: usize) -> Value {
        let pick = |b: &BarCur| -> Value {
            match id {
                "open" => Value::Num(b.open.clone()),
                "high" => Value::Num(b.high.clone()),
                "low" => Value::Num(b.low.clone()),
                "close" => Value::Num(b.close.clone()),
                "volume" => Value::Num(b.volume.clone()),
                "time" => Value::Num(Fraction::from_i64(b.time)),
                "hl2" => {
                    let s = b.high.add(&b.low).unwrap_or_else(|_| Fraction::zero());
                    Value::Num(s.div(&Fraction::from_u64(2)).unwrap_or_else(|_| Fraction::zero()))
                }
                "hlc3" => {
                    let s = b.high.add(&b.low).and_then(|x| x.add(&b.close)).unwrap_or_else(|_| Fraction::zero());
                    Value::Num(s.div(&Fraction::from_u64(3)).unwrap_or_else(|_| Fraction::zero()))
                }
                _ => {
                    let s = b.open.add(&b.high).and_then(|x| x.add(&b.low)).and_then(|x| x.add(&b.close)).unwrap_or_else(|_| Fraction::zero());
                    Value::Num(s.div(&Fraction::from_u64(4)).unwrap_or_else(|_| Fraction::zero()))
                }
            }
        };
        if n == 0 {
            pick(&self.cur_bar)
        } else if self.bar_hist.len() >= n {
            pick(&self.bar_hist[self.bar_hist.len() - n])
        } else {
            Value::Na
        }
    }

    fn bin_op(&mut self, kind: BinKind, a: Value, b: Value) -> Result<Value, String> {
        use BinKind::*;
        match kind {
            And => return Ok(Value::Bool(a.truthy() && b.truthy())),
            Or => return Ok(Value::Bool(a.truthy() || b.truthy())),
            _ => {}
        }
        let cmp_res = match kind {
            Lt | Le | Gt | Ge | Eq | Ne => match (a.as_num(), b.as_num()) {
                (Some(x), Some(y)) => Some(Fraction::cmp(x, y)),
                _ => {
                    if matches!(a, Value::Na) || matches!(b, Value::Na) {
                        return Ok(Value::Bool(false)); // na karşılaştırma → false (Pine uyumlu)
                    }
                    if matches!(kind, Eq | Ne) {
                        if let (Value::Str(x), Value::Str(y)) = (&a, &b) {
                            let eq = x == y;
                            return Ok(Value::Bool(if kind == Eq { eq } else { !eq }));
                        }
                    }
                    return Err(format!(
                        "karşılaştırma tipleri uyumsuz: {} {} {}",
                        a.type_name(),
                        op_sym(kind),
                        b.type_name()
                    ));
                }
            },
            _ => None,
        };
        if let Some(ord) = cmp_res {
            return Ok(Value::Bool(match kind {
                Lt => ord == Ordering::Less,
                Le => ord != Ordering::Greater,
                Gt => ord == Ordering::Greater,
                Ge => ord != Ordering::Less,
                Eq => ord == Ordering::Equal,
                Ne => ord != Ordering::Equal,
                _ => unreachable!(),
            }));
        }
        match (a.as_num(), b.as_num()) {
            (Some(x), Some(y)) => {
                let r = match kind {
                    Add => x.add(y),
                    Sub => x.sub(y),
                    Mul => x.mul(y),
                    Div => x.div(y),
                    Mod => match x.div(y).and_then(|q| floor_frac(&q)) {
                        Ok(fl) => match Fraction::new(fl, BigInt::one()) {
                            Ok(fi) => fi.mul(y).and_then(|p| x.sub(&p)),
                            Err(er) => Err(er),
                        },
                        Err(er) => Err(er),
                    },
                    _ => unreachable!(),
                };
                match r {
                    Ok(v) => Ok(Value::Num(v)),
                    Err(e) => {
                        self.warn(format!("aritmetik hatası ({} {} {}): {e} — na döndü", a.type_name(), op_sym(kind), b.type_name()));
                        Ok(Value::Na)
                    }
                }
            }
            _ => {
                if matches!(a, Value::Na) || matches!(b, Value::Na) {
                    return Ok(Value::Na);
                }
                Err(format!(
                    "aritmetik tipler uyumsuz: {} {} {}",
                    a.type_name(),
                    op_sym(kind),
                    b.type_name()
                ))
            }
        }
    }

    fn ta_ring(&mut self, node_id: usize, v: Value) -> Vec<Value> {
        let st = self.ta.entry(node_id).or_default();
        st.ring.push(v);
        if st.ring.len() > RING_CAP {
            st.ring.remove(0);
        }
        st.ring.clone()
    }

    /// ta.* çıkışını düğüm geçmişine iter (bar başına tek itiş — aynı düğüm
    /// aynı barda ikinci kez değerlendirilirse geçmiş bozulmaz).
    fn record_ta_out(&mut self, node_id: usize, v: Value) {
        if self.ta_out_bar.get(&node_id) == Some(&self.bar_idx) {
            return;
        }
        self.ta_out_bar.insert(node_id, self.bar_idx);
        let st = self.ta_out.entry(node_id).or_default();
        st.push(v);
        if st.len() > RING_CAP {
            st.remove(0);
        }
    }

    /// ta.* çağrısı üzerinde [n]: n=0 → bu bar; n>=1 → n bar önceki çıkış.
    /// Çağrı bu barda hiç çalışmadıysa geçmiş uzamaz — koşullu kapsamdaki ta.*
    /// ile aynı series-consistency kuralı geçerli.
    fn read_ta_hist(&self, node_id: usize, n: usize, cur: Value) -> Value {
        if n == 0 {
            return cur;
        }
        match self.ta_out.get(&node_id) {
            Some(v) => {
                let len = v.len();
                if len > n {
                    v[len - 1 - n].clone()
                } else {
                    Value::Na
                }
            }
            None => Value::Na,
        }
    }

    /// Adanmış when= kapısı: yoksa true (eski davranış korunur); varsa o bar
    /// değerlendirilip falsy/na → false (komut çalışmaz, fail-closed).
    /// strategy.entry/close/close_all/exit ortak kapısı — tek sahip.
    fn when_open(&mut self, args: &Vec<crate::parser::Arg>) -> Result<bool, String> {
        match args.iter().find(|a| a.name.as_deref() == Some("when")) {
            None => Ok(true),
            Some(w) => Ok(self.eval(&w.value)?.truthy()),
        }
    }

    fn eval_call(&mut self, name: &str, args: &Vec<crate::parser::Arg>, node_id: usize) -> Result<Value, String> {
        match name {
            /* ---------------- input ---------------- */
            "input.float" | "input.int" | "input.bool" | "input.string" | "input" => {
                match args.first() {
                    Some(a) => self.eval(&a.value),
                    None => Ok(Value::Na),
                }
            }
            /* ---------------- strategy ---------------- */
            "strategy.entry" => {
                // when=false/na → komut o barda hiç çalışmaz (fail-closed).
                if !self.when_open(args)? {
                    return Ok(Value::Na);
                }
                let dir = match args.get(1).map(|a| &a.value) {
                    Some(Expr::StrategyDir(l)) => *l,
                    Some(other) => {
                        let v = self.eval(other)?;
                        v.truthy()
                    }
                    None => true,
                };
                if self.pos_open {
                    self.warn("pyramiding desteklenmiyor — açık pozisyon varken strategy.entry yoksayıldı".into());
                } else {
                    self.bar.entry = true;
                    self.bar.entry_long = Some(dir);
                    self.pos_pending = true;
                    self.pos_dir = dir;
                    self.stop = None;
                    self.limit = None;
                }
                Ok(Value::Na)
            }
            "strategy.close_all" | "strategy.close" => {
                if !self.when_open(args)? {
                    return Ok(Value::Na);
                }
                if self.pos_open {
                    self.bar.exit = true;
                    self.pos_open = false;
                    self.pos_pending = false;
                    self.stop = None;
                    self.limit = None;
                }
                Ok(Value::Na)
            }
            "strategy.exit" => {
                if !self.when_open(args)? {
                    return Ok(Value::Na);
                }
                let mut new_stop = None;
                let mut new_limit = None;
                for a in args {
                    match a.name.as_deref() {
                        Some("stop") => {
                            let v = self.eval(&a.value)?;
                            new_stop = v.as_num().cloned();
                        }
                        Some("limit") => {
                            let v = self.eval(&a.value)?;
                            new_limit = v.as_num().cloned();
                        }
                        _ => {}
                    }
                }
                if new_stop.is_some() {
                    self.stop = new_stop;
                }
                if new_limit.is_some() {
                    self.limit = new_limit;
                }
                if self.pos_open {
                    let (h, l) = (self.cur_bar.high.clone(), self.cur_bar.low.clone());
                    let hit_stop = match (&self.stop, self.pos_dir) {
                        (Some(s), true) => Fraction::cmp(&l, s) != Ordering::Greater, // long: low <= stop
                        (Some(s), false) => Fraction::cmp(&h, s) != Ordering::Less,   // short: high >= stop
                        _ => false,
                    };
                    let hit_limit = match (&self.limit, self.pos_dir) {
                        (Some(t), true) => Fraction::cmp(&h, t) != Ordering::Less,     // long: high >= limit
                        (Some(t), false) => Fraction::cmp(&l, t) != Ordering::Greater, // short: low <= limit
                        _ => false,
                    };
                    if hit_stop || hit_limit {
                        self.bar.exit = true;
                        self.pos_open = false;
                        self.pos_pending = false;
                        self.stop = None;
                        self.limit = None;
                    }
                }
                Ok(Value::Na)
            }
            /* ---------------- ta.* ---------------- */
            "ta.sma" => {
                let src = self.eval_arg(args, 0)?;
                let len = self.len_arg(args, 1)?;
                let ring = self.ta_ring(node_id, src);
                Ok(sma_of(&ring, len))
            }
            "ta.ema" => {
                let src = self.eval_arg(args, 0)?;
                let len = self.len_arg(args, 1)?;
                let ring = self.ta_ring(node_id, src);
                let alpha = Fraction::new(BigInt::from(2u8), BigInt::from((len + 1) as u64)).map_err(|e| e.to_string())?;
                Ok(ema_of(&ring, len, alpha))
            }
            "ta.rma" => {
                let src = self.eval_arg(args, 0)?;
                let len = self.len_arg(args, 1)?;
                let ring = self.ta_ring(node_id, src);
                let alpha = Fraction::new(BigInt::one(), BigInt::from(len as u64)).map_err(|e| e.to_string())?;
                Ok(ema_of(&ring, len, alpha))
            }
            "ta.rsi" => {
                let src = self.eval_arg(args, 0)?;
                let len = self.len_arg(args, 1)?;
                let ring = self.ta_ring(node_id, src);
                Ok(rsi_of(&ring, len))
            }
            "ta.tr" => {
                let tr = self.current_tr();
                let _ = self.ta_ring(node_id, Value::Num(tr.clone()));
                Ok(Value::Num(tr))
            }
            "ta.atr" => {
                let len = self.len_arg(args, 0)?;
                let tr = self.current_tr();
                let ring = self.ta_ring(node_id, Value::Num(tr));
                let alpha = Fraction::new(BigInt::one(), BigInt::from(len as u64)).map_err(|e| e.to_string())?;
                Ok(ema_of(&ring, len, alpha))
            }
            "ta.highest" => {
                let (src, len_idx) = match args.len() {
                    0 => return Err("ta.highest için periyot gerekli".into()),
                    1 => (Value::Num(self.cur_bar.high.clone()), 0),
                    _ => (self.eval_arg(args, 0)?, 1),
                };
                let len = self.len_arg(args, len_idx)?;
                let ring = self.ta_ring(node_id, src);
                Ok(extreme_of(&ring, len, true))
            }
            "ta.lowest" => {
                let (src, len_idx) = match args.len() {
                    0 => return Err("ta.lowest için periyot gerekli".into()),
                    1 => (Value::Num(self.cur_bar.low.clone()), 0),
                    _ => (self.eval_arg(args, 0)?, 1),
                };
                let len = self.len_arg(args, len_idx)?;
                let ring = self.ta_ring(node_id, src);
                Ok(extreme_of(&ring, len, false))
            }
            "ta.change" => {
                let src = self.eval_arg(args, 0)?;
                let n = if args.len() > 1 { self.len_arg(args, 1)? } else { 1 };
                let ring = self.ta_ring(node_id, src);
                let cur = ring.last().cloned().unwrap_or(Value::Na);
                let prev = if ring.len() > n { ring[ring.len() - 1 - n].clone() } else { Value::Na };
                match (cur.as_num(), prev.as_num()) {
                    (Some(c), Some(p)) => c.sub(p).map(Value::Num).map_err(|e| e.to_string()),
                    _ => Ok(Value::Na),
                }
            }
            "ta.mom" => {
                let src = self.eval_arg(args, 0)?;
                let len = self.len_arg(args, 1)?;
                let ring = self.ta_ring(node_id, src);
                let cur = ring.last().cloned().unwrap_or(Value::Na);
                let prev = if ring.len() > len { ring[ring.len() - 1 - len].clone() } else { Value::Na };
                match (cur.as_num(), prev.as_num()) {
                    (Some(c), Some(p)) => c.sub(p).map(Value::Num).map_err(|e| e.to_string()),
                    _ => Ok(Value::Na),
                }
            }
            "ta.crossover" | "ta.crossunder" | "ta.cross" => {
                let a = self.eval_arg(args, 0)?;
                let b = self.eval_arg(args, 1)?;
                let res = {
                    let st = self.ta.entry(node_id).or_default();
                    let res = match (&st.cross_prev, a.as_num(), b.as_num()) {
                        (Some((pa, pb)), Some(an), Some(bn)) => match (pa.as_num(), pb.as_num()) {
                            (Some(pa), Some(pb)) => {
                                let a_above = Fraction::cmp(an, bn) == Ordering::Greater;
                                let a_below = Fraction::cmp(an, bn) == Ordering::Less;
                                let prev_a_le = Fraction::cmp(pa, pb) != Ordering::Greater;
                                let prev_a_ge = Fraction::cmp(pa, pb) != Ordering::Less;
                                match name {
                                    "ta.crossover" => a_above && prev_a_le,
                                    "ta.crossunder" => a_below && prev_a_ge,
                                    _ => (a_above && prev_a_le) || (a_below && prev_a_ge),
                                }
                            }
                            _ => false,
                        },
                        _ => false,
                    };
                    st.cross_prev = Some((a, b));
                    res
                };
                Ok(Value::Bool(res))
            }
            "ta.sum" => {
                let src = self.eval_arg(args, 0)?;
                let len = self.len_arg(args, 1)?;
                let ring = self.ta_ring(node_id, src);
                Ok(sum_of(&ring, len))
            }
            "ta.cum" => {
                let src = self.eval_arg(args, 0)?;
                let st = self.ta.entry(node_id).or_default();
                match (st.acc.clone(), src.as_num()) {
                    (None, Some(f)) => {
                        st.acc = Some(f.clone());
                        Ok(Value::Num(f.clone()))
                    }
                    (Some(a), Some(f)) => {
                        let s = a.add(f).map_err(|e| e.to_string())?;
                        st.acc = Some(s.clone());
                        Ok(Value::Num(s))
                    }
                    (a, _) => Ok(a.map(Value::Num).unwrap_or(Value::Na)),
                }
            }
            "ta.stdev" => {
                let src = self.eval_arg(args, 0)?;
                let len = self.len_arg(args, 1)?;
                let ring = self.ta_ring(node_id, src);
                Ok(stdev_of(&ring, len))
            }
            "ta.supertrend" => {
                // ta.supertrend(factor, atrLen) — çizgiyi döndürür (yukarı trendde
                // final_lower, aşağıda final_upper). close > çizgi ⇔ yukarı trend.
                // Öz-yineleme düğüm durumundan (TaState::st) gelir — seri tutarlılığı
                // için koşulsuz çağrılmalıdır (derleyici ta.* kuralı aynen geçerli).
                let factor = match self.eval_arg(args, 0)?.as_num() {
                    Some(f) if f.is_positive() => f.clone(),
                    _ => return Err("ta.supertrend: factor > 0 olmalı".into()),
                };
                let atr_len = self.len_arg(args, 1)?;
                let tr = self.current_tr();
                let state = self
                    .ta
                    .entry(node_id)
                    .or_default()
                    .st
                    .take();
                let (line, next) = supertrend_step(
                    state.as_ref(),
                    &self.cur_bar.high,
                    &self.cur_bar.low,
                    &self.cur_bar.close,
                    &tr,
                    &factor,
                    atr_len,
                );
                self.ta.entry(node_id).or_default().st = next;
                Ok(line)
            }
            "ta.stoch" => {
                // ta.stoch(src, high, low, len) — %K: 100·(src−lowest(low,len)) /
                // (highest(high,len)−lowest(low,len)). (src,high,low) üçlüsü düğüm
                // penceresinde taşınır; koşulsuz çağrı kuralı aynen geçerli.
                let src = self.eval_arg(args, 0)?;
                let len = if args.len() > 3 { self.len_arg(args, 3)? } else { return Err("ta.stoch: (src, high, low, len) bekleniyor".into()) };
                let st = &mut self.ta.entry(node_id).or_default();
                st.stoch.push((src.as_num().cloned().unwrap_or_else(Fraction::zero), self.cur_bar.high.clone(), self.cur_bar.low.clone()));
                if st.stoch.len() > RING_CAP {
                    st.stoch.remove(0);
                }
                let win = &st.stoch;
                if win.len() < len {
                    return Ok(Value::Na);
                }
                let w = &win[win.len() - len..];
                let hh = w.iter().map(|(_, h, _)| h.clone()).fold(Fraction::zero(), |a: Fraction, b: Fraction| Fraction::max(&a, &b));
                let ll = w.iter().map(|(_, _, l)| l.clone()).fold(Fraction::from_u64(u64::MAX), |a: Fraction, b: Fraction| Fraction::min(&a, &b));
                let src_v = match win.last() {
                    Some((v, _, _)) => v.clone(),
                    None => return Ok(Value::Na),
                };
                let span = match hh.sub(&ll) {
                    Ok(d) => d,
                    Err(_) => return Ok(Value::Na),
                };
                if span.is_zero() {
                    return Ok(Value::Num(Fraction::zero()));
                }
                let num = match src_v.sub(&ll) {
                    Ok(n) => n,
                    Err(_) => return Ok(Value::Na),
                };
                let k = match num.mul(&Fraction::from_u64(100)).and_then(|x| x.div(&span)) {
                    Ok(k) => k,
                    Err(_) => return Ok(Value::Na),
                };
                Ok(Value::Num(k))
            }
            "ta.adx" => {
                // ta.adx(len) — Wilder: TR/±DM RMA düzleştirmesi, DX, ADX=RMA(DX).
                // ADX, DX tohumlanana kadar Na döner (≈2×len bar — dürüst warmup).
                let len = self.len_arg(args, 0)?;
                if len < 2 {
                    return Err("ta.adx: len >= 2 olmalı".into());
                }
                let (high, low, close) =
                    (self.cur_bar.high.clone(), self.cur_bar.low.clone(), self.cur_bar.close.clone());
                let tr = self.current_tr();
                let prev = self.ta.entry(node_id).or_default().adx.take();
                let next = adx_step(prev.as_ref(), &high, &low, &close, &tr, len);
                let out = next.adx.clone().map(Value::Num).unwrap_or(Value::Na);
                self.ta.entry(node_id).or_default().adx = Some(next);
                Ok(out)
            }
            "ta.barssince" => {
                let c = self.eval_arg(args, 0)?;
                let st = self.ta.entry(node_id).or_default();
                let out = if c.truthy() {
                    st.count = Some(0);
                    Value::Num(Fraction::zero())
                } else if let Some(k) = st.count {
                    st.count = Some(k + 1);
                    Value::Num(Fraction::from_i64(k + 1))
                } else {
                    Value::Na
                };
                Ok(out)
            }
            /* ---------------- math.* ---------------- */
            "math.abs" => {
                let v = self.eval_arg(args, 0)?;
                match v.as_num() {
                    Some(f) => f.abs().map(Value::Num).map_err(|e| e.to_string()),
                    None => Ok(Value::Na),
                }
            }
            "math.max" | "math.min" => {
                let mut best: Option<Fraction> = None;
                for a in args {
                    let v = self.eval(&a.value)?;
                    if let Some(f) = v.as_num() {
                        best = Some(match best {
                            None => f.clone(),
                            Some(b) => {
                                if name == "math.max" {
                                    Fraction::max(&b, f)
                                } else {
                                    Fraction::min(&b, f)
                                }
                            }
                        });
                    }
                }
                Ok(best.map(Value::Num).unwrap_or(Value::Na))
            }
            "math.round" | "math.floor" | "math.ceil" => {
                let v = self.eval_arg(args, 0)?;
                match v.as_num() {
                    Some(f) => {
                        let bi = match name {
                            "math.round" => {
                                let half = Fraction::one().div(&Fraction::from_u64(2)).map_err(|e| e.to_string())?;
                                let shifted = f.add(&half).map_err(|e| e.to_string())?;
                                floor_frac(&shifted).map_err(|e| e.to_string())?
                            }
                            "math.floor" => floor_frac(f).map_err(|e| e.to_string())?,
                            _ => ceil_frac(f).map_err(|e| e.to_string())?,
                        };
                        Fraction::new(bi, BigInt::one())
                            .map(Value::Num)
                            .map_err(|e| e.to_string())
                    }
                    None => Ok(Value::Na),
                }
            }
            "math.pow" => {
                let base = self.eval_arg(args, 0)?;
                let exp = self.eval_arg(args, 1)?;
                match (base.as_num(), exp.as_num()) {
                    (Some(b), Some(e)) => {
                        if !e.is_integer() || e.is_negative() {
                            return Err("math.pow: üs negatif olmayan tam sayı olmalı (kesirli üste destek yok)".into());
                        }
                        let ei = e.to_f64_display();
                        if ei > u32::MAX as f64 {
                            return Err("math.pow: üs çok büyük".into());
                        }
                        b.pow_u(ei as u32).map(Value::Num).map_err(|er| er.to_string())
                    }
                    _ => Ok(Value::Na),
                }
            }
            /* ---------------- yardımcılar ---------------- */
            "nz" => {
                let v = self.eval_arg(args, 0)?;
                if matches!(v, Value::Na) {
                    match args.get(1) {
                        Some(a) => self.eval(&a.value),
                        None => Ok(Value::Num(Fraction::zero())),
                    }
                } else {
                    Ok(v)
                }
            }
            "plot" => {
                // plot("başlık") → yalnız başlık (veri yok); plot("başlık", seri),
                // plot(seri, "başlık") veya plot(seri, title="başlık") → seri kaydı
                // (kesirli display string; isimsiz Str argüman başlık sayılır).
                let mut title: Option<String> = None;
                let mut series_val: Option<Value> = None;
                for a in args {
                    if let (Expr::Str(s), None) = (&a.value, a.name.as_deref()) {
                        if title.is_none() {
                            title = Some(s.clone());
                            continue;
                        }
                    }
                    if a.name.as_deref() == Some("title") {
                        let v = self.eval(&a.value)?;
                        if let Value::Str(s) = v {
                            title = Some(s);
                        }
                        continue;
                    }
                    let v = self.eval(&a.value)?;
                    series_val = Some(v);
                }
                if let Some(sv) = series_val {
                    let key = title.unwrap_or_else(|| format!("plot{node_id}"));
                    if self.plot_bar.get(&key) != Some(&self.bar_idx) {
                        self.plot_bar.insert(key.clone(), self.bar_idx);
                        let disp = match sv {
                            Value::Num(f) => frac_display_trim(&f),
                            _ => "na".to_string(),
                        };
                        self.plot_series.entry(key).or_default().push(disp);
                    }
                }
                Ok(Value::Na)
            }
            // script bildirimi parser'da ayrıştırılır; buraya gelmez (güvenlik arm'ı)
            "strategy" | "indicator" => Ok(Value::Na),
            other => Err(format!("bilinmeyen fonksiyon '{other}' (parser'dan sızdı — hata bildirin)")),
        }
    }

    fn eval_arg(&mut self, args: &Vec<crate::parser::Arg>, idx: usize) -> Result<Value, String> {
        match args.get(idx) {
            Some(a) => self.eval(&a.value),
            None => Err(format!("fonksiyon argümanı {idx} eksik")),
        }
    }

    fn len_arg(&mut self, args: &Vec<crate::parser::Arg>, idx: usize) -> Result<usize, String> {
        let v = self.eval_arg(args, idx)?;
        match v.as_num() {
            Some(f) if f.is_integer() && !f.is_negative() => {
                let n = f.to_f64_display();
                if n == 0.0 {
                    Err("periyot 0 olamaz".into())
                } else if n > RING_CAP as f64 {
                    Err(format!(
                        "periyot {RING_CAP}'i (ring kapasitesi) aşamaz — bu uzunlukta ta.* penceresi kalıcı na üretirdi; dürüst hata tercih edildi"
                    ))
                } else {
                    Ok(n as usize)
                }
            }
            Some(_) => Err("periyot tam sayı olmalı".into()),
            None => Err("periyot na olamaz".into()),
        }
    }

    fn current_tr(&self) -> Fraction {
        let b = &self.cur_bar;
        let hl = b.high.sub(&b.low).unwrap_or_else(|_| Fraction::zero());
        match &self.prev_close {
            None => hl,
            Some(pc) => {
                let d1 = b.high.sub(pc).and_then(|x| x.abs()).unwrap_or_else(|_| Fraction::zero());
                let d2 = b.low.sub(pc).and_then(|x| x.abs()).unwrap_or_else(|_| Fraction::zero());
                let m = Fraction::max(&hl, &d1);
                Fraction::max(&m, &d2)
            }
        }
    }
}

fn op_sym(k: BinKind) -> &'static str {
    use BinKind::*;
    match k {
        Add => "+",
        Sub => "-",
        Mul => "*",
        Div => "/",
        Mod => "%",
        Lt => "<",
        Le => "<=",
        Gt => ">",
        Ge => ">=",
        Eq => "==",
        Ne => "!=",
        And => "and",
        Or => "or",
    }
}

fn sum_of(ring: &[Value], len: usize) -> Value {
    let start = ring.len().saturating_sub(len);
    let w = &ring[start..];
    if w.len() < len {
        return Value::Na;
    }
    let mut acc = Fraction::zero();
    for v in w {
        match v.as_num() {
            Some(f) => match acc.add(f) {
                Ok(a) => acc = a,
                Err(_) => return Value::Na,
            },
            None => return Value::Na,
        }
    }
    Value::Num(acc)
}

fn sma_of(ring: &[Value], len: usize) -> Value {
    let start = ring.len().saturating_sub(len);
    let w = &ring[start..];
    if w.len() < len {
        return Value::Na;
    }
    let mut acc = Fraction::zero();
    for v in w {
        match v.as_num() {
            Some(f) => match acc.add(f) {
                Ok(a) => acc = a,
                Err(_) => return Value::Na,
            },
            None => return Value::Na,
        }
    }
    match acc.div(&Fraction::from_u64(len as u64)) {
        Ok(v) => Value::Num(v),
        Err(_) => Value::Na,
    }
}

/// EMA/RMA: ilk `len` değer → SMA tohumu; sonra üstel yumuşatma.
/// alpha TAM KESİR gelir: EMA → 2/(len+1), RMA/Wilder → 1/len (Pine referansı).
fn ema_of(ring: &[Value], len: usize, alpha: Fraction) -> Value {
    if ring.len() < len || len == 0 {
        return Value::Na;
    }
    let start = ring.len() - len;
    let seed_w = &ring[start..];
    let mut seed = Fraction::zero();
    for v in seed_w.iter() {
        match v.as_num() {
            Some(f) => match seed.add(f) {
                Ok(a) => seed = a,
                Err(_) => return Value::Na,
            },
            None => return Value::Na,
        }
    }
    let mut cur = match seed.div(&Fraction::from_u64(len as u64)) {
        Ok(v) => v,
        Err(_) => return Value::Na,
    };
    let one_minus = match Fraction::one().sub(&alpha) {
        Ok(x) => x,
        Err(_) => return Value::Na,
    };
    // tohum sonrası kalan değerler (ring'in sonu = bu bar)
    for v in ring.iter().skip(start + 1) {
        let x = match v.as_num() {
            Some(f) => f,
            None => return Value::Na,
        };
        let a_term = match alpha.mul(x) {
            Ok(t) => t,
            Err(_) => return Value::Na,
        };
        let b_term = match one_minus.mul(&cur) {
            Ok(t) => t,
            Err(_) => return Value::Na,
        };
        cur = match a_term.add(&b_term) {
            Ok(t) => t,
            Err(_) => return Value::Na,
        };
    }
    Value::Num(cur)
}

/// BigInt tam-kök (Newton) — kesir-kök yaklaşımı için.
fn isqrt_bigint(n: &num_bigint::BigInt) -> num_bigint::BigInt {
    use num_bigint::BigInt;
    use num_traits::{One, Zero};
    if n <= &BigInt::zero() {
        return BigInt::zero();
    }
    let two = BigInt::from(2u8);
    // İlk tahmin: bit uzunluğunun yarısı kadar 1'ler.
    let bits = n.bits();
    let mut x = BigInt::one() << ((bits / 2 + 1) as usize);
    loop {
        let nx = (&x + n / &x) / &two;
        if nx >= x {
            break;
        }
        x = nx;
    }
    x
}

/// sqrt(f) ≈ isqrt(num·S)/isqrt(den·S) — S=10^24 ölçekle hassasiyet korunur.
/// Raporlama-sınırı yaklaşımı (half-even felsefesiyle aynı spirit): sinyal
/// eşikleri için deterministik, tam-aritmetik uyumlu değer.
fn sqrt_frac_approx(f: &Fraction) -> Fraction {
    use num_bigint::BigInt;
    
    let scale: BigInt = BigInt::from(10).pow(24);
    let num = f.num() * &scale;
    let den = f.den() * &scale;
    let rn = isqrt_bigint(&num);
    let rd = isqrt_bigint(&den);
    if rd == num_bigint::BigInt::from(0u8) {
        return Fraction::zero();
    }
    Fraction::new(rn, rd).unwrap_or_else(|_| Fraction::zero())
}

/// Popülasyon standart sapması: var = E[x²] − E[x]² (exact) → kök yaklaşık.
fn stdev_of(ring: &[Value], len: usize) -> Value {
    if ring.len() < len {
        return Value::Na;
    }
    let win = &ring[ring.len() - len..];
    let mut sum = Fraction::zero();
    let mut sum_sq = Fraction::zero();
    for v in win {
        let Some(x) = v.as_num() else { return Value::Na };
        sum = match sum.add(x) {
            Ok(t) => t,
            Err(_) => return Value::Na,
        };
        let sq = match x.mul(x) {
            Ok(t) => t,
            Err(_) => return Value::Na,
        };
        sum_sq = match sum_sq.add(&sq) {
            Ok(t) => t,
            Err(_) => return Value::Na,
        };
    }
    let n = Fraction::from_u64(len as u64);
    let mean = match sum.div(&n) {
        Ok(m) => m,
        Err(_) => return Value::Na,
    };
    let mean_sq = match mean.mul(&mean) {
        Ok(m) => m,
        Err(_) => return Value::Na,
    };
    let e_sq = match sum_sq.div(&n) {
        Ok(m) => m,
        Err(_) => return Value::Na,
    };
    let var = match e_sq.sub(&mean_sq) {
        Ok(v) => v,
        Err(_) => return Value::Na,
    };
    if var.is_negative() {
        return Value::Num(Fraction::zero());
    }
    Value::Num(sqrt_frac_approx(&var))
}

/// Supertrend tek-adım: taşıyıcı durumu güncelleyip çizgiyi döndürür.
/// İlk barda ATR=TR ile başlar (TradingView warmup farkı dokümante).
#[allow(clippy::too_many_arguments)]
fn supertrend_step(
    prev: Option<&StPrev>,
    high: &Fraction,
    low: &Fraction,
    close: &Fraction,
    tr: &Fraction,
    factor: &Fraction,
    atr_len: usize,
) -> (Value, Option<StPrev>) {
    let atr = match prev {
        Some(p) => {
            // RMA: atr = (prev*(len-1) + tr) / len
            let num = match p.atr.mul(&Fraction::from_u64((atr_len - 1) as u64)) {
                Ok(t) => t,
                Err(_) => return (Value::Na, None),
            };
            match num.add(tr).and_then(|t| t.div(&Fraction::from_u64(atr_len as u64))) {
                Ok(a) => a,
                Err(_) => return (Value::Na, None),
            }
        }
        None => tr.clone(),
    };
    let hl2 = match high.add(low).and_then(|t| t.div(&Fraction::from_u64(2))) {
        Ok(h) => h,
        Err(_) => return (Value::Na, None),
    };
    let band = match factor.mul(&atr) {
        Ok(b) => b,
        Err(_) => return (Value::Na, None),
    };
    let basic_upper = match hl2.add(&band) {
        Ok(b) => b,
        Err(_) => return (Value::Na, None),
    };
    let basic_lower = match hl2.sub(&band) {
        Ok(b) => b,
        Err(_) => return (Value::Na, None),
    };
    let (final_upper, final_lower, dir) = match prev {
        Some(p) => {
            let fu = if close > &p.final_upper {
                basic_upper
            } else if basic_upper < p.final_upper {
                basic_upper
            } else {
                p.final_upper.clone()
            };
            let fl = if close < &p.final_lower {
                basic_lower
            } else if basic_lower > p.final_lower {
                basic_lower
            } else {
                p.final_lower.clone()
            };
            let d = if p.dir == 1 && close < &p.final_lower {
                -1i8
            } else if p.dir == -1 && close > &p.final_upper {
                1i8
            } else {
                p.dir
            };
            (fu, fl, d)
        }
        None => (basic_upper, basic_lower, 1i8),
    };
    let line = if dir == 1 { final_lower.clone() } else { final_upper.clone() };
    (
        Value::Num(line),
        Some(StPrev { prev_close: close.clone(), atr, final_upper, final_lower, dir }),
    )
}

/// Faz 12: Wilder ADX tek-adım — taşıyıcı durumu güncelleyip yeni durumu döndürür.
fn adx_step(
    prev: Option<&AdxPrev>,
    high: &Fraction,
    low: &Fraction,
    close: &Fraction,
    tr: &Fraction,
    len: usize,
) -> AdxPrev {
    let len_f = Fraction::from_u64(len as u64);
    let _alpha = Fraction::new(num_bigint::BigInt::one(), num_bigint::BigInt::from(len as u64))
        .unwrap_or_else(|_| Fraction::zero());
    let rma = |prev_v: &Fraction, cur: &Fraction| -> Fraction {
        prev_v.mul(&Fraction::from_u64((len - 1) as u64))
            .and_then(|x| x.add(cur))
            .and_then(|x| x.div(&len_f))
            .unwrap_or_else(|_| cur.clone())
    };
    let (up_move, down_move) = match prev {
        Some(p) => (
            high.sub(&p.prev_high).unwrap_or_else(|_| Fraction::zero()),
            p.prev_low.sub(low).unwrap_or_else(|_| Fraction::zero()),
        ),
        None => (Fraction::zero(), Fraction::zero()),
    };
    let (pdm, mdm) = match prev {
        Some(_) => {
            if up_move > down_move && up_move.is_positive() {
                (up_move, Fraction::zero())
            } else if down_move > up_move && down_move.is_positive() {
                (Fraction::zero(), down_move)
            } else {
                (Fraction::zero(), Fraction::zero())
            }
        }
        None => (Fraction::zero(), Fraction::zero()),
    };
    let s_tr = match prev {
        Some(p) => rma(&p.s_tr, tr),
        None => tr.clone(),
    };
    let s_pdm = match prev {
        Some(p) => rma(&p.s_pdm, &pdm),
        None => pdm,
    };
    let s_mdm = match prev {
        Some(p) => rma(&p.s_mdm, &mdm),
        None => mdm,
    };
    // DX = 100·|+DI − −DI| / (+DI + −DI); sıfır bölende DX=0 (trend yok)
    let dx = (|| -> Option<Fraction> {
        let pdi = s_pdm.div(&s_tr).ok()?;
        let mdi = s_mdm.div(&s_tr).ok()?;
        let sum = pdi.add(&mdi).ok()?;
        if sum.is_zero() {
            return Some(Fraction::zero());
        }
        let diff = pdi.sub(&mdi).ok()?.abs().ok()?;
        diff.mul(&Fraction::from_u64(100)).and_then(|x| x.div(&sum)).ok()
    })()
    .unwrap_or_else(Fraction::zero);
    let adx = match prev.and_then(|p| p.adx.clone()) {
        Some(a) => rma(&a, &dx),
        // Tohum: ilk ~len bar sonunda ADX, o ana kadarki DX ortalamasıyla başlar
        None if s_tr.div(&len_f).map(|m| m >= *tr).unwrap_or(false) => dx.clone(),
        None => return AdxPrev {
            prev_high: high.clone(), prev_low: low.clone(), prev_close: close.clone(),
            s_tr, s_pdm, s_mdm, adx: None,
        },
    };
    AdxPrev { prev_high: high.clone(), prev_low: low.clone(), prev_close: close.clone(), s_tr, s_pdm, s_mdm, adx: Some(adx) }
}

fn rsi_of(ring: &[Value], len: usize) -> Value {
    let mut deltas: Vec<Fraction> = Vec::new();
    for w in ring.windows(2) {
        match (w[0].as_num(), w[1].as_num()) {
            (Some(a), Some(b)) => match b.sub(a) {
                Ok(d) => deltas.push(d),
                Err(_) => return Value::Na,
            },
            _ => return Value::Na,
        }
    }
    if deltas.len() < len {
        return Value::Na;
    }
    let mut gains: Vec<Fraction> = Vec::with_capacity(deltas.len());
    let mut losses: Vec<Fraction> = Vec::with_capacity(deltas.len());
    for d in &deltas {
        if d.is_positive() {
            gains.push(d.clone());
            losses.push(Fraction::zero());
        } else {
            gains.push(Fraction::zero());
            losses.push(d.neg().unwrap_or_else(|_| Fraction::zero()));
        }
    }
    let mut ag = Fraction::zero();
    let mut al = Fraction::zero();
    for x in gains.iter().take(len) {
        ag = match ag.add(x) {
            Ok(v) => v,
            Err(_) => return Value::Na,
        };
    }
    for x in losses.iter().take(len) {
        al = match al.add(x) {
            Ok(v) => v,
            Err(_) => return Value::Na,
        };
    }
    ag = match ag.div(&Fraction::from_u64(len as u64)) {
        Ok(v) => v,
        Err(_) => return Value::Na,
    };
    al = match al.div(&Fraction::from_u64(len as u64)) {
        Ok(v) => v,
        Err(_) => return Value::Na,
    };
    let p = Fraction::from_u64(len as u64);
    let pm1 = Fraction::from_u64((len - 1) as u64);
    for i in len..gains.len() {
        ag = match ag.mul(&pm1).and_then(|x| x.add(&gains[i])).and_then(|x| x.div(&p)) {
            Ok(v) => v,
            Err(_) => return Value::Na,
        };
        al = match al.mul(&pm1).and_then(|x| x.add(&losses[i])).and_then(|x| x.div(&p)) {
            Ok(v) => v,
            Err(_) => return Value::Na,
        };
    }
    if al.is_zero() {
        return Value::Num(Fraction::from_u64(100));
    }
    match ag
        .div(&al)
        .ok()
        .and_then(|rs| Fraction::one().add(&rs).ok())
        .and_then(|denom| Fraction::from_u64(100).div(&denom).ok())
        .and_then(|q| Fraction::from_u64(100).sub(&q).ok())
    {
        Some(v) => Value::Num(v),
        None => Value::Na,
    }
}

fn extreme_of(ring: &[Value], len: usize, max: bool) -> Value {
    let start = ring.len().saturating_sub(len);
    let w = &ring[start..];
    if w.len() < len {
        return Value::Na;
    }
    let mut best: Option<Fraction> = None;
    for v in w {
        match v.as_num() {
            Some(f) => {
                best = Some(match best {
                    None => f.clone(),
                    Some(b) => {
                        if max {
                            Fraction::max(&b, f)
                        } else {
                            Fraction::min(&b, f)
                        }
                    }
                });
            }
            None => return Value::Na,
        }
    }
    Value::Num(best.unwrap())
}

/// Fraction → aşağı yuvarlanmış tam sayı BigInt.
fn floor_frac(f: &Fraction) -> Result<BigInt, dca_math::fraction::MathError> {
    f.floor_div(&Fraction::one())
}

/// Fraction → yukarı yuvarlanmış tam sayı BigInt.
fn ceil_frac(f: &Fraction) -> Result<BigInt, dca_math::fraction::MathError> {
    f.ceil_div(&Fraction::one())
}

/// Plot değeri display: 8 ondalık half-even + sondaki sıfırlar kırılır
/// ("100.00000000" → "100", "100.50000000" → "100.5").
fn frac_display_trim(f: &Fraction) -> String {
    let s = f.to_half_even_display(8);
    if s.contains('.') {
        let t = s.trim_end_matches('0').trim_end_matches('.');
        t.to_string()
    } else {
        s
    }
}

/// Gömülü seri adları (parser'ın tanımlayıcı denetimi için).
pub fn is_builtin_series(id: &str) -> bool {
    matches!(id, "open" | "high" | "low" | "close" | "volume" | "time" | "hl2" | "hlc3" | "ohlc4")
}

/// Desteklenen fonksiyon tablosu (parser derleme-anı denetimi bunu kullanır).
pub fn is_known_function(name: &str) -> bool {
    matches!(
        name,
        "ta.sma" | "ta.ema" | "ta.rma" | "ta.rsi" | "ta.tr" | "ta.atr"
            | "ta.highest" | "ta.lowest" | "ta.change" | "ta.mom"
            | "ta.crossover" | "ta.crossunder" | "ta.cross"
            | "ta.sum" | "ta.cum" | "ta.barssince" | "ta.stdev" | "ta.supertrend" | "ta.stoch" | "ta.adx"
            | "math.abs" | "math.max" | "math.min" | "math.round" | "math.floor" | "math.ceil" | "math.pow"
            | "input.float" | "input.int" | "input.bool" | "input.string" | "input"
            | "strategy" | "indicator"
            | "strategy.entry" | "strategy.close_all" | "strategy.close" | "strategy.exit"
            | "nz" | "plot"
    )
}
