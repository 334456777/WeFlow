// AI 模型接口的接口格式（`ai_model_api_format`）和提问发往的地址，按 weflow 的规则计算
// （crates/weflow-core/src/model_api/），网页因此能在保存设置之前显示地址。
import type { ModelApiFormat } from '../api/contract.gen';
import { msg } from '../i18n';

/** 一种接口格式：名称和说明、官方服务的基础地址，以及提问发往的路径。 */
export type Format = { id: ModelApiFormat; label: string; text: string; base: string; path: string };

/** 可选的接口格式；`label` 和 `text` 显示时经过 t()。 */
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

/** 设置值所指的格式；其他值都按 `openai`，与 weflow 相同。 */
export const formatOf = (value: string | null | undefined): ModelApiFormat =>
  value?.trim().toLowerCase() === 'anthropic' ? 'anthropic' : 'openai';

/** 格式 `id` 的信息；不认识的按 OpenAI 的。 */
export const formatInfo = (id: ModelApiFormat): Format => FORMATS.find(f => f.id === id) ?? FORMATS[0];

/** 基础地址为 `base` 时提问发往的地址：为空时用这种格式的官方服务；OpenAI 格式加上 `/chat/completions`，
 * Anthropic 格式加上 `/v1/messages`（已以 `/v1` 结尾时只加 `/messages`）；`base` 已经是完整地址时原样使用。 */
export function endpointOf(format: ModelApiFormat, base: string): string {
  const root = base.trim().replace(/\/+$/, '') || formatInfo(format).base;
  if (format === 'openai') return root.endsWith('/chat/completions') ? root : root + '/chat/completions';
  for (const end of ['/v1/messages', '/v1']) if (root.endsWith(end)) return root.slice(0, -end.length) + '/v1/messages';
  return root + '/v1/messages';
}

/** 获取模型列表之前建议的模型名：Anthropic 现有的模型，能力最强的在前；OpenAI 格式不建议。 */
export const SUGGESTED_MODELS: Record<ModelApiFormat, readonly string[]> = {
  openai: [],
  anthropic: ['claude-opus-5-5', 'claude-sonnet-5-5', 'claude-haiku-5-5'],
};
