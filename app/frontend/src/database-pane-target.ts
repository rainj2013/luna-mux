const prefix = 'database-profile:'

// Public connection reference only. Credentials stay in the OS credential store.
export function databasePaneTarget(profileId: string, readOnly = false): string {
  return prefix + encodeURIComponent(profileId) + (readOnly ? '?readOnly=true' : '')
}

export function parseDatabasePaneTarget(targetId: string): { profileId: string; readOnly: boolean } | undefined {
  if (!targetId.startsWith(prefix)) return undefined
  const [encodedId = '', query] = targetId.slice(prefix.length).split('?')
  try {
    const profileId = decodeURIComponent(encodedId)
    return profileId ? { profileId, readOnly: new URLSearchParams(query).get('readOnly') === 'true' } : undefined
  } catch { return undefined }
}
