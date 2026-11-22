//! P4-2 — YZ Yardımcı (v3 `/api/ai/diagnose` + `/api/market/news-ai` karşılığı).
//!
//! v3'teki `ai-manager.ts` + `news-ai.ts` hizmetlerinin Rust portu:
//!  · Salt-okunur sandbox: bu modül yürütüm bacağına (engine place/cancel)
//!    dokunmaz; yalnız bot olayları (şelale), sözlük/SSS (RAG) ve haber
//!    duyarlılığını okuyup bir LLM sağlayıcısına bağlam olarak verir.
//!  · API anahtarları YALNIZ istek gövdesinde yaşar: diske yazılmaz,
//!    loglanmaz, yanıta konmaz (redaksiyon testle kilitli).
//!  · `builtin` sağlayıcı v3'teki Z.ai konteyner SDK'sını gerektirir; bu
//!    kurulumda paketlenmediği için dürüst ABSTAIN döner (sahte analiz yok).
//!  · news-ai: öğe başına tek-satır Türkçe özet; varsayılan kapalı sayılır
//!    (UI açık kullanıcı eylemiyle çağırır), öğe başına önbellek, hata →
//!    ABSTAIN (liste asla bloklanmaz).

use crate::state::{now_ms, AppState};
use crate::{get_market_news, normalize_pair};
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::{Arc, LazyLock};

// ---------------- Sağlayıcı kataloğu (v3 AI_PROVIDERS ile aynı kimlikler) ---

pub struct AiProviderInfo {
    pub id: &'static str,
    pub label: &'static str,
    pub models: &'static [&'static str],
    pub needs_key: bool,
    pub default_base_url: Option<&'static str>,
    pub note: &'static str,
}

pub const AI_PROVIDERS: &[AiProviderInfo] = &[
    AiProviderInfo { id: "builtin", label: "Yerleşik (paketlenmedi)", models: &["yok"], needs_key: false, default_base_url: None, note: "v3'teki Z.ai konteyner SDK'sı bu kurulumda yok — başka sağlayıcı seçin." },
    AiProviderInfo { id: "openai", label: "OpenAI", models: &["gpt-4o", "gpt-4o-mini"], needs_key: true, default_base_url: Some("https://api.openai.com/v1"), note: "api.openai.com — Bearer anahtar." },
    AiProviderInfo { id: "anthropic", label: "Anthropic", models: &["claude-3-5-sonnet-latest"], needs_key: true, default_base_url: None, note: "api.anthropic.com — x-api-key + anthropic-version başlığı." },
    AiProviderInfo { id: "gemini", label: "Google Gemini", models: &["gemini-2.0-flash"], needs_key: true, default_base_url: None, note: "generativelanguage.googleapis.com — x-goog-api-key başlığı." },
    AiProviderInfo { id: "deepseek", label: "DeepSeek", models: &["deepseek-chat", "deepseek-reasoner"], needs_key: true, default_base_url: Some("https://api.deepseek.com"), note: "api.deepseek.com — OpenAI-uyumlu şema." },
    AiProviderInfo { id: "ollama", label: "Yerel Ollama", models: &["llama3.1", "qwen2.5"], needs_key: false, default_base_url: Some("http://localhost:11434"), note: "Yerel makine — /api/chat (stream kapalı)." },
];

fn provider_info(id: &str) -> &'static AiProviderInfo {
    AI_PROVIDERS.iter().find(|p| p.id == id).unwrap_or(&AI_PROVIDERS[0])
}

/// Bilinmeyen sağlayıcı kimliği → dürüst varsayılan (v3 ile aynı kural).
fn resolve_provider(id: Option<&str>) -> &'static AiProviderInfo {
    match id {
        Some(s) if AI_PROVIDERS.iter().any(|p| p.id == s) => provider_info(s),
        _ => provider_info("builtin"),
    }
}

// ---------------- İstek / yanıt tipleri -------------------------------------

#[derive(Deserialize, Default)]
pub struct AiCallConfig {
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub base_url: Option<String>,
}

impl AiCallConfig {
    /// Loglanabilir özet — anahtar ASLA yer almaz (test kilitli).
    fn redacted(&self, resolved_provider: &str, resolved_model: &str) -> String {
        let key_state = match &self.api_key {
            Some(k) if !k.trim().is_empty() => "gizli-uzunluk",
            _ => "yok",
        };
        format!("provider={resolved_provider} model={resolved_model} apiKey={key_state}")
    }
}

#[derive(Deserialize)]
pub struct AiMessage {
    pub role: String,
    pub content: String,
}

#[derive(Deserialize, Default)]
pub struct DiagnoseBody {
    #[serde(default)]
    pub mode: Option<String>,
    #[serde(default)]
    pub bot_id: Option<String>,
    #[serde(default)]
    pub waterfall: Option<Value>,
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub history: Option<Vec<AiMessage>>,
    #[serde(default)]
    pub symbol: Option<String>,
    #[serde(default)]
    pub config: Option<AiCallConfig>,
}

#[derive(Serialize)]
struct AiBullet {
    title: String,
    text: String,
}

// ---------------- Sözlük + SSS (RAG kaynağı) --------------------------------

#[allow(dead_code)]
#[derive(Deserialize, Clone)]
struct GlossaryEntry {
    #[serde(default)]
    code: String,
    #[serde(default)]
    exchange: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    severity: String,
    #[serde(default)]
    plain_cause: String,
    #[serde(default)]
    bot_action: String,
    #[serde(default)]
    operator_steps: Vec<String>,
}

#[allow(dead_code)]
#[derive(Deserialize, Clone)]
struct FaqEntry {
    #[serde(default)]
    id: String,
    #[serde(default)]
    category: String,
    #[serde(default)]
    question: String,
    #[serde(default)]
    answer: String,
    #[serde(default)]
    keywords: Vec<String>,
}

