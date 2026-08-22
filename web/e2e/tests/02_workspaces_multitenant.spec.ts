import { test, expect } from '@playwright/test';

test.describe('02. Workspace Management & Multi-Tenancy', () => {
  const randomSuffix = Math.floor(Math.random() * 1000000);
  const testUser = {
    fullName: `Team Lead ${randomSuffix}`,
    email: `workspaces_${randomSuffix}@strata.dev`,
    password: 'SuperSecretPassword123!',
    workspaceName: `Primary Workspace ${randomSuffix}`,
  };

  test.beforeEach(async ({ page }) => {
    // Signup user
    await page.goto('/');
    await page.getByRole('banner').getByRole('button', { name: 'Sign In' }).click();
    await page.getByRole('button', { name: 'Sign Up' }).click();
    await page.getByPlaceholder('Pedro Farath').fill(testUser.fullName);
    await page.getByPlaceholder('developer@strata.pedrofarath.me').fill(testUser.email);
    await page.getByPlaceholder('••••••••••••').fill(testUser.password);
    await page.getByPlaceholder('My Core Team').fill(testUser.workspaceName);
    await page.getByRole('button', { name: /Create Account/i }).click();
    await expect(page.getByText(/Total Memories/i)).toBeVisible({ timeout: 10000 });
  });

  test('should create a new workspace and switch active workspace', async ({ page }) => {
    // Click on workspace selector in sidebar
    const wsSelectorBtn = page.locator('aside').getByRole('button', { name: new RegExp(testUser.workspaceName, 'i') }).first();
    await wsSelectorBtn.click();

    // Click "New Workspace"
    const newWsBtn = page.getByRole('button', { name: 'New Workspace' });
    await expect(newWsBtn).toBeVisible();
    await newWsBtn.click();

    // Fill workspace modal
    await expect(page.getByText('Create New Workspace')).toBeVisible();
    const secondaryWsName = `Mobile App Team ${randomSuffix}`;
    await page.getByPlaceholder(/Mobile App Team/i).fill(secondaryWsName);
    await page.getByRole('button', { name: /^Create Workspace$/i }).click();

    // Verify workspace is updated on the Dashboard
    await expect(page.getByRole('heading', { level: 1, name: secondaryWsName })).toBeVisible({ timeout: 5000 });
  });
});
