import type { NextConfig } from "next";

const localRun = process.env.DCA_LOCAL_RUN === "1";

const nextConfig: NextConfig = {
  output: "standalone",
  /* config options here */
  typescript: {
    // OPS-02: tip hatalı üretim çıktısını engellemek için build zamanında
    // typecheck zorunlu kılınmıştır. ignoreBuildErrors KALDIRILDI.
    ignoreBuildErrors: false,
  },
  reactStrictMode: false,
  // rewrite yok: tüm /api/* istekleri middleware.ts tarafından yakalanıp
  // :3002'ye proxy edilir (Bearer kimlik doğrulaması dahil). /api/health ve
  // /api/stream özel durumlar middleware'de doğrudan :3002'ye yönlendirilir.
  // Faz 29 NOTU: src/proxy.ts mevcutken Next 16.4.3 bu rewrite'ı UYGULAMIYOR
  // (gözlem: /api/* → 404). Uygulamanın motor erişimi tamamen /api/engine/*
  // proxy'sinden geçtiği için davranış değişmiyor; blok yalnızca geçmiş
  // sözleşme kaydı olarak tutuluyor (geri dönüş senaryosu).
  async rewrites() {
    return [];
  },
  async headers() {
    return [
      {
        source: "/:path*",
        headers: [
          { key: "X-DNS-Prefetch-Control", value: "on" },
          { key: "Strict-Transport-Security", value: "max-age=63072000; includeSubDomains; preload" },
          { key: "X-Frame-Options", value: "DENY" },
          { key: "X-Content-Type-Options", value: "nosniff" },
          { key: "Referrer-Policy", value: "strict-origin-when-cross-origin" },
          { key: "Permissions-Policy", value: "camera=(), microphone=(), geolocation=(), browsing-topics=()" },
          {
            key: "Content-Security-Policy",
            value: "default-src 'self'; script-src 'self' 'unsafe-eval' 'unsafe-inline'; style-src 'self' 'unsafe-inline'; font-src 'self' data:; img-src 'self' data: blob: https:; connect-src 'self' ws: wss: http: https:; frame-ancestors 'none';",
          },
        ],
      },
    ];
  },
  experimental: {
    ...(localRun ? { cpus: 1, webpackMemoryOptimizations: true } : {}),
    // Persist turbopack incremental state so restarts don't re-peak memory
    turbopackFileSystemCacheForDev: true,
    // Isolate webpack compile in a worker (sandbox memory ceiling)
    webpackBuildWorker: true,
  },
  webpack: (config) => {
    if (localRun) config.cache = false;
    // Dev source maps dominate webpack memory in this sandbox (host-global OOM
    // reaper targets next-server at >1.8GB RSS). Cheap compile > debuggability.
    if (config.devtool) config.devtool = false;
    config.watchOptions = { ...config.watchOptions, ignored: ["**/public/**", "**/tests/**"] };
    return config;
  },
};

export default nextConfig;
