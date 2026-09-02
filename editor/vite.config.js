import { defineConfig } from "vite";

export default defineConfig({
  // Relative asset URLs work both at / and at /<repository>/ on GitHub Pages.
  base: "./",
  build: {
    target: "es2022",
  },
});
