// What the workspace gives each page and what a page gives back (#401). A page keeps its own state, so it stays as it
// was when another page is opened and back; the workspace picks the page shown and gathers the console's cards.
import type { ReactNode } from 'react';
import type { Backend, Health } from '../api/backend';
import type { ChatSession } from '../api/chat';
import type { Profile } from '../api/setup';
import type { Card } from '../CardStack';
import type { Preview } from '../MessageMedia';
import type { SessionsState } from '../chat/useSessions';
import type { JobsState } from '../jobs/useJobs';

/** What every page shares: the server, the records read once for all pages, and the parts of the screen. */
export interface Shell {
  backend: Backend;
  /** The server answers now. */
  connected: boolean;
  /**
   * What changes something or follows it (jobs, the settings) can be used: the server answers, or the sample server
   * answers in its place (#896), keeping what changes in the page.
   */
  usable: boolean;
  /** What `/health` says of the server `usable` acts on: the server's, or the sample server's; null while neither. */
  health: Health | null;
  /**
   * The page shows the server's records (`holdsRecords`): connected, or gone after it was, when what was read stays
   * shown and what is not read yet is asked for (and fails, saying the server is not reached) until it is back.
   */
  held: boolean;
  /**
   * The page shows records (`recordsOf`): the server's (`held`), or the sample's (`sample`), which the sample server of
   * src/mock answers the page's reads with (#879).
   */
  records: boolean;
  /** The records are the sample's: no server has answered this page, or the one there does not suit it. */
  sample: boolean;
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

/** What a page gives the workspace. */
export interface Page {
  /** What the page shows beside the navigation; none while it has no records to show (the chat page's is shown). */
  view?: ReactNode;
  /** Its own cards of the console, given whether shown or not: the console keeps one order of them all. */
  cards?: Card[];
  /** The cards it shows, its own or shared ones such as 'tasks'; none, as for the view. */
  shown?: string[];
}
