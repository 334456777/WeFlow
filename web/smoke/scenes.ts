// What `bun run shots` (#827) photographs of the built page, over the synthetic account of
// crates/weflow-cli/examples/dev_account.rs: every page, the floating layers, the phone's layout, and the states of the
// shared controls (hover, held down, chosen) cut out with the shadow around them. Each scene starts with the page just
// opened in a tab of its own and is shot in the light and the dark theme.
import type { Page } from './cdp.ts';
import { picture } from './checks.ts';

/** Photographs the viewport, or the element `around` matches with `margin` CSS pixels of what surrounds it; `label`
 * tells the shots of one scene apart. The page is let settle first (shots.ts). */
export type Shoot = (label?: string, around?: { selector: string; text?: string; margin?: number }) => Promise<void>;

export interface Scene {
  /** Part of the file name: letters, digits and dashes. */
  name: string;
  /** On a phone's screen (390 × 844) rather than a desktop's. */
  phone?: boolean;
  /** Shot with `prefers-contrast: more` too. */
  contrast?: boolean;
  run(page: Page, shoot: Shoot): Promise<void>;
}

/** Where the mouse rests between shots and lets go of what it held: the page's margin above the records, which reacts
 * to nothing, on a desktop and on a phone alike. */
export const REST = { x: 194, y: 6 };

const messages = (page: Page, chat: string, ok: (n: number) => boolean) => page.until(`the messages of ${chat}`, ok,
  chat => document.querySelectorAll(`[aria-label="${chat}的记录"] .message-row`).length, chat);

/** The newest conversation, 「图片测试」, open with its 18 messages and every thumbnail but ⑨'s decoded. */
async function pictures(page: Page): Promise<void> {
  await messages(page, '图片测试', n => n === 18);
  await page.until('8 thumbnails decoded', n => n === 8,
    () => [...document.querySelectorAll<HTMLImageElement>('img.message-thumb')].filter(img => img.complete && img.naturalWidth > 0).length);
}

async function chat(page: Page, name: string, ok: (n: number) => boolean): Promise<void> {
  await pictures(page);
  await page.click('.conversation-list .conversation-row', name);
  await messages(page, name, ok);
}

/** Another page of the sidebar, once its link is the chosen one. */
async function go(page: Page, label: string): Promise<void> {
  await page.click('.sidebar .nav-item', label);
  await page.waitFor(`${label} chosen`, label => document.querySelector('.sidebar .nav-item.selected')?.getAttribute('aria-label') === label, label);
}

async function settings(page: Page, section: string): Promise<void> {
  await go(page, '设置');
  await page.click('.settings-nav-row', section);
  await page.waitFor(`the section ${section}`, section => document.querySelector('.settings-nav-row.active')?.textContent?.includes(section), section);
}

/** The element hovered, held down and let go away from it (no click), shot each time. */
async function states(page: Page, shoot: Shoot, name: string, selector: string, text?: string, margin = 24): Promise<void> {
  const around = { selector, text, margin };
  await page.hover(selector, text);
  await shoot(name + '-hover', around);
  await page.hold(selector, text);
  await shoot(name + '-held', around);
  await page.letGo(REST);
  await shoot(name + '-rest', around);
}

const whole = (name: string, label: string, ready?: (page: Page) => Promise<unknown>): Scene => ({
  name,
  async run(page, shoot) {
    await go(page, label);
    await ready?.(page);
    await shoot();
  },
});

const section = (name: string, label: string): Scene => ({
  name: 'settings-' + name,
  async run(page, shoot) {
    await settings(page, label);
    await shoot();
  },
});

