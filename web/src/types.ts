export type Category = 'chat' | 'moments' | 'payments';
export type Navigation = Category | 'contacts' | 'insights' | 'export' | 'backup' | 'settings';
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
  /** Read from `weflow serve` rather than the sample records. */
  live?: boolean;
  messageCount?: number;
  /** Unix seconds of the last message. */
  lastTime?: number;
}

export interface ExportResult {
  fileName: string;
  count: number;
  downloadUrl: string;
}