static GLOSSARY: LazyLock<Vec<GlossaryEntry>> = LazyLock::new(|| {
    match serde_json::from_str::<Vec<GlossaryEntry>>(include_str!("../data/glossary.json")) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("[ai] glossary.json okunamadı: {e} — RAG sözlüksüz çalışır");
            Vec::new()
        }
    }
});

static FAQ: LazyLock<Vec<FaqEntry>> = LazyLock::new(|| {
    match serde_json::from_str::<Vec<FaqEntry>>(include_str!("../data/faq.json")) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("[ai] faq.json okunamadı: {e} — RAG SSS'siz çalışır");
            Vec::new()
        }
    }
});

fn retrieve_faq(query: &str, limit: usize) -> Vec<FaqEntry> {
    let q = query.to_lowercase();
    let mut scored: Vec<(f64, FaqEntry)> = Vec::new();
    for f in FAQ.iter() {
        let mut s = 0.0;
        for kw in &f.keywords {
            if q.contains(&kw.to_lowercase()) {
                s += 2.0;
            }
        }
        for tok in q.split_whitespace() {
            if tok.len() < 3 {
                continue;
            }
            if f.question.to_lowercase().contains(tok) {
                s += 1.0;
            }
            if f.answer.to_lowercase().contains(tok) {
                s += 0.5;
            }
        }
        if s > 0.0 {
            scored.push((s, f.clone()));
        }
    }
    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    scored.into_iter().take(limit).map(|(_, f)| f).collect()
}

fn retrieve_glossary(query: &str, limit: usize) -> Vec<GlossaryEntry> {
    let q = query.to_lowercase();
    let mut scored: Vec<(f64, GlossaryEntry)> = Vec::new();
    for g in GLOSSARY.iter() {
        let mut s = 0.0;
        if !g.code.is_empty() && q.contains(&g.code.to_lowercase()) {
            s += 5.0;
        }
        if !g.exchange.is_empty() && q.contains(&g.exchange.to_lowercase()) {
            s += 2.0;
        }
        for tok in q.split_whitespace() {
            if tok.len() < 3 {
                continue;
            }
            if g.title.to_lowercase().contains(tok) {
                s += 1.0;
            }
            if g.plain_cause.to_lowercase().contains(tok) {
                s += 0.5;
            }
        }
        if s > 0.0 {
            scored.push((s, g.clone()));
        }
    }
    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    scored.into_iter().take(limit).map(|(_, g)| g).collect()
}

/// Şelale metninden sözlük kodlarını çıkar (v3 extractCodes ile aynı kurallar).
fn extract_codes(waterfall_json: &str) -> Vec<String> {
    let mut found: Vec<String> = Vec::new();
    for g in GLOSSARY.iter() {
        if g.code.len() >= 4 && waterfall_json.contains(&g.code) && !found.contains(&g.code) {
            found.push(g.code.clone());
        }
    }
    if (waterfall_json.contains("IN_FLIGHT") || waterfall_json.contains("IN-FLIGHT"))
        && !found.iter().any(|c| c == "IN-FLIGHT TIMEOUT")
    {
        found.push("IN-FLIGHT TIMEOUT".into());
    }
    if waterfall_json.contains("SAFETY_THROTTLED_VWAP_Z") && !found.iter().any(|c| c == "VWAP-Z") {
        found.push("VWAP-Z".into());
    }
    found.truncate(6);
    found
}

/// Model yanıtını "1. Ne Oldu / 2. Neden / 3. Tavsiye" maddelerine böl (v3 aynı).
fn parse_bullets(raw: &str) -> Vec<AiBullet> {
    let mut bullets: Vec<AiBullet> = Vec::new();
    let mut cur: Option<AiBullet> = None;
    for line in raw.lines().map(str::trim).filter(|l| !l.is_empty()) {
        let m = parse_bullet_head(line);
        if let Some((title, rest)) = m {
            if let Some(c) = cur.take() {
                bullets.push(c);
            }
            cur = Some(AiBullet { title, text: rest });
        } else if let Some(c) = cur.as_mut() {
            let t = line.strip_prefix('-').or_else(|| line.strip_prefix('*')).unwrap_or(line).trim();
            if !t.is_empty() {
                c.text.push(' ');
                c.text.push_str(t);
            }
        }
    }
    if let Some(c) = cur.take() {
        bullets.push(c);
    }
    bullets.truncate(3);
    bullets
}

fn parse_bullet_head(line: &str) -> Option<(String, String)> {
    let mut it = line.splitn(2, '.');
    let num = it.next()?.trim();
    if num.parse::<u32>().is_err() {
        return None;
    }
    let rest = it.next()?.trim();
    for key in ["Ne Oldu", "Neden", "Tavsiye"] {
        if let Some(after) = rest.strip_prefix(key) {
            let text = after.trim_start_matches([':', '-', ' ']).trim().to_string();
            return Some((key.to_string(), text));
        }
    }
    None
}

// ---------------- Sistem istemi (salt-okunur sandbox) ------------------------

const SYSTEM_PROMPT: &str = "Sen DCABOT adlı nicel işlem terminalinin SALT-OKUNUR teşhis kopyilotusun.\n\nKESİN KURALLAR:\n1. Hiçbir koşulda emir açmayı, iptal etmeyi veya yürütüm parametrelerini değiştirmeyi öneremezsin; placeOrder/cancel emrine erişimin YOKTUR.\n2. Yalnızca verilen bağlama dayan: iz şelalesi (bot olayları), hata sözlüğü, SSS, haber duyarlılığı. Bağlamda olmayan şeyi UYDURMA — \"bağlamda yok\" de.\n3. Dil: Türkçe, kısa, kurumsal, jargonsuz. Sayıları kaynak satırdan al.\n4. Yanıt biçimi: istenirse tam 3 madde (Ne Oldu / Neden / Tavsiye); sohbette en fazla 1 kısa paragraf + gerekirse maddeler.\n5. \"Tavsiye\" her zaman operasyonel gözlem niteliğindedir (ör. \"müdahale gerekmez\", \"MUTABAKAT raporunu çalıştırın\") — ASLA işlem kararları değildir.";

