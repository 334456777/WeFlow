// The API formats of the AI model API (`ai_model_api_format`) and the address a question goes to, by the rules weflow
// follows (crates/weflow-core/src/model_api/), so that the page shows the address before the settings are saved.
import type { ModelApiFormat } from '../api/contract.gen';
import { msg } from '../i18n';

export type Format = { id: ModelApiFormat; label: string; text: string; base: string; path: string };

/** Shown through t(). */
export const FORMATS: readonly Format[] = [
  {
    id: 'openai', label: msg('OpenAI 兼容'), text: msg('Chat Completions 接口，OpenAI 和大多数兼容服务都用它。'),
    base: 'https://api.openai.com/v1', path: '/chat/completions',
  },
  {
    id: 'anthropic', label: msg('Anthropic 兼容'), text: msg('Messages 接口，Claude 和兼容 Anthropic 的服务用它。'),
    base: 'https://api.anthropic.com', path: '/v1/messages',
  },
];

/** The format a setting names; anything else is `openai`, as for weflow. */
export const formatOf = (value: string | null | undefined): ModelApiFormat =>
  value?.trim().toLowerCase() === 'anthropic' ? 'anthropic' : 'openai';

export const formatInfo = (id: ModelApiFormat): Format => FORMATS.find(f => f.id === id) ?? FORMATS[0];

/** Where a question goes for the base URL `base`: the format's own service when it is empty; OpenAI's adds
 * `/chat/completions`, Anthropic's `/v1/messages` (only `/messages` after a `/v1`); a base that is the whole address
 * is taken as it is. */
export function endpointOf(format: ModelApiFormat, base: string): string {
  const root = base.trim().replace(/\/+$/, '') || formatInfo(format).base;
  if (format === 'openai') return root.endsWith('/chat/completions') ? root : root + '/chat/completions';
  for (const end of ['/v1/messages', '/v1']) if (root.endsWith(end)) return root.slice(0, -end.length) + '/v1/messages';
  return root + '/v1/messages';
}

/** Model names to suggest before the list is fetched: Anthropic's current ones, the most capable first. */
export const SUGGESTED_MODELS: Record<ModelApiFormat, readonly string[]> = {
  openai: [],
  anthropic: ['claude-opus-5-5', 'claude-sonnet-5-5', 'claude-haiku-5-5'],
};
