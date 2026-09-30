#!/usr/bin/env python3
"""Generates mock_wcdb.c from wcdb_sigs.json (the C ABI declared by the desktop app).

The mock stands in for libwcdb_api.so in tests. Functions echo their arguments as JSON
(`{"fn": "...", "args": [...]}`) so tests can verify argument marshalling, except a few
hand-written ones that return realistic chat data.
"""
import json, os, re

here = os.path.dirname(os.path.abspath(__file__))
sigs = json.load(open(os.path.join(here, 'wcdb_sigs.json')))

HAND_WRITTEN = {
    'wcdb_init', 'wcdb_shutdown', 'wcdb_open_account', 'wcdb_close_account', 'wcdb_free_string', 'wcdb_set_my_wxid',
    'wcdb_get_sessions', 'wcdb_get_messages', 'wcdb_get_contact', 'wcdb_get_group_nicknames', 'wcdb_get_group_members',
}

# Realistic canned payloads for functions whose output the Rust logic interprets.
CANNED = {
    'wcdb_get_contacts_compact': '[{"username":"wxid_bob","local_type":"1","nick_name":"Bob","remark":"Bobby"},{"username":"wxid_carol","local_type":"1","nick_name":"Carol"},{"username":"wxid_me","local_type":"1"},{"username":"gh_news","local_type":"1"},{"username":"medianote","local_type":"1"},{"username":"room1@chatroom","local_type":"2"}]',
    'wcdb_get_contact_type_counts': '{"private":2,"group":1,"official":1,"former_friend":0}',
    'wcdb_get_session_message_counts': '{"wxid_bob":120,"room1@chatroom":55}',
    'wcdb_get_message_dates': '["2024-01-01","2024-01-02"]',
    'wcdb_get_session_message_date_counts': '{"2024-01-01":3,"2024-01-02":0,"2024-01-03":5}',
    'wcdb_get_contact_status': '{"wxid_bob":{"isFolded":true,"isMuted":false}}',
    'wcdb_get_message_table_stats': '[{"db_path":"/x/message_0.db","table_name":"Msg_abc","count":"70","first_timestamp":"1700000000","last_timestamp":"1700009999"},{"db_path":"/x/message_1.db","table_name":"Msg_abc","count":"50","first_timestamp":"1690000000","last_timestamp":"1699999999"}]',
    'wcdb_get_session_message_type_stats_batch': '{"wxid_bob":{"total_messages":120,"voice_messages":4,"image_messages":10,"video_messages":1,"emoji_messages":2,"call_messages":1,"transfer_messages":1,"red_packet_messages":0,"first_timestamp":1690000000,"last_timestamp":1700009999},"room1@chatroom":{"total_messages":55,"voice_messages":0,"image_messages":3,"video_messages":0,"emoji_messages":9,"call_messages":0,"transfer_messages":0,"red_packet_messages":2,"first_timestamp":1695000000,"last_timestamp":1700000000,"group_my_messages":17,"group_sender_count":3}}',
    'wcdb_get_messages_by_type': '[{"local_id":"11","server_id":"5001","create_time":"1700000300","local_type":"3","message_content":"<msg><img md5=\\"aabbccddeeff00112233445566778899\\" /></msg>","sender_username":"wxid_bob","is_send":"0"},{"local_id":"12","create_time":"1700000200","local_type":"3","message_content":"<msg><img cdnmidimgurl=\\"3057aabbccddeeff0011223344556677_xx.dat\\" /></msg>","sender_username":"wxid_bob","is_send":"0"},{"local_id":"11","server_id":"5001","create_time":"1700000300","local_type":"3","message_content":"<msg><img md5=\\"aabbccddeeff00112233445566778899\\" /></msg>","sender_username":"wxid_bob","is_send":"0"}]',
    'wcdb_get_message_by_id': '{"local_id":"7","server_id":"9007199254740993","create_time":"1700000050","local_type":"1","message_content":"wxid_bob:found by id","sender_username":"wxid_bob","is_send":"0"}',
    'wcdb_get_message_by_svrid': '{"local_id":"8","server_id":"777","create_time":"1700000060","local_type":"1","message_content":"found by svrid","is_send":"1"}',
    'wcdb_get_display_names': '{"wxid_bob":"Bobby","wxid_me":"Me Nick"}',
    'wcdb_get_sns_timeline': '[{"id":"p1","tid":"11","username":"wxid_bob","nickname":"","createTime":1700000500,"contentDesc":"hello moments","type":1,"rawXml":"<TimelineObject><location city=\\"Shanghai\\" poiName=\\"Bund\\" latitude=\\"31.2\\" longitude=\\"121.5\\"/></TimelineObject>","media":[{"url":"http://mmsns.qpic.cn/a/150","thumb":"http://mmsns.qpic.cn/a/150","token":"TK","md5":"abc"}],"likes":["Carol"],"comments":[{"id":"1","nickname":"Carol","content":"nice","refCommentId":"","refNickname":""},{"id":"2","nickname":"Bob","content":"thx","refCommentId":"1"}]},{"id":"p2","username":"wxid_carol","nickname":"Carol","createTime":1700000400,"contentDesc":"video","type":15,"rawXml":"<x><enc key=\\"2105122989\\"/></x>","media":[{"url":"http://snsvideodownload.qq.com/v?x=1","thumb":"http://vweixinthumb.qpic.cn/t","token":"T2","key":"1"}],"likes":[],"comments":[]}]',
    'wcdb_get_sns_usernames': '["wxid_bob","wxid_carol"]',
    'wcdb_get_sns_export_stats': '{"total_posts":2,"total_friends":2,"my_posts":0}',
    'wcdb_get_group_stats': '{"sessions":{"room1@chatroom":{"senders":{"1":30,"2":10,"3":5}}},"idMap":{"1":"wxid_bob","2":"wxid_me","3":"wxid_quiet"},"hourly":{"9":4,"21":7},"typeCounts":{"1":30,"3":5,"47":3,"10000":2,"49":4}}',
    'wcdb_get_group_member_counts': '{"room1@chatroom":3}',
    'wcdb_get_aggregate_stats': '{"total":100,"sent":40,"received":60,"firstTime":1690000000,"lastTime":1700009999,"typeCounts":{"1":70,"3":10,"34":5,"43":3,"47":7,"49":5},"hourly":{"9":30,"21":70},"weekday":{"0":20,"1":30,"6":50},"daily":{"2024-01-01":60,"2024-01-02":40},"monthly":{"2024-01":60,"2024-02":40},"sessions":{"wxid_bob":{"total":70,"sent":30,"received":40,"lastTime":1700009999},"wxid_carol":{"total":30,"sent":10,"received":20,"lastTime":1699999999}},"idMap":{}}',
    'wcdb_get_contact_alias_map': '{"wxid_bob":"bobby_id"}',
    'wcdb_get_avatar_urls': '{"wxid_bob":"https://example.com/bob.png"}',
}