const NEWS_TR_SYSTEM: &str = "Sen finansal haber başlıklarını Türkçeleştiren sade bir özet asistanısın. Tek cümle yaz: en fazla 140 karakter, jargonsuz, herkese açık dil. ASLA alım/satım tavsiyesi verme, ASLA bilgi uydurma — verilen başlıkla sınırlı kal.";

// ---------------- Sağlayıcı çağrı zarfı -------------------------------------

pub(crate) struct ResolvedCall {
    pub(crate) provider: &'static AiProviderInfo,
    pub(crate) model: String,
    pub(crate) api_key: String,
    pub(crate) base_url: String,
}

/// SEC-FIND-04: SSRF koruması — harici ve yerel AI URL doğrulaması.
pub fn validate_ai_base_url(url: &str, provider_id: &str) -> Result<String, String> {
    let parsed = reqwest::Url::parse(url).map_err(|e| format!("geçersiz base_url: {e}"))?;
    let scheme = parsed.scheme();
    let host = parsed.host_str().ok_or_else(|| "URL ana makine içermiyor".to_string())?;
    if !parsed.username().is_empty() || parsed.password().is_some() || parsed.query().is_some() || parsed.fragment().is_some() {
        return Err("güvenlik kuralı: URL kimlik bilgisi, sorgu veya fragment içeremez".into());
    }

    if provider_id == "ollama" {
        if scheme != "http" {
            return Err("Ollama yalnızca yerel HTTP bağlantısını destekler".into());
        }
        if parsed.port_or_known_default() != Some(11434) {
            return Err("Ollama güvenliği: yalnızca 11434 portuna izin verilir".into());
        }
        let ip_host = parse_ip_host(host);
        let is_loopback = host.eq_ignore_ascii_case("localhost") || ip_host.is_some_and(|ip| ip.is_loopback());
        if !is_loopback {
            return Err("Ollama güvenliği: yalnızca yerel makine (localhost / 127.0.0.1) adreslerine izin verilir".into());
        }
        return Ok(parsed.as_str().trim_end_matches('/').to_string());
    }

    if scheme != "https" {
        return Err("güvenlik kuralı: harici AI sağlayıcıları yalnızca HTTPS protokolü kullanmalıdır".into());
    }
    if parsed.port_or_known_default() != Some(443) {
        return Err("güvenlik kuralı: harici AI sağlayıcıları yalnızca 443 portunu kullanabilir".into());
    }

    if let Some(ip) = parse_ip_host(host) {
        if !is_public_ip(ip) {
            return Err("güvenlik kuralı: dahili veya metadata IP adreslerine istek yapılamaz (SSRF koruması)".into());
        }
    }

    if host.eq_ignore_ascii_case("localhost")
        || host.to_ascii_lowercase().ends_with(".localhost")
        || host.ends_with(".internal")
        || host.ends_with(".local")
        || host.ends_with(".lan")
        || host.ends_with(".home")
        || host == "metadata.google.internal"
        || host == "instance-data"
    {
        return Err("güvenlik kuralı: dahili ana makine adlarına istek yapılamaz (SSRF koruması)".into());
    }

    Ok(parsed.as_str().trim_end_matches('/').to_string())
}

fn parse_ip_host(host: &str) -> Option<IpAddr> {
    host.strip_prefix('[')
        .and_then(|value| value.strip_suffix(']'))
        .unwrap_or(host)
        .parse()
        .ok()
}

fn is_public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_public_ipv4(v4),
        IpAddr::V6(v6) => {
            let segments = v6.segments();
            // IPv4-compatible and IPv4-mapped IPv6 can route to private IPv4.
            if segments[..6].iter().all(|part| *part == 0) {
                let octets = v6.octets();
                let embedded = Ipv4Addr::new(octets[12], octets[13], octets[14], octets[15]);
                return is_public_ipv4(embedded);
            }
            // Only globally allocated 2000::/3 addresses are accepted. The
            // documentation and 6to4 ranges are denied even within that block.
            (segments[0] & 0xe000) == 0x2000
                && segments[0] != 0x2002
                && segments[0] != 0x3fff
                && !(segments[0] == 0x2001 && segments[1] < 0x0200)
                && !v6.is_loopback()
                && !v6.is_unspecified()
                && !v6.is_multicast()
        }
    }
}

fn is_public_ipv4(ip: Ipv4Addr) -> bool {
    let octets = ip.octets();
    let in_cgnat = octets[0] == 100 && (octets[1] & 0xc0) == 64;
    let in_benchmark = octets[0] == 198 && (octets[1] & 0xfe) == 18;
    let in_special_192 = octets[0] == 192 && octets[1] == 0 && octets[2] == 0;
    let in_reserved = octets[0] >= 240;
    !(ip.is_loopback()
        || ip.is_private()
        || ip.is_link_local()
        || ip.is_broadcast()
        || ip.is_documentation()
        || ip.is_unspecified()
        || ip.is_multicast()
        || octets[0] == 0
        || in_cgnat
        || in_benchmark
        || in_special_192
        || in_reserved)
}

