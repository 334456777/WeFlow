import { act, cleanup, render } from '@testing-library/react';
import { createRef } from 'react';
import { afterEach, describe, expect, it } from '#test';
import { keySeed, VirtualList, type VirtualListHandle } from './VirtualList';

afterEach(cleanup);

const items = Array.from({ length: 5000 }, (_, i) => 'row ' + i);

describe('VirtualList', () => {
  it('draws only the rows in sight and sizes its space for all of them', () => {
    const { container } = render(<VirtualList items={items} rowHeight={58} padding={8} keyOf={item => item} renderRow={item => <span>{item}</span>} />);
    const rows = container.querySelectorAll('.virtual-row');
    expect(rows.length).toBeLessThan(40);
    expect(rows[0].textContent).toBe('row 0');
    expect((container.querySelector('.virtual-space') as HTMLElement).style.height).toBe(5000 * 58 + 16 + 'px');
    expect((rows[1] as HTMLElement).style.transform).toBe('translateY(66px)');
  });

  it('scrolls just enough to show a row and draws the rows there', () => {
    const ref = createRef<VirtualListHandle>();
    const { container } = render(<VirtualList ref={ref} items={items} rowHeight={58} keyOf={item => item} renderRow={item => <span>{item}</span>} />);
    const list = container.querySelector('.virtual-list') as HTMLElement;
    Object.defineProperty(list, 'clientHeight', { value: 580, configurable: true });
    act(() => ref.current!.scrollToIndex(1000));
    expect(list.scrollTop).toBe(1001 * 58 - 580);
    const texts = Array.from(container.querySelectorAll('.virtual-row'), row => row.textContent);
    expect(texts).toContain('row 1000');
    expect(texts).not.toContain('row 0');
    act(() => ref.current!.scrollToIndex(995));
    expect(list.scrollTop).toBe(1001 * 58 - 580);
  });

  it('draws its rows again only when the rows in sight change, not at every scroll (#961)', async () => {
    let drawn = 0;
    const { container } = render(<VirtualList items={items} rowHeight={58} keyOf={item => item} renderRow={item => { drawn += 1; return <span>{item}</span>; }} />);
    const list = container.querySelector('.virtual-list') as HTMLElement;
    Object.defineProperty(list, 'clientHeight', { value: 580, configurable: true });
    const scrollTo = async (top: number) => {
      list.scrollTop = top;
      list.dispatchEvent(new Event('scroll'));
      await act(() => new Promise(resolve => requestAnimationFrame(() => resolve(undefined))));
    };
    await scrollTo(1);
    const settled = drawn;
    await scrollTo(20);
    await scrollTo(57);
    expect(drawn).toBe(settled);
    await scrollTo(58 * 10);
    expect(drawn).toBeGreaterThan(settled);
    expect(Array.from(container.querySelectorAll('.virtual-row'), row => row.textContent)).toContain('row 14');
  });

  it('gives each row a seed of its item’s key, the same wherever the row is drawn (#831)', () => {
    const ref = createRef<VirtualListHandle>();
    const { container } = render(<VirtualList ref={ref} items={items} rowHeight={58} keyOf={item => item} renderRow={item => <span>{item}</span>} />);
    const seedOf = (text: string) => {
      const row = Array.from(container.querySelectorAll<HTMLElement>('.virtual-row')).find(row => row.textContent === text)!;
      return Number(row.style.getPropertyValue('--key-seed'));
    };
    const before = seedOf('row 3');
    expect(before).toBe(keySeed('row 3'));
    expect(before).toBeGreaterThanOrEqual(0);
    expect(before).toBeLessThan(1);
    expect(seedOf('row 4')).not.toBe(before);
    const list = container.querySelector('.virtual-list') as HTMLElement;
    Object.defineProperty(list, 'clientHeight', { value: 580, configurable: true });
    act(() => ref.current!.scrollToIndex(12));
    expect(seedOf('row 3')).toBe(before);
  });
});
