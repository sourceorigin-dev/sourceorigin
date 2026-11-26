/**
 * APIs in the developer console: registering one as a resource, defining a scope
 * and giving an app access to it, then the app asking for a token for the API:
 * its consent page names the API and describes the scope, and its access token is
 * for the API.
 */
import { expect, test } from '@playwright/test';
import { addPasskeys, authorization, authUrl, consent, serveClient, signUp } from './helpers';

/** The app the test registers. No server runs it: the test serves its pages itself. */
const client = 'https://apis-e2e.test';
const redirectUri = `${client}/callback`;

/** Runs against auth's developer console, with Chromium's virtual passkey authenticator. */
test('an API is registered with a scope, and an app given access gets tokens for it', async ({
  page,
  context
}) => {
  const { cdp } = await addPasskeys(context, page);
  await serveClient(context, cdp, client);
  const run = Date.now().toString(36);
  await signUp(page, `apis-${run}`);
  const api = `https://apis-${run}.test/api`;

  await page.goto('/developers/new');
  await page.getByLabel('App name', { exact: true }).fill('APIs E2E');
  await page.getByLabel('No, it runs in the browser or on a device').check();
  await page.getByRole('textbox', { name: 'Redirect URIs', exact: true }).fill(redirectUri);
  await page.getByRole('button', { name: 'Register app', exact: true }).click();
  const id = (await page.locator('#client-id').textContent()) ?? '';

  await page.goto('/developers');
  await page.getByRole('link', { name: 'Your APIs', exact: true }).click();
  await expect(page.getByText('You have no API yet.')).toBeVisible();
  // What a field cannot take is said on the page, and nothing is registered.
  await page.getByLabel('Identifier', { exact: true }).fill('https://apis.test/api?x');
  await page.getByLabel('Name', { exact: true }).fill('Notes API');
  await page.getByRole('button', { name: 'Register API', exact: true }).click();
  await expect(page.getByRole('alert')).toContainText('must not contain a query or a fragment');
  await page.getByLabel('Identifier', { exact: true }).fill(api);
  await page.getByLabel('Name', { exact: true }).fill('Notes API');
  await page.getByRole('button', { name: 'Register API', exact: true }).click();
  await expect(page.getByText('The API was registered.')).toBeVisible();
  await expect(page.getByRole('heading', { name: 'Notes API', level: 1 })).toBeVisible();

  await page.getByLabel('Scope', { exact: true }).fill('notes:read');
  await page.getByLabel('Description', { exact: true }).fill('Read your notes');
  await page.getByRole('button', { name: 'Define scope', exact: true }).click();
  await expect(page.getByText('Your changes were saved.')).toBeVisible();
  await page.getByLabel('Client ID', { exact: true }).fill(id);
  await page.getByLabel('Scopes', { exact: true }).fill('notes:read');
  await page.getByRole('button', { name: 'Grant access', exact: true }).click();
  await expect(page.getByText('Your changes were saved.')).toBeVisible();
  await expect(page.getByRole('button', { name: `Revoke the access of ${id}` })).toBeVisible();

  // The app asks for a token for the API: the consent page names it and says what
  // the scope allows, and the access token is for the API.
  const { url, verifier } = authorization(id, redirectUri, 'openid notes:read');
  await page.goto(`${url}&resource=${encodeURIComponent(api)}`);
  await expect(
    page.getByRole('heading', { name: 'APIs E2E will use Notes API as you' })
  ).toBeVisible();
  await expect(page.getByText('Read your notes', { exact: true })).toBeVisible();
  const code = await consent(page, 'APIs E2E', client);
  const answer = await page.request.post(new URL('/oauth/token', authUrl).href, {
    form: {
      grant_type: 'authorization_code',
      client_id: id,
      redirect_uri: redirectUri,
      code,
      code_verifier: verifier
    }
  });
  expect(answer.status()).toBe(200);
  const { access_token: token } = (await answer.json()) as { access_token: string };
  const claims = JSON.parse(Buffer.from(token.split('.')[1], 'base64url').toString()) as {
    aud: string;
    scope: string;
  };
  expect(claims.aud).toBe(api);
  expect(claims.scope.split(' ')).toContain('notes:read');

  // The account's connected apps say what the app can do with the API.
  await page.goto('/settings/apps');
  await expect(page.getByText('Uses Notes API: Read your notes')).toBeVisible();

  // Without its access, the app is refused the API, back on its own page.
  await page.goto('/developers/resources');
  await page.getByRole('link', { name: 'Notes API', exact: true }).click();
  await page.getByRole('button', { name: `Revoke the access of ${id}` }).click();
  await expect(page.getByText('No app has access yet.')).toBeVisible();
  await page.goto(`${authorization(id, redirectUri).url}&resource=${encodeURIComponent(api)}`);
  await page.waitForURL((url) => url.origin === client);
  expect(new URL(page.url()).searchParams.get('error')).toBe('invalid_target');
});
