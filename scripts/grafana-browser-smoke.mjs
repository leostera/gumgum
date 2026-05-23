#!/usr/bin/env node
import process from 'node:process';

const enabled = process.env.GUMGUM_BROWSER_SMOKE === '1';
const url = process.env.GUMGUM_GRAFANA_URL || '';
const user = process.env.GUMGUM_GRAFANA_USER || 'gumgum';
const password = process.env.GUMGUM_GRAFANA_PASSWORD || 'gumgum-local-dev';
const dashboard = process.env.GUMGUM_GRAFANA_DASHBOARD_QUERY || 'API Overview';

if (!enabled || !url) {
  console.log(`skip: set GUMGUM_BROWSER_SMOKE=1 and GUMGUM_GRAFANA_URL=https://grafana.<domain> to run browser smoke
optional env:
  GUMGUM_GRAFANA_USER=gumgum
  GUMGUM_GRAFANA_PASSWORD=...
  GUMGUM_GRAFANA_DASHBOARD_QUERY='API Overview'
  GUMGUM_BROWSER_HEADLESS=0
`);
  process.exit(0);
}

let chromium;
try {
  ({ chromium } = await import('playwright'));
} catch (error) {
  console.log('skip: playwright is not installed; run `npm install --no-save playwright` or use an environment with Playwright available');
  process.exit(0);
}

const browser = await chromium.launch({ headless: process.env.GUMGUM_BROWSER_HEADLESS !== '0' });
const page = await browser.newPage({ ignoreHTTPSErrors: true });
try {
  await page.goto(`${url.replace(/\/$/, '')}/login`, { waitUntil: 'domcontentloaded', timeout: 30_000 });
  await page.getByLabel(/email or username|username/i).fill(user);
  await page.getByLabel(/password/i).fill(password);
  await page.getByRole('button', { name: /log in/i }).click();
  await page.waitForLoadState('networkidle', { timeout: 30_000 });

  await page.goto(`${url.replace(/\/$/, '')}/api/search?query=${encodeURIComponent(dashboard)}`, {
    waitUntil: 'domcontentloaded',
    timeout: 30_000,
  });
  const body = await page.textContent('body');
  const results = JSON.parse(body || '[]');
  const match = results.find((item) => item.title === dashboard);
  if (!match?.url) {
    throw new Error(`dashboard ${dashboard} not found in Grafana search`);
  }

  await page.goto(`${url.replace(/\/$/, '')}${match.url}`, { waitUntil: 'networkidle', timeout: 60_000 });
  await page.getByText(dashboard, { exact: false }).waitFor({ timeout: 30_000 });
  await page.getByText(/Visits Total/i).waitFor({ timeout: 30_000 });
  console.log(`ok: Grafana browser smoke rendered ${dashboard}`);
} finally {
  await browser.close();
}
