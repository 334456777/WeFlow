# WeFlow HTTP API / Push Documentation

**English** | [简体中文](zh-CN/HTTP-API.md)

WeFlow provides a local HTTP API (GET and POST are supported) so that external scripts and tools can read chat messages, sessions, contacts, group members and exported media files. It can also push message events proactively over a fixed SSE address when new messages are detected.

## Enabling

Enable `API Service` on the application's settings page.

- Default listen address: `127.0.0.1`
- Default port: `5031`
- Base URL: `http://127.0.0.1:5031`
- Optionally enable `Active push`: when a newly received message is detected it is pushed to SSE subscribers via `GET /api/v1/push/messages`

**State persistence**: the state and port of the API service and active push are saved automatically and restored when WeFlow restarts.

## Authentication

**Access Token**: every `/api/v1/*` endpoint except the health check is protected by a token. Three ways to pass it (pick one):

1. **HTTP header (recommended)**: `Authorization: Bearer <your token>`
2. **Query parameter**: `?access_token=<your token>` (recommended for long-lived SSE connections)
3. **JSON body**: `{"access_token": "<your token>"}` (POST requests only)

## Endpoints

- `GET|POST /health`
- `GET|POST /api/v1/health`
- `GET|POST /api/v1/push/messages`
- `GET|POST /api/v1/messages`
- `GET|POST /api/v1/sessions`
- `GET /api/v1/sessions/:id/messages` (ChatLab Pull)
- `GET|POST /api/v1/contacts`
- `GET|POST /api/v1/group-members`
- `GET|POST /api/v1/media/*`

---

## 1. Health check

**Request**

```http
GET /health
```

or

```http
GET /api/v1/health
```

**Response**

```json
{
  "status": "ok"
}
```

---

## 2. Active push

Receive new-message events over a long-lived SSE connection. It shares the port with the HTTP API.

**Request**

```http
GET /api/v1/push/messages
```

### Notes

- `HTTP API service` must be enabled on the settings page first
- `Active push` must be enabled as well
- The response type is `text/event-stream`
- Event names are `message.new` and `message.revoke`
- Receivers should de-duplicate by `event + rawid`

### Event fields

- `event`
- `sessionId`
- `rawid`
- `avatarUrl`
- `sourceName`
- `groupName` (group chats only)
- `content`
- `timestamp` (message time, Unix timestamp in seconds)

### Example

```bash
curl -N "http://127.0.0.1:5031/api/v1/push/messages?access_token=YOUR_TOKEN"
```

Example event:

```text
event: message.new
data: {"event":"message.new","sessionId":"xxx@chatroom","sessionType":"group","rawid":"1234567890123456789","avatarUrl":"https://example.com/group.jpg","sourceName":"Li Si","groupName":"Project group","content":"[Image]","timestamp":1760000123}
```

Example revoke event:

```text
event: message.revoke
data: {"event":"message.revoke","sessionId":"wxid_xxx","sessionType":"other","rawid":"1234567890123456789","avatarUrl":"https://example.com/avatar.jpg","sourceName":"Zhang San","content":"The other party recalled a message (rawid: 1234567890123456789), content: \"Hello\"","timestamp":1760000180}
```

---

## 3. Get messages

> With POST, put the parameters in the JSON body (Content-Type: application/json)

Reads the messages of a session; raw JSON and ChatLab formats are supported.

**Request**

```http
GET /api/v1/messages
```

### Parameters

