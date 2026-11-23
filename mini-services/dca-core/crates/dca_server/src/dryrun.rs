//! Dry-run (v3 `/api/live/dryrun` karşılığı — Faz 1.1).
//!
//! POST /api/live/dryrun — M08 tick/step/minNotional hizalama, `sent:false`.
//! AĞA EMİR GİTMEZ: yalnızca halka-açık instrument kuralları (+ fiyatsız
//! MARKET'ta notional referansı için halka-açık ticker) okunur; vault'a
//! dokunulmaz, anahtar gerekmez. v3 ile birebir: Binance spot kuralları,
//! LIMIT+SELL → tickUp / diğerleri → tickDown, miktar hep aşağı,
//! minQty/minNotional ihlalinde otomatik BÜYÜTME YOK → `ok:false`.
//!
//! Bilinçli sapmalar (kodda işaretli):
//! - D1: v3 `MARKET + quoteOrderQty` kolunda adedi `quantity ?? '0'`'dan
//!   hesaplayıp hep `ok:false` döner (kaynak: `buildDryRunOrder`); burada
//!   quoteOrderQty doğrudan minNotional'a karşı doğrulanır.
//! - D2: v3 fiyatsız MARKET'ta notionalı `0` fiyattan hesaplayıp hep reddeder;
//!   burada halka-açık ticker `last_price` referans alınır ve yanıtta
//!   `refPrice` olarak açıklanır (salt-okunur, emir fiyatı değil).
//! - D3: STOP_MARKET / TAKE_PROFIT_MARKET'ta `price` yoksa `stopPrice`
//!   referans alınır (v3 yalnız `price`'a bakar).

use axum::extract::State;
use axum::Json;
use dca_math::fraction::Fraction;
use dca_math::Instrument;
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;

use super::{err422, normalize_pair};
use crate::state::{now_ms, AppState};

#[derive(Deserialize)]
pub struct DryRunReq {
    pub symbol: String,
    pub side: String,
    #[serde(rename = "type")]
    pub order_type: String,
    pub quantity: Option<String>,
    #[serde(alias = "quoteOrderQty")]
    pub quote_order_qty: Option<String>,
    pub price: Option<String>,
    #[serde(alias = "stopPrice")]
    pub stop_price: Option<String>,
    pub role: Option<String>,
    pub venue: Option<String>,
}

