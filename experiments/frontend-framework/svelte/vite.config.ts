import adapter from '@sveltejs/adapter-node';
import { sveltekit } from '@sveltejs/kit/vite';
import { defineConfig } from 'vite';

import { CSP_DIRECTIVES } from '../shared/src/csp.ts';

export default defineConfig({
  // The design tokens are imported from crates/ui, outside this folder.
  server: { fs: { allow: ['../../..'] } },
  // Vite's default bundles a worker into one file, which for Pierre's worker
  // is every grammar at once: 10.9 MB. As a module it loads grammars on demand.
  worker: { format: 'es' },
  plugins: [
    sveltekit({
      // Not compressed ahead of time: the React build is not either, and a
      // proxy would compress both the same way in production.
      adapter: adapter({ precompress: false }),
      // SvelteKit adds a nonce to script-src and style-src for what it writes
      // inline. hooks.server.ts loosens style-src where Pierre needs it.
      csp: {
        mode: 'nonce',
        directives: {
          ...CSP_DIRECTIVES,
          // SvelteKit's own root component writes its route announcer with a
          // style attribute, at src/runtime/components/root.svelte. This allows
          // that one attribute and no other; it must follow SvelteKit's text.
          'style-src-attr': ["'unsafe-hashes'", "'sha256-S8qMpvofolR8Mpjy4kQvEm7m1q8clzU4dfDH0AmvZjo='"],
        },
      },
    }),
  ],
});
