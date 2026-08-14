'use client';

import {
  type FormEvent,
  type KeyboardEvent,
  useEffect,
  useMemo,
  useRef,
  useState,
} from 'react';
import { useRouter } from 'next/navigation';
import type {
  CoreAgentSnapshot,
  CoreLiveStatus,
  FleetHost,
} from '@/lib/coreFleet';
import {
  fleetShellCompletions,
  fleetShellPrompt,
  runFleetShellCommand,
  type FleetShellContext,
  type FleetShellEffect,
  type FleetShellLine,
} from '@/lib/fleetShell';
import type { HealthSnapshotRow } from '@/lib/types';

type TranscriptEntry = {
  id: number;
  prompt?: string;
  command?: string;
  lines: FleetShellLine[];
};

type FleetShellProps = {
  hosts: FleetHost[];
  snapshots: Record<string, CoreAgentSnapshot>;
  healthByAgent: Record<string, HealthSnapshotRow>;
  liveStatus: CoreLiveStatus;
  loading: boolean;
  nowSeconds: number;
  refresh: () => void;
  onSelectAgent?: (agentId: string) => void;
};

const QUICK_COMMANDS = ['stats', 'hosts offline', 'services failed', 'containers stopped', 'health'];
const MAX_HISTORY = 100;
const MAX_TRANSCRIPT_ENTRIES = 30;
const INITIAL_TRANSCRIPT: TranscriptEntry[] = [
  {
    id: 0,
    lines: [
      { text: 'ShellFleet Fleet Shell', tone: 'accent' },
      { text: 'Read-only commands over durable fleet snapshots. Type `help` for the command index.' },
      { text: 'Try `stats`, `hosts warn`, or `use <host>`. Use `terminal` for an interactive root shell.', tone: 'dim' },
    ],
  },
];

const DESTINATION_ROUTES = {
  overview: '/overview',
  terminal: '/terminal',
  activity: '/activity',
  notifications: '/notifications',
} as const;

function commonPrefix(values: string[]): string {
  if (values.length === 0) return '';
  let prefix = values[0];
  for (const value of values.slice(1)) {
    let index = 0;
    while (index < prefix.length && index < value.length && prefix[index].toLowerCase() === value[index].toLowerCase()) {
      index += 1;
    }
    prefix = prefix.slice(0, index);
    if (!prefix) break;
  }
  return prefix;
}

