// How the generated API client is set up, read by the client when it is made
// (`runtimeConfigPath` in openapi-ts.config.ts), so no page has to import a
// module just to point the client at the API.
import type { CreateClientConfig } from './api/client.gen';
import { API_URL } from './csp';

export const createClientConfig: CreateClientConfig = (config) => ({ ...config, baseUrl: API_URL });
