// The contact list of `weflow serve` (#205), read once the page is opened and again for another server or account.
import { useCallback, useEffect, useRef, useState } from 'react';
import { ApiError } from '../api/client';
import { getContacts, type Contact } from '../api/contacts';

export type ContactsState =
  | { status: 'idle' }
  | { status: 'loading' }
  | { status: 'ready'; contacts: Contact[] }
  | { status: 'error'; message: string };

export function useContacts(enabled: boolean, scope: number): [ContactsState, () => void] {
  const [state, setState] = useState<ContactsState>({ status: 'idle' });
  const [round, setRound] = useState(0);
  const reload = useCallback(() => setRound(n => n + 1), []);
  // what was read is kept while the page is left and opened again, as long as the server and account are the same
  const read = useRef<string | null>(null);
  useEffect(() => {
    const key = scope + ':' + round;
    if (!enabled || read.current === key) return;
    read.current = key;
    const controller = new AbortController();
    let done = false;
    setState({ status: 'loading' });
    getContacts(controller.signal).then(contacts => { done = true; setState({ status: 'ready', contacts }); }, problem => {
      if (controller.signal.aborted) return;
      done = true;
      read.current = null;
      setState({ status: 'error', message: problem instanceof ApiError && problem.message ? problem.message : '读取通讯录失败' });
    });
    // left before the list came: read it again next time
    return () => { if (done) return; controller.abort(); if (read.current === key) read.current = null; };
  }, [enabled, scope, round]);
  return [state, reload];
}
