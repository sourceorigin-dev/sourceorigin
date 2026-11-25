/**
 * Signing a device without a browser in: an app registered in the developer
 * console with the device grant asks for its codes, the user types the user code
 * on auth's device page, which names the app and warns before it approves
 * anything, and the device's next poll collects its tokens. The device then shows
 * among the account's devices, named after its app, and its link's code only
 * fills the field in.
 */
import { expect, test } from '@playwright/test';
import { addPasskeys, authUrl, signUp } from './helpers';

/** The device grant's name at the token endpoint. */
const deviceGrant = 'urn:ietf:params:oauth:grant-type:device_code';

/** Runs against auth's pages, with Chromium's virtual passkey authenticator. */
test('a device signs in once its user approves its code', async ({ page, context }) => {
  await addPasskeys(context, page);
  await signUp(page, `device-${Date.now().toString(36)}`);

  // A command-line tool keeps no secret, and signs in with the device grant.
  await page.goto('/developers/new');
  await page.getByLabel('App name', { exact: true }).fill('Device E2E');
  await page.getByLabel('No, it runs in the browser or on a device').check();
  await page
    .getByRole('textbox', { name: 'Redirect URIs', exact: true })
    .fill('http://127.0.0.1/callback');
  await page.getByRole('switch', { name: 'Device code', exact: true }).check();
  await page.getByRole('switch', { name: 'Refresh tokens', exact: true }).check();
  await page.getByRole('switch', { name: 'offline_access' }).check();
  await page.getByRole('button', { name: 'Register app', exact: true }).click();
  const id = (await page.locator('#client-id').textContent()) ?? '';
  expect(id).toMatch(/^c_/);

  const started = await page.request.post(new URL('/oauth/device_authorization', authUrl).href, {
    form: { client_id: id, scope: 'openid offline_access' }
  });
  expect(started.status()).toBe(200);
  const codes = (await started.json()) as {
    device_code: string;
    user_code: string;
    verification_uri: string;
    verification_uri_complete: string;
  };
  expect(codes.verification_uri).toBe(new URL('/device', authUrl).href);
  const poll = () =>
    page.request.post(new URL('/oauth/token', authUrl).href, {
      form: { grant_type: deviceGrant, client_id: id, device_code: codes.device_code }
    });
  expect(((await (await poll()).json()) as { error: string }).error).toBe('authorization_pending');

  // The link fills the code in, and approves nothing by itself.
  await page.goto(codes.verification_uri_complete);
  await expect(page.getByRole('heading', { name: 'Sign in on a device' })).toBeVisible();
  const code = page.getByLabel('Code', { exact: true });
  await expect(code).toHaveValue(codes.user_code);
  await page.getByRole('button', { name: 'Continue', exact: true }).click();

  await expect(
    page.getByRole('heading', { name: 'Sign in to Device E2E on your device?' })
  ).toBeVisible();
  await expect(
    page.getByText('Only continue if you started this on your own device.')
  ).toBeVisible();
  await expect(page.getByText(/^Unverified app, registered by @device-/)).toBeVisible();
  await expect(page.locator('p.device-code')).toHaveText(codes.user_code);
  await page.getByRole('button', { name: 'Continue', exact: true }).click();
  await expect(page.getByRole('heading', { name: 'Your device is signed in' })).toBeVisible();

  const collected = await poll();
  expect(collected.status()).toBe(200);
  const tokens = (await collected.json()) as { id_token?: string; refresh_token?: string };
  expect(tokens.id_token).toBeTruthy();
  expect(tokens.refresh_token).toBeTruthy();
  expect(((await (await poll()).json()) as { error: string }).error).toBe('invalid_grant');

  // The device is listed apart from this browser, and signed out on its own.
  await page.goto('/settings/sessions');
  await expect(page.getByText('Device E2E on a device')).toBeVisible();
  await page.getByRole('button', { name: /^Sign out Device E2E on a device/ }).click();
  await expect(page.getByText('The device was signed out.')).toBeVisible();
  await expect(page.getByText('Device E2E on a device')).toBeHidden();
  const refreshed = await page.request.post(new URL('/oauth/token', authUrl).href, {
    form: { grant_type: 'refresh_token', client_id: id, refresh_token: tokens.refresh_token ?? '' }
  });
  expect(((await refreshed.json()) as { error: string }).error).toBe('invalid_grant');
});

/** A code nobody is waiting under is refused, and the page keeps what was typed. */
test('the device page refuses a code no device shows', async ({ page, context }) => {
  await addPasskeys(context, page);
  await signUp(page, `nodevice-${Date.now().toString(36)}`);
  await page.goto('/device');
  await page.getByLabel('Code', { exact: true }).fill('bcdf-ghjk');
  await page.getByRole('button', { name: 'Continue', exact: true }).click();
  await expect(page.getByText(/^No device is waiting for this code\./)).toBeVisible();
  await expect(page.getByLabel('Code', { exact: true })).toHaveValue('bcdf-ghjk');
});
