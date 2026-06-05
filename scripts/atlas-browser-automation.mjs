import fs from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { chromium } from 'playwright';

const TARGETS = {
  baidu: keyword => `https://www.baidu.com/s?wd=${encodeURIComponent(keyword)}`,
  bilibili: keyword => `https://search.bilibili.com/all?keyword=${encodeURIComponent(keyword)}`,
  douyin: keyword => `https://www.douyin.com/search/${encodeURIComponent(keyword)}`,
  kuaishou: keyword => `https://www.kuaishou.com/search/video?searchKey=${encodeURIComponent(keyword)}`,
  zhihu: keyword => `https://www.zhihu.com/search?type=content&q=${encodeURIComponent(keyword)}`,
  xiaohongshu: keyword => `https://www.xiaohongshu.com/search_result?keyword=${encodeURIComponent(keyword)}`,
  weibo: keyword => `https://s.weibo.com/weibo?q=${encodeURIComponent(keyword)}`,
};

const WRITE_ACTIONS = new Set(['click', 'type', 'press', 'submit']);

function fail(message) {
  console.log(JSON.stringify({ ok: false, error: message }));
  process.exit(0);
}

function parseRequest() {
  const raw = process.argv[2];
  if (!raw) fail('missing request');
  try {
    return JSON.parse(Buffer.from(raw, 'base64url').toString('utf8'));
  } catch (error) {
    fail(`invalid request: ${error.message}`);
  }
}

function searchUrl(target, keyword) {
  const resolver = TARGETS[target];
  if (!resolver) fail(`unsupported target: ${target}`);
  return resolver(keyword || 'AI 助手');
}

async function collectDomSummary(page) {
  try {
    return await page.evaluate(() => {
      const elements = Array.from(document.querySelectorAll('*'));
      const interactiveSelector = [
        'a[href]',
        'button',
        'input',
        'textarea',
        'select',
        '[role="button"]',
        '[role="link"]',
        '[contenteditable="true"]',
        '[tabindex]:not([tabindex="-1"])',
      ].join(',');
      const text = (document.body?.innerText || '').replace(/\s+/g, ' ').trim();
      const skeletonSignals = elements.filter(el => {
        const className = String(el.className || '').toLowerCase();
        const ariaBusy = el.getAttribute('aria-busy') === 'true';
        return ariaBusy || className.includes('skeleton') || className.includes('loading') || className.includes('placeholder');
      }).length;
      return {
        totalElements: elements.length,
        interactiveElements: document.querySelectorAll(interactiveSelector).length,
        links: document.querySelectorAll('a[href]').length,
        iframes: document.querySelectorAll('iframe').length,
        shadowRoots: elements.filter(el => el.shadowRoot).length,
        images: document.querySelectorAll('img, picture, svg').length,
        textChars: text.length,
        emptyPage: elements.length < 3 || text.length < 12,
        skeletonLike: skeletonSignals >= 3 && text.length < 120,
        truncatedElements: Math.max(0, elements.length - 2000),
      };
    });
  } catch {
    return null;
  }
}

async function main() {
  const request = parseRequest();
  const action = request.action || 'search';
  if (WRITE_ACTIONS.has(action) && !request.confirmed) {
    fail('write-like browser action requires confirmation');
  }

  const profileDir = request.profileDir || path.join(os.homedir(), '.atlas', 'browser-profile');
  await fs.mkdir(profileDir, { recursive: true });
  const launchOptions = {
    headless: request.headless ?? true,
    viewport: { width: 1365, height: 768 },
  };
  let context;
  try {
    context = await chromium.launchPersistentContext(profileDir, {
      ...launchOptions,
      channel: request.channel || 'chrome',
    });
  } catch (error) {
    if (request.channel) throw error;
    context = await chromium.launchPersistentContext(profileDir, launchOptions);
  }
  const page = context.pages()[0] || await context.newPage();
  let url = request.url;
  if (action === 'search') {
    url = searchUrl(request.target || 'baidu', request.keyword || 'AI 助手');
  }
  if (url) {
    await page.goto(url, { waitUntil: 'domcontentloaded', timeout: 30000 });
  }

  if (action === 'click') {
    await page.locator(request.selector).first().click({ timeout: 10000 });
  } else if (action === 'type') {
    await page.locator(request.selector).first().fill(request.text || '', { timeout: 10000 });
  } else if (action === 'press') {
    await page.keyboard.press(request.key || 'Enter');
  }

  await page.waitForTimeout(Number(request.settleMs || 800));
  const title = await page.title().catch(() => '');
  const finalUrl = page.url();
  const domSummary = await collectDomSummary(page);
  let screenshotPath = null;
  if (request.screenshot !== false) {
    const outDir = request.outDir || path.join(os.homedir(), '.atlas', 'browser-screenshots');
    await fs.mkdir(outDir, { recursive: true });
    screenshotPath = path.join(outDir, `atlas-browser-${Date.now()}.png`);
    await page.screenshot({ path: screenshotPath, fullPage: false });
  }
  await context.close();
  console.log(JSON.stringify({
    ok: true,
    action,
    target: request.target || null,
    title,
    url: finalUrl,
    screenshotPath,
    domSummary,
  }));
}

main().catch(error => fail(error.message));
