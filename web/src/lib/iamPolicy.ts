// Pure helpers for the IAM policy UI — node-testable, no React (arm64 vitest OOM
// guard). All action-picker parsing, validation, and normalization lives here.
// The React form component imports these and holds only UI state.

export interface IamPolicySummary {
  id: number;
  name: string;
  description: string | null;
  managed: boolean;
  statement_count: number;
  resource_bound: boolean;
  created_at: number;
}

export interface IamStatement {
  id: number;
  effect: string;
  actions: string[];
  resources: string[];
}

export interface IamPolicyDetail extends IamPolicySummary {
  statements: IamStatement[];
}

export interface ActionCategory {
  name: string;
  actions: string[];
}

export interface ActionsResponse {
  categories: ActionCategory[];
}

// ── action validation ─────────────────────────────────────────────────

/** Client-side mirror of the server `validate_actions`. */
export function validateActionPattern(
  pattern: string,
  allActions: string[],
): boolean {
  if (pattern === '*') return true;
  if (pattern.endsWith(':*')) {
    const prefix = pattern.slice(0, -2);
    return allActions.some((a) => a.startsWith(prefix + ':'));
  }
  return allActions.includes(pattern);
}

// ── textarea parsing ──────────────────────────────────────────────────

export type ParseResult =
  | { kind: 'valid'; actions: string[] }
  | { kind: 'invalid'; actions: string[]; errors: string[] }
  | { kind: 'malformed'; error: string };

/** Parse the textarea content into a three-state discriminated union. */
export function parseActionsTextarea(
  text: string,
  allActions: string[],
): ParseResult {
  let parsed: unknown;
  try {
    parsed = JSON.parse(text);
  } catch {
    return { kind: 'malformed', error: 'Invalid JSON' };
  }
  if (!Array.isArray(parsed)) {
    return { kind: 'malformed', error: 'Expected a JSON array' };
  }
  const actions: string[] = [];
  const errors: string[] = [];
  for (let i = 0; i < parsed.length; i++) {
    if (typeof parsed[i] !== 'string') {
      return { kind: 'malformed', error: 'Expected a JSON array of strings' };
    }
    const entry = parsed[i] as string;
    if (validateActionPattern(entry, allActions)) {
      actions.push(entry);
    } else {
      errors.push(entry);
    }
  }
  if (errors.length > 0) {
    return { kind: 'invalid', actions, errors };
  }
  return { kind: 'valid', actions };
}

// ── normalization ─────────────────────────────────────────────────────

/**
 * Normalize a selection into the deterministic form the server expects.
 * Rules: global `*` → `["*"]`; full categories → `category:*`;
 * deduplicate; sort `*` first, then `category:*` alphabetically,
 * then exact actions alphabetically.
 */
export function normalizeActions(
  selection: string[],
  allActions: string[],
): string[] {
  if (selection.length === 0) return [];
  if (selection.includes('*')) return ['*'];

  // derive category prefixes from allActions
  const categoryPrefixes = new Set<string>();
  for (const a of allActions) {
    const colon = a.indexOf(':');
    if (colon > 0) categoryPrefixes.add(a.substring(0, colon));
  }

  // collect category wildcards and exact actions from selection
  const wildcards = new Set<string>();
  const exacts = new Set<string>();
  for (const entry of selection) {
    if (entry.endsWith(':*')) {
      wildcards.add(entry);
    } else {
      exacts.add(entry);
    }
  }

  // Promote full categories: if every action in a category is in `exacts`,
  // replace them with a single `category:*`
  for (const prefix of categoryPrefixes) {
    const categoryActions = allActions.filter((a) => a.startsWith(prefix + ':'));
    if (categoryActions.length === 0) continue;
    if (categoryActions.every((a) => exacts.has(a))) {
      for (const a of categoryActions) exacts.delete(a);
      wildcards.add(`${prefix}:*`);
    }
  }

  // Remove individual actions that are subsumed by a wildcard
  for (const w of wildcards) {
    const prefix = w.slice(0, -2);
    for (const a of [...exacts]) {
      if (a.startsWith(prefix + ':')) {
        exacts.delete(a);
      }
    }
  }

  // Combine, sort, return
  const result = [...wildcards, ...exacts].sort((a, b) => {
    const aCat = a.endsWith(':*');
    const bCat = b.endsWith(':*');
    if (aCat && !bCat) return -1;
    if (!aCat && bCat) return 1;
    return a.localeCompare(b);
  });

  return result;
}

// ── wildcard expansion ───────────────────────────────────────────────

/** All known actions minus one specific action (grid uncheck-while-* rule). */
export function expandWildcardMinus(
  removedAction: string,
  allActions: string[],
): string[] {
  return allActions.filter((a) => a !== removedAction);
}

// ── form validation predicates ───────────────────────────────────────

export function validatePolicyName(name: string): string | null {
  const trimmed = name.trim();
  if (trimmed.length === 0) return 'Name is required';
  if (trimmed.length > 100) return 'Name must be <= 100 characters';
  return null;
}

export function validateDescription(desc: string): string | null {
  if (desc.length > 500) return 'Description must be <= 500 characters';
  return null;
}

// ── page guard predicates ────────────────────────────────────────────

export type GuardDecision =
  | { kind: 'loading' }
  | { kind: 'redirect-to-login' }
  | { kind: 'refuse'; message: string }
  | { kind: 'render' };

/**
 * Pure decision for the standard EE page guard. The component calls this
 * with session/role values and renders the appropriate state.
 */
export function eePageGuard(
  status: string,
  role: string | null,
  pageLabel: string,
): GuardDecision {
  if (status === 'loading' || status === 'pending_mfa') {
    return { kind: 'loading' };
  }
  if (status === 'guest') {
    return { kind: 'redirect-to-login' };
  }
  if (role !== 'admin') {
    return { kind: 'refuse', message: `/${pageLabel} requires the admin role.` };
  }
  return { kind: 'render' };
}
