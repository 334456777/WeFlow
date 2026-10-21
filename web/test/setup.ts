// Loaded before the tests (bunfig.toml, #361): a DOM from happy-dom where Vitest had jsdom, at jsdom's default
// address, and what Vite would have defined for the app (plugins/webMeta.ts). Where happy-dom does what jsdom did not,
// it is brought back to what the tests were written for.
import { GlobalRegistrator } from '@happy-dom/global-registrator';
import { WEB_META } from '../plugins/webMeta.ts';

const { fetch, Request, Response, Headers } = globalThis;
// jsdom has no matchMedia, so the app took the screen for a wide one; happy-dom's answers for a 1024px window, which
// the app lays out as narrow (max-width: 1100px). The tests of the narrow layout stub matchMedia.
GlobalRegistrator.register({ url: 'http://localhost:3000/', width: 1920, height: 1080 });
// Bun's own fetch, as Vitest left Node's: a request no test stubs fails at once, where happy-dom's would go to
// localhost:3000
Object.assign(globalThis, { fetch, Request, Response, Headers });

// happy-dom's WheelEvent is a UIEvent, without the pointer's place (clientX, …) that a MouseEvent has
class WheelEvent extends MouseEvent {
  static readonly DOM_DELTA_PIXEL = 0;
  static readonly DOM_DELTA_LINE = 1;
  static readonly DOM_DELTA_PAGE = 2;
  readonly deltaX: number;
  readonly deltaY: number;
  readonly deltaZ: number;
  readonly deltaMode: number;
  constructor(type: string, init: WheelEventInit = {}) {
    super(type, init);
    this.deltaX = init.deltaX ?? 0;
    this.deltaY = init.deltaY ?? 0;
    this.deltaZ = init.deltaZ ?? 0;
    this.deltaMode = init.deltaMode ?? 0;
  }
}
// Testing Library takes the event classes from the document's window
Object.assign(globalThis, { WheelEvent });
Object.assign(document.defaultView!, { WheelEvent });

// happy-dom computes no display for the inline elements (span, a, strong, svg, …) where jsdom and browsers say
// `inline`; the accessible name puts a space around an element that is not inline, so `<span>4</span>验证` was
// named "4 验证" instead of "4验证"
const computed = window.getComputedStyle.bind(window);
window.getComputedStyle = (element, pseudo) => {
  const style = computed(element, pseudo);
  if (style.display) return style;
  return new Proxy(style, {
    get(target, key) {
      if (key === 'display') return 'inline';
      if (key === 'getPropertyValue') return (name: string) => (name === 'display' ? 'inline' : target.getPropertyValue(name));
      const value = Reflect.get(target, key, target);
      return typeof value === 'function' ? value.bind(target) : value;
    },
  });
};

// A node in a failed expectation as its tag, as Vitest shows it: printed whole, with the document it holds on to,
// it takes seconds, long enough for a waitFor to time out before it looks again
Object.defineProperty(Node.prototype, Symbol.for('nodejs.util.inspect.custom'), {
  value(this: Node) {
    if (!(this instanceof Element)) return `${this.nodeName} ${JSON.stringify(this.textContent?.slice(0, 80) ?? '')}`;
    const attributes = [...this.attributes].map(a => ` ${a.name}="${a.value}"`).join('');
    return `<${this.localName}${attributes}>${this.children.length ? '…' : (this.textContent ?? '').slice(0, 80)}</${this.localName}>`;
  },
});

Object.assign(globalThis, { __WEFLOW_WEB__: WEB_META });
