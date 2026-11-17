// The little of the Chrome DevTools Protocol the smoke test (#789) needs, over Bun's own WebSocket: Chrome is started
// headless with a profile of its own, one tab is opened in it and driven with real input events (the built page is
// rendered by Preact, which listens to native events), and what the page logs as an error is gathered.
import { mkdirSync, rmSync } from 'node:fs';

type Message = { id?: number; method?: string; params?: any; result?: any; error?: { message: string }; sessionId?: string };

/** Where Chrome usually is: `CHROME` first, else the stable channel's place on each system. */
export function findChrome(): string {
  const candidates = [
    process.env.CHROME,
    ...(process.platform === 'win32'
      ? [process.env.PROGRAMFILES, process.env['PROGRAMFILES(X86)'], process.env.LOCALAPPDATA]
        .filter(Boolean).map(dir => dir + '\\Google\\Chrome\\Application\\chrome.exe')
      : process.platform === 'darwin'
        ? ['/Applications/Google Chrome.app/Contents/MacOS/Google Chrome']
        : ['google-chrome', 'google-chrome-stable', 'chromium', 'chromium-browser'].map(name => Bun.which(name))),
  ];
  const found = candidates.find(path => path && Bun.file(path).size > 0);
  if (!found) throw new Error('Chrome not found: set CHROME to its executable');
  return found;
}

export class Browser {
  private next = 1;
  private readonly waiting = new Map<number, { resolve: (value: any) => void; reject: (error: Error) => void; method: string }>();
  private readonly listeners = new Set<(message: Message) => void>();

  private constructor(private readonly process: Bun.Subprocess, private readonly socket: WebSocket, private readonly profile: string) {
    socket.addEventListener('message', event => {
      const message: Message = JSON.parse(String(event.data));
      if (message.id !== undefined) {
        const call = this.waiting.get(message.id);
        this.waiting.delete(message.id);
        if (message.error) call?.reject(new Error(`${call.method}: ${message.error.message}`));
        else call?.resolve(message.result);
      } else {
        for (const listener of this.listeners) listener(message);
      }
    });
    socket.addEventListener('close', () => {
      for (const call of this.waiting.values()) call.reject(new Error(`${call.method}: Chrome went away`));
      this.waiting.clear();
    });
  }

  /** Chrome headless on a debugging port it picks, with an empty profile in `profile` (removed on close), and `flags`. */
  static async launch(chrome: string, profile: string, flags: string[] = []): Promise<Browser> {
    rmSync(profile, { recursive: true, force: true });
    mkdirSync(profile, { recursive: true });
    const process = Bun.spawn([
      chrome, '--headless=new', '--remote-debugging-port=0', `--user-data-dir=${profile}`,
      '--no-first-run', '--no-default-browser-check', '--disable-background-networking', '--disable-extensions',
      '--disable-sync', '--password-store=basic', '--use-mock-keychain', '--mute-audio', ...flags, 'about:blank',
    ], { stdin: 'ignore', stdout: 'ignore', stderr: 'pipe' });
    // Chrome writes the address of its debugging endpoint on stderr; the rest is read and dropped
    const address = await new Promise<string>((resolve, reject) => {
      const timer = setTimeout(() => reject(new Error('Chrome did not open its debugging port')), 30_000);
      (async () => {
        let text = '';
        for await (const chunk of process.stderr.pipeThrough(new TextDecoderStream())) {
          text += chunk;
          const found = /DevTools listening on (ws:\/\/\S+)/.exec(text);
          if (found) {
            clearTimeout(timer);
            resolve(found[1]);
            text = '';
          }
        }
      })().catch(reject);
      process.exited.then(code => reject(new Error(`Chrome stopped (${code}) before its debugging port opened`)));
    });
    const socket = new WebSocket(address);
    await new Promise((resolve, reject) => {
      socket.addEventListener('open', resolve, { once: true });
      socket.addEventListener('error', () => reject(new Error('cannot connect to Chrome at ' + address)), { once: true });
    });
    return new Browser(process, socket, profile);
  }

