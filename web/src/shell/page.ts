// What the workspace gives each page and what a page gives back (#401). A page keeps its own state, so it stays as it
// was when another page is opened and back; the workspace picks the page shown and gathers the console's cards.
import type { ReactNode } from 'react';
import type { Backend } from '../api/backend';
import type { ChatSession } from '../api/chat';
import type { Profile } from '../api/setup';
import type { Card } from '../CardStack';
import type { Preview } from '../chat/MessageView';
import type { SessionsState } from '../chat/useSessions';
import type { JobsState } from '../jobs/useJobs';

/** What every page shares: the server, the records read once for all pages, and the parts of the screen. */
export interface Shell {
  backend: Backend;
  connected: boolean;
  /** Another server or account (#198): what a page read for the one before goes. */
  scope: number;
  /** The conversation list, which other pages name and open conversations from. */
  sessions: SessionsState;
  sessionList: ChatSession[];
  reloadSessions(): void;
  jobs: JobsState;
  profile: Profile | undefined;
  /** On a phone a page of two sides shows one at a time: the detail, once something is picked (#335). */
  mobileDetail: boolean;
  setMobileDetail(show: boolean): void;
  openPreview(preview: Preview): void;
  toast(message: string): void;
}

/** Opens the chat page at a conversation. */
export type ChatWith = (username: string) => void;
/** Opens the chat page at a message of a conversation. */
export type OpenAt = (sessionId: string, localId: number, createTime: number) => void;

export interface Page {
  /** What the page shows beside the navigation; none for a page that shows the sample instead (not connected). */
  view?: ReactNode;
  /** Its own cards of the console, given whether shown or not: the console keeps one order of them all. */
  cards?: Card[];
  /** The cards it shows, its own or shared ones such as 'tasks'; none, as for the view. */
  shown?: string[];
}
