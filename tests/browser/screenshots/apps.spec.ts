import { expect, test } from '@playwright/test';
import { addPasskeys, authorization, signUp } from '../helpers';
import { handle, shoot } from './docs';

/** The app the screenshots register. Nothing serves it: they stop at its consent. */
const app = 'https://notes.example';

/**
 * Settings, then registering an app in the developer console, its insights,
 * then its consent page.
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

  // A new app's steps are all too few to show, so the card looks the same every run.
  await page
    .getByRole('navigation', { name: 'App' })
    .getByRole('link', { name: 'Insights' })
    .click();
  await shoot(
    page,
    'apps/app-insights',
    page
      .locator('section')
      .filter({ has: page.getByRole('heading', { name: 'How far sign-ins get' }) })
  );

  await page.goto(authorization(id, `${app}/callback`, 'openid profile email').url);
  await expect(page.getByRole('heading', { name: 'Continue to Notes?' })).toBeVisible();
  await shoot(page, 'apps/consent');
});
