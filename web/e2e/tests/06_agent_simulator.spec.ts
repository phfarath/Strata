import { test, expect } from '@playwright/test';

test.describe('06. Agent Memory Recall Simulator & JTMS', () => {
  const randomSuffix = Math.floor(Math.random() * 1000000);
  const testUser = {
    fullName: `AI Engineer ${randomSuffix}`,
    email: `simulator_${randomSuffix}@strata.dev`,
    password: 'SuperSecretPassword123!',
  };

  test.beforeEach(async ({ page }) => {
    // Signup user
    await page.goto('/');
    await page.getByRole('button', { name: /Sign In/i }).first().click();
    await page.getByRole('button', { name: 'Sign Up' }).click();
    await page.getByPlaceholder('Pedro Farath').fill(testUser.fullName);
    await page.getByPlaceholder('developer@strata.pedrofarath.me').fill(testUser.email);
    await page.getByPlaceholder('••••••••••••').fill(testUser.password);
    await page.getByRole('button', { name: /Create Account/i }).click();
    await expect(page.getByText(/Total Memories/i)).toBeVisible({ timeout: 10000 });
  });

  test('should execute simulator query and display JTMS arbitration with low latency', async ({ page }) => {
    // Navigate to Simulator tab
    await page.locator('aside').getByRole('button', { name: 'Simulator' }).click();
    await expect(page.getByText('Agent Memory Recall Simulator')).toBeVisible();

    // Fill Query
    const textarea = page.locator('textarea');
    await textarea.fill('How to configure Postgres connection pool on Axum with Rustls?');

    // Click execute
    await page.getByRole('button', { name: /Execute Simulator Query/i }).click();

    // Verify loading and result
    await expect(page.getByText(/Latency:/i)).toBeVisible({ timeout: 5000 });
    await expect(page.getByText(/JTMS State:/i)).toBeVisible();
    await expect(page.getByText('CONSISTENT')).toBeVisible();
    await expect(page.getByText('Injected Agent Directive:')).toBeVisible();
  });
});
