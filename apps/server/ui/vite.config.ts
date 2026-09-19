import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';

// Le binaire serveur embarque `dist/` via `include_str!` (apps/server/src/ui.rs)
// et sert trois routes fixes : `/`, `/app.js` et `/style.css`. Les noms de
// sortie sont donc figes — pas de hash — et tout part dans un seul chunk pour
// qu'aucun fichier supplementaire ne soit a servir.
// Routes servies par le serveur Rust : en developpement (`npm run dev`), Vite
// sert l'interface et relaie ces appels au binaire ecoutant sur 8080.
const ROUTES_API = ['/health', '/audio', '/projets', '/projet', '/valider', '/visuel', '/affiner', '/annuler', '/reprendre'];

export default defineConfig({
  plugins: [react()],
  server: {
    proxy: Object.fromEntries(
      ROUTES_API.map((route) => [route, { target: 'http://127.0.0.1:8080', changeOrigin: true }]),
    ),
  },
  build: {
    outDir: 'dist',
    emptyOutDir: true,
    // Une application locale : la lisibilite d'un crash prime sur les octets.
    sourcemap: false,
    rollupOptions: {
      output: {
        inlineDynamicImports: true,
        entryFileNames: 'app.js',
        assetFileNames: (info) =>
          info.names?.some((nom) => nom.endsWith('.css')) ? 'style.css' : 'asset-[name][extname]',
      },
    },
  },
});