export const scenes: Scene[] = [
  { name: 'chat-pictures', contrast: true, async run(page, shoot) { await pictures(page); await shoot(); } },
  { name: 'chat-group', async run(page, shoot) { await chat(page, '周末爬山群', n => n === 50); await shoot(); } },
  { name: 'chat-private', async run(page, shoot) { await chat(page, '林沐', n => n > 0); await shoot(); } },
  { name: 'chat-links', async run(page, shoot) { await chat(page, '周舟', n => n > 0); await shoot(); } },
  {
    name: 'chat-official',
    async run(page, shoot) {
      await pictures(page);
      await page.click('.conversation-list .conversation-row', '微信支付');
      await page.waitFor('the pushes of 微信支付', () => document.querySelector('.biz-feed .biz-head') !== null);
      await shoot();
    },
  },
  {
    name: 'chat-console',
    async run(page, shoot) {
      await pictures(page);
      await page.click('#card-info');
      await page.untilTexts('the conversation’s facts', '#card-body-info .info-grid dd', texts => texts[0] === '18 条');
      await page.click('#card-search');
      await page.click('input[aria-label="搜索此会话的消息"]');
      await page.type('原图');
      await page.press('Enter');
      await page.untilTexts('the results', '.search-results .search-result', texts => texts.length === 5);
      // the console at its top: where the clicks left it scrolled depends on when the cards finished opening
      await page.eval(() => { document.querySelector('.console')!.scrollTop = 0; });
      await shoot();
    },
  },
  {
    name: 'sidebar-folded',
    async run(page, shoot) {
      await pictures(page);
      await page.click('.sidebar-toggle');
      await page.waitFor('the sidebar folded', () => document.querySelector('main.workspace')!.classList.contains('sidebar-collapsed'));
      await shoot();
    },
  },
  {
    name: 'preview',
    async run(page, shoot) {
      await pictures(page);
      await page.click(picture(16));
      await page.waitFor('⑧ in the preview', () => {
        const img = document.querySelector<HTMLImageElement>('dialog.preview-dialog[open] .preview-stage img');
        return img?.complete && img.naturalWidth > 0;
      });
      await shoot();
    },
  },
  {
    name: 'calendar',
    async run(page, shoot) {
      await pictures(page);
      await page.click('#card-body-export .date-field[aria-label^="开始日期"]');
      await page.waitFor('the calendar', () => document.querySelector('.calendar-popover .calendar-title') !== null);
      await shoot();
      await states(page, shoot, 'day', '.calendar-popover td.calendar-day:not([aria-disabled]):not(.today)', undefined, 12);
      await states(page, shoot, 'today', '.calendar-popover td.calendar-day.today', undefined, 12);
    },
  },
  whole('moments', '朋友圈', page => page.until('the posts', n => n === 3, () => document.querySelectorAll('article.moment').length)),
  whole('payments', '微信支付', page => page.until('the notices', n => n === 6, () => document.querySelectorAll('.payments .pay-card').length)),
  whole('contacts', '通讯录'),
  whole('resources', '资源浏览'),
  whole('analytics', '聊天分析'),
  whole('reports', '年度报告'),
  whole('footprint', '我的足迹'),
  whole('insights', '灵感信箱'),
  whole('export', '导出'),
  whole('backup', '数据库备份'),
  section('connection', '连接状态'),
  section('accounts', '账号'),
  section('appearance', '外观与浏览'),
  section('serve', '启动选项'),
  section('ai', 'AI 见解'),
  section('cache', '缓存'),
  section('web', '网页版本'),
  section('console', '命令控制台'),
  {
    name: 'states-chat',
    async run(page, shoot) {
      await pictures(page);
      await states(page, shoot, 'nav', '.sidebar .nav-item', '朋友圈');
      await states(page, shoot, 'nav-chosen', '.sidebar .nav-item.selected');
      await states(page, shoot, 'row', '.conversation-list .conversation-row', '林沐', 12);
      await states(page, shoot, 'row-chosen', '.conversation-list .conversation-row.active', undefined, 12);
      await states(page, shoot, 'segment', '.session-filter button', '群聊', 12);
      await states(page, shoot, 'segment-chosen', '.session-filter button[aria-pressed="true"], .session-filter button[aria-selected="true"]', undefined, 12);
      await states(page, shoot, 'icon-button', '.sidebar-toggle');
      await states(page, shoot, 'card-header', '#card-export');
      await states(page, shoot, 'chip', '#card-body-export .chip');
      await states(page, shoot, 'date-field', '#card-body-export .date-field');
      await states(page, shoot, 'primary', '#card-body-export .primary-button');
      await page.click('#card-search');
      await states(page, shoot, 'search', '#card-body-search .search-button');
    },
  },
  {
    name: 'states-settings',
    async run(page, shoot) {
      await settings(page, '外观与浏览');
      await states(page, shoot, 'segment', '.theme-group button', '深色', 12);
      await states(page, shoot, 'toggle', '.toggle', undefined, 12);
      await states(page, shoot, 'settings-row', '.settings-nav-row', '账号', 12);
    },
  },
  {
    name: 'states-export',
    async run(page, shoot) {
      await go(page, '导出');
      await states(page, shoot, 'checkbox', '.checkbox', undefined, 12);
      await page.click('.checkbox');
      await states(page, shoot, 'checkbox-checked', '.checkbox', undefined, 12);
    },
  },
  { name: 'phone-list', phone: true, async run(page, shoot) { await page.until('the conversations', n => n >= 10, () => document.querySelectorAll('.conversation-row').length); await shoot(); } },
  {
    name: 'phone-chat',
    phone: true,
    async run(page, shoot) {
      await page.click('.conversation-row', '林沐');
      await messages(page, '林沐', n => n > 0);
      await shoot();
    },
  },
  {
    name: 'phone-sheet',
    phone: true,
    async run(page, shoot) {
      await page.click('.mobile-tab[aria-haspopup="dialog"]');
      await page.waitFor('the sheet', () => document.querySelector('dialog.nav-sheet[open]') !== null);
      await shoot();
      // again on one of its pages, that page chosen in it
      await page.click('.nav-sheet-item', '设置');
      await page.waitFor('the sheet closed', () => document.querySelector('dialog.nav-sheet') === null);
      await page.click('.mobile-tab[aria-haspopup="dialog"]');
      await page.waitFor('the sheet with 设置 chosen', () => document.querySelector('dialog.nav-sheet[open] .nav-sheet-item.selected') !== null);
      await shoot('chosen');
    },
  },
  {
    name: 'phone-moments',
    phone: true,
    async run(page, shoot) {
      await page.click('.mobile-tab', '朋友圈');
      await page.until('the posts', n => n === 3, () => document.querySelectorAll('article.moment').length);
      await shoot();
    },
  },
];
