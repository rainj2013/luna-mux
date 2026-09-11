const prefix = 'database-profile:'

// Public connection reference only. Credentials stay in the OS credential store. The read-only
// flag is always written out, so a pane that was opened writable does not come back read-only.
export function databasePaneTarget(profileId: string, readOnly = true): string {
  return prefix + encodeURIComponent(profileId) + (readOnly ? '?readOnly=true' : '?readOnly=false')
}

export function parseDatabasePaneTarget(targetId: string): { profileId: string; readOnly: boolean } | undefined {
  if (!targetId.startsWith(prefix)) return undefined
  const [encodedId = '', query] = targetId.slice(prefix.length).split('?')
  try {
    const profileId = decodeURIComponent(encodedId)
    // Panes saved before the flag existed default to read-only, which is the safe default.
    const flag = new URLSearchParams(query).get('readOnly')
    return profileId ? { profileId, readOnly: flag === null ? true : flag === 'true' } : undefined
  } catch { return undefined }
}
