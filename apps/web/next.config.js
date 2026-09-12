/** @type {import('next').NextConfig} */
const nextConfig = {
  reactStrictMode: true,
  // @playground/api-types ships raw TypeScript source (no build step yet),
  // so Next needs to transpile it like first-party app code.
  transpilePackages: ["@playground/api-types"],
  experimental: {
    // lucide-react ships one module per icon behind a barrel file. Importing
    // six icons from the barrel pulls the barrel, and the barrel references
    // every icon in the set — tree-shaking recovers most of it but not the
    // module graph it had to walk to get there. This rewrites each named
    // import to its own deep path at compile time, which is the difference
    // between six icons and a manifest of fourteen hundred.
    optimizePackageImports: ["lucide-react"],
  },
};

module.exports = nextConfig;
