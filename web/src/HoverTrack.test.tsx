import { cleanup, fireEvent, render } from '@testing-library/react';
import { afterEach, describe, expect, it } from '#test';
import { HoverTrack } from './HoverTrack';

afterEach(cleanup);

/** 给这一组和它的项设定位置和大小：每项高 40、相隔 4，从这一组的顶部开始排。 */
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

    // 指针在两项之间的空隙里时凹陷留在原处
    fireEvent.pointerOver(group);
    expect(hollow.dataset.state).toBe('on');
    expect(hollow.style.translate).toBe('0px 0px');

    fireEvent.pointerOver(items[2]);
    expect(hollow.style.translate).toBe('0px 88px');
    // 滑动：过渡用样式表里的，没有关掉
    expect(hollow.style.transition).toBe('');

    fireEvent.pointerLeave(group);
    expect(hollow.dataset.state).toBe('off');
  });

  it('sinks into the press of the item under it, and lets go anywhere', () => {
    const { group, hollow, items } = setup();
    fireEvent.pointerOver(items[0]);
    fireEvent.pointerDown(items[0], { button: 0 });
    expect(hollow.dataset.pressed).toBe('');
    // 在这一组外面松开也算松开
    fireEvent.pointerUp(window);
    expect(hollow.dataset.pressed).toBeUndefined();
    // 选中的项自己显示按下；按其他鼠标键或者移到别处都不算按下
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
