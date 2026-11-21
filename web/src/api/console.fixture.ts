// For the tests of the command console (#217).
/** A slice of `/openapi.json` as `weflow serve` writes it. */
export const DOC = {
  paths: {
    '/chat/messages': { get: {
      operationId: 'chat_messages', tags: ['chat'], summary: '读取一个会话的消息', 'x-weflow-command': 'chat messages',
      parameters: [
        { name: 'session-id', in: 'query', required: true, schema: { type: 'string' }, description: '会话 ID' },
        { name: 'limit', in: 'query', schema: { type: 'integer', default: 50 } },
        { name: 'parsed', in: 'query', schema: { type: 'boolean' } },
        { $ref: '#/components/parameters/std' },
      ],
    } },
    '/api/chat/messages': { get: { operationId: 'api_chat_messages', 'x-weflow-command': 'chat messages', parameters: [] } },
    '/export/contacts': { post: {
      operationId: 'export_contacts', tags: ['export'], summary: '导出通讯录', 'x-weflow-command': 'export contacts',
      parameters: [{ $ref: '#/components/parameters/std' }, { $ref: '#/components/parameters/return' }],
      requestBody: { content: { 'application/json': { schema: {
        properties: { out: { type: 'string' }, format: { type: 'string' }, kind: { type: 'array', items: { type: 'string' } }, ids: { type: 'array', items: { type: 'integer' } } },
        required: ['out'],
      } } } },
    } },
    '/chat/group-hint': {
      get: { operationId: 'chat_group_hint_get', tags: ['chat'], summary: '群里我的消息数', 'x-weflow-command': 'chat group-hint', parameters: [{ name: 'chatroom-id', in: 'query', required: true, schema: { type: 'string' } }] },
      put: { operationId: 'chat_group_hint_put', tags: ['chat'], summary: '群里我的消息数', 'x-weflow-command': 'chat group-hint', parameters: [{ name: 'chatroom-id', in: 'query', required: true, schema: { type: 'string' } }, { name: 'set', in: 'query', required: true, schema: { type: 'integer' } }] },
    },
    '/lang': {
      put: { operationId: 'lang', tags: ['lang'], summary: '输出语言', 'x-weflow-command': 'lang', parameters: [{ name: 'lang', in: 'query', required: true, schema: { type: 'string', enum: ['en', 'zh'] } }] },
    },
    '/health': { get: { operationId: 'health', tags: ['server'] } },
  },
};
