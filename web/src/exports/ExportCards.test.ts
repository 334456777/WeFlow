import { describe, expect, it } from 'vitest';
import { DEFAULT_BATCH, exportParams, packed, toggleFormat } from './ExportCards';

describe('the export formats, several at once (#342)', () => {
  it('puts formats on in the list order, and takes them off but the last', () => {
    expect(toggleFormat(['txt'], 'html')).toEqual(['html', 'txt']);
    expect(toggleFormat(['html', 'txt'], 'html')).toEqual(['txt']);
    expect(toggleFormat(['txt'], 'txt')).toEqual(['txt']);
  });

  it('takes one of two formats writing the same extension', () => {
    expect(toggleFormat(['html', 'json'], 'arkme-json')).toEqual(['html', 'arkme-json']);
    expect(toggleFormat(['arkme-json'], 'json')).toEqual(['json']);
  });

  it('packs several files, or media with one, into a zip', () => {
    expect(packed(['html'], false)).toBe(false);
    expect(packed(['html', 'txt'], false)).toBe(true);
    expect(packed(['html'], true)).toBe(true);
    expect(DEFAULT_BATCH.media).toBe(false);
    expect(exportParams('wxid_a', { ...DEFAULT_BATCH, media: true })).toMatchObject({ format: ['html'], media: 'all', zip: true });
    expect(exportParams('wxid_a', DEFAULT_BATCH).zip).toBeUndefined();
  });
});