async fn checked_ai_client(base_url: &str, provider_id: &str) -> Result<reqwest::Client, String> {
    let normalized = validate_ai_base_url(base_url, provider_id)?;
    let parsed = reqwest::Url::parse(&normalized).map_err(|_| "geçersiz AI sağlayıcı URL'si".to_string())?;
    let host = parsed.host_str().ok_or_else(|| "AI sağlayıcı ana makinesi eksik".to_string())?;
    let port = parsed.port_or_known_default().ok_or_else(|| "AI sağlayıcı portu eksik".to_string())?;
    let literal_ip = parse_ip_host(host);
    let addresses: Vec<SocketAddr> = if let Some(ip) = literal_ip {
        vec![SocketAddr::new(ip, port)]
    } else {
        tokio::net::lookup_host((host, port))
            .await
            .map_err(|_| "AI sağlayıcı ana makinesi çözümlenemedi".to_string())?
            .collect()
    };
    if addresses.is_empty() {
        return Err("AI sağlayıcı ana makinesi adres döndürmedi".into());
    }
    for address in &addresses {
        let ip = address.ip();
        let allowed = if provider_id == "ollama" { ip.is_loopback() } else { is_public_ip(ip) };
        if !allowed {
            return Err("AI sağlayıcı ana makinesi dahili veya özel IP'ye çözümlendi (SSRF koruması)".into());
        }
    }

    let mut builder = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(35))
        .user_agent("dcabot-ai/1.0 (+internal)")
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy();
    if literal_ip.is_none() {
        builder = builder.resolve_to_addrs(host, &addresses);
    }
    builder.build().map_err(|_| "güvenli AI HTTP istemcisi oluşturulamadı".into())
}

pub(crate) fn resolve_call(cfg: Option<&AiCallConfig>) -> Result<ResolvedCall, String> {
    let provider = resolve_provider(cfg.and_then(|c| c.provider.as_deref()));
    let model = cfg
        .and_then(|c| c.model.clone())
        .map(|m| m.trim().to_string())
        .filter(|m| !m.is_empty())
        .unwrap_or_else(|| provider.models.first().unwrap_or(&"").to_string());
    let api_key = cfg.and_then(|c| c.api_key.clone()).unwrap_or_default();
    let base_url = if let Some(raw_u) = cfg.and_then(|c| c.base_url.as_deref()).map(str::trim).filter(|u| !u.is_empty()) {
        validate_ai_base_url(raw_u, provider.id)?
    } else {
        provider.default_base_url.unwrap_or("").to_string()
    };
    Ok(ResolvedCall { provider, model, api_key, base_url })
}

pub(crate) async fn call_provider(_app: &AppState, rc: &ResolvedCall, system: &str, user: &str, history: &[(String, String)]) -> Result<String, String> {
    match rc.provider.id {
        "builtin" => Err("yerleşik sağlayıcı bu kurulumda paketlenmedi (v3 Z.ai konteyner SDK'sı yok) — OpenAI / DeepSeek / Anthropic / Gemini / Yerel Ollama seçin".into()),
        "openai" | "deepseek" => {
            if rc.api_key.trim().is_empty() {
                return Err("API anahtarı girilmedi (yalnız bu istekte kullanılır, saklanmaz)".into());
            }
            let client = checked_ai_client(&rc.base_url, rc.provider.id).await?;
            let mut msgs = vec![json!({"role": "system", "content": system})];
            for (r, c) in history {
                msgs.push(json!({"role": r, "content": c}));
            }
            msgs.push(json!({"role": "user", "content": user}));
            let resp = client
                .post(format!("{}/chat/completions", rc.base_url))
                .header("Content-Type", "application/json")
                .header("Authorization", format!("Bearer {}", rc.api_key.trim()))
                .json(&json!({"model": rc.model, "messages": msgs, "temperature": 0.2}))
                .send()
                .await
                .map_err(|e| format!("sağlayıcı erişilemedi: {}", e.without_url()))?;
            if !resp.status().is_success() {
                return Err(format!("sağlayıcı HTTP {}", resp.status()));
            }
            let v: Value = resp.json().await.map_err(|e| format!("sağlayıcı yanıtı okunamadı: {e}"))?;
            Ok(v.pointer("/choices/0/message/content").and_then(|c| c.as_str()).unwrap_or("").trim().to_string())
        }
        "anthropic" => {
            if rc.api_key.trim().is_empty() {
                return Err("API anahtarı girilmedi (yalnız bu istekte kullanılır, saklanmaz)".into());
            }
            let client = checked_ai_client("https://api.anthropic.com", "anthropic").await?;
            let mut msgs = Vec::new();
            for (r, c) in history {
                msgs.push(json!({"role": r, "content": c}));
            }
            msgs.push(json!({"role": "user", "content": user}));
            let resp = client
                .post("https://api.anthropic.com/v1/messages")
                .header("Content-Type", "application/json")
                .header("x-api-key", rc.api_key.trim())
                .header("anthropic-version", "2023-06-01")
                .json(&json!({"model": rc.model, "max_tokens": 1024, "system": system, "messages": msgs}))
                .send()
                .await
                .map_err(|e| format!("sağlayıcı erişilemedi: {}", e.without_url()))?;
            if !resp.status().is_success() {
                return Err(format!("sağlayıcı HTTP {}", resp.status()));
            }
            let v: Value = resp.json().await.map_err(|e| format!("sağlayıcı yanıtı okunamadı: {e}"))?;
            let text = v
                .get("content")
                .and_then(|c| c.as_array())
                .map(|a| a.iter().filter_map(|p| p.get("text").and_then(|t| t.as_str())).collect::<Vec<_>>().join(""))
                .unwrap_or_default();
            Ok(text.trim().to_string())
        }
        "gemini" => {
            if rc.api_key.trim().is_empty() {
                return Err("API anahtarı girilmedi (yalnız bu istekte kullanılır, saklanmaz)".into());
            }
            let client = checked_ai_client("https://generativelanguage.googleapis.com", "gemini").await?;
            let mut contents = Vec::new();
            for (r, c) in history {
                let role = if r == "assistant" { "model" } else { "user" };
                contents.push(json!({"role": role, "parts": [{"text": c}]}));
            }
            contents.push(json!({"role": "user", "parts": [{"text": user}]}));
            let url = format!(
                "https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent",
                urlenc(&rc.model)
            );
            let resp = client
                .post(url)
                .header("Content-Type", "application/json")
                .header("x-goog-api-key", rc.api_key.trim())
                .json(&json!({"systemInstruction": {"parts": [{"text": system}]}, "contents": contents, "generationConfig": {"temperature": 0.2}}))
                .send()
                .await
                .map_err(|e| format!("sağlayıcı erişilemedi: {}", e.without_url()))?;
            if !resp.status().is_success() {
                return Err(format!("sağlayıcı HTTP {}", resp.status()));
            }
            let v: Value = resp.json().await.map_err(|e| format!("sağlayıcı yanıtı okunamadı: {e}"))?;
            let text = v
                .pointer("/candidates/0/content/parts")
                .and_then(|p| p.as_array())
                .map(|a| a.iter().filter_map(|p| p.get("text").and_then(|t| t.as_str())).collect::<Vec<_>>().join(""))
                .unwrap_or_default();
            Ok(text.trim().to_string())
        }
        "ollama" => {
            let base = if rc.base_url.is_empty() { "http://localhost:11434".to_string() } else { rc.base_url.clone() };
            let client = checked_ai_client(&base, "ollama").await?;
            let mut msgs = vec![json!({"role": "system", "content": system})];
            for (r, c) in history {
                msgs.push(json!({"role": r, "content": c}));
            }
            msgs.push(json!({"role": "user", "content": user}));
            let resp = client
                .post(format!("{base}/api/chat"))
                .header("Content-Type", "application/json")
                .json(&json!({"model": rc.model, "stream": false, "messages": msgs}))
                .send()
                .await
                .map_err(|e| format!("Ollama erişilemedi: {}", e.without_url()))?;
            if !resp.status().is_success() {
                return Err(format!("Ollama HTTP {}", resp.status()));
            }
            let v: Value = resp.json().await.map_err(|e| format!("Ollama yanıtı okunamadı: {e}"))?;
            Ok(v.get("message").and_then(|m| m.get("content")).and_then(|c| c.as_str()).unwrap_or("").trim().to_string())
        }
        _ => Err("bilinmeyen sağlayıcı".into()),
    }
}

