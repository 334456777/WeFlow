// 通讯录（#205）：`chat contacts --parsed`，与桌面版显示的联系人列表一致，在服务进程内读取（`/api/`）。
import { get } from './client';
import { msg } from '../i18n';
import { keptRead } from './kept';

/** `chat contacts --parsed` 的一个联系人及其种类：契约的类型。 */
export type { Contact, ContactKind } from './contract.gen';
import type { Contact, ContactKind } from './contract.gen';

/** 联系人的种类，按通讯录页的分类标签排列。 */
export const CONTACT_KINDS: ContactKind[] = ['friend', 'group', 'official', 'former_friend'];
/** 每种联系人的名称，用 `msg()` 标出，显示时经过 `t()`。 */
export const contactKindLabels: Record<ContactKind, string> = { friend: msg('好友'), group: msg('群聊'), official: msg('公众号'), former_friend: msg('曾经的好友') };


/** `chat contacts --parsed`：联系人列表，去掉没有用户名或种类的行。 */
export async function getContacts(signal?: AbortSignal): Promise<Contact[]> {
  const data = await get<unknown>('api/chat/contacts', { parsed: true }, signal);
  return Array.isArray(data) ? data.filter((c): c is Contact => typeof c?.username === 'string' && typeof c?.type === 'string') : [];
}

/**
 * 解析过的联系人列表，供按它显示人名的各页面使用，如通讯录和足迹（#402）：每个服务端和账号只读一次（`keptRead`）；
 * `fresh` 时重新读取（重试时用它）。
 */
export const contactList = (fresh = false) => keptRead('contacts', () => getContacts(), fresh);

/** 名称、微信号或用户名包含 `query` 的联系人，不区分大小写。 */
export function matchContacts(contacts: readonly Contact[], query: string): Contact[] {
  const needle = query.trim().toLocaleLowerCase();
  if (!needle) return [...contacts];
  return contacts.filter(c => [c.displayName, c.remark, c.nickname, c.alias, c.username]
    .some(v => v?.toLocaleLowerCase().includes(needle)));
}

/** 每种联系人的个数。 */
export function countKinds(contacts: readonly Contact[]): Record<ContactKind, number> {
  const counts: Record<ContactKind, number> = { friend: 0, group: 0, official: 0, former_friend: 0 };
  for (const c of contacts) if (c.type in counts) counts[c.type] += 1;
  return counts;
}

/** `export contacts --format` 可选的格式。 */
export const CONTACT_FORMATS = [
  { value: 'excel', label: 'Excel' }, { value: 'csv', label: 'CSV' }, { value: 'json', label: 'JSON' }, { value: 'html', label: 'HTML' },
  { value: 'txt', label: 'TXT' }, { value: 'sql', label: 'SQL' },
] as const;
/** `export contacts` 的一种格式。 */
export type ContactFormat = typeof CONTACT_FORMATS[number]['value'];
