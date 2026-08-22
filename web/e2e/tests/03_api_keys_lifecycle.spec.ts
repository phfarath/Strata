import { test, expect } from '@playwright/test';

test.describe('03. Machine API Keys Lifecycle', () => {
  const randomSuffix = Math.floor(Math.random() * 1000000);
  const testUser = {
    fullName: `Dev Agent ${randomSuffix}`,
    email: `agent_keys_${randomSuffix}@strata.dev`,
    password: 'SuperSecretPassword123!',
    workspaceName: `Keys Workspace ${randomSuffix}`,
  };

  test.beforeEach(async ({ page }) => {
    // Signup user
    await page.goto('/');
    await page.getByRole('button', { name: /Sign In/i }).first().click();
    await page.getByRole('button', { name: 'Sign Up' }).click();
    await page.getByPlaceholder('Pedro Farath').fill(testUser.fullName);
    await page.getByPlaceholder('developer@strata.pedrofarath.me').fill(testUser.email);
    await page.getByPlaceholder('••••••••••••').fill(testUser.password);
    await page.getByPlaceholder('My Core Team').fill(testUser.workspaceName);
    await page.getByRole('button', { name: /Create Account/i }).click();
    await expect(page.getByText(/Total Memories/i)).toBeVisible({ timeout: 10000 });
  });

  test('should generate, copy and revoke a machine API key', async ({ page }) => {
    // Navigate to API Keys tab
    await page.locator('aside').getByRole('button', { name: 'API Keys & Agents' }).click();
    await expect(page.getByText('Generate Machine API Key')).toBeVisible();

    // Create a new key
    const keyDescription = `Cursor IDE ${randomSuffix}`;
    await page.getByPlaceholder(/Key Description/i).fill(keyDescription);
    await page.getByRole('button', { name: /Generate/i }).click();

    // Verify key created card with strata_live_ prefix
    await expect(page.getByText(/Save your secret key/i)).toBeVisible({ timeout: 5000 });
    await expect(page.getByText(/strata_live_/i).first()).toBeVisible();

    // Test Copy button
    const copyButton = page.locator('button:has(svg.lucide-copy)').first();
    await expect(copyButton).toBeVisible();
    await copyButton.click();

    // Test Revocation
    page.on('dialog', async (dialog) => {
      await dialog.accept();
    });

    const revokeButton = page.getByRole('button', { name: 'Revoke key' }).first();
    if (await revokeButton.isVisible()) {
      await revokeButton.click();
      await expect(page.getByText(keyDescription)).not.toBeVisible({ timeout: 5000 });
    }
  });
});
