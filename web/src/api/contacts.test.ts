import { describe, expect, it } from 'vitest';
import { contactLine } from '../contacts/ContactList';
import { countKinds, matchContacts, type Contact } from './contacts';

const CONTACTS: Contact[] = [
  { username: 'wxid_b', displayName: '小陈', remark: '小陈', nickname: '陈屿', alias: 'ChenYu', region: '中国 上海', type: 'friend' },
  { username: 'g@chatroom', displayName: '家人群', type: 'group' },
  { username: 'wxid_c', displayName: '秋秋', nickname: '秋秋', type: 'friend' },
  { username: 'gh_x', displayName: '城市手记', detailDescription: '一条小街', type: 'official' },
  { username: 'wxid_d', displayName: '阿杰', type: 'former_friend' },
];

describe('contacts', () => {
  it('finds a contact by any of its names, its WeChat ID or its username, ignoring case', () => {
    const names = (query: string) => matchContacts(CONTACTS, query).map(c => c.displayName);
    expect(names('陈屿')).toEqual(['小陈']);
    expect(names('chenyu')).toEqual(['小陈']);
    expect(names('GH_')).toEqual(['城市手记']);
    expect(names('  ')).toHaveLength(CONTACTS.length);
  });

  it('counts each kind', () => {
    expect(countKinds(CONTACTS)).toEqual({ friend: 2, group: 1, official: 1, former_friend: 1 });
  });

  it('says under the name the nickname a remark hides and the region, else the kind', () => {
    expect(CONTACTS.map(contactLine)).toEqual(['陈屿 · 中国 上海', '群聊', '好友', '一条小街', '曾经的好友']);
  });
});
