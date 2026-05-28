import type { NextConfig } from "next";

const nextConfig: NextConfig = {
  output: "export",
  async rewrites() {
    if (process.env.NODE_ENV === "development") {
      return [
        {
          source: "/api/:path*",
          destination: "http://127.0.0.1:8787/api/:path*",
        },
      ];
    }
    return [];
  },
};

export default nextConfig;
