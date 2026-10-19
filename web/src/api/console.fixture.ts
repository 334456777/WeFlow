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
    '/sns/block-delete': {
      get: { operationId: 'sns_block_delete_get', tags: ['sns'], summary: '朋友圈防删除', 'x-weflow-command': 'sns block-delete', parameters: [{ name: 'action', in: 'query', required: true, schema: { type: 'string', enum: ['check'] } }] },
      delete: { operationId: 'sns_block_delete_delete', tags: ['sns'], summary: '朋友圈防删除', 'x-weflow-command': 'sns block-delete', 'x-weflow-refused': true, parameters: [{ name: 'action', in: 'query', required: true, schema: { type: 'string', enum: ['uninstall'] } }] },
    },
    '/health': { get: { operationId: 'health', tags: ['server'] } },
  },
};
