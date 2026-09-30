import { useSyncExternalStore } from 'react'
import { en } from './locales/en'

/**
 * Lightweight i18n for the renderer.
 *
 * Source strings stay in Chinese and act as the lookup key: `t('首页')`.
 * - locale `zh-CN`: returns the key unchanged.
 * - locale `en`: returns the English entry, or the key if none exists yet
 *   (so a missing translation degrades to the original Chinese, never to blank).
 * Placeholders use `{name}`: `t('共 {count} 条', { count: 3 })`.
 *
 * English is the default. The active locale is, in order:
 *   1. the user's explicit choice (`en` or `zh-CN`), stored in localStorage so every
 *      window picks it up synchronously and follows changes via the `storage` event;
 *   2. otherwise the system locale (`navigator.language`, which Electron derives from
 *      the OS / LANG / LC_*): Chinese only when it starts with `zh`, else English.
 */

export type Locale = 'en' | 'zh-CN'
export type LanguagePreference = 'auto' | Locale

export const LANGUAGE_STORAGE_KEY = 'weflow-language'

const listeners = new Set<() => void>()

function readPreference(): LanguagePreference {
  try {
    const value = window.localStorage.getItem(LANGUAGE_STORAGE_KEY)
    if (value === 'en' || value === 'zh-CN' || value === 'auto') return value
  } catch {
    // localStorage can be unavailable; fall back to auto
  }
  return 'auto'
}

export function resolveLocale(preference: LanguagePreference, systemLocale: string): Locale {
  if (preference === 'en' || preference === 'zh-CN') return preference
  return /^zh([-_]|$)/i.test(systemLocale.trim()) ? 'zh-CN' : 'en'
}

function computeLocale(): Locale {
  const system = typeof navigator !== 'undefined' ? navigator.language || '' : ''
  return resolveLocale(readPreference(), system)
}

let currentLocale: Locale = computeLocale()

function applyDocumentLanguage() {
  if (typeof document !== 'undefined') {
    document.documentElement.lang = currentLocale
  }
}
applyDocumentLanguage()

function refresh() {
  const next = computeLocale()
  if (next === currentLocale) return
  currentLocale = next
  applyDocumentLanguage()
  listeners.forEach((listener) => listener())
  // Many labels are built once at module load (constants, option lists), so a
  // language change takes effect through a window reload rather than a re-render.
  if (typeof window !== 'undefined') window.location.reload()
}

if (typeof window !== 'undefined') {
  // Another window changed the language.
  window.addEventListener('storage', (event) => {
    if (event.key === LANGUAGE_STORAGE_KEY) refresh()
  })
}

export function getLocale(): Locale {
  return currentLocale
}

export function getLanguagePreference(): LanguagePreference {
  return readPreference()
}

export function setLanguagePreference(preference: LanguagePreference): void {
  try {
    window.localStorage.setItem(LANGUAGE_STORAGE_KEY, preference)
  } catch {
    // ignore, the in-memory switch below still applies for this window
  }
  refresh()
}

function subscribe(listener: () => void) {
  listeners.add(listener)
  return () => {
    listeners.delete(listener)
  }
}

export type TranslateParams = Record<string, unknown>

export function format(template: string, params?: TranslateParams): string {
  if (!params) return template
  return template.replace(/\{(\w+)\}/g, (match, name: string) =>
    Object.prototype.hasOwnProperty.call(params, name) ? String(params[name]) : match
  )
}

export function translate(locale: Locale, key: string, params?: TranslateParams): string {
  const text = locale === 'en' ? en[key] ?? key : key
  return format(text, params)
}

/** Locale for Date/Number formatting (toLocaleString etc.). */
export function formatLocale(): string {
  return currentLocale === 'en' ? 'en-US' : 'zh-CN'
}

/** Translate outside React (stores, services). Uses the current locale at call time. */
export function t(key: string, params?: TranslateParams): string {
  return translate(currentLocale, key, params)
}

/** Translate inside React; re-renders the component when the language changes. */
export function useI18n() {
  const locale = useSyncExternalStore(subscribe, getLocale, getLocale)
  return {
    locale,
    t: (key: string, params?: TranslateParams) => translate(locale, key, params)
  }
}
