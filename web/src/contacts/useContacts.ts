// 通讯录页打开后读取的 `weflow serve` 的联系人列表（#205）：与其他页面读的是同一份（`contactList`），
// 按服务端和账号保留。
import { useCallback, useEffect, useRef, useState } from 'react';
import { ApiError } from '../api/client';
import { t } from '../i18n';
import { contactList, type Contact } from '../api/contacts';

/** 联系人列表的读取状态：还没读、正在读、已读到，或者失败的原因。 */
export type ContactsState =
  | { status: 'idle' }
  | { status: 'loading' }
  | { status: 'ready'; contacts: Contact[] }
  | { status: 'error'; message: string };

/** `enabled` 之后的联系人列表，以及重新读取它的函数（不用 `contactList` 保留的结果）。 */
export function useContacts(enabled: boolean, scope: number): [ContactsState, () => void] {
  const [state, setState] = useState<ContactsState>({ status: 'idle' });
  const [round, setRound] = useState(0);
  const again = useRef(false);
  const reload = useCallback(() => { again.current = true; setRound(n => n + 1); }, []);
  // 只要服务端和账号没变，离开页面再回来时保留已读到的列表
  const read = useRef<string | null>(null);
  useEffect(() => {
    const key = scope + ':' + round;
    if (!enabled || read.current === key) return;
    read.current = key;
    let left = false;
    let done = false;
    setState({ status: 'loading' });
    const fresh = again.current;
    again.current = false;
    contactList(fresh).then(contacts => { if (left) return; done = true; setState({ status: 'ready', contacts }); }, problem => {
      if (left) return;
      done = true;
      read.current = null;
      setState({ status: 'error', message: problem instanceof ApiError && problem.message ? problem.message : t('读取通讯录失败') });
    });
    // 列表读到之前就离开了页面：下次打开时重新读取
    return () => { left = true; if (!done && read.current === key) read.current = null; };
  }, [enabled, scope, round]);
  return [state, reload];
}