fn urlenc(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
            o.push(b as char);
        } else {
            o.push_str(&format!("%{b:02X}"));
        }
    }
    o
}

// ---------------- Haber duyarlılığı (bulkhead) --------------------------------

/// v3 getMarketNewsSentiment karşılığı: haber servisi düşerse yürütüm
/// etkilenmez — yalnız "okunamadı" notu döner.
async fn news_sentiment_summary(app: &Arc<AppState>, symbol: Option<&str>) -> (bool, String) {
    let Some(sym) = symbol.map(|s| normalize_pair(s)).filter(|s| s.len() >= 4) else {
        return (false, "sembol verilmedi".into());
    };
    let v = get_market_news(app, &sym).await;
    let items = v.get("items").and_then(|i| i.as_array()).cloned().unwrap_or_default();
    if items.is_empty() {
        return (false, "haber duyarlılığı şu an okunamadı (bulkhead — yürütüm etkilenmez)".into());
    }
    let heads: Vec<String> = items
        .iter()
        .take(6)
        .map(|n| {
            let t = n.get("title").and_then(|x| x.as_str()).unwrap_or("");
            let s = n.get("sentiment").and_then(|x| x.as_f64()).unwrap_or(0.0);
            format!("{}{s} {}", if s >= 0.0 { "+" } else { "" }, t.chars().take(60).collect::<String>())
        })
        .collect();
    let sum: f64 = items.iter().take(6).filter_map(|n| n.get("sentiment").and_then(|x| x.as_f64())).sum();
    (true, format!("son 6 başlık toplam duyarlılık {sum:.2}: {}", heads.join(" | ")))
}

// ---------------- Şelale (bot olayları) ---------------------------------------

async fn waterfall_for_bot(app: &Arc<AppState>, bot_id: &str) -> Vec<Value> {
    app.db.conn.lock().map(|c| {
        let mut stmt = match c.prepare("SELECT level, gate, message, ts FROM events WHERE bot_id=?1 ORDER BY rowid ASC LIMIT 120") {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };
        stmt.query_map([bot_id], |r| {
            Ok(json!({
                "ts": r.get::<_, i64>(3)?,
                "level": r.get::<_, String>(0)?,
                "gate": r.get::<_, String>(1)?,
                "message": r.get::<_, String>(2)?,
            }))
        }).map(|it| it.filter_map(|r| r.ok()).collect()).unwrap_or_default()
    }).unwrap_or_default()
}

fn bot_symbol(app: &Arc<AppState>, bot_id: &str) -> Option<String> {
    app.db.conn.lock().ok().and_then(|c| {
        c.query_row("SELECT symbol FROM bots WHERE id=?1", [bot_id], |r| r.get::<_, String>(0)).ok()
    })
}

// ---------------- POST /api/ai/diagnose --------------------------------------

