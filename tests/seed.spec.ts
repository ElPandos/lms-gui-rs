// spec: N/A (seed file)
// seed: tests/seed.spec.ts
//
// Seed test for LMS GUI — bootstraps the page context that Planner/Generator
// agents use when exploring and generating tests.
//
// This file is referenced by specs/*.md as the seed. It:
// 1. Navigates to the dashboard root
// 2. Verifies the app is reachable
// 3. Provides a ready-to-use `page` for agent-generated tests

import { test, expect } from '@playwright/test';

test.describe('Seed', () => {
  test('dashboard loads', async ({ page }) => {
    // Navigate to the dashboard root
    await page.goto('/');

    // Verify the app responded and rendered.
    await expect(page).toHaveTitle(/LMS Dashboard/, { timeout: 10_000 });
    await expect(page.getByRole('heading', { name: 'Available Models' })).toBeVisible();
  });
});
