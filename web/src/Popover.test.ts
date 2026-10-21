import { afterEach, describe, expect, it } from '#test';
import { placeOf, visibleBox } from './Popover';

const viewport = (width: number, height: number) => {
  Object.defineProperty(window, 'innerWidth', { configurable: true, value: width });
  Object.defineProperty(window, 'innerHeight', { configurable: true, value: height });
};
const anchor = (left: number, top: number, width: number, height: number) => new DOMRect(left, top, width, height);
afterEach(() => viewport(1024, 768));

describe('placeOf (#239)', () => {
  it('opens below, or above where only that has room', () => {
    viewport(1366, 768);
    expect(placeOf(anchor(100, 100, 200, 32), 316, 340)).toMatchObject({ top: 140, left: 100, maxHeight: undefined });
    const above = placeOf(anchor(100, 600, 200, 32), 316, 340);
    expect(above.top).toBe(600 - 8 - 340);
    expect(above.origin).toContain('bottom');
  });

  it('scrolls inside on the larger side when neither has room', () => {
    viewport(1024, 600);
    const place = placeOf(anchor(100, 280, 200, 32), 316, 340);
    expect(place.maxHeight).toBe(600 - 16 - 312 - 8);
    expect(place.top).toBe(320);
    expect(place.top + place.maxHeight!).toBeLessThanOrEqual(600 - 16);
    // the side above is larger: it ends at the anchor
    const up = placeOf(anchor(100, 400, 200, 32), 316, 500);
    expect(up.maxHeight).toBe(400 - 8 - 16);
    expect(up.top).toBe(16);
  });

  it('stays inside a narrow screen', () => {
    viewport(320, 640);
    const place = placeOf(anchor(16, 100, 288, 40), 316, 340);
    expect(place.left).toBeGreaterThanOrEqual(0);
    expect(place.left + 316).toBeLessThanOrEqual(320);
    expect(place.maxWidth).toBeUndefined();
    // narrower than the layer: it starts at the edge and scrolls across
    viewport(300, 640);
    expect(placeOf(anchor(8, 100, 280, 40), 316, 340)).toMatchObject({ left: 0, maxWidth: 300 });
  });

});

describe('visibleBox', () => {
  it('is empty while the control is scrolled out of the window or of a list', () => {
    viewport(1024, 768);
    const list = document.createElement('div');
    list.style.overflowY = 'auto';
    const control = document.createElement('button');
    list.append(control);
    document.body.append(list);
    list.getBoundingClientRect = () => anchor(0, 100, 400, 300);
    const at = (top: number) => { control.getBoundingClientRect = () => anchor(0, top, 100, 32); return visibleBox(control); };
    const out = (box: { top: number; bottom: number }) => box.bottom < box.top;
    expect(out(at(200))).toBe(false);
    // above the list, though still in the window
    expect(out(at(40))).toBe(true);
    expect(out(at(-50))).toBe(true);
    expect(out(at(380))).toBe(false);
    list.remove();
  });
});
