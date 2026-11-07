// One contact (#205): what WeChat keeps of them, and the way to their conversation.
import { ChatCircleDots } from '@phosphor-icons/react';
import { contactKindLabels, type Contact } from '../api/contacts';
import { Avatar } from '../Avatar';
import { msg, t } from '../i18n';

export function ContactDetail({ contact, canChat, onChat }: {
  contact: Contact;
  /** Whether the chat list has a conversation with them. */
  canChat: boolean;
  onChat(): void;
}) {
  const facts: [string, string | undefined][] = [
    [msg('备注'), contact.remark],
    [msg('昵称'), contact.nickname],
    [msg('微信号'), contact.alias],
    [msg('地区'), contact.region],
    [contact.type === 'official' ? msg('简介') : msg('签名'), contact.detailDescription],
  ];
  return <article className="contact-card" aria-label={t('{name}的资料', { name: contact.displayName })}>
    <header>
      <Avatar name={contact.displayName} url={contact.avatarUrl} />
      <div><h2>{contact.displayName}</h2><p>{t(contactKindLabels[contact.type])}</p></div>
    </header>
    <dl className="contact-facts">
      {facts.filter(([, value]) => value).map(([label, value]) => <div key={label}><dt>{t(label)}</dt><dd>{value}</dd></div>)}
      {contact.labels?.length ? <div><dt>{t('标签')}</dt><dd className="contact-labels">{contact.labels.map(l => <span key={l} className="badge">{l}</span>)}</dd></div> : null}
      <div><dt>ID</dt><dd className="info-id">{contact.username}</dd></div>
    </dl>
    <div className="contact-actions">
      <button type="button" className="primary-button" disabled={!canChat} onClick={onChat}><ChatCircleDots size={16} />{t('查看聊天记录')}</button>
      {!canChat && <p className="card-note">{contact.type === 'group' ? t('本机没有这个群的聊天记录。') : t('本机没有和他的聊天记录。')}</p>}
    </div>
  </article>;
}