| Parameter | Type   | Required | Description                                                                                   |
| --------- | ------ | -------- | --------------------------------------------------------------------------------------------- |
| `talker`  | string | Yes      | Session ID. For a private chat usually the other party's `wxid`; for a group `xxx@chatroom`   |
| `limit`   | number | No       | Number of messages, default `100`, range `1~10000`                                            |
| `offset`  | number | No       | Pagination offset, default `0`                                                                |
| `start`   | string | No       | Start time, `YYYYMMDD` or a timestamp                                                         |
| `end`     | string | No       | End time, `YYYYMMDD` or a timestamp                                                           |
| `keyword` | string | No       | Filter on the message display text                                                            |
| `chatlab` | string | No       | `1/true` outputs ChatLab format                                                               |
| `format`  | string | No       | `json` or `chatlab`                                                                           |
| `media`   | string | No       | `1/true` exports media and returns media URLs; alias `meiti`                                  |
| `image`   | string | No       | With `media=1`, controls image export; alias `tupian`                                         |
| `voice`   | string | No       | With `media=1`, controls voice export; alias `vioce`                                          |
| `video`   | string | No       | With `media=1`, controls video export                                                         |
| `emoji`   | string | No       | With `media=1`, controls sticker export                                                       |

### Examples

```bash
curl "http://127.0.0.1:5031/api/v1/messages?talker=wxid_xxx&limit=20"
curl "http://127.0.0.1:5031/api/v1/messages?talker=xxx@chatroom&chatlab=1"
curl "http://127.0.0.1:5031/api/v1/messages?talker=wxid_xxx&start=20260101&end=20260131"
curl "http://127.0.0.1:5031/api/v1/messages?talker=xxx@chatroom&media=1&image=1&voice=0&video=0&emoji=0"
```

### JSON response fields

Top level:

- `success`
- `talker`
- `count`
- `hasMore`
- `media.enabled`
- `media.exportPath`
- `media.count`
- `messages`

Per message:

- `localId`
- `serverId`
- `localType`
- `createTime`
- `isSend`
- `senderUsername`
- `content`
- `rawContent`
- `parsedContent`
- `replyToMessageId` (the `serverId` of the message being replied to; quote messages only)
- `quote` (snapshot of the quoted message: its ID, sender, content and type)
- `mediaType`
- `mediaFileName`
- `mediaUrl`
- `mediaLocalPath`

**Example response**

```json
{
  "success": true,
  "talker": "xxx@chatroom",
  "count": 3,
  "hasMore": true,
  "media": {
    "enabled": true,
    "exportPath": "C:\\Users\\Alice\\Documents\\WeFlow\\api-media",
    "count": 1
  },
  "messages": [
    {
      "localId": 123,
      "serverId": "6116895530414915131",
      "localType": 1,
      "createTime": 1738713600,
      "isSend": 0,
      "senderUsername": "wxid_member",
      "content": "Hello",
      "rawContent": "Hello",
      "parsedContent": "Hello"
    },
    {
      "localId": 125,
      "serverId": "6116895530414915133",
      "localType": 244813135921,
      "createTime": 1738713700,
      "isSend": 0,
      "senderUsername": "wxid_member",
      "content": "Got it",
      "rawContent": "<msg>...</msg>",
      "parsedContent": "Got it",
      "replyToMessageId": "6116895530414915131",
      "quote": {
        "platformMessageId": "6116895530414915131",
        "sender": "wxid_other",
        "accountName": "Zhang San",
        "content": "Hello",
        "type": 0
      }
    },
    {
      "localId": 124,
      "localType": 3,
      "createTime": 1738713660,
      "isSend": 0,
      "senderUsername": "wxid_member",
      "content": "[Image]",
      "mediaType": "image",
      "mediaFileName": "abc123.jpg",
      "mediaUrl": "http://127.0.0.1:5031/api/v1/media/xxx@chatroom/images/abc123.jpg",
      "mediaLocalPath": "C:\\Users\\Alice\\Documents\\WeFlow\\api-media\\xxx@chatroom\\images\\abc123.jpg"
    }
  ]
}
```

### ChatLab response

With `chatlab=1` or `format=chatlab` a ChatLab structure is returned:

- `chatlab.version`
- `chatlab.exportedAt`
- `chatlab.generator`
- `meta.name`
- `meta.platform`
- `meta.type`
- `meta.groupId`
- `meta.groupAvatar`
- `meta.ownerId`
- `members[].platformId`
- `members[].accountName`
- `members[].groupNickname`
- `members[].avatar`
- `messages[].sender`
- `messages[].accountName`
- `messages[].groupNickname`
- `messages[].timestamp`
- `messages[].type`
- `messages[].content`
- `messages[].platformMessageId`
- `messages[].replyToMessageId`
- `messages[].mediaPath`