/// body: { mode: 'diagnose'|'chat', bot_id?, waterfall?, message?, history?,
/// symbol?, config? } — config yalnız bu istekte yaşar.
pub async fn ai_diagnose(State(app): State<Arc<AppState>>, Json(body): Json<DiagnoseBody>) -> axum::response::Response {
    let t0 = now_ms();
    let rc = match resolve_call(body.config.as_ref()) {
        Ok(r) => r,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({
                    "ok": false,
                    "verdict": "ERROR",
                    "note": format!("Geçersiz AI yapılandırması: {e}"),
                    "elapsedMs": now_ms() - t0,
                })),
            )
                .into_response();
        }
    };
    let cfg_desc = body.config.as_ref().map(|c| c.redacted(rc.provider.id, &rc.model)).unwrap_or_else(|| format!("provider={} model={} apiKey=yok", rc.provider.id, rc.model));
    tracing::info!("[ai] diagnose isteği {}", cfg_desc);
    let mode = body.mode.as_deref().unwrap_or("diagnose");
    if mode == "chat" {
        return ai_chat_inner(&app, &rc, body, t0).await.into_response();
    }
    // ── diagnose ──
    let waterfall: Vec<Value> = match body.waterfall {
        Some(Value::Array(a)) if !a.is_empty() => a,
        _ => match body.bot_id.as_deref() {
            Some(id) if !id.trim().is_empty() => waterfall_for_bot(&app, id.trim()).await,
            _ => Vec::new(),
        },
    };
    if waterfall.is_empty() {
        return Json(json!({
            "ok": false, "verdict": "ABSTAIN", "provider": rc.provider.id, "model": rc.model,
            "note": "Analiz edilecek iz şelalesi boş — bot_id yanlış olabilir veya olaylar henüz yazılmadı.",
            "elapsedMs": now_ms() - t0,
        }))
        .into_response();
    }
    let wtext = serde_json::to_string(&waterfall).unwrap_or_default();
    let codes = extract_codes(&wtext);
    let mut gloss: Vec<Value> = Vec::new();
    for c in &codes {
        if gloss.len() >= 4 {
            break;
        }
        if let Some(g) = GLOSSARY.iter().find(|g| &g.code == c) {
            gloss.push(json!({"code": g.code, "title": g.title, "plainCause": g.plain_cause, "botAction": g.bot_action}));
        }
    }
    let symbol = body.symbol.clone().or_else(|| body.bot_id.as_deref().and_then(|id| bot_symbol(&app, id)));
    let (news_ok, news_summary) = news_sentiment_summary(&app, symbol.as_deref()).await;
    let ctx = json!({"traceWaterfall": waterfall, "errorGlossary": gloss, "newsSentiment": news_summary});
    let mut ctx_s = ctx.to_string();
    if ctx_s.len() > 9000 {
        ctx_s.truncate(9000);
    }
    let user = format!("Aşağıdaki iz şelalesini analiz et ve TAM 3 madde üret. Biçim:\n1. Ne Oldu: <zaman ve sembol + ölçülen değer>\n2. Neden: <kapı/kural + hangi eşikle kısıldı veya geçti>\n3. Tavsiye: <operasyonel gözlem — müdahale gerekip gerekmediği>\n\nBAĞLAM:\n{ctx_s}");
    match call_provider(&app, &rc, SYSTEM_PROMPT, &user, &[]).await {
        Ok(raw) if raw.len() >= 40 => Json(json!({
            "ok": true, "verdict": "OK", "provider": rc.provider.id,
            "model": format!("{} · {}", rc.provider.label, rc.model),
            "bullets": parse_bullets(&raw), "elapsedMs": now_ms() - t0,
            "sources": {"glossaryCodes": codes, "newsOk": news_ok},
        }))
        .into_response(),
        Ok(raw) => Json(json!({
            "ok": false, "verdict": "ABSTAIN", "provider": rc.provider.id, "model": rc.model,
            "note": format!("model yanıtı çok kısa ({} karakter) — sahte analiz üretilmedi", raw.len()),
            "elapsedMs": now_ms() - t0,
        }))
        .into_response(),
        Err(e) => Json(json!({
            "ok": false, "verdict": "ERROR", "provider": rc.provider.id, "model": rc.model,
            "note": format!("model çağrısı başarısız: {}", e.chars().take(160).collect::<String>()),
            "elapsedMs": now_ms() - t0,
        }))
        .into_response(),
    }
}

