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
  async rewrites() {
    // Faz 21 (P0-9): /api/engine/* istemci çağrıları MIDDLEWARE'de yakalanıp
    // Bearer'la motora taşınır (rewrites middleware'i geçer; route-handler
    // yaklaşımı afterFiles rewrite dinamik route'lardan önce çalıştığı için
    // işlemedi). Bu rewrite kalan /api/* yollarını taşır — tarayıcıdan gelen
    // bu istekler anahtarsızdır, motor 401 verir; anahtar tarayıcıya asla
    // girmez.
    return [
      {
        source: "/api/:path*",
        destination: "http://127.0.0.1:3002/api/:path*",
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