  send<T = any>(method: string, params: object = {}, sessionId?: string): Promise<T> {
    const id = this.next++;
    return new Promise<T>((resolve, reject) => {
      this.waiting.set(id, { resolve, reject, method });
      this.socket.send(JSON.stringify({ id, method, params, ...(sessionId ? { sessionId } : {}) }));
    });
  }

  on(listener: (message: Message) => void): () => void {
    this.listeners.add(listener);
    return () => { this.listeners.delete(listener); };
  }

  /**
   * A new tab, attached to on the same connection. `isolated`: in a window and a browser context of its own, with its
   * own storage and connections, and shown however many others are open (only the front tab of a window is visible);
   * the context goes with the tab.
   */
  async newPage({ isolated = false } = {}): Promise<Page> {
    const context = isolated ? (await this.send('Target.createBrowserContext')).browserContextId as string : undefined;
    const { targetId } = await this.send('Target.createTarget', { url: 'about:blank', ...(context ? { browserContextId: context, newWindow: true } : {}) });
    const { sessionId } = await this.send('Target.attachToTarget', { targetId, flatten: true });
    const page = new Page(this, targetId, sessionId, context);
    await page.init();
    return page;
  }

  async close(): Promise<void> {
    try {
      await Promise.race([this.send('Browser.close'), Bun.sleep(5_000)]);
    } catch {
      // gone already
    }
    this.socket.close();
    this.process.kill();
    await this.process.exited;
    // Chrome's helpers may hold the profile's files a moment longer
    for (let attempt = 0; attempt < 10; attempt++) {
      try {
        rmSync(this.profile, { recursive: true, force: true });
        return;
      } catch {
        await Bun.sleep(200);
      }
    }
  }
}

/** Something the page logged or threw, or a request of it that failed. */
export type Problem = { source: string; text: string };

export class Page {
  /** Errors since the page was opened: console.error, uncaught exceptions and failed loads. */
  readonly problems: Problem[] = [];
  private loaded = Promise.withResolvers<void>();
  /** The requests sent and not answered yet, by id: the address and its kind (`Fetch`, `EventSource`, `Image`…). */
  private readonly requests = new Map<string, { url: string; type: string }>();
  private readonly unlisten: () => void;

  constructor(private readonly browser: Browser, private readonly targetId: string, private readonly sessionId: string,
    private readonly context?: string) {
    this.unlisten = browser.on(message => {
      if (message.sessionId !== sessionId) return;
      const params = message.params;
      switch (message.method) {
        case 'Network.requestWillBeSent':
          this.requests.set(params.requestId, { url: params.request.url, type: params.type });
          break;
        case 'Network.loadingFinished':
        case 'Network.loadingFailed':
          this.requests.delete(params.requestId);
          break;
        case 'Runtime.consoleAPICalled':
          if (params.type === 'error' || params.type === 'assert') {
            this.problems.push({ source: 'console.' + params.type, text: params.args.map(describe).join(' ') });
          }
          break;
        case 'Runtime.exceptionThrown': {
          const details = params.exceptionDetails;
          this.problems.push({ source: 'exception', text: details.exception?.description ?? details.text });
          break;
        }
        case 'Log.entryAdded':
          if (params.entry.level === 'error') this.problems.push({ source: params.entry.source, text: `${params.entry.text} ${params.entry.url ?? ''}`.trim() });
          break;
        case 'Page.loadEventFired':
          this.loaded.resolve();
          break;
      }
    });
  }

  send<T = any>(method: string, params: object = {}): Promise<T> {
    return this.browser.send<T>(method, params, this.sessionId);
  }

  async init(): Promise<void> {
    await Promise.all(['Page.enable', 'Runtime.enable', 'Log.enable', 'Network.enable'].map(method => this.send(method)));
  }

  /** The requests of the page still waiting for their answer, event streams left out: what a check waiting in vain may
   * be waiting for. */
  waiting(): string[] {
    return [...this.requests.values()].filter(request => request.type !== 'EventSource').map(request => request.url);
  }