In group chats `groupNickname` comes from the member's group nickname first; if the source data lacks it, it falls back to empty or the display name.

---

## 4. Get the session list

> With POST, put the parameters in the JSON body (Content-Type: application/json)

**Request**

```http
GET /api/v1/sessions
```

### Parameters

| Parameter | Type   | Required | Description                                |
| --------- | ------ | -------- | ------------------------------------------ |
| `keyword` | string | No       | Matches `username` or `displayName`        |
| `limit`   | number | No       | Default `100`                              |

### Response fields

- `success`
- `count`
- `sessions[].username`
- `sessions[].displayName`
- `sessions[].type`
- `sessions[].lastTimestamp`
- `sessions[].unreadCount`

**Example response**

```json
{
  "success": true,
  "count": 1,
  "sessions": [
    {
      "username": "xxx@chatroom",
      "displayName": "Project group",
      "type": 2,
      "lastTimestamp": 1738713600,
      "unreadCount": 0
    }
  ]
}
```

---

## 4.1 Get the session list (ChatLab format)

With `format=chatlab` the response follows the ChatLab Pull protocol and can be used directly as a ChatLab remote data source.

**Request**

```http
GET /api/v1/sessions?format=chatlab
```

### Parameters

| Parameter | Type   | Required | Description                         |
| --------- | ------ | -------- | ----------------------------------- |
| `format`  | string | Yes      | Set to `chatlab`                    |
| `keyword` | string | No       | Matches `username` or `displayName` |
| `limit`   | number | No       | Default `100`                       |

### Response

```json
{
  "sessions": [
    {
      "id": "xxx@chatroom",
      "name": "Project group",
      "platform": "wechat",
      "type": "group",
      "messageCount": 58000,
      "lastMessageAt": 1738713600
    }
  ]
}
```

| Field           | Description                                        |
| --------------- | -------------------------------------------------- |
| `id`            | Session ID (WeChat username)                       |
| `name`          | Session display name                               |
| `platform`      | Always `wechat`                                    |
| `type`          | `group` (group chat) or `private` (private chat)   |
| `messageCount`  | Message count (an estimate, may be inexact)        |
| `lastMessageAt` | Unix timestamp (seconds) of the last message       |

---

## 4.2 Pull session messages (ChatLab Pull)

Returns chat data in the standard ChatLab format, with incremental pulls and pagination.

**Request**

```http
GET /api/v1/sessions/:id/messages
```

### Parameters

| Parameter | Type   | Required | Description                                                   |
| --------- | ------ | -------- | ------------------------------------------------------------- |
| `:id`     | string | Yes      | Session ID (path parameter)                                   |
| `since`   | number | No       | Unix timestamp (seconds); only messages after it are returned |
| `end`     | number | No       | Unix timestamp (seconds), upper time bound                    |
| `limit`   | number | No       | Per-call limit, default and maximum `5000`                    |
| `offset`  | number | No       | Pagination offset, default `0`                                |

### Response

Standard ChatLab JSON plus a `sync` pagination block:

```json
{
  "chatlab": {
    "version": "0.0.2",
    "exportedAt": 1738713600,
    "generator": "WeFlow"
  },
  "meta": {
    "name": "Project group",
    "platform": "wechat",
    "type": "group",
    "groupId": "xxx@chatroom",
    "ownerId": "wxid_xxx"
  },
  "members": [
    {
      "platformId": "wxid_a",
      "accountName": "Zhang San",
      "groupNickname": "Product",
      "avatar": "https://example.com/avatar.jpg"
    }
  ],
  "messages": [
    {
      "sender": "wxid_a",
      "accountName": "Zhang San",
      "timestamp": 1738713600,
      "type": 0,
      "content": "Hello",
      "platformMessageId": "123456"
    }
  ],
  "sync": {
    "hasMore": true,
    "nextSince": 1738713600,
    "nextOffset": 5000,
    "watermark": 1738714000
  }
}
```

