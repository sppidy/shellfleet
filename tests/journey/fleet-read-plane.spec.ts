import { execFileSync } from 'node:child_process';
import path from 'node:path';
import { expect, test } from '@playwright/test';

const composeFile = path.resolve(process.cwd(), '../tests/journey/docker-compose.yml');

function compose(...args: string[]) {
  execFileSync('docker', ['compose', '-f', composeFile, ...args], { stdio: 'inherit' });
}

test.afterAll(() => {
  if (process.env.SHELLFLEET_JOURNEY_KEEP_STACK === '1') return;
  try {
    compose('down', '-v', '--remove-orphans');
  } catch {
    // A failed cleanup must not hide the journey assertion that caused it.
  }
});

test('Fleet remains durable across reload, disconnect, SSE loss, and reconnect', async ({ page }) => {
  await page.goto('/overview');
  const hostRow = page.getByRole('row').filter({ hasText: 'journey-agent' }).first();

  await expect(hostRow).toContainText('online', { timeout: 30_000 });
  await expect(page.getByText('1 reporting')).toBeVisible({ timeout: 30_000 });

  const fleetShell = page.getByRole('region', { name: 'Fleet Shell' });
  const shellInput = fleetShell.getByRole('combobox', { name: 'Fleet Shell command' });
  await expect(fleetShell).toBeVisible();
  await shellInput.fill('stats');
  await shellInput.press('Enter');
  await expect(fleetShell.getByText(/FLEET 1\/1 online/)).toBeVisible();
  await shellInput.fill('use journey-agent');
  await shellInput.press('Enter');
  await expect(fleetShell.getByText('context set to journey-agent')).toBeVisible();
  await expect(fleetShell.getByText('[journey-agent] $')).toBeVisible();

  await page.reload();
  await expect(hostRow).toContainText('online');
  const initialFleet = await page.request.get('/api/core/v1/fleet');
  expect(initialFleet.ok()).toBeTruthy();
  const initialPayload = await initialFleet.json();
  expect(initialPayload.hosts).toHaveLength(1);
  expect(initialPayload.hosts[0].system).not.toBeNull();
  expect(initialPayload.hosts[0].system.value.payload.cpu_count).toBeGreaterThan(0);

  compose('stop', 'agent');
  await expect(hostRow).toContainText('offline', { timeout: 50_000 });
  await expect(hostRow).toContainText('journey-agent');
  await expect(hostRow.getByRole('cell').nth(2)).not.toHaveText('—');
  await shellInput.fill('stats journey-agent');
  await shellInput.press('Enter');
  await expect(fleetShell.getByText(/HOST journey-agent\s+OFFLINE/)).toBeVisible();

  await page.route('**/api/core/v1/events', (route) => route.abort('failed'));
  await page.reload();
  await expect(hostRow).toContainText('offline');
  await expect(page.getByText(/live updates disconnected/i)).toBeVisible();

  await page.unroute('**/api/core/v1/events');
  compose('start', 'agent');
  await expect(hostRow).toContainText('online', { timeout: 15_000 });

  const finalFleet = await page.request.get('/api/core/v1/fleet');
  expect(finalFleet.ok()).toBeTruthy();
  const finalPayload = await finalFleet.json();
  expect(finalPayload.hosts).toHaveLength(1);
  expect(finalPayload.hosts[0].agent_id).toBe('journey-agent-id');

  // Android browsers can expose a desktop-class CSS viewport near 930px.
  // Keep navigation off-canvas and stack the selected-host panes throughout
  // that compact range instead of squeezing two dashboards side-by-side.
  await page.setViewportSize({ width: 930, height: 1_800 });
  await page.goto('/?agent=journey-agent');
  await expect(page.getByRole('button', { name: 'Open menu' })).toBeVisible();
  await expect(page.locator('.h-splitter')).toHaveCSS('flex-direction', 'column');
  const sidebarBox = await page.locator('.sidebar').boundingBox();
  expect(sidebarBox).not.toBeNull();
  expect((sidebarBox?.x ?? 0) + (sidebarBox?.width ?? 0)).toBeLessThanOrEqual(1);

  await page.setViewportSize({ width: 1_440, height: 900 });
  await expect(page.getByRole('button', { name: 'Open menu' })).toBeHidden();
  await expect(page.locator('.h-splitter')).toHaveCSS('flex-direction', 'row');
});