async fn ai_chat_inner(app: &Arc<AppState>, rc: &ResolvedCall, body: DiagnoseBody, t0: i64) -> Json<Value> {
    let message = body.message.unwrap_or_default().trim().chars().take(2000).collect::<String>();
    if message.is_empty() {
        return Json(json!({"ok": false, "verdict": "ABSTAIN", "note": "soru boş"}));
    }
    let history: Vec<(String, String)> = body
        .history
        .unwrap_or_default()
        .into_iter()
        .filter(|m| (m.role == "user" || m.role == "assistant") && !m.content.is_empty())
        .take(8)
        .map(|m| (m.role, m.content.chars().take(2000).collect()))
        .collect();
    let faq_hits = retrieve_faq(&message, 3);
    let gloss_hits = retrieve_glossary(&message, 3);
    let symbol = body.symbol.clone().or_else(|| body.bot_id.as_deref().and_then(|id| bot_symbol(app, id)));
    let (news_ok, news_summary) = news_sentiment_summary(app, symbol.as_deref()).await;
    // Güvenlik refleksi: emir verme talepleri açıkça reddedilir (yetki yok).
    if faq_hits.is_empty() && gloss_hits.is_empty() && message.to_lowercase().contains("emir") {
        return Json(json!({
            "ok": true, "verdict": "OK", "provider": rc.provider.id,
            "model": format!("{} · {}", rc.provider.label, rc.model),
            "text": "Emir açma/iptal yetkim yok — bu tasarım gereği (salt-okunur kum havuzu kuralı). Teşhis, sözlük ve SSS konularında yardımcı olabilirim.",
            "elapsedMs": now_ms() - t0,
        }));
    }
    let mut parts: Vec<String> = Vec::new();
    if !faq_hits.is_empty() {
        parts.push(format!("SSS:\n{}", faq_hits.iter().map(|f| format!("— {}\n{}", f.question, f.answer)).collect::<Vec<_>>().join("\n")));
    }
    if !gloss_hits.is_empty() {
        parts.push(format!("Hata Sözlüğü:\n{}", gloss_hits.iter().map(|g| format!("— {} ({}) {}: {} | Bot eylemi: {}", g.code, g.exchange, g.title, g.plain_cause, g.bot_action)).collect::<Vec<_>>().join("\n")));
    }
    parts.push(format!("Haber: {news_summary}"));
    let user = format!("Kullanıcı sorusu: {message}\n\nBAĞLAM (yalnız bundan yanıtla; bağlamda yoksa \"bağlamda yok\" de):\n{}", parts.join("\n\n"));
    let hist6: Vec<(String, String)> = history.into_iter().take(6).collect();
    match call_provider(app, rc, SYSTEM_PROMPT, &user, &hist6).await {
        Ok(raw) if raw.len() >= 5 => Json(json!({
            "ok": true, "verdict": "OK", "provider": rc.provider.id,
            "model": format!("{} · {}", rc.provider.label, rc.model),
            "text": raw, "elapsedMs": now_ms() - t0,
            "sources": {"faqIds": faq_hits.iter().map(|f| &f.id).collect::<Vec<_>>(), "glossaryCodes": gloss_hits.iter().map(|g| &g.code).collect::<Vec<_>>(), "newsOk": news_ok},
        })),
        Ok(_) => Json(json!({
            "ok": false, "verdict": "ABSTAIN", "provider": rc.provider.id, "model": rc.model,
            "note": "model yanıtı boş — sahte yanıt üretilmedi", "elapsedMs": now_ms() - t0,
        })),
        Err(e) => Json(json!({
            "ok": false, "verdict": "ERROR", "provider": rc.provider.id, "model": rc.model,
            "note": format!("model çağrısı başarısız: {}", e.chars().take(160).collect::<String>()),
            "elapsedMs": now_ms() - t0,
        })),
    }
}

// ---------------- POST /api/market/news-ai -----------------------------------
/// body: { id, config? } — öğe başına tek-satır Türkçe özet. Önbellekli.
/// Hata → ABSTAIN (liste asla bloklanmaz).
#[derive(Deserialize)]
pub struct NewsAiBody {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub config: Option<AiCallConfig>,
}

pub async fn news_ai(State(app): State<Arc<AppState>>, Json(body): Json<NewsAiBody>) -> impl IntoResponse {
    if body.id.trim().is_empty() {
        return (StatusCode::BAD_REQUEST, Json(json!({"ok": false, "reason": "id zorunlu"})));
    }
    // Önbellek: bir kez üretilen özet saklanır.
    if let Some(s) = app.news_ai_cache.lock().await.get(body.id.trim()).cloned() {
        return (StatusCode::OK, Json(json!({"ok": true, "summaryTr": s, "reason": Value::Null, "cached": true})));
    }
    // Öğeyi haber önbelleğinde bul.
    let found: Option<(String, String)> = {
        let cache = app.news_cache.lock().await;
        let mut out = None;
        for (_, (_, v)) in cache.iter() {
            if let Some(items) = v.get("items").and_then(|i| i.as_array()) {
                for n in items {
                    if n.get("id").and_then(|x| x.as_str()) == Some(body.id.trim()) {
                        if let Some(prev) = n.get("aiSummary").and_then(|x| x.as_str()) {
                            out = Some((n.get("title").and_then(|x| x.as_str()).unwrap_or("").to_string(), format!("CACHED:{prev}")));
                        } else {
                            out = Some((n.get("title").and_then(|x| x.as_str()).unwrap_or("").to_string(), String::new()));
                        }
                        break;
                    }
                }
            }
            if out.is_some() {
                break;
            }
        }
        out
    };
    let (title, cached_mark) = match found {
        Some(t) => t,
        None => return (StatusCode::OK, Json(json!({"ok": false, "summaryTr": Value::Null, "reason": "Haber bulunamadı (önce /api/market/news çağırın)", "cached": false}))),
    };
    if let Some(prev) = cached_mark.strip_prefix("CACHED:") {
        app.news_ai_cache.lock().await.insert(body.id.trim().to_string(), prev.to_string());
        return (StatusCode::OK, Json(json!({"ok": true, "summaryTr": prev, "reason": Value::Null, "cached": true})));
    }
    if title.trim().is_empty() {
        return (StatusCode::OK, Json(json!({"ok": false, "summaryTr": Value::Null, "reason": "Başlık boş", "cached": false})));
    }
    let rc = match resolve_call(body.config.as_ref()) {
        Ok(r) => r,
        Err(e) => return (StatusCode::BAD_REQUEST, Json(json!({"ok": false, "summaryTr": Value::Null, "reason": format!("Geçersiz AI yapılandırması: {e}"), "cached": false}))),
    };
    let user = format!("Başlık: {title}\nBunu tek cümlelik Türkçe sade özete dönüştür (en fazla 140 karakter).");
    match call_provider(&app, &rc, NEWS_TR_SYSTEM, &user, &[]).await {
        Ok(raw) => {
            let line = raw.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("").chars().take(200).collect::<String>();
            if line.is_empty() {
                return (StatusCode::OK, Json(json!({"ok": false, "summaryTr": Value::Null, "reason": "Model yanıtı boş — sahte özet üretilmedi", "cached": false})));
            }
            app.news_ai_cache.lock().await.insert(body.id.trim().to_string(), line.clone());
            // Haber önbelleğindeki öğeye de işle (sonraki /news yanıtları taşır).
            {
                let mut cache = app.news_cache.lock().await;
                for (_, (_, v)) in cache.iter_mut() {
                    if let Some(items) = v.get_mut("items").and_then(|i| i.as_array_mut()) {
                        for n in items.iter_mut() {
                            if n.get("id").and_then(|x| x.as_str()) == Some(body.id.trim()) {
                                n["aiSummary"] = json!(line);
                            }
                        }
                    }
                }
            }
            (StatusCode::OK, Json(json!({"ok": true, "summaryTr": line, "reason": Value::Null, "cached": false})))
        }
        Err(e) => (StatusCode::OK, Json(json!({"ok": false, "summaryTr": Value::Null, "reason": format!("Özet üretilemedi: {}", e.chars().take(120).collect::<String>()), "cached": false}))),
    }
}

