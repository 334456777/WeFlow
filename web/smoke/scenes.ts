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

/** The element hovered, held down and let go away from it (no click), shot each time; let go at `away`, by default
 * REST, or in a dialog somewhere in it, as a click on its backdrop closes it. */
async function states(page: Page, shoot: Shoot, name: string, selector: string, text?: string, margin = 24, away = REST): Promise<void> {
  const around = { selector, text, margin };
  await page.hover(selector, text);
  await shoot(name + '-hover', around);
  await page.hold(selector, text);
  await shoot(name + '-held', around);
  await page.letGo(away);
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
      await states(page, shoot, 'title', '.calendar-popover button.calendar-title', undefined, 12);
      // the months of the year, one of them chosen
      await page.click('.calendar-popover button.calendar-title');
      await page.waitFor('the months', () => document.querySelectorAll('.calendar-popover .calendar-choice').length === 12);
      await states(page, shoot, 'month', '.calendar-popover .calendar-choice:not(.chosen):not(:disabled)', undefined, 12);
      await states(page, shoot, 'month-chosen', '.calendar-popover .calendar-choice.chosen', undefined, 12);
    },
  },
  {
    name: 'states-preview',
    async run(page, shoot) {
      await pictures(page);
      await page.click(picture(16));
      await page.waitFor('⑧ in the preview', () => {
        const img = document.querySelector<HTMLImageElement>('dialog.preview-dialog[open] .preview-stage img');
        return img?.complete && img.naturalWidth > 0;
      });
      // let go on the preview's title, which reacts to nothing
      const title = await page.point('.preview-content header');
      await states(page, shoot, 'step', '.preview-step[aria-label="上一个"]', undefined, 12, title);
      await states(page, shoot, 'original', '.original-button', undefined, 12, title);
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
  {
    // the rows and controls of the pages, which the scenes above show at rest only (#829)
    name: 'states-pages',
    async run(page, shoot) {
      await pictures(page);
      await page.click('#card-search');
      await page.click('input[aria-label="搜索此会话的消息"]');
      await page.type('原图');
      await page.press('Enter');
      await page.untilTexts('the results', '.search-results .search-result', texts => texts.length === 5);
      await states(page, shoot, 'search-result', '.search-results .search-result', undefined, 12);
      await chat(page, '周末爬山群', n => n === 50);
      await page.click('#card-info');
      await page.until('the members', n => n > 3, () => document.querySelectorAll('.member-row').length);
      // the card opens and the console moves meanwhile: the first member in view once all is still
      await page.until('the console still, the first member in view', s => !s.moving.length && s.top >= 0 && s.bottom <= s.height, () => {
        const moving = document.getAnimations()
          .filter(animation => animation.playState === 'running' && animation.effect?.getTiming().iterations !== Infinity)
          .map(animation => (animation as CSSTransition).transitionProperty ?? (animation as CSSAnimation).animationName ?? '?');
        const row = document.querySelector('.member-row')!;
        if (!moving.length) row.scrollIntoView({ block: 'center' });
        const box = row.getBoundingClientRect();
        return { moving, top: box.top, bottom: box.bottom, height: innerHeight };
      });
      await states(page, shoot, 'member', '.member-row', undefined, 12);
      await go(page, '资源浏览');
      await page.until('the tiles', n => n > 0, () => document.querySelectorAll('.resource-open').length);
      await states(page, shoot, 'resource', '.resource-open', undefined, 16);
      await states(page, shoot, 'group-picker', '.group-picker', undefined, 16);
      await settings(page, '命令控制台');
      await page.until('the commands', n => n > 3, () => document.querySelectorAll('.command-item').length);
      await states(page, shoot, 'command', '.command-item', undefined, 8);
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
      await states(page, shoot, 'tab', '.mobile-tab', '朋友圈', 8);
      await states(page, shoot, 'tab-chosen', '.mobile-tab[aria-current="page"]', undefined, 8);
      await page.click('.mobile-tab[aria-haspopup="dialog"]');
      await page.waitFor('the sheet', () => document.querySelector('dialog.nav-sheet[open]') !== null);
      await shoot();
      // let go in the gap between the first two pages of the sheet: off the dialog itself, whose click closes it
      const gap = await page.eval(() => {
        const [a, b] = [...document.querySelectorAll('.nav-sheet-item')].map(item => item.getBoundingClientRect());
        return { x: (a.right + b.left) / 2, y: a.top + a.height / 2 };
      });
      await states(page, shoot, 'sheet-item', '.nav-sheet-item', '设置', 12, gap);
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
