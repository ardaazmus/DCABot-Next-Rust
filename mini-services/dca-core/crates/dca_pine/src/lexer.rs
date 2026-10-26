//! Pine alt-küme lexer: satır bazlı tokenizasyon.
//! Girinti (indent) blok yapısını belirler; yorumlar (`//`) atılır,
//! `//@version=5` pragma'sı yutulur. Ifade satırları tek satırda olmalıdır.

#[derive(Debug, Clone, PartialEq)]
pub enum Tok {
    Num(String),       // ondalık literal ("14", "0.02")
    Str(String),       // "..." string literal
    Ident(String),     // tanımlayıcı; nokta içerebilir: ta.ema, strategy.long, input.float
    Op(String),        // operatörler ve ayraçlar
    Newline,           // satır sonu
}

#[derive(Debug, Clone)]
pub struct Line {
    pub indent: usize,
    pub tokens: Vec<Tok>,
    /// 1-tabanlı kaynak satır numarası (hata mesajları için).
    pub no: usize,
}

pub fn lex(src: &str) -> Result<Vec<Line>, String> {
    let mut out = Vec::new();
    for (idx, raw) in src.lines().enumerate() {
        let no = idx + 1;
        // pragma / yorum satırları
        let trimmed = raw.trim_start();
        if trimmed.starts_with("//") || trimmed.is_empty() {
            continue;
        }
        let indent = raw.len() - raw.trim_start().len();
        if raw.chars().take(indent).any(|c| c != ' ') {
            return Err(format!("satır {no}: girintide sekme (tab) kullanılamaz — boşluk kullanın"));
        }
        let toks = tokenize(trimmed, no)?;
        if toks.is_empty() {
            continue;
        }
        out.push(Line { indent, tokens: toks, no });
    }
    Ok(out)
}

fn tokenize(s: &str, no: usize) -> Result<Vec<Tok>, String> {
    // satır içi yorumu at
    let s = match s.find("//") {
        Some(p) => &s[..p],
        None => s,
    };
    let chars: Vec<char> = s.chars().collect();
    let mut toks = Vec::new();
    let mut i = 0usize;
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        // sayı literal
        if c.is_ascii_digit() {
            let start = i;
            while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
                i += 1;
            }
            let lit: String = chars[start..i].iter().collect();
            if lit.matches('.').count() > 1 {
                return Err(format!("satır {no}: geçersiz sayı literal '{lit}'"));
            }
            toks.push(Tok::Num(lit));
            continue;
        }
        // string literal
        if c == '"' {
            let start = i + 1;
            let mut j = start;
            let mut closed = false;
            while j < chars.len() {
                if chars[j] == '"' {
                    closed = true;
                    break;
                }
                j += 1;
            }
            if !closed {
                return Err(format!("satır {no}: string tırnağı kapanmamış"));
            }
            toks.push(Tok::Str(chars[start..j].iter().collect()));
            i = j + 1;
            continue;
        }
        // tanımlayıcı (nokta'lı namespace dahil: ta.ema)
        if c.is_ascii_alphabetic() || c == '_' {
            let start = i;
            while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '_' || chars[i] == '.') {
                // ".." gibi çift nokta reddet
                if chars[i] == '.' && i + 1 < chars.len() && chars[i + 1] == '.' {
                    break;
                }
                i += 1;
            }
            let id: String = chars[start..i].iter().collect();
            if id.ends_with('.') {
                return Err(format!("satır {no}: geçersiz tanımlayıcı '{id}'"));
            }
            toks.push(Tok::Ident(id));
            continue;
        }
        // operatörler
        let two: String = chars[i..(i + 2).min(chars.len())].iter().collect();
        let op = match two.as_str() {
            ":=" | "<=" | ">=" | "==" | "!=" => {
                i += 2;
                two
            }
            _ => {
                let single = c.to_string();
                match "+-*/%<>?:,()[]=".contains(c) {
                    true => {
                        i += 1;
                        single
                    }
                    false => return Err(format!("satır {no}: beklenmeyen karakter '{c}'")),
                }
            }
        };
        toks.push(Tok::Op(op));
    }
    toks.push(Tok::Newline);
    Ok(toks)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lexes_namespaces_and_operators() {
        let lines = lex("x = ta.ema(close, 14) // trailing").unwrap();
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].indent, 0);
        assert!(matches!(&lines[0].tokens[0], Tok::Ident(id) if id == "x"));
        assert!(lines[0].tokens.iter().any(|t| matches!(t, Tok::Ident(i) if i == "ta.ema")));
    }

    #[test]
    fn strips_version_pragma_and_comments() {
        let lines = lex("//@version=5\nstrategy(\"t\")\n// full comment\n").unwrap();
        assert_eq!(lines.len(), 1);
    }

    #[test]
    fn rejects_tabs_in_indent() {
        assert!(lex("if a\n\tb = 1").is_err());
    }

    #[test]
    fn rejects_unknown_char() {
        assert!(lex("x = 1 & 2").is_err());
    }

    #[test]
    fn indents_tracked() {
        let lines = lex("if ta.crossover(close, open)\n    strategy.entry(\"L\", strategy.long)").unwrap();
        assert_eq!(lines[1].indent, 4);
    }
}