ROOM_ROWS = [
    {"local_id": "3", "server_id": "9007199254740993", "create_time": "1700000120", "local_type": "1", "message_content": "wxid_bob:third", "sender_username": "wxid_bob", "is_send": "0"},
    {"local_id": "2", "create_time": "1700000060", "local_type": "1", "message_content": "second", "sender_username": "wxid_me", "is_send": "1"},
    {"local_id": "1", "create_time": "1700000000", "local_type": "10000", "message_content": '<sysmsg type="x"><plain>Bob joined</plain></sysmsg>', "sender_username": "room1@chatroom", "is_send": "0"},
]
PRIVATE_ROWS = [
    {"local_id": "3", "server_id": "9007199254740993", "create_time": "1700000120", "local_type": "3", "message_content": "", "sender_username": "wxid_bob", "is_send": "0"},
    {"local_id": "2", "create_time": "1700000060", "local_type": "1", "message_content": "wxid_bob:hi <there>", "sender_username": "wxid_bob", "is_send": "0"},
    {"local_id": "1", "create_time": "1700000000", "local_type": "1", "message_content": 'hello, "world"', "is_send": "1"},
]

def c_str(obj):
    j = json.dumps(obj, separators=(',', ':'), ensure_ascii=False)
    return j.replace('\\', '\\\\').replace('"', '\\"')

CTYPE = {'int32': 'int32_t', 'int64': 'int64_t'}

