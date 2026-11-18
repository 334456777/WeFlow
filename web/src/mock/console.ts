// The commands the sample's command console lists (#896): a slice of `/openapi.json` as `weflow serve` writes it, of
// the commands the sample answers, and the CLI tools the settings open in the console.

interface Schema { type?: string; enum?: string[]; items?: Schema; default?: unknown; description?: string }
interface Param { name: string; schema: Schema; required?: boolean; description?: string }

const STD = { $ref: '#/components/parameters/std' };
const RETURN = { $ref: '#/components/parameters/return' };

const string = (description: string): Schema => ({ type: 'string', description });
const flag = (description: string): Schema => ({ type: 'boolean', description });

/** One operation: its parameters in the query (GET, DELETE) or in a JSON body (the others). */
function op(command: string, method: 'get' | 'post' | 'put', summary: string, params: Param[] = [], file = false) {
  const words = command.split(' ');
  const inQuery = method === 'get';
  return {
    ['/' + words.join('/')]: {
      [method]: {
        operationId: words.join('_').replace(/-/g, '_'), tags: [words[0]], summary, 'x-weflow-command': command,
        parameters: [
          ...(inQuery ? params.map(p => ({ name: p.name, in: 'query', required: Boolean(p.required), schema: p.schema, description: p.description ?? p.schema.description })) : []),
          STD, ...(file ? [RETURN] : []),
        ],
        ...(inQuery || !params.length ? {} : {
          requestBody: { content: { 'application/json': { schema: {
            properties: Object.fromEntries(params.map(p => [p.name, p.schema])), required: params.filter(p => p.required).map(p => p.name),
          } } } },
        }),
      },
    },
  };
}

const FORMATS = ['html', 'txt', 'excel', 'json', 'arkme-json', 'chatlab', 'chatlab-jsonl', 'weclone', 'sql'];

export const OPENAPI = {
  openapi: '3.1.0',
  info: { title: 'weflow serve（示例数据）', version: __WEFLOW_WEB__.apiVersion },
  paths: {
    ...op('chat sessions', 'get', '列出会话，最新在前'),
    ...op('chat messages', 'get', '某个会话的一页消息', [
      { name: 'session-id', required: true, schema: string('会话 ID：对方的 wxid，或群聊的 xxx@chatroom') },
      { name: 'limit', schema: { type: 'integer', default: 50, description: '最多读取的条数' } },
    ]),
    ...op('chat search', 'get', '在某个会话或所有会话中搜索消息', [
      { name: 'keyword', required: true, schema: string('要找的文字') },
      { name: 'session-id', schema: string('只在这个会话里找') },
    ]),
    ...op('chat contacts', 'get', '列出所有联系人', [{ name: 'parsed', schema: flag('按类型整理后输出') }]),
    ...op('analytics overall', 'get', '私聊的总体统计'),
    ...op('sns timeline', 'get', '朋友圈动态，最新在前', [
      { name: 'user', schema: string('只看这个人发的') }, { name: 'keyword', schema: string('只看含这些文字的') },
    ]),
    ...op('insight records', 'get', '灵感信箱的见解，最新在前', [{ name: 'keyword', schema: string('只看含这些文字的') }]),
    ...op('config profiles', 'get', '列出配置档案和各自的设置情况'),
    ...op('config get', 'get', '读取当前档案的一项设置', [{ name: 'key', required: true, schema: string('设置项，如 db_path') }]),
    ...op('config set', 'put', '保存当前档案的一项设置', [
      { name: 'key', required: true, schema: string('设置项，如 serve_port') }, { name: 'value', required: true, schema: string('要保存的值') },
    ]),
    ...op('db open', 'get', '用当前档案的目录和密钥打开数据库，显示概要'),
    ...op('key db', 'post', '从微信取数据库密钥', [{ name: 'save', schema: flag('保存到当前档案') }]),
    ...op('export messages', 'post', '导出一个会话的聊天记录', [
      { name: 'session-id', required: true, schema: string('会话 ID') },
      { name: 'format', schema: { type: 'array', items: { type: 'string', enum: FORMATS }, description: '导出格式，可以多选' } },
      { name: 'start', schema: string('开始日期，YYYY-MM-DD') }, { name: 'end', schema: string('结束日期，YYYY-MM-DD') },
      { name: 'zip', schema: flag('打包成 zip') },
    ], true),
    ...op('export contacts', 'post', '导出通讯录', [
      { name: 'format', schema: { type: 'string', enum: ['excel', 'csv', 'json', 'html', 'txt', 'sql'], description: '导出格式' } },
      { name: 'parsed', schema: flag('按类型整理后导出') },
    ], true),
    ...op('image auto-download start', 'post', '微信下载聊天图片时改为下载原图（只支持 Windows）'),
  },
};

/** What `db open` shows of the sample's databases. */
export const DB_SUMMARY = {
  databases: ['contact/contact.db', 'session/session.db', 'message/message_0.db', 'message/message_1.db', 'message/media_0.db', 'sns/sns.db'],
  wal: ['message/message_0.db-wal'],
};
