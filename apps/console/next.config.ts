import { execFileSync } from "node:child_process";
import path from "node:path";
import type { NextConfig } from "next";

/** The commit this console was built from, for "About harbard". */
function commit(): string {
  try {
    return execFileSync("git", ["rev-parse", "--short", "HEAD"], { cwd: import.meta.dirname, encoding: "utf8" }).trim();
  } catch {
    return "";
  }
}

const nextConfig: NextConfig = {
  // The console is a local operations surface: no image optimization service,
  // no runtime network beyond the store API. Keep the output boring.
  reactStrictMode: true,
  env: { NEXT_PUBLIC_HARBARD_COMMIT: process.env.NEXT_PUBLIC_HARBARD_COMMIT ?? commit() },
  poweredByHeader: false,
  typedRoutes: true,
  // Ships TypeScript source; the workspace has no build step for it.
  transpilePackages: ["@harbard/huginn"],
  images: { unoptimized: true },
  // The console is watched full-screen for hours; a floating dev badge sitting
  // on top of the surface is chrome nobody asked for.
  devIndicators: false,
  // The console is opened at 127.0.0.1, localhost, and — via portless — a
  // stable named host (harbard.local / harbard.localhost). Without these the
  // Next 16 dev server treats the host as foreign and blocks HMR / assets.
  allowedDevOrigins: [
    "127.0.0.1",
    "localhost",
    "harbard.local",
    "*.harbard.local",
    "harbard.localhost",
    "*.harbard.localhost",
  ],
  // Monorepo/worktree safety: pin Turbopack to the repo root.
  turbopack: { root: path.resolve(import.meta.dirname, "../..") },
  // The Content-Security-Policy is nonce-based and lives in proxy.ts (it
  // needs a per-request value). Here we keep the static, cache-safe headers.
  async headers() {
    return [
      {
        source: "/:path*",
        headers: [
          { key: "X-Content-Type-Options", value: "nosniff" },
          { key: "Referrer-Policy", value: "no-referrer" },
          { key: "X-Frame-Options", value: "DENY" },
        ],
      },
    ];
  },
};

export default nextConfig;
