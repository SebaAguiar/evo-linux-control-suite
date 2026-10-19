// @ts-check
import { defineConfig } from "astro/config";

// GitHub Pages de proyecto: https://sebaaguiar.github.io/evo-linux-control-suite/
// Con dominio propio, dejá base en "/".
export default defineConfig({
  base: "/evo-linux-control-suite",
  site: "https://sebaaguiar.github.io",
  trailingSlash: "ignore",
});
