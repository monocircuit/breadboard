import { defineConfig } from "eslint/config";
import { FlatConfigLoader } from "@monocircuit/eslint-config";

const flatConfig = await new FlatConfigLoader({
  javascript: {
    globals: "node",
  },
  json: true,
}).load();

export default defineConfig([
  // The spike's build output (#307).
  { ignores: ["spikes/*/target/**"] },
  { extends: [flatConfig] },
  // Command line tools: the console is their output.
  { rules: { "no-console": "off" } },
]);
