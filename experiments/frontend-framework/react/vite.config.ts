import { tanstackStart } from '@tanstack/react-start/plugin/vite';
import viteReact from '@vitejs/plugin-react';
import { defineConfig } from 'vite';

export default defineConfig({
  // The design tokens are imported from crates/ui, outside this folder.
  server: { fs: { allow: ['../../..'] } },
  // Vite's default bundles a worker into one file, which for Pierre's worker
  // is every grammar at once: 10.9 MB. As a module it loads grammars on demand.
  worker: { format: 'es' },
  plugins: [tanstackStart(), viteReact()],
});