out = []
out.append('''// GENERATED by gen_mock.py - do not edit by hand.
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static char g_last[16384];
static char g_cur_session[256];
static int g_cur_done = 0;

// Test hook: JSON of the arguments of the most recent generated call.
const char* mock_last_call(void) { return g_last; }

static int ret_json(void** out, const char* s) {
    if (out) *out = strdup(s);
    return 0;
}

static size_t esc(char* dst, size_t cap, const char* src) {
    size_t n = 0;
    if (!src) { n += snprintf(dst + n, cap - n, "null"); return n; }
    n += snprintf(dst + n, cap - n, "\\"");
    for (; *src && n + 8 < cap; ++src) {
        unsigned char c = (unsigned char)*src;
        if (c == '"' || c == '\\\\') { dst[n++] = '\\\\'; dst[n++] = c; }
        else if (c == '\\n') { dst[n++] = '\\\\'; dst[n++] = 'n'; }
        else dst[n++] = c;
    }
    n += snprintf(dst + n, cap - n, "\\"");
    return n;
}

void wcdb_free_string(void* ptr) { free(ptr); }
int32_t wcdb_init(void) { return 0; }
int32_t wcdb_shutdown(void) { return 0; }
int32_t wcdb_open_account(const char* path, const char* key, int64_t* handle) {
    if (!path || !key || strlen(key) == 0) return -1;
    *handle = 1;
    return 0;
}
int32_t wcdb_close_account(int64_t handle) { return 0; }
int32_t wcdb_set_my_wxid(int64_t handle, const char* wxid) { return 0; }

int32_t wcdb_get_sessions(int64_t handle, void** out) {
    return ret_json(out, "[{\\"username\\":\\"wxid_bob\\",\\"summary\\":\\"hi\\"},{\\"username\\":\\"room1@chatroom\\"}]");
}

// Newest first, 3 rows at offset 0, nothing after.
int32_t wcdb_get_messages(int64_t handle, const char* session, int32_t limit, int32_t offset, void** out) {
    if (offset > 0) return ret_json(out, "[]");
    if (session && strstr(session, "chatroom")) {
        return ret_json(out,
            "[{\\"local_id\\":\\"3\\",\\"server_id\\":\\"9007199254740993\\",\\"create_time\\":\\"1700000120\\",\\"local_type\\":\\"1\\",\\"message_content\\":\\"wxid_bob:third\\",\\"sender_username\\":\\"wxid_bob\\",\\"is_send\\":\\"0\\"},"
            "{\\"local_id\\":\\"2\\",\\"create_time\\":\\"1700000060\\",\\"local_type\\":\\"1\\",\\"message_content\\":\\"second\\",\\"sender_username\\":\\"wxid_me\\",\\"is_send\\":\\"1\\"},"
            "{\\"local_id\\":\\"1\\",\\"create_time\\":\\"1700000000\\",\\"local_type\\":\\"10000\\",\\"message_content\\":\\"<sysmsg type=\\\\\\"x\\\\\\"><plain>Bob joined</plain></sysmsg>\\",\\"sender_username\\":\\"room1@chatroom\\",\\"is_send\\":\\"0\\"}]");
    }
    return ret_json(out,
        "[{\\"local_id\\":\\"3\\",\\"server_id\\":\\"9007199254740993\\",\\"create_time\\":\\"1700000120\\",\\"local_type\\":\\"3\\",\\"message_content\\":\\"\\",\\"sender_username\\":\\"wxid_bob\\",\\"is_send\\":\\"0\\"},"
        "{\\"local_id\\":\\"2\\",\\"create_time\\":\\"1700000060\\",\\"local_type\\":\\"1\\",\\"message_content\\":\\"wxid_bob:hi <there>\\",\\"sender_username\\":\\"wxid_bob\\",\\"is_send\\":\\"0\\"},"
        "{\\"local_id\\":\\"1\\",\\"create_time\\":\\"1700000000\\",\\"local_type\\":\\"1\\",\\"message_content\\":\\"hello, \\\\\\"world\\\\\\"\\",\\"is_send\\":\\"1\\"}]");
}

int32_t wcdb_get_contact(int64_t handle, const char* username, void** out) {
    if (username && strcmp(username, "wxid_bob") == 0)
        return ret_json(out, "{\\"username\\":\\"wxid_bob\\",\\"nickName\\":\\"Bob\\",\\"remark\\":\\"Bobby\\",\\"alias\\":\\"bb\\"}");
    if (username && strcmp(username, "wxid_me") == 0)
        return ret_json(out, "{\\"username\\":\\"wxid_me\\",\\"nickName\\":\\"Me Nick\\"}");
    return ret_json(out, "{}");
}

int32_t wcdb_get_group_nicknames(int64_t handle, const char* chatroom, void** out) {
    return ret_json(out, "{\\"wxid_bob\\":\\"Bob in room\\"}");
}

int32_t wcdb_get_group_members(int64_t handle, const char* chatroom, void** out) {
    return ret_json(out, "{\\"members\\":[{\\"username\\":\\"wxid_bob\\"},{\\"username\\":\\"wxid_me\\"},{\\"username\\":\\"wxid_quiet\\"}]}");
}
''')

