/** @type {import('next').NextConfig} */
const nextConfig = {
  reactStrictMode: true,
  // @playground/api-types ships raw TypeScript source (no build step yet),
  // so Next needs to transpile it like first-party app code.
  transpilePackages: ["@playground/api-types"],
};

module.exports = nextConfig;
