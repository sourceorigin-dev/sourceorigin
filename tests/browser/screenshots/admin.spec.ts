import { test } from '@playwright/test';
import { adminUrl, administrator, grantAdmin, openDashboard } from '../helpers';
import { handle, shoot } from './docs';

test.skip(!adminUrl || !grantAdmin, 'Set E2E_ADMIN_URL and E2E_GRANT_ADMIN for the dashboard.');

/** The admin dashboard, as an administrator sees it. */
test('admin dashboard', async ({ browser }) => {
  const admin = await administrator(browser, handle('sam'));
  // Signing in to the dashboard asks for the passkey on auth's handoff page.
  await openDashboard(admin, '');
  await shoot(admin, 'admin/overview');
});
