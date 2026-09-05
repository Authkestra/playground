import { defineConfig } from "vitest/config";
import path from "path";

export default defineConfig({
  test: {
    environment: "node",
  },
  resolve: {
    alias: {
      "@": path.resolve(__dirname, "./"),
      "@playground/api-types": path.resolve(__dirname, "../../packages/api-types/index.ts"),
    },
  },
});
