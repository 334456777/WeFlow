import { describe, expect, it } from '#test';
import { contactsExportParams, contactsExportSummary, DEFAULT_CONTACTS_EXPORT } from './ContactCards';

describe('the contacts export (#708)', () => {
  it('names no kind when every kind is picked, and the picked ones in the page order otherwise', () => {
    expect(contactsExportParams(DEFAULT_CONTACTS_EXPORT)).toEqual({ format: 'excel', parsed: true, kind: undefined, media: undefined, zip: undefined });
    expect(contactsExportParams({ ...DEFAULT_CONTACTS_EXPORT, kinds: ['official', 'friend'] }).kind).toEqual(['friend', 'official']);
  });

  it('packs the HTML page with its avatars into a zip', () => {
    const html = { ...DEFAULT_CONTACTS_EXPORT, format: 'html' as const, avatars: true };
    expect(contactsExportParams(html)).toMatchObject({ format: 'html', media: ['avatar'], zip: true });
    expect(contactsExportSummary(html)).toContain('头像');
  });

  it('takes no avatars for the other formats, even when they were picked on the HTML page before', () => {
    const excel = { ...DEFAULT_CONTACTS_EXPORT, avatars: true };
    expect(contactsExportParams(excel)).toMatchObject({ media: undefined, zip: undefined });
    expect(contactsExportSummary(excel)).not.toContain('头像');
  });
});
