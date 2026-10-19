// One contact (#205): what WeChat keeps of them, and the way to their conversation.
import { ChatCircleDots } from '@phosphor-icons/react';
import { contactKindLabels, type Contact } from '../api/contacts';
import { Avatar } from '../Avatar';

export function ContactDetail({ contact, canChat, onChat }: {
  contact: Contact;
  /** Whether the chat list has a conversation with them. */
  canChat: boolean;
  onChat(): void;
}) {
  const facts: [string, string | undefined][] = [
    ['备注', contact.remark],
    ['昵称', contact.nickname],
    ['微信号', contact.alias],
    ['地区', contact.region],
    [contact.type === 'official' ? '简介' : '签名', contact.detailDescription],
  ];
  return <article className="contact-card" aria-label={contact.displayName + '的资料'}>
    <header>
      <Avatar name={contact.displayName} url={contact.avatarUrl} />
      <div><h2>{contact.displayName}</h2><p>{contactKindLabels[contact.type]}</p></div>
    </header>
    <dl className="contact-facts">
      {facts.filter(([, value]) => value).map(([label, value]) => <div key={label}><dt>{label}</dt><dd>{value}</dd></div>)}
      {contact.labels?.length ? <div><dt>标签</dt><dd className="contact-labels">{contact.labels.map(l => <span key={l} className="badge">{l}</span>)}</dd></div> : null}
      <div><dt>ID</dt><dd className="info-id">{contact.username}</dd></div>
    </dl>
    <div className="contact-actions">
      <button type="button" className="primary-button" disabled={!canChat} onClick={onChat}><ChatCircleDots size={16} />查看聊天记录</button>
      {!canChat && <p className="card-note">本机没有和{contact.type === 'group' ? '这个群' : '他'}的聊天记录。</p>}
    </div>
  </article>;
}
