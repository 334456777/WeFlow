export type Category = 'chat' | 'moments' | 'payments';
export type Navigation = Category | 'contacts' | 'resources' | 'analytics' | 'reports' | 'footprint' | 'insights' | 'export' | 'backup' | 'settings';
export type ExportFormat = 'HTML' | 'JSON' | 'Excel';

export interface ExportResult {
  fileName: string;
  count: number;
  downloadUrl: string;
}
