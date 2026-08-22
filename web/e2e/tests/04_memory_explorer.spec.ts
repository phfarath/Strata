import { test, expect } from '@playwright/test';

test.describe('04. Memory Explorer & Inspector', () => {
  const randomSuffix = Math.floor(Math.random() * 1000000);
  const testUser = {
    fullName: `Researcher ${randomSuffix}`,
    email: `explorer_${randomSuffix}@strata.dev`,
    password: 'SuperSecretPassword123!',
  };

  test.beforeEach(async ({ page }) => {
    // Signup user
    await page.goto('/');
    await page.getByRole('banner').getByRole('button', { name: 'Sign In' }).click();
    await page.getByRole('button', { name: 'Sign Up' }).click();
    await page.getByPlaceholder('Pedro Farath').fill(testUser.fullName);
    await page.getByPlaceholder('developer@strata.pedrofarath.me').fill(testUser.email);
    await page.getByPlaceholder('••••••••••••').fill(testUser.password);
    await page.getByRole('button', { name: /Create Account/i }).click();
    await expect(page.getByText(/Total Memories/i)).toBeVisible({ timeout: 10000 });
  });

  test('should search memories and display details in the Memory Inspector panel', async ({ page }) => {
    // Navigate to Memory Explorer
    await page.locator('aside').getByRole('button', { name: 'Memory Explorer' }).click();
    await expect(page.getByPlaceholder(/Search memories/i)).toBeVisible();

    // Verify default list rendered
    await expect(page.locator('h4').filter({ hasText: 'Strict Security Headers' })).toBeVisible();
    await expect(page.getByText(/Anti-Pattern: OpenSSL/i).first()).toBeVisible();

    // Test Search input
    await page.getByPlaceholder(/Search memories/i).fill('OpenSSL');
    await expect(page.getByText(/Anti-Pattern: OpenSSL/i).first()).toBeVisible();
    await expect(page.locator('h4').filter({ hasText: 'Strict Security Headers' })).not.toBeVisible();

    // Click on memory card to inspect
    await page.getByText(/Anti-Pattern: OpenSSL/i).first().click();

    // Verify Inspector details on the right panel
    await expect(page.getByText('Memory Inspector')).toBeVisible();
    await expect(page.getByText('STATEMENT BEDROCK')).toBeVisible();
    await expect(page.getByText(/Always prefer pure Rust TLS with rustls/i).first()).toBeVisible();

    // Clear search and test filter pill
    await page.getByPlaceholder(/Search memories/i).fill('');
    await page.getByRole('button', { name: 'Facts' }).click();
    await expect(page.locator('h4').filter({ hasText: 'Strict Security Headers' })).toBeVisible();
    await expect(page.locator('h4').filter({ hasText: 'Anti-Pattern: OpenSSL' })).not.toBeVisible();
  });
});
