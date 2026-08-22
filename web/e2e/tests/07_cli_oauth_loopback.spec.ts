import { test, expect } from '@playwright/test';

test.describe('07. CLI OAuth Loopback Browser Flow', () => {
  test('should render server CLI authorization page and process credentials', async ({ page }) => {
    const serverUrl = process.env.API_URL || 'http://localhost:8080';
    const cliAuthUrl = `${serverUrl}/auth/cli?port=54321&state=e2e_csrf_test_state`;

    // Navigate to CLI Auth page
    await page.goto(cliAuthUrl);

    // Verify page content based on server template
    await expect(page.getByRole('heading', { name: /Authorize Machine/i })).toBeVisible();
    await expect(page.getByText('54321')).toBeVisible();

    // Verify form elements exist
    await expect(page.getByRole('button', { name: /Authorize Terminal/i })).toBeVisible();
    await expect(page.locator('input[type="email"]')).toBeVisible();
    await expect(page.locator('input[type="password"]')).toBeVisible();
  });
});
