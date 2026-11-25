import { defineConfig } from '@hey-api/openapi-ts';

// The TypeScript client both candidates use, generated from the stub API's
// OpenAPI document (`stub-api --openapi > openapi.json`).
export default defineConfig({
  input: '../openapi.json',
  output: 'src/api',
  plugins: [
    '@hey-api/typescript',
    '@hey-api/sdk',
    { name: '@hey-api/client-fetch', runtimeConfigPath: './src/client-config.ts' },
  ],
});
