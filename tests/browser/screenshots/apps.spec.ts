import { expect, test } from '@playwright/test';
import { addPasskeys, authorization, authUrl, signUp } from '../helpers';
import { handle, shoot } from './docs';

/** The app the screenshots register. Nothing serves it: they stop at its consent. */
const app = 'https://notes.example';

/**
 * Settings, then registering an app in the developer console, then its consent
 * page, then the device page approving a device the app signs in with a code.
 */
test('accounts and apps', async ({ page, context }) => {
  await addPasskeys(context, page);
  await signUp(page, handle());
  await shoot(page, 'account/settings');

  await page.goto('/developers/new');
  await page.getByLabel('App name', { exact: true }).fill('Notes');
  await page.getByLabel('No, it runs in the browser or on a device').check();
  await page.getByRole('textbox', { name: 'Redirect URIs', exact: true }).fill(`${app}/callback`);
  await shoot(
    page,
    'apps/new-app',
    page.locator('form').filter({ has: page.getByRole('button', { name: 'Register app' }) })
  );
  await page.getByRole('button', { name: 'Register app', exact: true }).click();
  // Read before `shoot` replaces it with the one the guides show.
  const id = (await page.locator('#client-id').textContent()) ?? '';
  await shoot(page, 'apps/app-overview');

  await page.goto(authorization(id, `${app}/callback`, 'openid profile email').url);
  await expect(page.getByRole('heading', { name: 'Continue to Notes?' })).toBeVisible();
  await shoot(page, 'apps/consent');

  const settings = `/developers/clients/${id}/settings`;
  await page.goto(settings);
  await page.getByRole('switch', { name: 'Device code', exact: true }).check();
  await page.getByRole('button', { name: 'Save changes', exact: true }).click();
  await expect(page.getByText('Your changes were saved.')).toBeVisible();
  const started = await page.request.post(new URL('/oauth/device_authorization', authUrl).href, {
    form: { client_id: id, scope: 'openid profile' }
  });
  const { verification_uri_complete: link } = (await started.json()) as {
    verification_uri_complete: string;
  };
  await page.goto(link);
  await page.getByRole('button', { name: 'Continue', exact: true }).click();
  await expect(
    page.getByRole('heading', { name: 'Sign in to Notes on your device?' })
  ).toBeVisible();
  await shoot(page, 'apps/device-approval');
});