export default function FleetShell({
  hosts,
  snapshots,
  healthByAgent,
  liveStatus,
  loading,
  nowSeconds,
  refresh,
  onSelectAgent,
}: FleetShellProps) {
  const router = useRouter();
  const [scopeAgentId, setScopeAgentId] = useState<string | null>(null);
  const [input, setInput] = useState('');
  const [history, setHistory] = useState<string[]>([]);
  const [historyCursor, setHistoryCursor] = useState<number | null>(null);
  const [historyDraft, setHistoryDraft] = useState('');
  const [transcript, setTranscript] = useState<TranscriptEntry[]>(INITIAL_TRANSCRIPT);
  const nextEntryId = useRef(1);
  const outputRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLInputElement>(null);

  const validScopeAgentId = scopeAgentId && hosts.some((host) => host.agent_id === scopeAgentId)
    ? scopeAgentId
    : null;
  const context = useMemo<FleetShellContext>(() => ({
    hosts,
    snapshots,
    healthByAgent,
    scopeAgentId: validScopeAgentId,
    liveStatus,
    commandHistory: history,
    nowSeconds,
  }), [healthByAgent, history, hosts, liveStatus, nowSeconds, snapshots, validScopeAgentId]);

  const prompt = fleetShellPrompt(context);
  const completions = useMemo(
    () => input.trim() ? fleetShellCompletions(input, context) : [],
    [context, input],
  );

  useEffect(() => {
    const output = outputRef.current;
    if (output) output.scrollTop = output.scrollHeight;
  }, [transcript]);

  function applyEffect(effect: FleetShellEffect | undefined) {
    if (!effect) return;
    if (effect.type === 'refresh') {
      refresh();
      return;
    }
    if (effect.type === 'set-scope') {
      setScopeAgentId(effect.agentId);
      return;
    }
    if (effect.type === 'navigate') {
      router.push(DESTINATION_ROUTES[effect.target]);
      return;
    }
    if (effect.view === 'dashboard' && onSelectAgent) {
      onSelectAgent(effect.agentId);
      return;
    }
    const params = new URLSearchParams({ agent: effect.agentId });
    if (effect.view !== 'dashboard') params.set('tab', effect.view);
    router.push(`/?${params.toString()}`);
  }

  function execute(rawCommand: string) {
    const command = rawCommand.trim();
    if (!command) return;
    const nextHistory = [...history, command].slice(-MAX_HISTORY);
    const executionContext = { ...context, commandHistory: nextHistory };
    const commandPrompt = fleetShellPrompt(executionContext);
    const result = runFleetShellCommand(command, executionContext);

    setHistory(nextHistory);
    setHistoryCursor(null);
    setHistoryDraft('');
    setInput('');
    if (result.clear) {
      setTranscript([]);
    } else {
      const entry: TranscriptEntry = {
        id: nextEntryId.current,
        prompt: commandPrompt,
        command,
        lines: result.lines,
      };
      nextEntryId.current += 1;
      setTranscript((current) => [...current, entry].slice(-MAX_TRANSCRIPT_ENTRIES));
    }
    applyEffect(result.effect);
  }

  function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    execute(input);
  }

  function recallHistory(direction: -1 | 1) {
    if (history.length === 0) return;
    if (direction === -1) {
      if (historyCursor === null) {
        setHistoryDraft(input);
        const next = history.length - 1;
        setHistoryCursor(next);
        setInput(history[next]);
      } else {
        const next = Math.max(0, historyCursor - 1);
        setHistoryCursor(next);
        setInput(history[next]);
      }
      return;
    }
    if (historyCursor === null) return;
    if (historyCursor >= history.length - 1) {
      setHistoryCursor(null);
      setInput(historyDraft);
    } else {
      const next = historyCursor + 1;
      setHistoryCursor(next);
      setInput(history[next]);
    }
  }

  function handleKeyDown(event: KeyboardEvent<HTMLInputElement>) {
    if (event.key === 'ArrowUp') {
      event.preventDefault();
      recallHistory(-1);
      return;
    }
    if (event.key === 'ArrowDown') {
      event.preventDefault();
      recallHistory(1);
      return;
    }
    if (event.key === 'Tab' && completions.length > 0) {
      event.preventDefault();
      const completion = completions.length === 1 ? completions[0] : commonPrefix(completions);
      if (completion.length > input.trimStart().length) setInput(`${completion} `);
      return;
    }
    if (event.key === 'Escape') {
      setInput('');
      setHistoryCursor(null);
    }
  }

  return (
    <section className="panel fleet-shell" aria-label="Fleet Shell">
      <div className="panel-head">
        <div className="panel-title">
          <span className="ico">›_</span> FLEET SHELL
          <span className="meta">{loading ? 'syncing snapshots…' : `${liveStatus} · read-only`}</span>
        </div>
        <div className="panel-actions">
          <span className="kbd-hint"><kbd>tab</kbd> complete&nbsp; <kbd>↑↓</kbd> history</span>
          <button type="button" className="btn sm ghost" onClick={() => setTranscript([])}>
            clear
          </button>
        </div>
      </div>

      <div
        ref={outputRef}
        className="fleet-shell-output"
        role="log"
        aria-live="polite"
        aria-label="Fleet Shell output"
        onClick={() => inputRef.current?.focus()}
      >
        {transcript.length === 0 ? (
          <div className="fleet-shell-empty">transcript cleared · type `help`</div>
        ) : transcript.map((entry) => (
          <div className="fleet-shell-entry" key={entry.id}>
            {entry.command && (
              <div className="fleet-shell-command">
                <span className="fleet-shell-prompt">[{entry.prompt}] $</span> {entry.command}
              </div>
            )}
            {entry.lines.map((outputLine, index) => (
              <div className={`fleet-shell-line ${outputLine.tone ?? 'normal'}`} key={`${entry.id}-${index}`}>
                {outputLine.text || '\u00a0'}
              </div>
            ))}
          </div>
        ))}
      </div>

      <div className="fleet-shell-quick" aria-label="Fleet Shell quick commands">
        <span>quick:</span>
        {QUICK_COMMANDS.map((command) => (
          <button type="button" key={command} onClick={() => execute(command)}>
            {command}
          </button>
        ))}
      </div>

      <form className="fleet-shell-form" onSubmit={submit} aria-label="Run Fleet Shell command">
        <label className="sr-only" htmlFor="fleet-shell-input">Fleet Shell command</label>
        <span className="fleet-shell-prompt" aria-hidden="true">[{prompt}] $</span>
        <input
          ref={inputRef}
          id="fleet-shell-input"
          value={input}
          onChange={(event) => {
            setInput(event.target.value);
            setHistoryCursor(null);
          }}
          onKeyDown={handleKeyDown}
          placeholder="type a command…"
          autoComplete="off"
          autoCapitalize="none"
          spellCheck={false}
          role="combobox"
          aria-autocomplete="list"
          aria-controls="fleet-shell-completions"
          aria-expanded={completions.length > 0}
        />
        <button type="submit" className="fleet-shell-run" disabled={!input.trim()} aria-label="Run command">
          run ↵
        </button>
      </form>

      {completions.length > 0 && (
        <div id="fleet-shell-completions" className="fleet-shell-completions" aria-label="Command completions">
          {completions.slice(0, 6).map((completion) => (
            <button type="button" key={completion} onClick={() => {
              setInput(`${completion} `);
              inputRef.current?.focus();
            }}>
              {completion}
            </button>
          ))}
          {completions.length > 6 && <span>+{completions.length - 6} more</span>}
        </div>
      )}
    </section>
  );
}
