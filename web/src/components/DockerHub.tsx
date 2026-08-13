'use client';

import Containers from './Containers';
import ContainerStats from './ContainerStats';
import ContainerImages from './ContainerImages';
import ContainerNetworks from './ContainerNetworks';
import ContainerVolumes from './ContainerVolumes';
import SwarmStacks from './SwarmStacks';
import SystemPrune from './SystemPrune';
import Deploy from './Deploy';

export type DockerSubtab =
  | 'containers'
  | 'stats'
  | 'images'
  | 'networks'
  | 'volumes'
  | 'stacks'
  | 'deploy'
  | 'prune';

export const DOCKER_SUBTABS: DockerSubtab[] = [
  'containers',
  'stats',
  'images',
  'networks',
  'volumes',
  'stacks',
  'deploy',
  'prune',
];

const SUBTAB_DEFS: { id: DockerSubtab; label: string; hint?: string }[] = [
  { id: 'containers', label: 'containers', hint: 'running + stopped' },
  { id: 'stats',      label: 'stats',      hint: 'live cpu/mem' },
  { id: 'images',     label: 'images' },
  { id: 'networks',   label: 'networks' },
  { id: 'volumes',    label: 'volumes' },
  { id: 'stacks',     label: 'stacks',     hint: 'swarm' },
  { id: 'deploy',     label: 'deploy',     hint: 'compose / service' },
  { id: 'prune',      label: 'prune',      hint: 'reclaim space' },
];

type Props = {
  agentId: string;
  subtab: DockerSubtab;
  onSubtabChange: (s: DockerSubtab) => void;
  /**
   * When false, the Stacks subtab is hidden — the agent's docker
   * daemon isn't part of a swarm, so listing services would return
   * empty anyway. Defaults to true (legacy + non-gated cases).
   */
  swarmAvailable?: boolean;
};

export default function DockerHub({
  agentId,
  subtab,
  onSubtabChange,
  swarmAvailable = true,
}: Props) {
  const visibleSubtabs = swarmAvailable
    ? SUBTAB_DEFS
    : SUBTAB_DEFS.filter((s) => s.id !== 'stacks');
  return (
    <div style={{ display: 'flex', flex: 1, minHeight: 0 }}>
      <nav
        aria-label="docker subnav"
        style={{
          width: 168,
          flexShrink: 0,
          borderRight: '1px solid var(--line)',
          background: 'var(--bg-1)',
          padding: '8px 0',
          overflowY: 'auto',
          display: 'flex',
          flexDirection: 'column',
          gap: 2,
        }}
      >
        <div
          style={{
            padding: '4px 12px 8px',
            color: 'var(--fg-3)',
            fontFamily: 'var(--mono)',
            fontSize: 10,
            letterSpacing: '0.08em',
            textTransform: 'uppercase',
          }}
        >
          docker
        </div>
        {visibleSubtabs.map((s, i) => {
          const active = s.id === subtab;
          return (
            <button
              key={s.id}
              type="button"
              onClick={() => onSubtabChange(s.id)}
              style={{
                display: 'flex',
                flexDirection: 'column',
                alignItems: 'flex-start',
                gap: 2,
                width: '100%',
                padding: '6px 12px',
                border: 0,
                background: active ? 'var(--bg-2)' : 'transparent',
                borderLeft: `2px solid ${active ? 'var(--accent)' : 'transparent'}`,
                cursor: 'pointer',
                fontFamily: 'var(--mono)',
                color: active ? 'var(--fg)' : 'var(--fg-2)',
                textAlign: 'left',
              }}
            >
              <span style={{ display: 'inline-flex', gap: 8, alignItems: 'baseline', fontSize: 12 }}>
                <span style={{ color: 'var(--fg-3)', fontSize: 10 }}>
                  {String(i + 1).padStart(2, '0')}
                </span>
                <span>{s.label}</span>
              </span>
              {s.hint && (
                <span style={{ fontSize: 10, color: 'var(--fg-3)', paddingLeft: 22 }}>
                  {s.hint}
                </span>
              )}
            </button>
          );
        })}
      </nav>

      <div style={{ flex: 1, minWidth: 0, display: 'flex', flexDirection: 'column' }}>
        {subtab === 'containers' && <Containers agentId={agentId} />}
        {subtab === 'stats'      && <ContainerStats agentId={agentId} />}
        {subtab === 'images'     && <ContainerImages agentId={agentId} />}
        {subtab === 'networks'   && <ContainerNetworks agentId={agentId} />}
        {subtab === 'volumes'    && <ContainerVolumes agentId={agentId} />}
        {subtab === 'stacks'     && <SwarmStacks agentId={agentId} />}
        {subtab === 'deploy'     && <Deploy agentId={agentId} />}
        {subtab === 'prune'      && <SystemPrune agentId={agentId} />}
      </div>
    </div>
  );
}