### The sync block

| Field        | Description                                        |
| ------------ | -------------------------------------------------- |
| `hasMore`    | Whether more data is available                     |
| `nextSince`  | `since` value for the next request                 |
| `nextOffset` | `offset` value for the next request                |
| `watermark`  | Upper time bound of this pull (seconds timestamp)  |

**Connecting ChatLab**: in ChatLab's settings add a remote data source, set `baseUrl` to `http://127.0.0.1:5031/api/v1` and the token to the API token configured in WeFlow.

---

## 5. Get the contact list

> With POST, put the parameters in the JSON body (Content-Type: application/json)

**Request**

```http
GET /api/v1/contacts
```

### Parameters

| Parameter | Type   | Required | Description                                                   |
| --------- | ------ | -------- | ------------------------------------------------------------- |
| `keyword` | string | No       | Matches `username`, `nickname`, `remark`, `displayName`       |
| `limit`   | number | No       | Default `100`                                                 |

### Response fields

- `success`
- `count`
- `contacts[].username`
- `contacts[].displayName`
- `contacts[].remark`
- `contacts[].nickname`
- `contacts[].alias`
- `contacts[].avatarUrl`
- `contacts[].type`

**Example response**

```json
{
  "success": true,
  "count": 1,
  "contacts": [
    {
      "username": "wxid_xxx",
      "displayName": "Zhang San",
      "remark": "Client Zhang San",
      "nickname": "Zhang San",
      "alias": "zhangsan",
      "avatarUrl": "https://example.com/avatar.jpg",
      "type": "friend"
    }
  ]
}
```

---

## 6. Get the group member list

> With POST, put the parameters in the JSON body (Content-Type: application/json)

Returns each group member's `wxid`, group nickname, remark, WeChat ID and more.

**Request**

```http
GET /api/v1/group-members
```

### Parameters

| Parameter              | Type   | Required | Description                                              |
| ---------------------- | ------ | -------- | -------------------------------------------------------- |
| `chatroomId`           | string | Yes      | Group ID; `talker` is accepted as well                   |
| `includeMessageCounts` | string | No       | `1/true` adds each member's message count                |
| `withCounts`           | string | No       | Alias of `includeMessageCounts`                          |
| `forceRefresh`         | string | No       | `1/true` bypasses the in-memory cache and refreshes      |

### Response fields

- `success`
- `chatroomId`
- `count`
- `fromCache`
- `updatedAt`
- `members[].wxid`
- `members[].displayName`
- `members[].nickname`
- `members[].remark`
- `members[].alias`
- `members[].groupNickname`
- `members[].avatarUrl`
- `members[].isOwner`
- `members[].isFriend`
- `members[].messageCount`

**Example requests**

```bash
curl "http://127.0.0.1:5031/api/v1/group-members?chatroomId=xxx@chatroom"
curl "http://127.0.0.1:5031/api/v1/group-members?chatroomId=xxx@chatroom&includeMessageCounts=1&forceRefresh=1"
```

**Example response**

```json
{
  "success": true,
  "chatroomId": "xxx@chatroom",
  "count": 2,
  "fromCache": false,
  "updatedAt": 1760000000000,
  "members": [
    {
      "wxid": "wxid_member_a",
      "displayName": "Client A",
      "nickname": "Jia",
      "remark": "Client A",
      "alias": "kehua",
      "groupNickname": "Party A",
      "avatarUrl": "https://example.com/a.jpg",
      "isOwner": true,
      "isFriend": true,
      "messageCount": 128
    },
    {
      "wxid": "wxid_member_b",
      "displayName": "Li Si",
      "nickname": "Li Si",
      "remark": "",
      "alias": "",
      "groupNickname": "",
      "avatarUrl": "",
      "isOwner": false,
      "isFriend": false,
      "messageCount": 0
    }
  ]
}
```

