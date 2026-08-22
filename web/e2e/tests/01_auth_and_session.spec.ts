import { test, expect } from '@playwright/test';

test.describe('01. Authentication & Session Flow', () => {
  const randomSuffix = Math.floor(Math.random() * 1000000);
  const testUser = {
    fullName: `DevUser ${randomSuffix}`,
    email: `e2e_dev_${randomSuffix}@strata.dev`,
    password: 'SuperSecretPassword123!',
    workspace: `Workspace Alpha ${randomSuffix}`,
  };

  test('should render Landing Page and open Auth modal', async ({ page }) => {
    await page.goto('/');

    // Check title and brand elements
    await expect(page).toHaveTitle(/Strata/i);
    await expect(page.getByText('Persistent cognitive memory layer for coding agents')).toBeVisible();

    // Click Sign In on navbar
    await page.getByRole('button', { name: /Sign In/i }).first().click();

    // Verify Auth Modal is displayed
    await expect(page.getByText('Sign In to Console')).toBeVisible();
    await expect(page.getByRole('button', { name: 'Sign Up' })).toBeVisible();
  });

  test('should register a new user, auto-create workspace and persist session', async ({ page }) => {
    await page.goto('/');

    // Open Auth Modal and switch to Sign Up
    await page.getByRole('button', { name: /Sign In/i }).first().click();
    await page.getByRole('button', { name: 'Sign Up' }).click();
    await expect(page.getByText('Create Developer Account')).toBeVisible();

    // Fill form
    await page.getByPlaceholder('Pedro Farath').fill(testUser.fullName);
    await page.getByPlaceholder('developer@strata.pedrofarath.me').fill(testUser.email);
    await page.getByPlaceholder('••••••••••••').fill(testUser.password);
    await page.getByPlaceholder('My Core Team').fill(testUser.workspace);

    // Submit Sign Up
    await page.getByRole('button', { name: /Create Account/i }).click();

    // Verify Dashboard is visible
    await expect(page.getByText(/Total Memories/i)).toBeVisible({ timeout: 10000 });
    await expect(page.getByText(testUser.fullName).first()).toBeVisible();

    // Verify JWT is saved in localStorage
    const token = await page.evaluate(() => localStorage.getItem('strata_token'));
    expect(token).toBeTruthy();

    // Reload page (F5) to test session restoration
    await page.reload();

    // Should remain logged in without returning to Landing Page
    await expect(page.getByText(/Total Memories/i)).toBeVisible({ timeout: 10000 });
    await expect(page.getByText(testUser.fullName).first()).toBeVisible();
  });

  test('should logout and login successfully with existing credentials', async ({ page }) => {
    await page.goto('/');

    // Open Sign In modal
    await page.getByRole('banner').getByRole('button', { name: 'Sign In' }).click();
    await expect(page.getByText('Sign In to Console')).toBeVisible();

    // Fill Login Form
    await page.getByPlaceholder('developer@strata.pedrofarath.me').fill(testUser.email);
    await page.getByPlaceholder('••••••••••••').fill(testUser.password);
    await page.locator('form').getByRole('button', { name: 'Sign In' }).click();

    // Verify Console opens
    await expect(page.getByText(/Total Memories/i)).toBeVisible({ timeout: 10000 });

    // Logout
    const logoutBtn = page.getByRole('button', { name: /Sign Out/i });
    await expect(logoutBtn).toBeVisible();
    await logoutBtn.click();

    // Verify return to Landing page
    await expect(page.getByText('Persistent cognitive memory layer for coding agents')).toBeVisible({ timeout: 5000 });
    const clearedToken = await page.evaluate(() => localStorage.getItem('strata_token'));
    expect(clearedToken).toBeNull();
  });
});
