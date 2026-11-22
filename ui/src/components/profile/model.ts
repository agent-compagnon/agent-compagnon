// What choosing a profile does, said before it applies: the environments it exposes, sensitive ones named.

import type { AppSnapshot, ProfileChoice } from '@/api/types'

export interface ProfileEffect {
  exposed: string[]
  sensitive: string[]
}

export function profileEffect(snapshot: AppSnapshot, choice: ProfileChoice): ProfileEffect {
  const environments = choice.defaultEnvironments.flatMap((id) => {
    const env = snapshot.environments.find((e) => e.id === id)
    return env === undefined ? [] : [env]
  })
  return {
    exposed: environments.map((e) => e.label),
    sensitive: environments.filter((e) => e.sensitive).map((e) => e.label),
  }
}

/** The profiles in the order of the configuration, the strictest (the fallback) last. */
export function orderedChoices(snapshot: AppSnapshot): ProfileChoice[] {
  return [...snapshot.profile.choices.filter((p) => !p.strictest), ...snapshot.profile.choices.filter((p) => p.strictest)]
}

export function activeChoice(snapshot: AppSnapshot): ProfileChoice | undefined {
  return snapshot.profile.choices.find((p) => p.id === snapshot.profile.active)
}
