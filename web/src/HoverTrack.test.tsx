import { cleanup, fireEvent, render } from '@testing-library/react';
import { afterEach, describe, expect, it } from 'vitest';
import { HoverTrack } from './HoverTrack';

afterEach(cleanup);

/** Gives the group and its items boxes: items 40 high, 4 apart, from the group's top. */
function layout(group: HTMLElement) {
  group.getBoundingClientRect = () => ({ left: 10, top: 100, width: 200, height: 300, right: 210, bottom: 400, x: 10, y: 100, toJSON() {} });
  group.querySelectorAll<HTMLElement>('.item').forEach((item, i) => {
    item.getBoundingClientRect = () => ({ left: 10, top: 100 + i * 44, width: 200, height: 40, right: 210, bottom: 140 + i * 44, x: 10, y: 100 + i * 44, toJSON() {} });
  });
}

function setup() {
  const { container } = render(<nav><HoverTrack items=".item" /><button className="item">a</button><button className="item selected">b</button><button className="item">c</button></nav>);
  const group = container.querySelector('nav')!;
  layout(group);
  const hollow = group.querySelector<HTMLElement>('.hover-hollow')!;
  const items = group.querySelectorAll<HTMLElement>('.item');
  return { group, hollow, items };
}

describe('HoverTrack', () => {
  it('appears in place, then slides from item to item, and fades on leaving', () => {
    const { group, hollow, items } = setup();
    expect(group.classList.contains('hover-track-host')).toBe(true);
    expect(hollow.dataset.state).toBeUndefined();

    fireEvent.pointerOver(items[0]);
    expect(hollow.dataset.state).toBe('on');
    expect(hollow.style.translate).toBe('0px 0px');
    expect(hollow.style.height).toBe('40px');

    // the gap between two items keeps it where it is
    fireEvent.pointerOver(group);
    expect(hollow.dataset.state).toBe('on');
    expect(hollow.style.translate).toBe('0px 0px');

    fireEvent.pointerOver(items[2]);
    expect(hollow.style.translate).toBe('0px 88px');
    // sliding: the transition is the stylesheet's, not switched off
    expect(hollow.style.transition).toBe('');

    fireEvent.pointerLeave(group);
    expect(hollow.dataset.state).toBe('off');
  });

  it('sinks into the press of the item under it, and lets go anywhere', () => {
    const { group, hollow, items } = setup();
    fireEvent.pointerOver(items[0]);
    fireEvent.pointerDown(items[0], { button: 0 });
    expect(hollow.dataset.pressed).toBe('');
    // released outside the group too
    fireEvent.pointerUp(window);
    expect(hollow.dataset.pressed).toBeUndefined();
    // the selection takes its own press; another button, or moving on, does not press
    fireEvent.pointerOver(items[1]);
    fireEvent.pointerDown(items[1], { button: 0 });
    expect(hollow.dataset.pressed).toBeUndefined();
    fireEvent.pointerOver(items[2]);
    fireEvent.pointerDown(items[2], { button: 2 });
    expect(hollow.dataset.pressed).toBeUndefined();
    fireEvent.pointerDown(items[2], { button: 0 });
    fireEvent.pointerLeave(group);
    expect(hollow.dataset.pressed).toBeUndefined();
  });

  it('fades over a selected item, and ignores touch', () => {
    const { hollow, items } = setup();
    fireEvent.pointerOver(items[0]);
    fireEvent.pointerOver(items[1]);
    expect(hollow.style.translate).toBe('0px 44px');
    expect(hollow.dataset.state).toBe('quiet');
    fireEvent.pointerOver(items[2]);
    expect(hollow.dataset.state).toBe('on');

    const touch = new Event('pointerover', { bubbles: true }) as Event & { pointerType: string };
    touch.pointerType = 'touch';
    items[0].dispatchEvent(touch);
    expect(hollow.style.translate).toBe('0px 88px');
  });
});