fn nonempty(o: &Option<String>) -> Option<String> {
    o.as_ref().map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

/// Basamak sayısı: hizalı değer zaten filtrenin tam katı olduğundan
/// `to_half_even_display(8)` kırpması kayıpsızdır (v3 `decimalPlaces` karşılığı).
fn dp_of(f: &Fraction) -> usize {
    let s = f.to_half_even_display(8);
    match s.find('.') {
        Some(i) => s[i + 1..].trim_end_matches('0').len(),
        None => 0,
    }
}

fn base36(mut n: i64) -> String {
    const C: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    if n <= 0 {
        return "0".into();
    }
    let mut out = Vec::new();
    while n > 0 {
        out.push(C[(n % 36) as usize] as char);
        n /= 36;
    }
    out.iter().rev().collect()
}

/// Adet kotası (saf M08): önce minQty, sonra notional. Otomatik büyütme YOK —
/// ihlal `Err(reason)` döner, çağıran `ok:false` yazar.
pub fn check_qty_limits(
    qty: &Fraction,
    ref_price: &Fraction,
    min_qty: &Fraction,
    min_notional: &Fraction,
    dp_qty: usize,
) -> Result<Fraction, String> {
    if *qty < *min_qty {
        return Err(format!(
            "M08: qty < minQty ({}) — otomatik büyütülmez",
            min_qty.to_half_even_display(dp_qty)
        ));
    }
    let notional = qty.mul(ref_price).map_err(|e| e.to_string())?;
    if notional < *min_notional {
        return Err(format!(
            "M08: notional < minNotional ({}) — otomatik büyütülmez",
            min_notional.to_half_even_display(2)
        ));
    }
    Ok(notional)
}

/// Kota kotası (saf M08, D1): quoteOrderQty doğrudan minNotional'a karşı.
pub fn check_quote_limits(quote: &Fraction, min_notional: &Fraction) -> Result<(), String> {
    if *quote < *min_notional {
        return Err(format!(
            "M08: quoteOrderQty < minNotional ({}) — otomatik büyütülmez",
            min_notional.to_half_even_display(2)
        ));
    }
    Ok(())
}

pub async fn dryrun(
    State(app): State<Arc<AppState>>,
    Json(req): Json<DryRunReq>,
) -> Result<Json<Value>, axum::response::Response> {
    let fr = |s: &str| Fraction::from_decimal_str(s).map_err(|e| err422(e.to_string()));
    let side = req.side.trim().to_uppercase();
    if side != "BUY" && side != "SELL" {
        return Err(err422("side BUY|SELL olmalı"));
    }
    let otype = req.order_type.trim().to_uppercase();
    match otype.as_str() {
        "MARKET" | "LIMIT" | "STOP_MARKET" | "TAKE_PROFIT_MARKET" => {}
        _ => return Err(err422("type MARKET|LIMIT|STOP_MARKET|TAKE_PROFIT_MARKET olmalı")),
    }
    let symbol = normalize_pair(&req.symbol);
    if symbol.is_empty() {
        return Err(err422("symbol zorunlu"));
    }
    let qty_s = nonempty(&req.quantity);
    let quote_s = nonempty(&req.quote_order_qty);
    if qty_s.is_none() && quote_s.is_none() {
        return Err(err422("quantity veya quoteOrderQty zorunlu"));
    }
    let venue = nonempty(&req.venue).map(|v| v.to_uppercase()).unwrap_or_else(|| "BINANCE".into());
    let adapter = app.adapter(&venue).ok_or_else(|| err422("venue desteklenmiyor"))?;
    let category = "spot";
    // exchangeInfo ağır bir uçtur (MB'lerce) — venue-nitelikli anahtarla
    // instruments_cache'te tutulur (motorun Bybit "spot:X" anahtarlarıyla
    // çakışmaz; fallback varsayılan YOK — kural yoksa fail-closed).
    let cache_key = format!("{venue}:{category}:{symbol}");
    // M-3: cache TTL'li — (cached_at_ms, liste); bayat kayıt yeniden çekilir.
    let cached: Option<Vec<Instrument>> = {
        let cache = app.instruments_cache.lock().await;
        cache
            .get(&cache_key)
            .filter(|(at, _)| crate::state::now_ms() - *at < crate::state::INSTRUMENTS_CACHE_TTL_MS)
            .map(|(_, l)| l.clone())
    };
    let insts: Vec<Instrument> = match cached {
        Some(l) if !l.is_empty() => l,
        _ => {
            let fetched = adapter
                .instruments(category, Some(&symbol))
                .await
                .map_err(|e| err422(format!("instrument kuralları alınamadı ({venue} spot): {e} — fail-closed")))?;
            if fetched.is_empty() {
                return Err(err422(format!("instrument bulunamadı: {symbol} ({venue} spot)")));
            }
            crate::state::insert_bounded_cache(
                &mut *app.instruments_cache.lock().await,
                cache_key,
                fetched.clone(),
                crate::state::now_ms(),
                crate::state::INSTRUMENTS_CACHE_TTL_MS,
                256,
            );
            fetched
        }
    };
    let rule = insts
        .iter()
        .find(|i| i.symbol == symbol)
        .or_else(|| insts.first())
        .ok_or_else(|| err422(format!("instrument bulunamadı: {symbol} ({venue} spot)")))?;

    let dp_tick = dp_of(&rule.tick_size);
    let dp_step = dp_of(&rule.step_size);
    let is_limit = otype == "LIMIT";
    // D3: stop emirlerinde price yoksa stopPrice referans alınır.
    let price_s = nonempty(&req.price).or_else(|| {
        if otype == "STOP_MARKET" || otype == "TAKE_PROFIT_MARKET" {
            nonempty(&req.stop_price)
        } else {
            None
        }
    });
    let mut alignment: Vec<Value> = Vec::new();

    let price_raw = match &price_s {
        Some(p) => fr(p)?,
        None => Fraction::zero(),
    };
    // v3 priceIntent: LIMIT+SELL → long_tp_up (tickUp), LIMIT → buy_limit_down,
    // diğerleri → other (tickDown). Kafes başlangıcı 0 (Binance spot
    // PRICE_FILTER minPrice=0 — bizdeki m02 tick_down/up ile aynı kafes).
    let tick_up = is_limit && side == "SELL";
    let price_aligned = if tick_up {
        dca_math::m02::tick_up(&price_raw, &rule.tick_size).map_err(|e| err422(e))?
    } else {
        dca_math::m02::tick_down(&price_raw, &rule.tick_size).map_err(|e| err422(e))?
    };
    if let Some(p) = &price_s {
        if price_aligned != price_raw {
            alignment.push(json!({
                "field": "price",
                "raw": p,
                "aligned": price_aligned.to_half_even_display(dp_tick),
                "rule": if tick_up { "tickUp (M08)" } else { "tickDown (M08)" },
            }));
        }
    }

    // Notional referans fiyatı: LIMIT'te hizalı limit fiyatı; fiyatsız MARKET'ta
    // halka-açık ticker (D2 — salt-okunur, emir fiyatı değil).
    let mut ref_note: Option<String> = None;
    let ref_price = if price_s.is_some() {
        price_aligned.clone()
    } else if otype == "MARKET" {
        let t = adapter
            .ticker(category, &symbol)
            .await
            .map_err(|_| err422(format!("MARKET dry-run için referans fiyat alınamadı: {symbol} ticker yok (fail-closed)")))?;
        if t.last_price <= Fraction::zero() {
            return Err(err422("referans fiyat sıfır — fail-closed"));
        }
        ref_note = Some(format!(
            "refPrice {} (ticker last — yalnız notional kontrolü için, emir fiyatı değil)",
            t.last_price.to_half_even_display(dp_tick)
        ));
        t.last_price
    } else {
        price_aligned.clone()
    };

    // Adet kolu: MARKET + yalnız-quote → D1; diğer hallerde quantity zorunlu.
    let (ok, reason, qty_disp): (bool, Option<String>, Option<String>) =
        if otype == "MARKET" && qty_s.is_none() {
            let qq = quote_s.clone().unwrap_or_default();
            let q = fr(&qq)?;
            match check_quote_limits(&q, &rule.min_notional) {
                Ok(()) => (true, None, None),
                Err(r) => (false, Some(r), None),
            }
        } else {
            let qs = qty_s.clone().ok_or_else(|| err422("bu emir tipi için quantity zorunlu"))?;
            let raw = fr(&qs)?;
            let q = dca_math::m02::qty_down(&raw, &rule.step_size).map_err(|e| err422(e))?;
            if q != raw {
                alignment.push(json!({
                    "field": "quantity",
                    "raw": qs,
                    "aligned": q.to_half_even_display(dp_step),
                    "rule": "qtyDown (M08)",
                }));
            }
            match check_qty_limits(&q, &ref_price, &rule.min_qty, &rule.min_notional, dp_step) {
                Ok(_) => (true, None, Some(q.to_half_even_display(dp_step))),
                Err(r) => (false, Some(r), Some(q.to_half_even_display(dp_step))),
            }
        };

    let role = nonempty(&req.role)
        .map(|r| {
            r.to_lowercase()
                .chars()
                .filter(|c| c.is_ascii_alphanumeric() || *c == '_')
                .take(24)
                .collect::<String>()
        })
        .filter(|r| !r.is_empty())
        .unwrap_or_else(|| "base".into());
    let mut order = serde_json::Map::new();
    order.insert("symbol".into(), Value::String(rule.symbol.clone()));
    order.insert("side".into(), Value::String(side.clone()));
    order.insert("type".into(), Value::String(otype.clone()));
    order.insert(
        "newClientOrderId".into(),
        Value::String(format!("dca-grid-{}-{}", role, base36(now_ms()))),
    );
    if otype == "MARKET" && qty_s.is_none() {
        order.insert("quoteOrderQty".into(), Value::String(quote_s.clone().unwrap_or_default()));
    } else if let Some(qd) = &qty_disp {
        order.insert("quantity".into(), Value::String(qd.clone()));
    }
    if otype == "LIMIT" {
        order.insert(
            "price".into(),
            Value::String(price_aligned.to_half_even_display(dp_tick)),
        );
        order.insert("timeInForce".into(), Value::String("GTC".into()));
    }
    if let Some(sp) = nonempty(&req.stop_price) {
        order.insert("stopPrice".into(), Value::String(sp));
    }

    let note = if ok {
        format!(
            "DRY-RUN: emir ağa gönderilmedi (§11.2-4). Payload {venue} spot M08 tick/step/minNotional ile hizalandı.{}",
            ref_note.map(|r| format!(" {r}.")).unwrap_or_default()
        )
    } else {
        format!(
            "DRY-RUN: emir AĞA GÖNDERİLEMEZ — {} (otomatik büyütme yasak, M08).",
            reason.clone().unwrap_or_else(|| "M08 hizalama reddi".into())
        )
    };
    let mut body = json!({
        "order": order,
        "alignment": alignment,
        "ok": ok,
        "note": note,
        "sent": false,
        "venue": venue,
        "category": category,
        "symbol": symbol,
    });
    if let Some(r) = reason {
        body["reason"] = Value::String(r);
    }
    Ok(Json(body))
}

#[cfg(test)]
mod dryrun_tests {
    use super::*;

    fn f(s: &str) -> Fraction {
        Fraction::from_decimal_str(s).unwrap()
    }

    #[test]
    fn dp_of_matches_filter_decimals() {
        assert_eq!(dp_of(&f("0.01")), 2);
        assert_eq!(dp_of(&f("0.0001")), 4);
        assert_eq!(dp_of(&f("0.1")), 1);
        assert_eq!(dp_of(&f("1")), 0);
        assert_eq!(dp_of(&f("0.00000001")), 8);
    }

    #[test]
    fn base36_shapes_client_id_suffix() {
        assert_eq!(base36(0), "0");
        assert_eq!(base36(36), "10");
        assert!(!base36(1700000000000).is_empty());
    }

    #[test]
    fn qty_below_minqty_rejected_not_enlarged() {
        // step=0.0001 altında 0.00005 → aşağı 0'a iner; otomatik büyütülmez.
        let q = dca_math::m02::qty_down(&f("0.00005"), &f("0.0001")).unwrap();
        assert_eq!(q, Fraction::zero());
        let r = check_qty_limits(&q, &f("80000"), &f("0.0001"), &f("5"), 4);
        assert!(r.is_err() && r.unwrap_err().contains("minQty"));
    }

    #[test]
    fn qty_ok_but_notional_below_min_rejected() {
        // 0.0001 BTC @ 30000 = 3 USDT < 5 minNotional → ret.
        let r = check_qty_limits(&f("0.0001"), &f("30000"), &f("0.0001"), &f("5"), 4);
        assert!(r.is_err() && r.unwrap_err().contains("minNotional"));
    }

    #[test]
    fn qty_ok_passes_with_notional() {
        let r = check_qty_limits(&f("0.001"), &f("80000"), &f("0.0001"), &f("5"), 4).unwrap();
        assert_eq!(r, f("80"));
    }

    #[test]
    fn quote_path_validates_against_min_notional() {
        assert!(check_quote_limits(&f("10"), &f("5")).is_ok());
        let r = check_quote_limits(&f("4.99"), &f("5"));
        assert!(r.is_err() && r.unwrap_err().contains("minNotional"));
    }

    #[test]
    fn limit_sell_price_rounds_up_buy_down() {
        // v3 priceIntent birebiri: LIMIT SELL → tickUp, LIMIT BUY → tickDown.
        assert_eq!(
            dca_math::m02::tick_up(&f("100.231"), &f("0.01")).unwrap(),
            f("100.24")
        );
        assert_eq!(
            dca_math::m02::tick_down(&f("100.239"), &f("0.01")).unwrap(),
            f("100.23")
        );
    }
}
