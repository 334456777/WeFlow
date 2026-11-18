// Types the pages share: the pages of the navigation, and the result of an export made in the page.

/**
 * What a page belongs to: Moments and the payment records are their own, every other page is the chat's. The
 * settings show the cards of the one opened last.
 */
export type Category = 'chat' | 'moments' | 'payments';
/** Every page of the navigation. */
export type Navigation = Category | 'contacts' | 'resources' | 'analytics' | 'reports' | 'footprint' | 'insights' | 'export' | 'backup' | 'settings';
/** The formats of an export the page writes itself (the payment records). */
export type ExportFormat = 'HTML' | 'JSON' | 'Excel';

/** A finished export, offered as a download. */
export interface ExportResult {
  fileName: string;
  /** How many records it holds. */
  count: number;
  downloadUrl: string;
}
