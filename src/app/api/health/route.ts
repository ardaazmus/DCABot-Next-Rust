export const dynamic = "force-dynamic";

const ALLOWED_HOSTS = new Set(["127.0.0.1", "localhost"]);
const ENGINE_HOST = "127.0.0.1";

function enginePort(): string {
  const raw = process.env.NEXT_PUBLIC_ENGINE_PORT || "";
  return /^\d{2,5}$/.test(raw) && Number(raw) > 0 && Number(raw) < 65536 ? raw : "3002";
}

export async function GET() {
  let target: URL;
  try {
    target = new URL(`http://${ENGINE_HOST}:${enginePort()}/api/health`);
  } catch {
    return new Response(JSON.stringify({ status: "error", error: "bad engine endpoint" }), {
      status: 500,
      headers: { "Content-Type": "application/json" },
    });
  }

  if (target.protocol !== "http:" || !ALLOWED_HOSTS.has(target.hostname)) {
    return new Response(JSON.stringify({ status: "error", error: "engine endpoint not allowed" }), {
      status: 403,
      headers: { "Content-Type": "application/json" },
    });
  }

  try {
    const upstream = await fetch(target, { cache: "no-store" });
    const data = await upstream.text();
    return new Response(data, {
      status: upstream.status,
      headers: { "Content-Type": "application/json" },
    });
  } catch {
    return new Response(
      JSON.stringify({ status: "error", error: "engine unreachable" }),
      {
        status: 502,
        headers: { "Content-Type": "application/json" },
      }
    );
  }
}