for name, (ret, params) in sigs.items():
    if name in HAND_WRITTEN:
        continue
    decl = []
    body_args = []
    out_json = None
    post = []
    for i, p in enumerate(params):
        m = re.match(r'(.*?)(\w+)$', p.strip())
        ty, pname = m.group(1).strip(), m.group(2)
        if ty == '_Out_ void**':
            decl.append(f'void** {pname}'); out_json = pname
        elif ty == '_Out_ int32*':
            decl.append(f'int32_t* {pname}')
            post.append(f'if ({pname}) *{pname} = {"42" if "count" in pname.lower() else "0"};')
        elif ty == '_Out_ int64*':
            decl.append(f'int64_t* {pname}'); post.append(f'if ({pname}) *{pname} = 7;')
        elif ty == 'const char*':
            decl.append(f'const char* {pname}'); body_args.append(('s', pname))
        elif ty in CTYPE:
            decl.append(f'{CTYPE[ty]} {pname}'); body_args.append(('i64' if ty == 'int64' else 'i32', pname))
        else:
            raise SystemExit(f'unhandled param {p} in {name}')
    rtype = 'void' if ret == 'void' else 'int32_t'
    out.append(f'{rtype} {name}({", ".join(decl) or "void"}) {{')
    if ret == 'void':
        out.append('}\n')
        continue
    out.append('    char buf[16384]; size_t n = 0;')
    out.append(f'    n += snprintf(buf + n, sizeof(buf) - n, "{{\\"fn\\":\\"{name}\\",\\"args\\":[");')
    first = True
    for kind, pname in body_args:
        sep = '' if first else ','
        first = False
        if kind == 's':
            out.append(f'    n += snprintf(buf + n, sizeof(buf) - n, "{sep}"); n += esc(buf + n, sizeof(buf) - n, {pname});')
        elif kind == 'i64':
            out.append(f'    n += snprintf(buf + n, sizeof(buf) - n, "{sep}%lld", (long long){pname});')
        else:
            out.append(f'    n += snprintf(buf + n, sizeof(buf) - n, "{sep}%d", (int){pname});')
    out.append('    n += snprintf(buf + n, sizeof(buf) - n, "]}");')
    for line in post:
        out.append('    ' + line)
    if name in ('wcdb_open_message_cursor', 'wcdb_open_message_cursor_lite'):
        out.append('    snprintf(g_cur_session, sizeof(g_cur_session), "%s", sessionId ? sessionId : ""); g_cur_done = 0;')
    out.append('    snprintf(g_last, sizeof(g_last), "%s", buf);')
    if name == 'wcdb_fetch_message_batch':
        out.append('    if (strcmp(g_cur_session, "room1@chatroom") == 0 || strcmp(g_cur_session, "wxid_bob") == 0) {')
        out.append('        if (outHasMore) *outHasMore = 0;')
        out.append('        if (g_cur_done) return ret_json(outJson, "[]");')
        out.append('        g_cur_done = 1;')
        out.append('        return ret_json(outJson, strcmp(g_cur_session, "room1@chatroom") == 0 ? "%s" : "%s");' % (c_str(ROOM_ROWS), c_str(PRIVATE_ROWS)))
        out.append('    }')
    if out_json and name in CANNED:
        payload = CANNED[name].replace('\\', '\\\\').replace('"', '\\"')
        out.append(f'    return ret_json({out_json}, "{payload}");')
    elif out_json:
        out.append(f'    return ret_json({out_json}, buf);')
    else:
        out.append('    return 0;')
    out.append('}\n')

open(os.path.join(here, 'mock_wcdb.c'), 'w').write('\n'.join(out))
print('wrote mock_wcdb.c')