Notes:

- `displayName` is the primary display name inside the application.
- `groupNickname` is the member's nickname in that group.
- `remark` is the remark you gave the contact.
- `alias` is the WeChat ID.
- `groupNickname` is empty when the WeChat source data has no group nickname.

---

## 7. Moments endpoints

### 7.1 Get the Moments timeline

```http
GET /api/v1/sns/timeline
```

Parameters:

| Parameter   | Type   | Required | Description                                                                                          |
| ----------- | ------ | -------- | ---------------------------------------------------------------------------------------------------- |
| `limit`     | number | No       | Number of posts, default 20, range `1~200`                                                           |
| `offset`    | number | No       | Offset, default 0                                                                                    |
| `usernames` | string | No       | Filter by poster, comma separated, e.g. `wxid_a,wxid_b`                                              |
| `keyword`   | string | No       | Keyword filter (post text)                                                                           |
| `start`     | string | No       | Start time, `YYYYMMDD` or a seconds/milliseconds timestamp                                           |
| `end`       | string | No       | End time, `YYYYMMDD` or a seconds/milliseconds timestamp                                             |
| `media`     | number | No       | Whether to return directly accessible media URLs, default `1`                                        |
| `replace`   | number | No       | With `media=1`, whether resolved URLs overwrite `media.url/thumb`, default `1`                       |
| `inline`    | number | No       | With `media=1`, whether to return inline `data:` URLs, default `0`                                   |

Examples:

```bash
curl "http://127.0.0.1:5031/api/v1/sns/timeline?limit=5"
curl "http://127.0.0.1:5031/api/v1/sns/timeline?usernames=wxid_a,wxid_b&keyword=travel"
curl "http://127.0.0.1:5031/api/v1/sns/timeline?limit=3&media=1&replace=1"
curl "http://127.0.0.1:5031/api/v1/sns/timeline?limit=3&media=1&inline=1"
```

Media fields (`media=1`):

- `media[].url/thumb`: the fields you should normally use directly.
- With `replace=1` (default), `media[].url/thumb` are replaced with accessible URLs, equivalent to `resolvedUrl/resolvedThumbUrl`.
- With `replace=0`, `media[].url/thumb` keep the original WeChat URLs; combine them with the `raw/proxy/resolved` fields below to choose for yourself.
- `media[].rawUrl/rawThumb`: the original Moments URLs
- `media[].proxyUrl/proxyThumbUrl`: directly accessible proxy URLs
- `media[].resolvedUrl/resolvedThumbUrl`: the final usable URLs (may be `data:` URLs when `inline=1`)
- `media[].token/key/encIdx`: access/decryption parameters from the WeChat source data. You usually do not need to handle them; if you call `/api/v1/sns/media/proxy` by hand, pass the current entry's `url` and `key` back unchanged.
- `media[].livePhoto`: the video part of a Live Photo. The outer `media[].url/thumb` is still the cover image; `livePhoto` provides its own set of `url/thumb/raw*/proxy*/resolved*` fields.
- With `media=0`, `raw*/proxy*/resolved*` are not added; the endpoint returns only the original `url/thumb` and the source fields (such as `key/token/encIdx`).

### 7.2 Get Moments posters

```http
GET /api/v1/sns/usernames
```

### 7.3 Get Moments export statistics

```http
GET /api/v1/sns/export/stats
```

Parameters:

| Parameter | Type   | Required | Description                                   |
| --------- | ------ | -------- | --------------------------------------------- |
| `fast`    | number | No       | `1` uses fast statistics (cache first)        |

### 7.4 Moments media proxy

```http
GET /api/v1/sns/media/proxy
```

Parameters:

| Parameter | Type          | Required | Description                               |
| --------- | ------------- | -------- | ----------------------------------------- |
| `url`     | string        | Yes      | Original media URL                        |
| `key`     | string/number | No       | Decryption key (needed by some resources) |

### 7.5 Export Moments

