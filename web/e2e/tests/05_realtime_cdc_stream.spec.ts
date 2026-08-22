import { test, expect } from '@playwright/test';

test.describe('05. CDC Realtime Delta Stream (WebSockets)', () => {
  const randomSuffix = Math.floor(Math.random() * 1000000);
  const testUser = {
    fullName: `Streamer ${randomSuffix}`,
    email: `streamer_${randomSuffix}@strata.dev`,
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

  test('should connect to WebSocket and provide controls for Pause, Resume and Clear', async ({ page }) => {
    // Navigate to Stream tab
    await page.locator('aside').getByRole('button', { name: 'CDC Delta Stream' }).click();
    await expect(page.getByText('CDC Realtime Delta Stream')).toBeVisible();

    // Verify Terminal viewport and live WebSocket frame received from server
    await expect(page.getByText(/frames/i)).toBeVisible();
    await expect(page.getByText('connected', { exact: true })).toBeVisible();

    // Test Pause / Resume toggle button
    const pauseBtn = page.getByRole('button', { name: /Pause/i });
    await expect(pauseBtn).toBeVisible();
    await pauseBtn.click();

    const resumeBtn = page.getByRole('button', { name: /Resume/i });
    await expect(resumeBtn).toBeVisible();
    await resumeBtn.click();

    // Test Clear button
    const clearBtn = page.getByTitle('Clear stream log');
    await expect(clearBtn).toBeVisible();
    await clearBtn.click();
  });
});
