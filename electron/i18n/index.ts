import { app } from 'electron'
import { en } from './en'

/**
 * Main-process i18n. Same model as the renderer: the Chinese source string is the key,
 * English is used unless the system locale is Chinese or the user picked Chinese.
 *
 * Locale precedence: the `uiLanguage` config value ('en' | 'zh-CN'), then the system
 * locale (`app.getLocale()`, which follows LANG / LC_* on Linux); Chinese only when it
 * starts with `zh`, otherwise English.
 */
export type Locale = 'en' | 'zh-CN'
export type LanguagePreference = 'auto' | Locale

let preferenceProvider: () => LanguagePreference | undefined = () => undefined
let cached: Locale | null = null

export function setLanguagePreferenceProvider(provider: () => LanguagePreference | undefined): void {
  preferenceProvider = provider
  cached = null
}

/** Call after the stored language preference changes. */
export function refreshLocale(): void {
  cached = null
}

function systemLocale(): string {
  try {
    const locale = app.getLocale()
    if (locale) return locale
  } catch {
    // app.getLocale can throw very early in startup; fall through to env vars
  }
  for (const key of ['LC_ALL', 'LC_MESSAGES', 'LANG', 'LANGUAGE']) {
    const value = process.env[key]
    if (value && value.trim()) return value
  }
  return ''
}

export function resolveLocale(preference: LanguagePreference | undefined, system: string): Locale {
  if (preference === 'en' || preference === 'zh-CN') return preference
  return /^zh([-_.]|$)/i.test(system.trim()) ? 'zh-CN' : 'en'
}

export function getLocale(): Locale {
  if (!cached) cached = resolveLocale(preferenceProvider(), systemLocale())
  return cached
}

export function format(template: string, params?: Record<string, unknown>): string {
  if (!params) return template
  return template.replace(/\{(\w+)\}/g, (match, name: string) =>
    Object.prototype.hasOwnProperty.call(params, name) ? String(params[name]) : match
  )
}

/** Translate a Chinese source string for the active locale. */
export function mt(key: string, params?: Record<string, unknown>): string {
  const text = getLocale() === 'en' ? en[key] ?? key : key
  return format(text, params)
}
