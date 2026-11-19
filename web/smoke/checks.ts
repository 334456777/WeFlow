// What the smoke test (#789) does on the built page, over the synthetic account of
// crates/weflow-cli/examples/dev_account.rs: the pages and the interactions Preact may render otherwise than React
// (#787), checked by the text shown and the number of elements, never by pixels. Each check starts with the page just
// opened in a tab of its own; the runner fails it too when the page logs an error meanwhile.
import type { Page } from './cdp.ts';
import { HIDDEN_MS } from '../src/api/presence.ts';

export interface Check {
  name: string;
  /** On a phone's screen (390 × 844) rather than a desktop's. */
  phone?: boolean;
  run(page: Page, browsing: Browsing): Promise<void>;
}

/** What a check may do beyond its own tab. */
export interface Browsing {
  /** Where the page is opened: `/ui/` with the token, as `weflow serve` prints it. */
  address: string;
  /** The page in another tab of the same browser, brought to the front and connected; closed with the check's tab. */
  open(): Promise<Page>;
}

/** The number of elements `selector` matches, waited for until `ok` takes it. */
function count(page: Page, what: string, selector: string, ok: (n: number) => boolean): Promise<number> {
  return page.until(what, ok, selector => document.querySelectorAll(selector).length, selector);
}

/** Waits until none of `tabs`, all in the background, holds an event stream any more (presence.ts); fails when one
 * still does well after `HIDDEN_MS`, saying which. */
async function released(tabs: Page[]): Promise<void> {
  const deadline = Date.now() + HIDDEN_MS + 20_000;
  while (tabs.some(tab => tab.streams().length)) {
    if (Date.now() > deadline) {
      const held = tabs.flatMap((tab, i) => tab.streams().length ? [`tab ${i + 1}: ${tab.streams().join(', ')}`] : []);
      throw new Error(`tabs in the background still hold their event streams ${HIDDEN_MS / 1000 + 20} s on: ${held.join('; ')}`);
    }
    await Bun.sleep(200);
  }
}

/** The labels of the rows of the list on the left (conversations, posters, merchants), in order. */
function rows(page: Page, what: string, ok: (labels: string[]) => boolean): Promise<(string | null)[]> {
  return page.until(what, labels => ok(labels as string[]),
    () => [...document.querySelectorAll('.conversation-list .conversation-row')].map(row => row.getAttribute('aria-label')));
}

/** Waits for the conversation list of the records, read from the server. */
const conversations = (page: Page) => rows(page, 'the conversations', labels => labels.includes('选择林沐'));

const same = (a: unknown[], b: unknown[]) => a.length === b.length && a.every((value, i) => value === b[i]);

async function navigate(page: Page, label: string, panel: string): Promise<void> {
  await page.click('.sidebar .nav-item', label);
  await page.waitFor(`the ${panel} panel`, panel => document.querySelector(`.main-area > .panel[aria-label="${panel}"]`) !== null, panel);
}

/** The picture of 「图片测试」 whose message is `id` (dev_account.rs: ① is 2, ⑧ is 16), opened in the preview. */
export const picture = (id: number) => `.message-row:has(img.message-thumb[src*="local-id=${id}&"]) button[aria-label="预览图片"]`;

