/**
 * What the AI pet can do in the hub. Codex calls these (dynamic tools); the
 * main process hands each call to the hub, which acts on what the user is
 * looking at and answers. Nothing here writes to GitHub or to disk.
 */
export const AI_TOOL_NAMES = ['name_words', 'search_assets', 'open_asset', 'asset_info', 'add_to_chest', 'add_to_collection', 'offer_choices'] as const
export type AiToolName = (typeof AI_TOOL_NAMES)[number]

export type AiToolResult = { ok: true; data: unknown } | { ok: false; error: string }

/** An asset as the tools name it: "owner/repo@branch:path/in/repo". */
const REF = 'An asset reference exactly as returned by search_assets ("owner/repo@branch:path").'

/** Tool definitions for Codex; `types` are the asset types the hub knows (its tabs). */
export function aiToolSpecs(types: readonly string[]) {
  return [
    {
      type: 'function',
      name: 'name_words',
      description:
        'The words asset file names are actually made of in a scope (repo / branch, optionally one type), most used first. ' +
        'Call it before searching for anything described by meaning, in another language or by look, then search with words from this list.',
      inputSchema: {
        type: 'object',
        properties: {
          type: { type: 'string', enum: [...types] },
          where: { type: 'string', enum: ['branch', 'repo', 'all'] },
          repo: { type: 'string', description: 'A repo the user named; part of its name is enough' }
        },
        additionalProperties: false
      }
    },
    {
      type: 'function',
      name: 'search_assets',
      description:
        'Search Unity assets and show the results in the hub (it fills the search box and picks the type tab). ' +
        'Searches the open branch, every branch of the open repo, or every indexed repo. Understands synonyms and typos; ' +
        'use short English keywords as in file names (e.g. "fire", "explosion smoke"). Returns the best matches with refs.',
      inputSchema: {
        type: 'object',
        properties: {
          query: { type: 'string', description: 'Keywords; " | " between alternatives matches any of them, e.g. "fire | flame | burn"' },
          type: { type: 'string', enum: [...types], description: 'Only this asset type (tab), e.g. vfx for effects' },
          where: { type: 'string', enum: ['branch', 'repo', 'all'], description: 'branch: the open branch (default); repo: every branch of the open repo; all: every repo' },
          repo: { type: 'string', description: 'A repo the user named; part of its name is enough ("pizza"). Opens that repo in the hub and searches every branch of it (where is then ignored).' }
        },
        required: ['query'],
        additionalProperties: false
      }
    },
    {
      type: 'function',
      name: 'open_asset',
      description: 'Open an asset in the hub: its repo and branch, with the preview and details shown.',
      inputSchema: { type: 'object', properties: { ref: { type: 'string', description: REF } }, required: ['ref'], additionalProperties: false }
    },
    {
      type: 'function',
      name: 'asset_info',
      description: 'Details of an asset: size, measured stats (triangles, textures…), whether it fits the playable budget, and what it depends on.',
      inputSchema: { type: 'object', properties: { ref: { type: 'string', description: REF } }, required: ['ref'], additionalProperties: false }
    },
    {
      type: 'function',
      name: 'add_to_chest',
      description: 'Add assets of the open branch to the chest (the export list). The user exports it themselves.',
      inputSchema: { type: 'object', properties: { refs: { type: 'array', items: { type: 'string', description: REF }, minItems: 1, maxItems: 20 } }, required: ['refs'], additionalProperties: false }
    },
    {
      type: 'function',
      name: 'add_to_collection',
      description: 'Add assets to a collection by name (made when it does not exist). "Favourites" is the starred list.',
      inputSchema: {
        type: 'object',
        properties: { refs: { type: 'array', items: { type: 'string', description: REF }, minItems: 1, maxItems: 20 }, collection: { type: 'string' } },
        required: ['refs', 'collection'],
        additionalProperties: false
      }
    },
    {
      type: 'function',
      name: 'offer_choices',
      description:
        'Show reply buttons under your message, so the user answers with one click. Use it instead of asking in words when a request is unclear ' +
        'and the answer changes what you do. Then end your turn with a one-line question. A clicked button sends its label as the user\'s reply.',
      inputSchema: {
        type: 'object',
        properties: {
          options: { type: 'array', items: { type: 'string', maxLength: 40 }, minItems: 2, maxItems: 4, description: 'Short labels in the user\'s language' },
          other: { type: 'boolean', description: 'Add an "Other…" button that lets the user type their own answer' }
        },
        required: ['options'],
        additionalProperties: false
      }
    }
  ]
}

export function parseAssetRef(ref: unknown): { repo: string; branch: string; path: string } | undefined {
  const m = typeof ref === 'string' ? /^([^@\s]+\/[^@\s]+)@([^:]+):(.+)$/.exec(ref.trim()) : null
  return m ? { repo: m[1], branch: m[2], path: m[3] } : undefined
}

/**
 * Repos a name the user gave could mean, best first tier only: the whole name
 * ("luna_pizza"), then a whole word of it ("pizza" in luna_pizza, not in
 * luna_pizzasimulator), then any part. More than one means it is ambiguous.
 */
export function matchRepos(fullNames: readonly string[], named: string): string[] {
  const want = named.trim().toLowerCase().replace(/^.*\//, '')
  if (!want) return []
  const nameOf = (r: string) => r.slice(r.indexOf('/') + 1).toLowerCase()
  const tiers = [
    (n: string) => n === want,
    (n: string) => n.split(/[_\-.\s]+/).includes(want),
    (n: string) => n.includes(want)
  ]
  for (const tier of tiers) {
    const found = fullNames.filter((r) => tier(nameOf(r)))
    if (found.length) return found
  }
  return []
}

export const assetRef = (repo: string, branch: string, path: string) => `${repo}@${branch}:${path}`
