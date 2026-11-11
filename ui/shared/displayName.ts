/** How a GitHub login is shown: the company prefix ("SCVN-", "SC-") left off, so "SCVN-Dan" reads "Dan". */
export function displayName(login: string): string {
  const name = login.replace(/^(scvn|sc)[-_]/i, '')
  return name || login
}
