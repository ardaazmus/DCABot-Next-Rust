//! Pine alt-küme parser: satır bazlı özyinelemeli iniş.
//! Bloklar girinti ile belirlenir (Pine Script v5 stili).
//! Her Call düğümüne derleme anında deterministik bir node_id atanır —
//! evaluator, ta.* durumlarını bu id ile saklar (çalıştırmalar arası tutarlı).

use crate::lexer::{lex, Line, Tok};
use std::collections::BTreeSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinKind {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Lt,
    Le,
    Gt,
    Ge,
    Eq,
    Ne,
    And,
    Or,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnKind {
    Neg,
    Not,
}

#[derive(Debug, Clone)]
pub struct Arg {
    pub name: Option<String>,
    pub value: Expr,
}

#[derive(Debug, Clone)]
pub enum Expr {
    Num(String),        // ondalık literal — evaluator'da kesirli Fraction'a çevrilir
    Str(String),
    Bool(bool),
    Na,
    Ident(String),
    /// strategy.long (true) / strategy.short (false)
    StrategyDir(bool),
    Bin(BinKind, Box<Expr>, Box<Expr>),
    Un(UnKind, Box<Expr>),
    Ternary(Box<Expr>, Box<Expr>, Box<Expr>),
    /// name(args...) — node_id, ta durum anahtarıdır
    Call { name: String, args: Vec<Arg>, node_id: usize },
    /// seri geçmiş erişimi: expr[n]
    Hist(Box<Expr>, usize),
}

#[derive(Debug, Clone)]
pub enum Stmt {
    /// `name = expr` (is_var: `var name = expr`)
    Declare { name: String, is_var: bool, via_input: bool, expr: Expr, line: usize },
    /// `name := expr`
    Reassign { name: String, expr: Expr, line: usize },
    ExprStmt(Expr, usize),
    If {
        cond: Expr,
        then: Vec<Stmt>,
        elifs: Vec<(Expr, Vec<Stmt>)>,
        else_: Option<Vec<Stmt>>,
        line: usize,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptDecl {
    pub kind: DeclKind,
    pub title: String,
    /// strategy("ad", pyramiding=N) — desteklenmez; >1 ise dürüst uyarı üretilir.
    pub pyramiding: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeclKind {
    Strategy,
    Indicator,
}

#[derive(Debug, Clone)]
pub struct Program {
    pub stmts: Vec<Stmt>,
    pub decl: Option<ScriptDecl>,
    /// plot() başlıkları (bilgi amaçlı; değerler evaluate edilmez)
    pub plot_titles: Vec<String>,
    /// input.* ile tanımlanan değişken adları (UI parametre paneli için)
    pub input_names: Vec<String>,
    /// kullanılan fonksiyon adları (doğrulama çıktısı için)
    pub functions_used: BTreeSet<String>,
    /// Derleme-anı dürüst uyarılar (sessiz davranış sapmalarını görünür kılar):
    /// - ta.* koşullu kapsam içinde (TradingView "series consistency" uyarısının
    ///   birebir karşılığı — Pine da koşullu bloğu atlayan ta çağrısını o bar
    ///   ÇALIŞTIRMADIĞI için seri kayar; biz de birebir aynı davranışı verir
    ///   ve kullanıcıyı uyarır)
    /// - pyramiding=N desteklenmez (1 olarak işlenir)
    pub notes: Vec<String>,
    pub next_node_id: usize,
}

/// Compile: kaynak kodu AST'e çevirir. Bilinmeyen fonksiyon/sembol derleme
/// HATASIDIR — desteklenen alt küme kullanıcıya açıkça document edilir.
pub fn compile(src: &str) -> Result<Program, String> {
    let lines = lex(src)?;
    let mut p = Parser {
        lines,
        pos: 0,
        next_node_id: 1,
        plot_titles: Vec::new(),
        input_names: Vec::new(),
        functions_used: BTreeSet::new(),
        decl: None,
        notes: Vec::new(),
    };
    let stmts = p.parse_block(0, true)?;
    if p.pos < p.lines.len() {
        let l = &p.lines[p.pos];
        return Err(format!("satır {}: beklenmeyen girinti (blok dışı satır)", l.no));
    }
    // Post-pass: tanımlanmamış değişken denetimi (yazım hataları derlemede
    // yakalanır — sessiz na yayılımı yerine net hata).
    let mut declared: BTreeSet<String> = BTreeSet::new();
    collect_declared(&stmts, &mut declared);
    validate_idents(&stmts, &declared)?;
    // Dürüst uyarılar: koşullu kapsam içindeki ta.* çağrıları (Pine serisi kayması)
    let mut notes: BTreeSet<String> = BTreeSet::new();
    collect_conditional_ta(&stmts, false, &mut notes);
    let mut notes: Vec<String> = notes.into_iter().collect();
    notes.extend(p.notes);
    Ok(Program {
        stmts,
        decl: p.decl,
        plot_titles: p.plot_titles,
        input_names: p.input_names,
        functions_used: p.functions_used,
        notes,
        next_node_id: p.next_node_id,
    })
}

/// Koşullu kapsam (if gövdesi / ternary kolu) içindeki ta.* çağrılarını toplar.
/// NOT: If/ternary KOŞULU her barda değerlendirilir → koşul konumundaki ta.*
/// çağrısı koşullu DEĞİLDİR (and/or iki kol da her barda çalışır — evaluator
/// kısa devre yapmaz). Yalnız gövde/kol konumu koşulludur.
fn collect_conditional_ta(stmts: &[Stmt], in_cond: bool, out: &mut BTreeSet<String>) {
    for st in stmts {
        match st {
            Stmt::Declare { expr, line, .. } => collect_expr_cond_ta(expr, *line, in_cond, out),
            Stmt::Reassign { expr, line, .. } => collect_expr_cond_ta(expr, *line, in_cond, out),
            Stmt::ExprStmt(e, line) => collect_expr_cond_ta(e, *line, in_cond, out),
            Stmt::If { cond, then, elifs, else_, line } => {
                // koşul: her barda çalışır → koşulsuz bağlam
                collect_expr_cond_ta(cond, *line, false, out);
                collect_conditional_ta(then, true, out);
                for (_, b) in elifs {
                    collect_conditional_ta(b, true, out);
                }
                if let Some(b) = else_ {
                    collect_conditional_ta(b, true, out);
                }
            }
        }
    }
}

fn collect_expr_cond_ta(e: &Expr, line: usize, in_cond: bool, out: &mut BTreeSet<String>) {
    match e {
        Expr::Num(_) | Expr::Str(_) | Expr::Bool(_) | Expr::Na | Expr::StrategyDir(_) => {}
        Expr::Ident(_) => {}
        Expr::Bin(_, a, b) => {
            collect_expr_cond_ta(a, line, in_cond, out);
            collect_expr_cond_ta(b, line, in_cond, out);
        }
        Expr::Un(_, a) => collect_expr_cond_ta(a, line, in_cond, out),
        Expr::Ternary(c, a, b) => {
            collect_expr_cond_ta(c, line, in_cond, out);
            collect_expr_cond_ta(a, line, true, out);
            collect_expr_cond_ta(b, line, true, out);
        }
        Expr::Hist(inner, _) => collect_expr_cond_ta(inner, line, in_cond, out),
        Expr::Call { name, args, .. } => {
            if in_cond && name.starts_with("ta.") {
                out.insert(format!(
                    "satır {line}: '{name}' koşullu kapsam içinde — bar atlandığında serisi kayar (TradingView 'series consistency' uyarısıyla aynı davranış); çağrıyı üst scope'a çıkarın"
                ));
            }
            for a in args {
                collect_expr_cond_ta(&a.value, line, in_cond, out);
            }
        }
    }
}

fn collect_declared(stmts: &[Stmt], out: &mut BTreeSet<String>) {
    for st in stmts {
        match st {
            Stmt::Declare { name, .. } => {
                out.insert(name.clone());
            }
            Stmt::Reassign { name, .. } => {
                out.insert(name.clone());
            }
            Stmt::ExprStmt(..) => {}
            Stmt::If { then, elifs, else_, .. } => {
                collect_declared(then, out);
                for (_, b) in elifs {
                    collect_declared(b, out);
                }
                if let Some(b) = else_ {
                    collect_declared(b, out);
                }
            }
        }
    }
}

fn validate_idents(stmts: &[Stmt], declared: &BTreeSet<String>) -> Result<(), String> {
    for st in stmts {
        match st {
            Stmt::Declare { expr, line, .. } => validate_expr_idents(expr, declared, *line)?,
            Stmt::Reassign { expr, line, .. } => validate_expr_idents(expr, declared, *line)?,
            Stmt::ExprStmt(e, line) => validate_expr_idents(e, declared, *line)?,
            Stmt::If { cond, then, elifs, else_, line } => {
                validate_expr_idents(cond, declared, *line)?;
                validate_idents(then, declared)?;
                for (ec, eb) in elifs {
                    validate_expr_idents(ec, declared, *line)?;
                    validate_idents(eb, declared)?;
                }
                if let Some(eb) = else_ {
                    validate_idents(eb, declared)?;
                }
            }
        }
    }
    Ok(())
}

fn validate_expr_idents(e: &Expr, declared: &BTreeSet<String>, line: usize) -> Result<(), String> {
    match e {
        Expr::Ident(id) => {
            if crate::eval::is_builtin_series(id) || declared.contains(id) {
                Ok(())
            } else {
                Err(format!("satır {line}: tanımlanmamış değişken '{id}'"))
            }
        }
        Expr::Num(_) | Expr::Str(_) | Expr::Bool(_) | Expr::Na | Expr::StrategyDir(_) => Ok(()),
        Expr::Bin(_, a, b) => {
            validate_expr_idents(a, declared, line)?;
            validate_expr_idents(b, declared, line)
        }
        Expr::Un(_, a) => validate_expr_idents(a, declared, line),
        Expr::Ternary(c, a, b) => {
            validate_expr_idents(c, declared, line)?;
            validate_expr_idents(a, declared, line)?;
            validate_expr_idents(b, declared, line)
        }
        Expr::Hist(inner, _) => validate_expr_idents(inner, declared, line),
        Expr::Call { args, .. } => {
            for a in args {
                validate_expr_idents(&a.value, declared, line)?;
            }
            Ok(())
        }
    }
}

struct Parser {
    lines: Vec<Line>,
    pos: usize,
    next_node_id: usize,
    plot_titles: Vec<String>,
    input_names: Vec<String>,
    functions_used: BTreeSet<String>,
    decl: Option<ScriptDecl>,
    notes: Vec<String>,
}

impl Parser {
    fn parse_block(&mut self, min_indent: usize, at_top: bool) -> Result<Vec<Stmt>, String> {
        // Blok: min_indent'ten büyük girintili ardışık satırlar; top-level'da
        // girinti 0 (= min_indent) serbest.
        let first_indent = self.lines.get(self.pos).map(|l| l.indent).unwrap_or(0);
        if !at_top && self.pos >= self.lines.len() {
            return Err("blok gövdesi bekleniyordu (dosya sona erdi)".into());
        }
        if !at_top && first_indent <= min_indent {
            return Err(format!(
                "satır {}: blok gövdesi bekleniyordu (girinti artmalı)",
                self.lines[self.pos].no
            ));
        }
        let block_indent = first_indent;
        let mut stmts = Vec::new();
        while self.pos < self.lines.len() {
            let indent = self.lines[self.pos].indent;
            if indent < block_indent {
                break;
            }
            if indent > block_indent {
                let l = &self.lines[self.pos];
                return Err(format!("satır {}: tutarsız girinti ({} > {block_indent})", l.no, indent));
            }
            if let Some(st) = self.parse_statement(block_indent)? {
                stmts.push(st);
            }
        }
        Ok(stmts)
    }
    /// Tek satırlık ifade (statement). Boş/yorum satırları lexer'da zaten atıldı.
    fn parse_statement(&mut self, block_indent: usize) -> Result<Option<Stmt>, String> {
        let line = &self.lines[self.pos];
        let no = line.no;
        let toks = line.tokens.clone();

        // if / else if / else
        if let Some(Tok::Ident(id)) = toks.first() {
            if id == "if" {
                let cond = self.parse_expr_line(&toks, 1, no)?;
                self.pos += 1;
                let then = self.parse_block(block_indent, false)?;
                let mut elifs = Vec::new();
                let mut else_ = None;
                loop {
                    if self.pos >= self.lines.len() || self.lines[self.pos].indent != block_indent {
                        break;
                    }
                    let nt = &self.lines[self.pos].tokens;
                    let is_else_if = matches!(nt.first(), Some(Tok::Ident(k)) if k == "else")
                        && matches!(nt.get(1), Some(Tok::Ident(k)) if k == "if");
                    let is_else = matches!(nt.first(), Some(Tok::Ident(k)) if k == "else") && nt.len() == 1;
                    if is_else_if {
                        let (nt, no2) = {
                            let l = &self.lines[self.pos];
                            (l.tokens.clone(), l.no)
                        };
                        let cond = self.parse_expr_line(&nt, 2, no2)?;
                        self.pos += 1;
                        let body = self.parse_block(block_indent, false)?;
                        elifs.push((cond, body));
                    } else if is_else {
                        self.pos += 1;
                        else_ = Some(self.parse_block(block_indent, false)?);
                        break;
                    } else {
                        break;
                    }
                }
                return Ok(Some(Stmt::If { cond, then, elifs, else_, line: no }));
            }
        }

        // declaration / reassignment
        if let (Some(Tok::Ident(name)), Some(Tok::Op(op))) = (toks.first(), toks.get(1)) {
            match op.as_str() {
                "=" => {
                    let expr = self.parse_expr_line(&toks, 2, no)?;
                    self.pos += 1;
                    let via_input = matches!(&expr, Expr::Call { name, .. } if name.starts_with("input."));
                    if via_input {
                        self.input_names.push(name.clone());
                    }
                    self.check_known_expr(&expr)?;
                    return Ok(Some(Stmt::Declare { name: name.clone(), is_var: false, via_input, expr, line: no }));
                }
                ":=" => {
                    let expr = self.parse_expr_line(&toks, 2, no)?;
                    self.pos += 1;
                    self.check_known_expr(&expr)?;
                    return Ok(Some(Stmt::Reassign { name: name.clone(), expr, line: no }));
                }
                _ => {}
            }
        }
        // `var name = expr`
        if let (Some(Tok::Ident(kw)), Some(Tok::Ident(name)), Some(Tok::Op(op))) = (toks.first(), toks.get(1), toks.get(2)) {
            if kw == "var" && op == "=" {
                let expr = self.parse_expr_line(&toks, 3, no)?;
                self.pos += 1;
                let via_input = matches!(&expr, Expr::Call { name, .. } if name.starts_with("input."));
                if via_input {
                    self.input_names.push(name.clone());
                }
                self.check_known_expr(&expr)?;
                return Ok(Some(Stmt::Declare { name: name.clone(), is_var: true, via_input, expr, line: no }));
            }
        }
        // expression statement (strategy.entry(...) vb.)
        let expr = self.parse_expr_line(&toks, 0, no)?;
        self.pos += 1;
        self.check_known_expr(&expr)?;
        // script bildirimi: strategy("ad") / indicator("ad") — tek seferlik kayıt
        if let Expr::Call { name, args, .. } = &expr {
            if name == "strategy" || name == "indicator" {
                if self.decl.is_some() {
                    return Err(format!("satır {no}: çift script bildirimi (strategy/indicator zaten var)"));
                }
                let title = args.first().and_then(|a| match &a.value {
                    Expr::Str(s) => Some(s.clone()),
                    _ => None,
                });
                // pyramiding=N: parse edilir; >1 ise DÜRÜST uyarı (sessiz yutma yok)
                let mut pyramiding = None;
                for a in args {
                    if a.name.as_deref() == Some("pyramiding") {
                        match &a.value {
                            Expr::Num(n) => match n.parse::<i64>() {
                                Ok(v) => {
                                    if v > 1 {
                                        pyramiding = Some(v);
                                        self.notes.push(format!(
                                            "strategy(pyramiding={v}): pyramiding desteklenmiyor — 1 olarak işlenir (açık pozisyon varken strategy.entry yoksayılır ve uyarı üretilir)"
                                        ));
                                    }
                                }
                                Err(_) => self.notes.push(format!(
                                    "strategy(pyramiding='{n}'): değer okunamadı — 1 olarak işlenir"
                                )),
                            },
                            _ => self.notes.push(
                                "strategy(pyramiding=…): değer sayısal literal değil — 1 olarak işlenir".into(),
                            ),
                        }
                    }
                }
                self.decl = Some(ScriptDecl {
                    kind: if name == "strategy" { DeclKind::Strategy } else { DeclKind::Indicator },
                    title: title.unwrap_or_else(|| "untitled".into()),
                    pyramiding,
                });
                return Ok(None); // bildirim satırı karar üretmez
            }
        }
        Ok(Some(Stmt::ExprStmt(expr, no)))
    }

    fn parse_expr_line(&mut self, toks: &[Tok], start: usize, no: usize) -> Result<Expr, String> {
        let mut p = ExprParser { toks, pos: start, no, owner: self };
        let e = p.parse_ternary()?;
        if !matches!(p.peek(), None | Some(Tok::Newline)) {
            return Err(format!("satır {no}: ifadenin sonunda beklenmeyen token"));
        }
        Ok(e)
    }

    /// Derleme anı sembol denetimi: bilinmeyen fonksiyon/tanımlayıcı reddedilir.
    fn check_known_expr(&mut self, e: &Expr) -> Result<(), String> {
        match e {
            Expr::Num(_) | Expr::Str(_) | Expr::Bool(_) | Expr::Na | Expr::StrategyDir(_) => Ok(()),
            Expr::Ident(id) => {
                if crate::eval::is_builtin_series(id) {
                    Ok(())
                } else {
                    // değişken mi bilinmiyor (evaluator'da da denetlenir) — derlemede
                    // input/var dışı tanımlayıcıya izin ver (blok içinde geç tanım serbest)
                    Ok(())
                }
            }
            Expr::Bin(_, a, b) => {
                self.check_known_expr(a)?;
                self.check_known_expr(b)
            }
            Expr::Un(_, a) => self.check_known_expr(a),
            Expr::Ternary(c, a, b) => {
                self.check_known_expr(c)?;
                self.check_known_expr(a)?;
                self.check_known_expr(b)
            }
            Expr::Hist(inner, _) => self.check_known_expr(inner),
            Expr::Call { name, args, node_id } => {
                if !crate::eval::is_known_function(name) {
                    return Err(format!("bilinmeyen fonksiyon '{name}' — desteklenen alt küme dokümantasyonuna bakın"));
                }
                if name == "plot" {
                    // başlık: ilk isimsiz Str argüman veya title="..." — eval
                    // tarafındaki anahtarlamayla birebir aynı öncelik.
                    let mut title = None;
                    for a in args {
                        if a.name.as_deref() == Some("title") {
                            if let Expr::Str(s) = &a.value {
                                title = Some(s.clone());
                                break;
                            }
                        } else if a.name.is_none() {
                            if let Expr::Str(s) = &a.value {
                                title = Some(s.clone());
                                break;
                            }
                        }
                    }
                    self.plot_titles.push(title.unwrap_or_else(|| format!("plot{node_id}")));
                }
                self.functions_used.insert(name.clone());
                for a in args {
                    self.check_known_expr(&a.value)?;
                }
                Ok(())
            }
        }
    }
}

struct ExprParser<'a> {
    toks: &'a [Tok],
    pos: usize,
    no: usize,
    owner: &'a mut Parser,
}

impl<'a> ExprParser<'a> {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.pos)
    }

    fn eat_op(&mut self, op: &str) -> bool {
        if let Some(Tok::Op(o)) = self.peek() {
            if o == op {
                self.pos += 1;
                return true;
            }
        }
        false
    }

    fn expect_op(&mut self, op: &str) -> Result<(), String> {
        if self.eat_op(op) {
            Ok(())
        } else {
            Err(format!("satır {}: '{op}' bekleniyordu", self.no))
        }
    }

    /// ternary → or → and → cmp → add → mul → unary → postfix(hist) → primary
    fn parse_ternary(&mut self) -> Result<Expr, String> {
        let cond = self.parse_or()?;
        if self.eat_op("?") {
            let a = self.parse_ternary()?;
            self.expect_op(":")?;
            let b = self.parse_ternary()?;
            return Ok(Expr::Ternary(Box::new(cond), Box::new(a), Box::new(b)));
        }
        Ok(cond)
    }

    fn parse_or(&mut self) -> Result<Expr, String> {
        let mut left = self.parse_and()?;
        loop {
            if let Some(Tok::Ident(id)) = self.peek() {
                if id == "and" {
                    self.pos += 1;
                    let right = self.parse_and()?;
                    left = Expr::Bin(BinKind::Or, Box::new(left), Box::new(right));
                    continue;
                }
            }
            break;
        }
        Ok(left)
    }

    fn parse_and(&mut self) -> Result<Expr, String> {
        let mut left = self.parse_cmp()?;
        loop {
            if let Some(Tok::Ident(id)) = self.peek() {
                if id == "or" {
                    self.pos += 1;
                    let right = self.parse_cmp()?;
                    left = Expr::Bin(BinKind::And, Box::new(left), Box::new(right));
                    continue;
                }
            }
            break;
        }
        Ok(left)
    }

    fn parse_cmp(&mut self) -> Result<Expr, String> {
        let left = self.parse_add()?;
        let kind = match self.peek() {
            Some(Tok::Op(o)) => match o.as_str() {
                "<" => Some(BinKind::Lt),
                "<=" => Some(BinKind::Le),
                ">" => Some(BinKind::Gt),
                ">=" => Some(BinKind::Ge),
                "==" => Some(BinKind::Eq),
                "!=" => Some(BinKind::Ne),
                _ => None,
            },
            _ => None,
        };
        if let Some(k) = kind {
            self.pos += 1;
            let right = self.parse_add()?;
            return Ok(Expr::Bin(k, Box::new(left), Box::new(right)));
        }
        Ok(left)
    }

    fn parse_add(&mut self) -> Result<Expr, String> {
        let mut left = self.parse_mul()?;
        loop {
            let k = match self.peek() {
                Some(Tok::Op(o)) if o == "+" => BinKind::Add,
                Some(Tok::Op(o)) if o == "-" => BinKind::Sub,
                _ => break,
            };
            self.pos += 1;
            let right = self.parse_mul()?;
            left = Expr::Bin(k, Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn parse_mul(&mut self) -> Result<Expr, String> {
        let mut left = self.parse_unary()?;
        loop {
            let k = match self.peek() {
                Some(Tok::Op(o)) if o == "*" => BinKind::Mul,
                Some(Tok::Op(o)) if o == "/" => BinKind::Div,
                Some(Tok::Op(o)) if o == "%" => BinKind::Mod,
                _ => break,
            };
            self.pos += 1;
            let right = self.parse_unary()?;
            left = Expr::Bin(k, Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn parse_unary(&mut self) -> Result<Expr, String> {
        if self.eat_op("-") {
            let e = self.parse_unary()?;
            return Ok(Expr::Un(UnKind::Neg, Box::new(e)));
        }
        if let Some(Tok::Ident(id)) = self.peek() {
            if id == "not" {
                self.pos += 1;
                let e = self.parse_unary()?;
                return Ok(Expr::Un(UnKind::Not, Box::new(e)));
            }
        }
        self.parse_postfix()
    }

    fn parse_postfix(&mut self) -> Result<Expr, String> {
        let mut e = self.parse_primary()?;
        while let Some(Tok::Op(o)) = self.peek() {
            if o == "[" {
                self.pos += 1;
                let idx = match self.peek() {
                    Some(Tok::Num(n)) => {
                        let v: i64 = n
                            .parse()
                            .map_err(|_| format!("satır {}: geçmiş indeksi tam sayı olmalı ('{n}')", self.no))?;
                        if v < 0 {
                            return Err(format!("satır {}: geçmiş indeksi negatif olamaz", self.no));
                        }
                        self.pos += 1;
                        v as usize
                    }
                    _ => return Err(format!("satır {}: geçmiş indeksi sayı literal olmalı", self.no)),
                };
                self.expect_op("]")?;
                e = Expr::Hist(Box::new(e), idx);
            } else {
                break;
            }
        }
        Ok(e)
    }

    fn parse_primary(&mut self) -> Result<Expr, String> {
        match self.peek().cloned() {
            Some(Tok::Num(n)) => {
                self.pos += 1;
                Ok(Expr::Num(n))
            }
            Some(Tok::Str(s)) => {
                self.pos += 1;
                Ok(Expr::Str(s))
            }
            Some(Tok::Ident(id)) => {
                self.pos += 1;
                match id.as_str() {
                    "true" => return Ok(Expr::Bool(true)),
                    "false" => return Ok(Expr::Bool(false)),
                    "na" => return Ok(Expr::Na),
                    "strategy.long" => return Ok(Expr::StrategyDir(true)),
                    "strategy.short" => return Ok(Expr::StrategyDir(false)),
                    "and" | "or" | "not" | "if" | "else" | "var" => {
                        return Err(format!("satır {}: ayrılmış sözcük '{id}' ifade içinde kullanılamaz", self.no));
                    }
                    _ => {}
                }
                if self.eat_op("(") {
                    // fonksiyon çağrısı
                    let mut args = Vec::new();
                    if !self.eat_op(")") {
                        loop {
                            // isimli argüman tespiti: `ident = expr` (izleyen token '='
                            // ve ondan sonra operatör DEĞİL ise)
                            let is_named = matches!(self.toks.get(self.pos + 1), Some(Tok::Op(o)) if o == "=")
                                && !matches!(self.toks.get(self.pos + 2), Some(Tok::Op(_)));
                            let name = if is_named {
                                let n = match self.peek() {
                                    Some(Tok::Ident(n)) => n.clone(),
                                    _ => unreachable!("isimli argüman başlangıcı ident olmalı"),
                                };
                                self.pos += 2; // ident + '='
                                Some(n)
                            } else {
                                None
                            };
                            let value = self.parse_ternary()?;
                            args.push(Arg { name, value });
                            if self.eat_op(",") {
                                continue;
                            }
                            self.expect_op(")")?;
                            break;
                        }
                    }
                    let node_id = self.owner.next_node_id;
                    self.owner.next_node_id += 1;
                    return Ok(Expr::Call { name: id, args, node_id });
                }
                Ok(Expr::Ident(id))
            }
            Some(Tok::Op(o)) if o == "(" => {
                self.pos += 1;
                let e = self.parse_ternary()?;
                self.expect_op(")")?;
                Ok(e)
            }
            other => Err(format!("satır {}: beklenmeyen token {other:?}", self.no)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EMA_CROSS: &str = r#"//@version=5
strategy("EMA Cross", overlay=true)
fastLen = input.int(9, "Fast EMA")
slowLen = input.int(21, "Slow EMA")
fast = ta.ema(close, fastLen)
slow = ta.ema(close, slowLen)
longCross = ta.crossover(fast, slow)
if longCross
    strategy.entry("Long", strategy.long)
if ta.crossunder(fast, slow)
    strategy.close_all()
plot("Fast EMA")
"#;

    #[test]
    fn compiles_ema_cross() {
        let p = compile(EMA_CROSS).unwrap();
        assert_eq!(p.decl.as_ref().unwrap().kind, DeclKind::Strategy);
        assert_eq!(p.input_names, vec!["fastLen", "slowLen"]);
        assert!(p.functions_used.contains("ta.ema"));
        assert!(p.functions_used.contains("strategy.entry"));
        assert_eq!(p.plot_titles, vec!["Fast EMA"]);
    }

    #[test]
    fn rejects_unknown_function() {
        let src = "x = ta.superwave(close, 14)\n";
        let err = compile(src).unwrap_err();
        assert!(err.contains("bilinmeyen fonksiyon"), "got: {err}");
    }

    #[test]
    fn rejects_unterminated_block() {
        let src = "if ta.crossover(close, open)\nstrategy.entry(\"L\", strategy.long)\n";
        assert!(compile(src).is_err(), "blok gövdesi girintisiz olamaz");
    }

    #[test]
    fn parses_ternary_and_history() {
        let src = "prev = close[1]\nd = close - close[2]\ns = close > prev ? 1 : 0\n";
        compile(src).unwrap();
    }

    #[test]
    fn node_ids_deterministic() {
        let a = compile(EMA_CROSS).unwrap();
        let b = compile(EMA_CROSS).unwrap();
        assert_eq!(a.next_node_id, b.next_node_id);
    }
}
