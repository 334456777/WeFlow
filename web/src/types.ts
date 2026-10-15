export type Category = 'chat' | 'moments' | 'articles' | 'payments';
export type Navigation = Category | 'settings';
export type ConsoleMode = 'export' | 'search' | 'info' | 'media';
export type ExportFormat = 'HTML' | 'JSON' | 'Excel';
export type MessageKind = 'text' | 'image' | 'file';

export interface Message {
  id: string;
  sender: string;
  direction: 'incoming' | 'outgoing';
  date: string;
  time: string;
  kind: MessageKind;
  text: string;
  asset?: string;
  fileName?: string;
}

export interface Conversation {
  id: string;
  category: Category;
  name: string;
  /** Picture URL; without one the first character of the name is shown. */
  avatar?: string;
  preview: string;
  stamp: string;
  description: string;
  messages: Message[];
}

export interface ExportOptions {
  format: ExportFormat;
  from: string;
  to: string;
  includeMedia: boolean;
}

export interface ExportResult {
  fileName: string;
  count: number;
  downloadUrl: string;
}
