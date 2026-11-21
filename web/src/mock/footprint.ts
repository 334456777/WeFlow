// The sample's footprint, counted from its own conversations as `insight footprint` counts a range of days: who wrote
// to me and whom I answered, each stretch of a private chat, and where I was @-mentioned.
import type { Footprint, FootprintMention, FootprintSegment, FootprintSession, InsightReply, MentionGroup } from '../api/contract.gen';
import { dayEnd, dayStart } from './clock';
import { threads, type Message } from './chats';
import { ME, nameOf } from './people';

/** A pause this long ends a stretch of a chat. */
const PAUSE = 3600;

const isPrivate = (session: string) => !session.endsWith('@chatroom') && !session.startsWith('gh_') && session !== 'filehelper';

function stretches(messages: Message[]): Message[][] {
  const parts: Message[][] = [];
  for (const m of messages) {
    const part = parts[parts.length - 1];
    if (part && m.createTime - part[part.length - 1].createTime < PAUSE) part.push(m);
    else parts.push([m]);
  }
  return parts;
}

/** When I first answered after the first message to me, or 0. */
function answer(messages: Message[]): { firstIn: number; firstReply: number } {
  const firstIn = messages.find(m => m.isSend === 0)?.createTime ?? 0;
  const firstReply = firstIn ? messages.find(m => m.isSend === 1 && m.createTime > firstIn)?.createTime ?? 0 : 0;
  return { firstIn, firstReply };
}

/** `insight footprint` of the sample between two days (`YYYY-MM-DD`, either may be empty). */
export function footprint(start: string, end: string): Footprint {
  const from = start ? dayStart(start) : 0;
  const to = end ? dayEnd(end) : Number.MAX_SAFE_INTEGER;
  const sessions: FootprintSession[] = [];
  const segments: FootprintSegment[] = [];
  const mentions: FootprintMention[] = [];
  for (const [session, all] of threads) {
    const messages = all.filter(m => m.createTime >= from && m.createTime <= to && m.localType !== 10000);
    if (!messages.length) continue;
    if (isPrivate(session)) {
      const { firstIn, firstReply } = answer(messages);
      const incoming = messages.filter(m => m.isSend === 0).length;
      sessions.push({
        session_id: session, incoming_count: incoming, outgoing_count: messages.length - incoming, replied: Boolean(firstReply),
        first_incoming_ts: firstIn, first_reply_ts: firstReply, latest_ts: messages[messages.length - 1].createTime,
        anchor_local_id: messages[0].localId, anchor_create_time: messages[0].createTime,
      });
      stretches(messages).forEach((part, i) => {
        const inner = answer(part);
        const partIn = part.filter(m => m.isSend === 0).length;
        const first = part[0];
        const last = part[part.length - 1];
        segments.push({
          session_id: session, segment_index: i, start_ts: first.createTime, end_ts: last.createTime, duration_sec: last.createTime - first.createTime,
          incoming_count: partIn, outgoing_count: part.length - partIn, message_count: part.length, replied: Boolean(inner.firstReply),
          first_incoming_ts: inner.firstIn, first_reply_ts: inner.firstReply, latest_ts: last.createTime,
          anchor_local_id: first.localId, anchor_create_time: first.createTime,
        });
      });
    } else if (session.endsWith('@chatroom')) {
      for (const m of messages) {
        if (m.senderUsername !== ME && m.parsedContent.includes('@' + nameOf(ME))) {
          mentions.push({ session_id: session, local_id: m.localId, create_time: m.createTime, sender_username: m.senderUsername ?? '', message_content: m.parsedContent, source: 'at' });
        }
      }
    }
  }
  const groups = new Map<string, MentionGroup>();
  for (const m of mentions) {
    const group = groups.get(m.session_id) ?? { session_id: m.session_id, count: 0, latest_ts: 0 };
    group.count += 1;
    group.latest_ts = Math.max(group.latest_ts, m.create_time);
    groups.set(m.session_id, group);
  }
  const inbound = sessions.filter(s => s.incoming_count > 0);
  const replied = sessions.filter(s => s.replied);
  return {
    summary: {
      private_inbound_people: inbound.length, private_replied_people: replied.length,
      private_outbound_people: sessions.filter(s => s.outgoing_count > 0).length,
      private_reply_rate: inbound.length ? replied.length / inbound.length : 0,
      mention_count: mentions.length, mention_group_count: groups.size,
    },
    private_sessions: sessions.sort((a, b) => b.latest_ts - a.latest_ts),
    private_segments: segments.sort((a, b) => a.start_ts - b.start_ts),
    mentions: mentions.sort((a, b) => a.create_time - b.create_time),
    mention_groups: [...groups.values()].sort((a, b) => b.count - a.count),
    diagnostics: {
      truncated: false, private_truncated: false, mention_truncated: false, scanned_dbs: 4, elapsed_ms: 38,
      private_session_count: sessions.length, group_session_count: groups.size,
    },
  };
}

/** What `insight footprint-summary` answers: the AI's few sentences, written for the sample. */
export function footprintSummary(payload: string): InsightReply {
  let label = '这段时间';
  try { label = (JSON.parse(payload) as { rangeLabel?: string }).rangeLabel || label; } catch { /* the sample's own sentence */ }
  return {
    message: '生成成功',
    insight: label + '里，大部分对话都围绕 WeFlow：和林沐聊新皮肤和深色模式，和陈屿聊按页解密的性能，'
      + '还帮顾南把十年的聊天记录导了出来。开发群里有人 @ 你，问发版和示例数据的进展。'
      + '找你的人你基本都回复了，可以抽空看看用户交流群里还没解决的图片密钥问题。（示例数据）',
  };
}