  /** Opens `url` in the tab; fails when it has not loaded in fifteen seconds (its requests may be waiting their turn). */
  async goto(url: string): Promise<void> {
    this.loaded = Promise.withResolvers<void>();
    this.requests.clear();
    const shown = url.replace(/token=[^&]*/, 'token=…');
    const { errorText } = await this.send('Page.navigate', { url });
    if (errorText) throw new Error(`cannot open ${shown}: ${errorText}`);
    let timer: ReturnType<typeof setTimeout> | undefined;
    const late = new Promise<never>((_, reject) => {
      timer = setTimeout(() => reject(new Error(`${shown} did not load in 15 s`)), 15_000);
    });
    try {
      await Promise.race([this.loaded.promise, late]);
    } finally {
      clearTimeout(timer);
    }
  }

  /** The value of `fn(...args)` run in the page (awaited when it is a promise). */
  async eval<T, A extends unknown[] = []>(fn: (...args: A) => T | Promise<T>, ...args: A): Promise<T> {
    const { result, exceptionDetails } = await this.send('Runtime.evaluate', {
      expression: `(${fn})(...${JSON.stringify(args)})`, returnByValue: true, awaitPromise: true,
    });
    if (exceptionDetails) throw new Error('in the page: ' + (exceptionDetails.exception?.description ?? exceptionDetails.text));
    return result.value as T;
  }

  /**
   * Runs `fn(...args)` in the page until `ok` takes its value, and returns that value. After ten seconds it fails
   * with `what` and the last value: what the page showed instead.
   */
  async until<T, A extends unknown[] = []>(what: string, ok: (value: T) => boolean, fn: (...args: A) => T, ...args: A): Promise<T> {
    const deadline = Date.now() + 10_000;
    for (;;) {
      const value = await this.eval(fn, ...args);
      if (ok(value)) return value;
      if (Date.now() > deadline) throw new Error(`timed out waiting for ${what}; the page had ${JSON.stringify(value)}`);
      await Bun.sleep(50);
    }
  }

  /** {@link until} the value is truthy. */
  waitFor<T, A extends unknown[] = []>(what: string, fn: (...args: A) => T, ...args: A): Promise<NonNullable<T>> {
    return this.until(what, Boolean, fn, ...args) as Promise<NonNullable<T>>;
  }

  /** The visible text of each element `selector` matches, in order. */
  texts(selector: string): Promise<string[]> {
    return this.eval(selector => [...document.querySelectorAll<HTMLElement>(selector)].map(element => element.innerText.trim()), selector);
  }

  /** Waits until the texts of `selector` pass `ok`, and returns them. */
  untilTexts(what: string, selector: string, ok: (texts: string[]) => boolean): Promise<string[]> {
    return this.until(what, ok, selector => [...document.querySelectorAll<HTMLElement>(selector)].map(element => element.innerText.trim()), selector);
  }

  /**
   * A click with the mouse in the middle of the first element `selector` matches whose text or label has `text`, once
   * nothing covers that point: an element out of view is scrolled to first (a list may scroll it back, so it is
   * measured again until it stays).
   */
  async click(selector: string, text?: string): Promise<void> {
    const point = await this.point(selector, text);
    await this.send('Input.dispatchMouseEvent', { type: 'mouseMoved', ...point });
    await this.send('Input.dispatchMouseEvent', { type: 'mousePressed', ...point, button: 'left', clickCount: 1 });
    await this.send('Input.dispatchMouseEvent', { type: 'mouseReleased', ...point, button: 'left', clickCount: 1 });
  }

  /** The mouse moved over the middle of the element, found as {@link click} finds it, and left there. */
  async hover(selector: string, text?: string): Promise<void> {
    await this.send('Input.dispatchMouseEvent', { type: 'mouseMoved', ...await this.point(selector, text) });
  }

  /** The left button pressed in the middle of the element, found as {@link click} finds it, and held down. */
  async hold(selector: string, text?: string): Promise<void> {
    const point = await this.point(selector, text);
    await this.send('Input.dispatchMouseEvent', { type: 'mouseMoved', ...point });
    await this.send('Input.dispatchMouseEvent', { type: 'mousePressed', ...point, button: 'left', clickCount: 1 });
  }

