import { describe, expect, it } from 'vitest';
import { commandsOf, formProblems, paramsOf, shellLine } from './console';
import { DOC } from './console.fixture';


describe('console', () => {
  it('reads every command of the document, without the server routes and the in-process copies', () => {
    const commands = commandsOf(DOC);
    expect(commands.map(c => c.method + ' ' + c.command)).toEqual(['GET chat messages', 'POST export contacts', 'GET sns block-delete', 'DELETE sns block-delete']);
    const [messages, contacts, , refused] = commands;
    expect(messages.fields).toEqual([
      { name: 'session-id', type: 'string', required: true, description: '会话 ID', defaultValue: undefined },
      { name: 'limit', type: 'integer', required: false, description: '', defaultValue: '50' },
      { name: 'parsed', type: 'boolean', required: false, description: '', defaultValue: undefined },
    ]);
    expect([messages.file, contacts.file, refused.refused, commands[2].refused]).toEqual([false, true, true, false]);
    expect(contacts.fields.map(f => [f.name, f.type, f.required, f.integers ?? false])).toEqual([
      ['out', 'string', true, false], ['format', 'string', false, false], ['kind', 'list', false, false], ['ids', 'list', false, true],
    ]);
    expect(refused.fields[0]).toMatchObject({ type: 'choice', options: ['uninstall'] });
  });

  it('turns a form into parameters, and says what it misses', () => {
    const [messages, contacts] = commandsOf(DOC);
    expect(formProblems(messages, { 'session-id': ' ', limit: '5x', parsed: false })).toEqual(['请填写 session-id。', 'limit 要填整数。']);
    expect(paramsOf(messages, { 'session-id': ' wxid_a ', limit: '20', parsed: true })).toEqual({ 'session-id': 'wxid_a', limit: 20, parsed: true });
    expect(paramsOf(messages, { 'session-id': 'wxid_a', limit: '', parsed: false })).toEqual({ 'session-id': 'wxid_a' });
    expect(paramsOf(contacts, { out: 'a.json', format: '', kind: 'friend，group\nofficial', ids: '1, 2' })).toEqual({ out: 'a.json', kind: ['friend', 'group', 'official'], ids: [1, 2] });
    expect(formProblems(contacts, { out: 'a', format: '', kind: '', ids: '1,b' })).toEqual(['ids 的每一项都要是整数。']);
  });

  it('writes a command line to copy, quoting what a shell would split', () => {
    expect(shellLine(['weflow', '--json', 'chat', 'search', '周末 散步', "it's"])).toBe("weflow --json chat search '周末 散步' 'it'\\''s'");
  });
});