export const checks: Check[] = [
  {
    name: 'records: the conversation list, a conversation, older messages on scrolling up',
    async run(page) {
      await rows(page, 'the conversations', labels => ['选择图片测试', '选择周末爬山群', '选择林沐', '选择微信支付'].every(l => labels.includes(l)));
      // the newest conversation is open: a note and a picture per case, the thumbnails decoded but ⑨'s, which has no file
      await count(page, 'the messages of 图片测试', '[aria-label="图片测试的记录"] .message-row', n => n === 18);
      await page.until('8 thumbnails decoded', n => n === 8,
        () => [...document.querySelectorAll<HTMLImageElement>('img.message-thumb')].filter(img => img.complete && img.naturalWidth > 0).length);

      // the group: its latest 50 messages, then older ones a page at a time up to the first of its 240
      await page.click('.conversation-row', '周末爬山群');
      const pane = '[aria-label="周末爬山群的记录"]';
      await count(page, 'the latest page of the group', `${pane} .message-row`, n => n === 50);
      const deadline = Date.now() + 20_000;
      while (await page.eval(pane => document.querySelectorAll(`${pane} .message-row`).length, pane) < 240) {
        if (Date.now() > deadline) throw new Error('scrolling up did not load the 240 messages of the group');
        await page.wheel(pane, -3000);
        await Bun.sleep(100);
      }
      await page.untilTexts('the start of the group', `${pane} .history-edge`, texts => texts[0] === '没有更早的消息了');
      const first = await page.texts(`${pane} .message-row`);
      if (!first[0]) throw new Error('the first message of the group is empty');
    },
  },
  {
    name: 'conversation search: by name and by kind',
    async run(page) {
      await rows(page, 'the conversations', labels => labels.length >= 10);
      await page.click('input[aria-label="搜索会话"]');
      await page.type('林');
      await rows(page, 'only 林沐', labels => same(labels, ['选择林沐']));
      await page.clear('input[aria-label="搜索会话"]');
      await rows(page, 'every conversation again', labels => labels.length >= 10);
      await page.click('.session-filter button', '群聊');
      await rows(page, 'only the group', labels => same(labels, ['选择周末爬山群']));
      await page.click('.session-filter button', '公众号');
      await rows(page, 'only the WeChat Pay account', labels => same(labels, ['选择微信支付']));
      await page.click('.session-filter button', '全部');
      await rows(page, 'every conversation again', labels => labels.length >= 10);
    },
  },
  {
    name: 'sidebar: folded and unfolded, its links working folded',
    async run(page) {
      const state = () => ({
        folded: document.querySelector('main.workspace')!.classList.contains('sidebar-collapsed'),
        label: document.querySelector('.sidebar-toggle')!.getAttribute('aria-label'),
        width: Math.round(document.querySelector('.sidebar')!.getBoundingClientRect().width),
      });
      await page.click('.sidebar-toggle');
      await page.until('the sidebar folded', s => s.folded && s.label === '展开侧边栏' && s.width < 100, state);
      await navigate(page, '朋友圈', '朋友圈浏览');
      await page.click('.sidebar-toggle');
      await page.until('the sidebar unfolded', s => !s.folded && s.label === '收起侧边栏' && s.width > 150, state);
    },
  },
  {
    name: 'settings: the colour theme, light, dark and the system’s',
    async run(page) {
      await navigate(page, '设置', '设置');
      await page.click('.settings-nav-row', '外观与浏览');
      const theme = () => {
        const [r, g, b] = getComputedStyle(document.documentElement).backgroundColor.match(/\d+/g)!.map(Number);
        return {
          theme: document.documentElement.dataset.theme,
          pressed: document.querySelector('.theme-group button[aria-pressed="true"]')?.textContent,
          light: (r + g + b) / 3 > 128,
          saved: localStorage.getItem('weflow.theme'),
        };
      };
      await page.click('.theme-group button', '深色');
      await page.until('the dark theme', t => t.theme === 'dark' && t.pressed === '深色' && !t.light && t.saved === 'dark', theme);
      await page.click('.theme-group button', '浅色');
      await page.until('the light theme', t => t.theme === 'light' && t.pressed === '浅色' && t.light && t.saved === 'light', theme);
      await page.click('.theme-group button', '跟随系统');
      await page.until('the system’s theme (light)', t => t.theme === 'light' && t.pressed === '跟随系统' && t.saved === null, theme);
      // the system turning dark, which the page hears of
      await page.send('Emulation.setEmulatedMedia', { features: [{ name: 'prefers-color-scheme', value: 'dark' }] });
      await page.until('the system’s theme (dark)', t => t.theme === 'dark' && !t.light, theme);
    },
  },
  {
    // index.html hides the page while a skin other than the default is on its way; skin.ts shows it, here at once
    name: 'a skin kept that this page does not have: the default skin, the page shown before the first paint gives up (#830)',
    async run(page, browsing) {
      await page.eval(() => localStorage.setItem('weflow.skin', 'gone'));
      try {
        await page.goto(browsing.address);
        await page.until('the default skin, shown within 2.5 s', s => s.skin === 'neumorphism' && !s.pending && s.shown && s.at < 2500, () => ({
          skin: document.documentElement.dataset.skin,
          pending: 'skinPending' in document.documentElement.dataset,
          shown: getComputedStyle(document.body).visibility === 'visible',
          at: performance.now(),
        }));
        await conversations(page);
        // and while a skin's sheet is on its way, nothing shows
        const hidden = await page.eval(() => {
          document.documentElement.dataset.skinPending = '';
          const visibility = getComputedStyle(document.body).visibility;
          delete document.documentElement.dataset.skinPending;
          return visibility;
        });
        if (hidden !== 'hidden') throw new Error('the page shows while a skin is pending: ' + hidden);
      } finally {
        await page.eval(() => localStorage.removeItem('weflow.skin'));
      }
    },
  },
  {
    // no skin has shapes yet: the page is given a skin's attribute and tokens of its own (styles/roles.css)
    name: 'shapes: a button drawn by its layer, clipped there and not itself, its hover too; chips turned each its own way (#831)',
    async run(page) {
      await conversations(page);
      await page.eval(() => {
        document.documentElement.dataset.shapes = '';
        const style = document.createElement('style');
        style.id = 'shapes-check';
        style.textContent = ':root { --button-clip: polygon(0 0, 100% 4px, 100% 100%, 0 calc(100% - 4px)); '
          + '--button-drop: drop-shadow(0 2px 2px rgb(0 0 0 / .3)); --option-tilt: 3deg; --row-hover-track: none; }';
        document.head.append(style);
      });
      try {
        const look = () => {
          const shadow = (name: string) => {
            const probe = document.body.appendChild(document.createElement('div'));
            probe.style.boxShadow = `var(${name})`;
            const value = getComputedStyle(probe).boxShadow;
            probe.remove();
            return value;
          };
          const button = document.querySelector('.primary-button')!;
          const own = getComputedStyle(button), layer = getComputedStyle(button, '::before');
          return {
            own: [own.position, own.isolation, own.clipPath, own.backgroundColor, own.boxShadow, own.filter.startsWith('drop-shadow')].join(' '),
            layer: [layer.content, layer.clipPath.startsWith('polygon'), layer.backgroundColor !== 'rgba(0, 0, 0, 0)'].join(' '),
            lift: layer.boxShadow === shadow('--button-shadow') ? 'rest' : layer.boxShadow === shadow('--button-shadow-hover') ? 'hover' : layer.boxShadow,
            // the export's formats, side by side
            turns: [...document.querySelector('.chip')!.parentElement!.querySelectorAll(':scope > .chip')].map(chip => parseFloat(getComputedStyle(chip).rotate)),
            hollow: getComputedStyle(document.querySelector('.hover-hollow')!).display,
          };
        };
        // the button's own shadow fades out as the layer takes it
        const drawn = await page.until('the button drawn by its layer, not itself', s => s.lift === 'rest'
          && s.own === 'relative isolate none rgba(0, 0, 0, 0) none true' && s.layer === '"" true true', look);
        if (drawn.turns.length < 5 || drawn.turns.some(turn => !(Math.abs(turn) <= 3)) || new Set(drawn.turns).size < drawn.turns.length) {
          throw new Error('the chips’ turns: ' + drawn.turns.join(', '));
        }
        if (drawn.hollow !== 'none') throw new Error('the hover hollow of a skin without one: ' + drawn.hollow);
        await page.hover('.primary-button');
        await page.until('the button’s hover on its layer', s => s.lift === 'hover', look);
      } finally {
        await page.eval(() => {
          delete document.documentElement.dataset.shapes;
          document.getElementById('shapes-check')?.remove();
        });
      }
    },
  },
  {
    // no skin decorates yet: the page is given a skin's tokens of its own (Backdrop.tsx, motion.ts pageIn)
    name: 'decoration: under the page and out of the way, another at each page turn, the skin’s page-in; still with reduced motion (#832)',
    async run(page) {
      await conversations(page);
      // none for neumorphism
      const none = await page.eval(() => document.querySelectorAll('.backdrop').length);
      if (none) throw new Error(`${none} decoration layers under neumorphism`);
      await page.eval(() => {
        const style = document.createElement('style');
        style.id = 'backdrop-check';
        style.textContent = ':root { --backdrop-count: 2; --page-in: probe-in 400ms linear; } @keyframes probe-in { from { opacity: .5; } }';
        document.head.append(style);
      });
      const state = () => {
        const layer = document.querySelector<HTMLElement>('.backdrop');
        const look = layer && getComputedStyle(layer);
        const panel = document.querySelector('.main-area > .panel')!;
        return {
          layer: layer && look ? [layer.getAttribute('aria-hidden'), look.position, look.zIndex, look.pointerEvents].join(' ') : 'none',
          turn: layer?.dataset.turn,
          playing: panel.getAnimations().some(animation => (animation as CSSAnimation).animationName === 'probe-in'),
          // given back to the style sheets once over
          own: (panel as HTMLElement).style.animation,
          page: panel.getAttribute('aria-label'),
        };
      };
      try {
        // the layer comes at the next turn, the second decoration up
        await page.click('.sidebar .nav-item', '朋友圈');
        const shown = await page.until('the next decoration, the page brought in by the skin', s => s.page === '朋友圈浏览' && s.turn === '1' && s.playing, state);
        if (shown.layer !== 'true fixed -1 none') throw new Error('the decoration: ' + shown.layer);
        await page.until('the page in, its animation given back', s => !s.playing && s.own === '', state);
        await page.click('.sidebar .nav-item', '微信支付');
        await page.until('the decoration after it, the first again, the page in', s => s.page === '微信支付浏览' && s.turn === '0' && !s.playing, state);
        // reduced motion: no animation, and the decoration stays
        await page.send('Emulation.setEmulatedMedia', { features: [{ name: 'prefers-reduced-motion', value: 'reduce' }] });
        await page.click('.sidebar .nav-item', '聊天');
        const still = await page.until('the records again', s => Boolean(s.page?.endsWith('记录浏览')), state);
        if (still.turn !== '0' || still.playing) throw new Error('with reduced motion: ' + JSON.stringify(still));
      } finally {
        await page.eval(() => document.getElementById('backdrop-check')?.remove());
      }
    },
  },
  {
    name: 'picture preview: opened, the one before, closed with Escape and the close button, opened as it flies back',
    async run(page) {
      await count(page, 'the messages of 图片测试', '[aria-label="图片测试的记录"] .message-row', n => n === 18);
      const preview = () => {
        const dialog = document.querySelector<HTMLDialogElement>('dialog.preview-dialog')!;
        const img = dialog.querySelector<HTMLImageElement>('.preview-stage img');
        return {
          open: dialog.open,
          place: dialog.querySelector('header .num')?.textContent?.trim() ?? null,
          decoded: Boolean(img?.complete && img.naturalWidth > 0),
          src: img?.getAttribute('src') ?? '',
        };
      };
      await page.click(picture(16));
      await page.until('⑧ in the preview', p => p.open && p.place === '8 / 9' && p.decoded && p.src.includes('local-id=16&'), preview);
      await page.click('dialog.preview-dialog button[aria-label="上一个"]');
      await page.until('⑦ in the preview', p => p.open && p.place === '7 / 9' && p.decoded && p.src.includes('local-id=14&'), preview);
      await page.press('Escape');
      await page.until('the preview closed', p => !p.open, preview);
      await page.click(picture(16));
      await page.until('⑧ in the preview again', p => p.open && p.place === '8 / 9' && p.decoded, preview);
      // ⑧ flies back into its thumbnail; a click on the thumbnail meanwhile opens it again (#844)
      const thumb = await page.eval(selector => {
        const box = document.querySelector(selector)!.getBoundingClientRect();
        return { x: box.x + box.width / 2, y: box.y + box.height / 2 };
      }, picture(16));
      await page.click('dialog.preview-dialog button[aria-label="关闭预览"]');
      await page.until('⑧ flying back', p => !p.open, preview);
      await page.clickAt(thumb);
      await page.until('⑧ opened as it flew back', p => p.open && p.place === '8 / 9' && p.decoded, preview);
      await page.click('dialog.preview-dialog button[aria-label="关闭预览"]');
      await page.until('the preview closed', p => !p.open, preview);
    },
  },
  {
    name: 'Moments: the timeline, filtered by poster',
    async run(page) {
      await navigate(page, '朋友圈', '朋友圈浏览');
      const posts = (what: string, ok: (texts: string[]) => boolean) => page.untilTexts(what, 'article.moment .moment-text', ok);
      await posts('the three posts', texts => same(texts, ['山顶的风很大，但是值得', '周末的书店', '新学的菜，第一次就成功了']));
      await rows(page, 'the posters', labels => same(labels, ['全部朋友圈，3 条', '只看陈一苇，1 条', '只看林沐，1 条', '只看我，1 条']));
      await page.click('.conversation-row', '只看林沐');
      await posts('林沐’s post', texts => same(texts, ['山顶的风很大，但是值得']));
      await page.click('.conversation-row', '全部朋友圈');
      await posts('the three posts again', texts => texts.length === 3);
    },
  },
  {
    name: 'WeChat Pay: the merchants, the payments and their total',
    async run(page) {
      await navigate(page, '微信支付', '微信支付浏览');
      await rows(page, 'the merchants', labels => same(labels, ['全部商户，6 笔', '只看山野户外用品，2 笔', '只看周末书店，1 笔', '只看楼下面馆，1 笔', '只看街角便利店，1 笔', '只看城市地铁，1 笔']));
      const payments = (what: string, ok: (labels: string[]) => boolean) => page.until(what, labels => ok(labels as string[]),
        () => [...document.querySelectorAll('.payments .pay-card')].map(card => card.getAttribute('aria-label')));
      await payments('the six notices', labels => labels.length === 6 && labels[0] === '楼下面馆，28.00 元');
      await page.untilTexts('the refund', '.payments .pay-amount.refund', texts => same(texts, ['+89.00']));
      await page.untilTexts('the total of six', '#card-pay-summary .card-summary', texts => same(texts, ['6 笔']));
      await page.click('.conversation-row', '山野户外用品');
      await payments('the two of 山野户外用品', labels => labels.length === 2 && labels.every(l => l.startsWith('山野户外用品')));
      await page.untilTexts('the total of two', '#card-pay-summary .card-summary', texts => same(texts, ['2 笔']));
    },
  },
  {
    name: 'calendar popover: opened, another month, a day picked, closed with Escape',
    async run(page) {
      const start = '#card-body-export .date-field[aria-label^="开始日期"]';
      await page.click(start);
      const title = () => document.querySelector('.calendar-popover .calendar-title')?.textContent ?? null;
      const shown = await page.waitFor('the calendar', title);
      await count(page, 'today in the calendar', '.calendar-popover td.calendar-day.today', n => n === 1);
      await page.click('.calendar-popover button[aria-label="上一个月"]');
      await page.until('the month before', t => t !== null && t !== shown, title);
      await page.click('.calendar-popover td.calendar-day:not([aria-disabled="true"])');
      await page.until('the start date picked', (label: string | null) => label !== null && !label.includes('未选择'),
        start => document.querySelector(start)?.getAttribute('aria-label') ?? null, start);
      if (await page.eval(() => document.querySelector('.calendar-popover') === null)) await page.click(start);
      await page.waitFor('the calendar open', () => document.querySelector('.calendar-popover') !== null);
      await page.press('Escape');
      await page.waitFor('the calendar closed', () => document.querySelector('.calendar-popover') === null);
    },
  },
  {
    name: 'console: cards opened and closed, a search in the conversation',
    async run(page) {
      await page.untilTexts('the cards', '.console .card-title', texts => same(texts, ['导出', '任务中心', '搜索', '会话资料', '媒体文件']));
      const collapsed = (id: string) => page.eval(id => document.getElementById('card-' + id)!.closest('.card')!.classList.contains('collapsed'), id);
      await page.click('#card-info');
      await page.untilTexts('the conversation’s facts', '#card-body-info .info-grid dd', texts => texts[0] === '18 条');
      if (await collapsed('info')) throw new Error('the info card did not open');
      await page.click('#card-search');
      await page.click('input[aria-label="搜索此会话的消息"]');
      await page.type('原图');
      await page.press('Enter');
      await page.untilTexts('the results', '.search-results .search-result', texts => texts.length === 5 && texts.every(t => t.includes('原图')));
      await page.untilTexts('their count', '.search-results .results-count', texts => texts[0]?.startsWith('找到 5 条记录'));
      await page.click('#card-info');
      await page.until('the info card closed', Boolean, id => document.getElementById('card-' + id)!.closest('.card')!.classList.contains('collapsed'), 'info');
    },
  },
  {
    // 周舟's chat ends with two link cards (dev_account.rs): Preact passes any address to href as it is
    name: 'link cards: a web page opens in a new tab, javascript: is no link (#818)',
    async run(page) {
      const policy = await page.eval(async () => (await fetch(location.pathname)).headers.get('content-security-policy') ?? '');
      if (!/script-src 'self' 'sha256-[A-Za-z0-9+/]+=*';/.test(policy) || !policy.includes("frame-ancestors 'none'")) {
        throw new Error('the page is sent without its Content-Security-Policy: ' + policy);
      }
      await conversations(page);
      await page.click('.conversation-row', '周舟');
      const pane = '[aria-label="周舟的记录"]';
      await count(page, 'the two link cards', `${pane} .link-bubble`, n => n === 2);
      const links = await page.eval(pane => [...document.querySelectorAll(`${pane} a`)].map(a => [a.getAttribute('href'), a.getAttribute('target')]), pane);
      if (JSON.stringify(links) !== JSON.stringify([['https://example.com/route', '_blank']])) throw new Error('the links of the chat: ' + JSON.stringify(links));
    },
  },
  {
    name: 'phone: a conversation and back, the tabs at the bottom',
    phone: true,
    async run(page) {
      const visible = (selector: string) => page.eval(selector => (document.querySelector(selector)?.getBoundingClientRect().width ?? 0) > 0, selector);
      await rows(page, 'the conversations', labels => labels.length >= 10);
      if (await visible('.sidebar')) throw new Error('the sidebar shows on a phone');
      await page.click('.conversation-row', '林沐');
      await page.waitFor('林沐’s messages in full width', () => {
        const pane = document.querySelector('[aria-label="林沐的记录"]')?.getBoundingClientRect();
        return pane && pane.width > 300 && document.querySelectorAll('[aria-label="林沐的记录"] .message-row').length > 0;
      });
      await page.click('button[aria-label="返回对话列表"]');
      await page.waitFor('the list again', () => (document.querySelector('.session-pane')?.getBoundingClientRect().width ?? 0) > 300);
      await page.click('.mobile-tab', '朋友圈');
      await count(page, 'the posts', 'article.moment', n => n === 3);
    },
  },
  {
    // Chrome opens six connections at most to a server, for all its tabs, and each page holds an event stream: one in
    // the background lets go of it after a while (src/api/presence.ts)
    name: 'tabs: eight in one browser, the newest still reads its records (#799)',
    async run(page, browsing) {
      await conversations(page);
      // five tabs hold five streams and leave a connection for the requests; a sixth opened before the first lets go
      // would wait for it, up to HIDDEN_MS (#878)
      const tabs = [page];
      for (let n = 2; n <= 5; n++) {
        const tab = await browsing.open();
        await conversations(tab);
        tabs.push(tab);
      }
      // the four behind the newest let go HIDDEN_MS after they were hidden, or later where Chrome slows down the timers
      // of a busy machine's background tabs
      await released(tabs.slice(0, -1));
      for (let n = 6; n <= 8; n++) await conversations(await browsing.open());
    },
  },
  {
    // the page left goes into the back/forward cache, where it holds no event stream
    name: 'the page left and opened again, seven times in one tab (#799)',
    async run(page, browsing) {
      await conversations(page);
      for (let n = 0; n < 7; n++) {
        await page.goto(browsing.address);
        await page.waitFor('the page to connect to weflow serve', () => document.querySelector('.connection.ready') !== null);
        await conversations(page);
      }
    },
  },
];

/** On a server whose language is English: the page in English, its English chunk loaded. */
export const englishChecks: Check[] = [
  {
    name: 'English: the navigation, a page and the console',
    async run(page) {
      await page.until('the links in English', labels => ['Chats', 'Moments', 'WeChat Pay', 'Contacts', 'Settings'].every(l => labels.includes(l)),
        () => [...document.querySelectorAll('.sidebar .nav-item')].map(item => item.getAttribute('aria-label')));
      await page.untilTexts('the cards in English', '.console .card-title', texts => texts.includes('Tasks'));
      await page.click('.sidebar .nav-item', 'Moments');
      await count(page, 'the posts', 'article.moment', n => n === 3);
      const chinese = await page.eval(() => /[一-鿿]/.test(document.querySelector('.sidebar')!.textContent ?? ''));
      if (chinese) throw new Error('the sidebar has Chinese in it');
    },
  },
];
