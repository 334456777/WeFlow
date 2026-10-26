// The contacts page (#205): the list, read once the page is opened, and one contact beside it; counted and exported
// from the console.
import { useEffect, useState } from 'react';
import type { ContactKind } from '../api/contacts';
import type { ChatWith, Page, Shell } from '../shell/page';
import { BrowserPanel, ConnectedList } from '../shell/panels';
import { ContactsExport, contactsExportSummary, ContactsSummary, DEFAULT_CONTACTS_EXPORT } from './ContactCards';
import { ContactDetail } from './ContactDetail';
import { ContactList } from './ContactList';
import { useContacts } from './useContacts';

export function useContactsPage(shell: Shell, active: boolean, chatWith: ChatWith): Page {
  const { backend, connected, scope, setMobileDetail } = shell;
  const [state, reload] = useContacts(active && connected, scope);
  const [kind, setKind] = useState<ContactKind>('friend');
  const [id, setId] = useState<string | null>(null);
  useEffect(() => { setId(null); }, [scope]);
  const [exportOptions, setExportOptions] = useState(DEFAULT_CONTACTS_EXPORT);
  const contacts = state.status === 'ready' ? state.contacts : null;
  const contact = contacts?.find(c => c.username === id);
  const cards = [
    {
      id: 'contacts-summary', title: '通讯录统计', summary: contacts ? contacts.length.toLocaleString('zh-CN') + ' 个' : '',
      content: <ContactsSummary contacts={contacts} />,
    },
    {
      id: 'contacts-export', title: '导出通讯录', summary: contactsExportSummary(exportOptions),
      content: <ContactsExport options={exportOptions} onChange={setExportOptions} ready={Boolean(contacts)} />,
    },
  ];
  const view = <BrowserPanel label="通讯录" detail={shell.mobileDetail} onBack={() => setMobileDetail(false)} back="返回联系人列表" title={contact?.displayName ?? '联系人资料'} paneLabel="联系人资料"
    list={<ConnectedList backend={backend} offline="连接 weflow serve 后，可以在这里查看通讯录。">
      <ContactList state={state} kind={kind} onKind={setKind} selected={id} onRetry={reload}
        onSelect={username => { setId(username); setMobileDetail(true); }} />
    </ConnectedList>}>
    {connected && (contact
      ? <ContactDetail key={contact.username} contact={contact} canChat={shell.sessionList.some(s => s.username === contact.username)}
        onChat={() => chatWith(contact.username)} />
      : <div className="pane-message">{contacts && <p>选择左侧的一个联系人。</p>}</div>)}
  </BrowserPanel>;
  return { view, cards, shown: ['tasks', ...(connected ? cards.map(card => card.id) : [])] };
}
