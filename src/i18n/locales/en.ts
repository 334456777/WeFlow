import { sidebarDict } from './en/sidebar'
import { appDict } from './en/app'
import { chatDict } from './en/chat'
import { analyticsDict } from './en/analytics'
import { exportDict } from './en/export'
import { settingsDict } from './en/settings'
import { onboardingDict } from './en/onboarding'
import { featuresDict } from './en/features'

/**
 * English translations, keyed by the original Chinese source string.
 * Generated groups live under ./en; every t('…') key in the renderer must have an entry.
 */
export const en: Record<string, string> = {
  ...sidebarDict,
  ...appDict,
  ...chatDict,
  ...analyticsDict,
  ...exportDict,
  ...settingsDict,
  ...onboardingDict,
  ...featuresDict
}
