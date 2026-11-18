import { describe, expect, it } from '#test';
import { endpointOf, formatOf } from './modelApi';

describe('model API addresses', () => {
  it('follows the rules weflow follows', () => {
    expect(endpointOf('openai', '')).toBe('https://api.openai.com/v1/chat/completions');
    expect(endpointOf('openai', ' https://api.ohmygpt.com/v1/ ')).toBe('https://api.ohmygpt.com/v1/chat/completions');
    expect(endpointOf('openai', 'https://x.test/v1/chat/completions')).toBe('https://x.test/v1/chat/completions');
    expect(endpointOf('anthropic', '')).toBe('https://api.anthropic.com/v1/messages');
    expect(endpointOf('anthropic', 'https://gw.test/anthropic')).toBe('https://gw.test/anthropic/v1/messages');
    expect(endpointOf('anthropic', 'https://gw.test/anthropic/v1/')).toBe('https://gw.test/anthropic/v1/messages');
    expect(endpointOf('anthropic', 'https://gw.test/v1/messages')).toBe('https://gw.test/v1/messages');
  });

  it('reads a format as weflow does', () => {
    expect(formatOf(' Anthropic ')).toBe('anthropic');
    expect(formatOf('claude')).toBe('openai');
    expect(formatOf(null)).toBe('openai');
  });
});