  /** The button held down by {@link hold} carried to `at` and let go there: what was pressed gets no click. */
  async letGo(at: { x: number; y: number }): Promise<void> {
    await this.send('Input.dispatchMouseEvent', { type: 'mouseMoved', ...at, button: 'left', buttons: 1 });
    await this.send('Input.dispatchMouseEvent', { type: 'mouseReleased', ...at, button: 'left', clickCount: 1 });
  }

  /** Where {@link click} clicks the element: its middle, once nothing covers it. */
  point(selector: string, text?: string): Promise<{ x: number; y: number }> {
    return this.waitFor(`${selector}${text ? ` with 「${text}」` : ''} to click`, (selector, text) => {
      const element = [...document.querySelectorAll<HTMLElement>(selector)]
        .find(element => !text || (element.textContent ?? '').includes(text) || element.getAttribute('aria-label')?.includes(text));
      if (!element) return null;
      const box = element.getBoundingClientRect();
      if (!box.width || !box.height) return null;
      const x = box.x + box.width / 2, y = box.y + box.height / 2;
      const hit = document.elementFromPoint(x, y);
      if (hit && element.contains(hit)) return { x, y };
      if (y < 0 || y > innerHeight || x < 0 || x > innerWidth || !hit) element.scrollIntoView({ block: 'center', inline: 'center' });
      return null;
    }, selector, text ?? '');
  }

  /** Turns the mouse wheel over the middle of the element `selector` matches: `deltaY` below 0 scrolls up. */
  async wheel(selector: string, deltaY: number): Promise<void> {
    const point = await this.waitFor(`${selector} to scroll`, selector => {
      const box = document.querySelector(selector)?.getBoundingClientRect();
      return box?.width ? { x: box.x + box.width / 2, y: box.y + box.height / 2 } : null;
    }, selector);
    await this.send('Input.dispatchMouseEvent', { type: 'mouseWheel', ...point, deltaX: 0, deltaY });
  }

  /** Types `text` into the focused element, as an IME would commit it. */
  async type(text: string): Promise<void> {
    await this.send('Input.insertText', { text });
  }

  /** Presses and lets go of a key: Enter submits a form, Backspace deletes what is selected. */
  async press(key: 'Escape' | 'Enter' | 'Backspace'): Promise<void> {
    const code = { Escape: 27, Enter: 13, Backspace: 8 }[key];
    const text = key === 'Enter' ? { text: '\r' } : {};
    await this.send('Input.dispatchKeyEvent', { type: text.text ? 'keyDown' : 'rawKeyDown', key, code: key, windowsVirtualKeyCode: code, ...text });
    await this.send('Input.dispatchKeyEvent', { type: 'keyUp', key, code: key, windowsVirtualKeyCode: code });
  }

  /** Empties the text field `selector` matches as a person would: selects what is in it and deletes it. */
  async clear(selector: string): Promise<void> {
    await this.click(selector);
    await this.eval(selector => document.querySelector<HTMLInputElement>(selector)?.select(), selector);
    await this.press('Backspace');
  }

  /** The viewport as a PNG, or only `clip` of it (in CSS pixels). */
  async screenshot(clip?: { x: number; y: number; width: number; height: number }): Promise<Uint8Array> {
    const { data } = await this.send('Page.captureScreenshot', { format: 'png', ...(clip ? { clip: { ...clip, scale: 1 } } : {}) });
    return Uint8Array.fromBase64(data);
  }

  /** Closes the tab, and with it the page's connections; an isolated tab's context too. */
  async close(): Promise<void> {
    this.unlisten();
    try {
      await this.browser.send('Target.closeTarget', { targetId: this.targetId });
      if (this.context) await this.browser.send('Target.disposeBrowserContext', { browserContextId: this.context });
    } catch {
      // gone already
    }
  }
}

function describe(arg: { type: string; value?: unknown; description?: string }): string {
  return arg.value !== undefined ? String(arg.value) : arg.description ?? arg.type;
}