```http
POST /api/v1/sns/export
Content-Type: application/json
```

Example body:

```json
{
  "outputDir": "C:\\Users\\Alice\\Desktop\\sns-export",
  "format": "json",
  "usernames": "wxid_a,wxid_b",
  "keyword": "travel",
  "exportMedia": true,
  "exportImages": true,
  "exportLivePhotos": true,
  "exportVideos": true,
  "start": "20250101",
  "end": "20251231"
}
```

`format` supports `json`, `html` and `arkmejson` (also written `arkme-json`).

### 7.6 Moments delete-blocking switch

```http
GET  /api/v1/sns/block-delete/status
POST /api/v1/sns/block-delete/install
POST /api/v1/sns/block-delete/uninstall
```

### 7.7 Delete a single Moments post

```http
DELETE /api/v1/sns/post/{postId}
```

---

## 8. Access exported media

> With POST, put the parameters in the JSON body (Content-Type: application/json)

After `media=1` is enabled on the message endpoint, images, voice, video and stickers are first exported to a local cache directory, and accessible HTTP URLs are returned.

**Request**

```http
GET /api/v1/media/{relativePath}
```

### Examples

```bash
curl "http://127.0.0.1:5031/api/v1/media/xxx@chatroom/images/abc123.jpg"
curl "http://127.0.0.1:5031/api/v1/media/xxx@chatroom/voices/voice_100.wav"
curl "http://127.0.0.1:5031/api/v1/media/xxx@chatroom/videos/video_200.mp4"
curl "http://127.0.0.1:5031/api/v1/media/xxx@chatroom/emojis/emoji_300.gif"
```

### Supported Content-Types

| Extension        | Content-Type |
| ---------------- | ------------ |
| `.png`           | `image/png`  |
| `.jpg` / `.jpeg` | `image/jpeg` |
| `.gif`           | `image/gif`  |
| `.webp`          | `image/webp` |
| `.wav`           | `audio/wav`  |
| `.mp3`           | `audio/mpeg` |
| `.mp4`           | `video/mp4`  |

Common error response:

```json
{
  "error": "Media not found"
}
```

---

## 9. Usage examples

### PowerShell

```powershell
$headers = @{ "Authorization" = "Bearer YOUR_TOKEN" }
$body = @{ talker = "wxid_xxx"; limit = 10 } | ConvertTo-Json

Invoke-RestMethod -Uri "http://127.0.0.1:5031/api/v1/messages" -Method POST -Headers $headers -Body $body -ContentType "application/json"
```

### cURL

```bash
# GET with a token header
curl -H "Authorization: Bearer YOUR_TOKEN" "http://127.0.0.1:5031/api/v1/messages?talker=wxid_xxx"

# POST with a JSON body
curl -X POST http://127.0.0.1:5031/api/v1/messages \
  -H "Authorization: Bearer YOUR_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"talker": "xxx@chatroom", "chatlab": true}'
```

### Python

```python
import requests

BASE_URL = "http://127.0.0.1:5031"
headers = {"Authorization": "Bearer YOUR_TOKEN", "Content-Type": "application/json"}

# Get messages with POST
messages = requests.post(
    f"{BASE_URL}/api/v1/messages",
    json={"talker": "xxx@chatroom", "limit": 50},
    headers=headers
).json()

# Get group members with GET
members = requests.get(
    f"{BASE_URL}/api/v1/group-members",
    params={"chatroomId": "xxx@chatroom", "includeMessageCounts": 1},
    headers=headers
).json()
```

---

## 10. Notes

1. The API listens only on the local `127.0.0.1` and is not exposed to the internet.
2. The database connection must be completed in WeFlow before use.
3. `start` and `end` accept `YYYYMMDD` and timestamps; a plain `YYYYMMDD` `end` is extended to `23:59:59` of that day.
4. A group member's `groupNickname` depends on the WeChat source data; it is not filled in automatically when missing.
5. Media links are only accessible after the corresponding messages have been exported with `media=1`.
