/**
 * Wizard-side live advisory warning computation (Task 16-b).
 *
 * Mirrors the Rust Risk Motoru advisory surface: same codes, same frozen
 * Turkish messages. Pure function — no engine calls. The launch 422 body
 * (`code: "RISK_ACKNOWLEDGMENT_REQUIRED"` + `warnings[]`) remains the
 * authoritative engine verdict; this helper only pre-renders the live panel
 * so the user sees consequences before pressing launch.
 */
import type { AdvisoryWarning, UserRiskPolicy } from "@/lib/engine-types";

export interface WizardRiskInput {
  kind: string;
  stop_loss_pct: string | null;
  max_safety_orders: number;
  risk_policy: UserRiskPolicy;
  /** calc/dca M20 verdict (viable=false → SL cannot cover the ladder deviation). */
  m20?: { viable: boolean; firstUnreachableLevel: number | null };
  /** calc/grid fee-breaker verdict (false → grid step below total fees). */
  feeBreakerPass?: boolean;
}

const M20_MSG =
  "Stop-Loss seviyeniz güvenlik merdiveninin toplam sapmasından dar. Erken stop durumunda kalan kademelere ayrılan sermaye ölü kalır — stratejiniz erken pes eder.";
const FEE_MSG =
  "Izgara adımı borsa alış-satış komisyon toplamından dar; her turda net bakiyeniz komisyona erir. (Sıfır komisyon kampanyası dışında zarardır.)";
const SALVO_MSG =
  "Açılış tamponu kapalı — ilk 300 saniyedeki sahte volatilite iğnelerinde en tepeden veya en dipten maliyetlenebilirsiniz (spread 550 bps'e kadar açılabilir).";
const SESSION_MSG =
  "Seans dışı alımlara izin verildi — likidite çok sığdır, spread geniştir; emirler açılışta daha kötü fiyattan dolabilir.";
const MACRO_MSG =
  "Haber anında (FOMC/TÜFE) işlem izni verildi — ani iki yönlü fitillerde saniyeler içinde stop olma veya merdivenin hızla tükenme riski var.";
const GAP_MSG =
  "Gap konsolidasyonu kapalı — atlanan her seviye tek tek, ayrı ayrı komisyonla doldurulur; ortalama maliyet daha yavaş iyileşir.";
const DD_MSG =
  "Günlük devre kesici kapalı — arka arkaya gelen ters piyasa hareketlerinde bot durmaksızın zarar yazabilir ve kasanızı tüketebilir.";
const FILL_MSG =
  "İyimser dolum modu — fiyata teğet geçen fitiller emriniz dolmuş gibi sayılır; canlı borsada bu emirlerin çoğu dolmaz ve gerçek sonuçlar backtestten kötü çıkar.";

export function computeWizardWarnings(cfg: WizardRiskInput): AdvisoryWarning[] {
  const out: AdvisoryWarning[] = [];
  const rp = cfg.risk_policy;
  const isGrid = cfg.kind.includes("GRID");

  if (!isGrid && cfg.m20 && cfg.m20.viable === false) {
    out.push({
      code: "M20_SL_CONFLICT",
      title: "M20 SL-Merdiven Çakışması",
      message: M20_MSG,
      severity: "WARN",
    });
  }

  if (isGrid && cfg.feeBreakerPass === false) {
    out.push({ code: "NEGATIVE_FEE_MARGIN", title: "Grid Komisyon Kırıcı", message: FEE_MSG, severity: "WARN" });
  }

  if (rp.opening_salvo_buffer === "PERMISSIVE_WITH_WARNING") {
    out.push({
      code: "OPENING_SALVO_DISABLED",
      title: "Açılış Salvo Tamponu Kapalı",
      message: SALVO_MSG,
      severity: "WARN",
    });
  }

  if (rp.session_guard === "PERMISSIVE_WITH_WARNING" && rp.session_type !== "CRYPTO_24_7") {
    out.push({ code: "SESSION_OVERRIDE", title: "Seans Dışı İşleme İzni", message: SESSION_MSG, severity: "WARN" });
  }

  if (rp.macro_blackout === "PERMISSIVE_WITH_WARNING") {
    out.push({
      code: "MACRO_OVERRIDE",
      title: "Makro Kara Pencere Devre Dışı",
      message: MACRO_MSG,
      severity: "WARN",
    });
  }

  if (rp.gap_consolidation === "PERMISSIVE_WITH_WARNING") {
    out.push({ code: "GAP_CONSOLIDATION_OFF", title: "Gap Konsolidasyonu Kapalı", message: GAP_MSG, severity: "INFO" });
  }

  const activeSo = rp.active_safety_orders ?? 1;
  if (activeSo > 1) {
    out.push({
      code: "MULTI_SAFETY_RISK",
      title: "Çoklu Güvenlik Emri Riski",
      message: `Aynı anda ${activeSo} güvenlik emri borsada bekleyecek — 1 saniyelik flash-crash iğnesinde tüm seviyeler aynı anda dolarak marj çağrısına (Margin Call) düşebilirsiniz.`,
      severity: "WARN",
    });
  }

  if ((rp.daily_dd_breaker_pct ?? "2") === "0") {
    out.push({ code: "DD_BREAKER_OFF", title: "Günlük Devre Kesici Kapalı", message: DD_MSG, severity: "WARN" });
  }

  if (rp.fill_model === "OPTIMISTIC") {
    out.push({ code: "OPTIMISTIC_FILL", title: "İyimser Dolum Modeli", message: FILL_MSG, severity: "WARN" });
  }

  return out;
}
