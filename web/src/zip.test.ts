import { strFromU8, unzipSync } from 'fflate';
import { describe, expect, it } from '#test';
import { zip } from './zip';

const utf8 = (text: string) => new TextEncoder().encode(text);

describe('zip', () => {
  it('holds each file, deflated, as another reader unpacks it', async () => {
    const rows = Array.from({ length: 2000 }, (_, i) => '<row r="' + String(i + 1) + '">记录 ' + String(i) + '</row>').join('');
    const archive = await zip({ 'a.txt': utf8('123456789'), 'xl/sheet.xml': utf8(rows), 'empty': utf8('') });
    const files = unzipSync(archive);
    expect(Object.keys(files)).toEqual(['a.txt', 'xl/sheet.xml', 'empty']);
    expect(strFromU8(files['a.txt'])).toBe('123456789');
    expect(strFromU8(files['xl/sheet.xml'])).toBe(rows);
    expect(files['empty'].length).toBe(0);
    expect(archive.length).toBeLessThan(rows.length / 4);
  });

  it('records the CRC-32, sizes and offsets in both headers', async () => {
    const archive = await zip({ 'a.txt': utf8('123456789'), 'b.txt': utf8('b') });
    const view = new DataView(archive.buffer);
    const u16 = (at: number) => view.getUint16(at, true);
    const u32 = (at: number) => view.getUint32(at, true);
    expect(u32(0)).toBe(0x04034b50);
    // CRC-32 的标准校验值，即 `123456789` 的 CRC-32
    expect(u32(14)).toBe(0xcbf43926);
    expect(u32(22)).toBe(9);
    const end = archive.length - 22;
    expect(u32(end)).toBe(0x06054b50);
    expect(u16(end + 10)).toBe(2);
    const central = u32(end + 16);
    expect(central + u32(end + 12)).toBe(end);
    expect(u32(central)).toBe(0x02014b50);
    expect(u32(central + 16)).toBe(0xcbf43926);
    expect(u32(central + 42)).toBe(0);
    const second = central + 46 + 5;
    expect(u32(second)).toBe(0x02014b50);
    expect(u32(u32(second + 42))).toBe(0x04034b50);
    // 修改时间里的年份，从 1980 年算起
    expect((u16(12) >> 9) + 1980).toBe(new Date().getFullYear());
  });
});