// ---------------- GET /api/ai/providers (katalog, anahtarsız) ----------------

pub async fn ai_providers() -> Json<Value> {
    Json(json!({
        "providers": AI_PROVIDERS.iter().map(|p| json!({
            "id": p.id, "label": p.label, "models": p.models,
            "needsKey": p.needs_key, "defaultBaseUrl": p.default_base_url, "note": p.note,
        })).collect::<Vec<_>>(),
    }))
}

// ---------------- Birim testleri (ağ YOK) ------------------------------------

#[cfg(test)]
mod ai_tests {
    use super::*;

    #[test]
    fn faq_retrieval_finds_dca_entry() {
        let hits = retrieve_faq("DCA botu nasıl çalışır", 3);
        assert!(!hits.is_empty(), "DCA sorusu SSS'de bulunmalı");
        assert!(hits.iter().any(|f| f.id == "dca-nedir"), "dca-nedir girdisi dönmeli");
    }

    #[test]
    fn glossary_retrieval_finds_code() {
        let hits = retrieve_glossary("emir reddedildi -2010", 3);
        assert!(!hits.is_empty(), "kod sorgusu sözlükte bulunmalı");
        assert!(hits.iter().any(|g| g.code == "-2010"));
    }

    #[test]
    fn bullets_parse_three_items() {
        let raw = "1. Ne Oldu: fiyat 82000'e düştü\nara satır\n2. Neden: Gate 4 kısıldı\n3. Tavsiye: müdahale gerekmez";
        let b = parse_bullets(raw);
        assert_eq!(b.len(), 3);
        assert_eq!(b[0].title, "Ne Oldu");
        assert!(b[0].text.contains("ara satır"), "devam satırı ilk maddeye eklenmeli");
        assert_eq!(b[2].title, "Tavsiye");
    }

    #[test]
    fn codes_extracted_from_waterfall() {
        let w = r#"[{"message":"reject -2010 clientOrderId"},{"message":"IN_FLIGHT bekleme"},{"message":"SAFETY_THROTTLED_VWAP_Z"}]"#;
        let c = extract_codes(w);
        assert!(c.contains(&"-2010".to_string()));
        assert!(c.contains(&"IN-FLIGHT TIMEOUT".to_string()));
        assert!(c.contains(&"VWAP-Z".to_string()));
    }

    #[test]
    fn unknown_provider_defaults_builtin() {
        assert_eq!(resolve_provider(Some("yok-boyle")).id, "builtin");
        assert_eq!(resolve_provider(None).id, "builtin");
        assert_eq!(resolve_provider(Some("ollama")).id, "ollama");
    }

    #[test]
    fn config_redaction_never_leaks_key() {
        let c = AiCallConfig {
            provider: Some("openai".into()),
            model: Some("gpt-4o".into()),
            api_key: Some("sk-gizli-anahtar-123".into()),
            base_url: None,
        };
        let s = c.redacted("openai", "gpt-4o");
        assert!(!s.contains("sk-gizli-anahtar-123"), "redakte özet anahtarı taşımamalı");
        assert!(s.contains("openai"));
    }

    #[test]
    fn validate_ai_base_url_blocks_ssrf_and_permits_safe() {
        // Block cloud metadata
        assert!(validate_ai_base_url("http://169.254.169.254/latest/meta-data", "openai").is_err());
        assert!(validate_ai_base_url("https://169.254.169.254/latest/meta-data", "openai").is_err());

        // Block internal / loopback for cloud providers
        assert!(validate_ai_base_url("http://127.0.0.1:8080", "openai").is_err());
        assert!(validate_ai_base_url("https://127.0.0.1:8080", "openai").is_err());
        assert!(validate_ai_base_url("http://localhost:8080", "openai").is_err());
        assert!(validate_ai_base_url("https://192.168.1.1/admin", "openai").is_err());
        assert!(validate_ai_base_url("https://10.0.0.1/api", "openai").is_err());
        assert!(validate_ai_base_url("https://metadata.google.internal", "openai").is_err());

        // Block non-https for cloud providers
        assert!(validate_ai_base_url("http://api.openai.com/v1", "openai").is_err());
        assert!(validate_ai_base_url("ftp://api.openai.com/v1", "openai").is_err());

        // Safe cloud provider URL
        assert_eq!(
            validate_ai_base_url("https://api.openai.com/v1", "openai").unwrap(),
            "https://api.openai.com/v1"
        );
        assert_eq!(
            validate_ai_base_url("https://api.deepseek.com", "deepseek").unwrap(),
            "https://api.deepseek.com"
        );

        // Ollama: allows local loopback, blocks remote
        assert_eq!(
            validate_ai_base_url("http://localhost:11434", "ollama").unwrap(),
            "http://localhost:11434"
        );
        assert_eq!(
            validate_ai_base_url("http://127.0.0.1:11434", "ollama").unwrap(),
            "http://127.0.0.1:11434"
        );
        assert!(validate_ai_base_url("http://external-evil.com:11434", "ollama").is_err());
        assert!(validate_ai_base_url("http://192.168.1.50:11434", "ollama").is_err());
    }
}
