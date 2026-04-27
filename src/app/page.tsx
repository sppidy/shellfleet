'use client';

import { Suspense, useEffect, useState } from 'react';
import { apiFetch } from '@/lib/api';
import { useRouter, useSearchParams } from 'next/navigation';
import { useWebSocket } from '@/components/providers/WebSocketProvider';
import { useSession } from '@/components/providers/SessionProvider';
import AgentList from '@/components/AgentList';
import ServiceList from '@/components/ServiceList';
import Terminal from '@/components/Terminal';
import ConfigEditor from '@/components/ConfigEditor';
import SystemStats from '@/components/SystemStats';
import Containers from '@/components/Containers';
import ContainerImages from '@/components/ContainerImages';
import ContainerNetworks from '@/components/ContainerNetworks';
import ContainerVolumes from '@/components/ContainerVolumes';
import SwarmStacks from '@/components/SwarmStacks';
import ContainerStats from '@/components/ContainerStats';
import SystemPrune from '@/components/SystemPrune';
import AptManager from '@/components/AptManager';
import Metrics from '@/components/Metrics';
import JournalStream from '@/components/JournalStream';
import FleetOverview from '@/components/FleetOverview';
import Deploy from '@/components/Deploy';
import HealthProbes from '@/components/HealthProbes';
import Backups from '@/components/Backups';
import AgentLabels from '@/components/AgentLabels';
import CommandPalette from '@/components/CommandPalette';
import { Loader2Icon, MenuIcon, XIcon } from 'lucide-react';

type Tab =
  | 'dashboard'
  | 'containers'
  | 'stats'
  | 'metrics'
  | 'journal'
  | 'images'
  | 'networks'
  | 'volumes'
  | 'stacks'
  | 'prune'
  | 'deploy'
  | 'updates'
  | 'health'
  | 'backups'
  | 'config';

const TABS: Tab[] = [
  'dashboard',
  'containers',
  'stats',
  'metrics',
  'journal',
  'images',
  'networks',
  'volumes',
  'stacks',
  'prune',
  'deploy',
  'updates',
  'health',
  'backups',
  'config',
];

const TAB_DEFS: { id: Tab; label: string; badge?: () => string | null }[] = [
  { id: 'dashboard', label: 'overview' },
  { id: 'containers', label: 'containers' },
  { id: 'stats', label: 'stats' },
  { id: 'metrics', label: 'metrics' },
  { id: 'journal', label: 'journal' },
  { id: 'images', label: 'images' },
  { id: 'networks', label: 'networks' },
  { id: 'volumes', label: 'volumes' },
  { id: 'stacks', label: 'stacks' },
  { id: 'prune', label: 'prune' },
  { id: 'deploy', label: 'deploy' },
  { id: 'updates', label: 'updates' },
  { id: 'health', label: 'health' },
  { id: 'backups', label: 'backups' },
  { id: 'config', label: 'config' },
];

export default function Home() {
  return (
    <Suspense
      fallback={
        <div className="center-screen">
          <Loader2Icon className="w-6 h-6 animate-spin" style={{ color: 'var(--fg-2)' }} />
        </div>
      }
    >
      <HomeBody />
    </Suspense>
  );
}

function HomeBody() {
  const router = useRouter();
  const searchParams = useSearchParams();
  const { isConnected, agents } = useWebSocket();
  const { user, role, mfaEnabled, status, logout } = useSession();

  const agentFromUrl = searchParams.get('agent');
  const tabFromUrl = searchParams.get('tab');
  const initialAgent =
    agentFromUrl && agents.includes(agentFromUrl)
      ? agentFromUrl
      : agentFromUrl && agents.includes(`${agentFromUrl}-id`)
        ? `${agentFromUrl}-id`
        : null;
  const initialTab: Tab = TABS.includes(tabFromUrl as Tab) ? (tabFromUrl as Tab) : 'dashboard';

  const [selectedAgent, setSelectedAgent] = useState<string | null>(initialAgent);
  const [activeTab, setActiveTab] = useState<Tab>(initialTab);
  const [unreadCount, setUnreadCount] = useState<number>(0);
  const [sidebarOpen, setSidebarOpen] = useState(false);
  const [backupsEnabled, setBackupsEnabled] = useState(false);

  useEffect(() => {
    let cancelled = false;
    const load = async () => {
      try {
        const res = await apiFetch('/api/features');
        if (!res.ok) return;
        const data: { backups_enabled: boolean } = await res.json();
        if (!cancelled) setBackupsEnabled(data.backups_enabled);
      } catch {
        /* ignore */
      }
    };
    void load();
  }, []);

  useEffect(() => {
    setSidebarOpen(false);
  }, [selectedAgent, activeTab]);

  useEffect(() => {
    if (!backupsEnabled && activeTab === 'backups') {
      setActiveTab('dashboard');
    }
  }, [backupsEnabled, activeTab]);

  useEffect(() => {
    let cancelled = false;
    const load = async () => {
      try {
        const res = await apiFetch('/api/notifications/unread-count');
        if (!res.ok) return;
        const j: { unread: number } = await res.json();
        if (!cancelled) setUnreadCount(j.unread);
      } catch {
        /* ignore */
      }
    };
    void load();
    const t = setInterval(load, 10_000);
    return () => {
      cancelled = true;
      clearInterval(t);
    };
  }, []);

  useEffect(() => {
    if (agentFromUrl === null) {
      setSelectedAgent(null);
    } else {
      const candidate = agents.includes(agentFromUrl)
        ? agentFromUrl
        : agents.includes(`${agentFromUrl}-id`)
          ? `${agentFromUrl}-id`
          : null;
      setSelectedAgent(candidate);
    }
    if (TABS.includes(tabFromUrl as Tab)) {
      setActiveTab(tabFromUrl as Tab);
    } else if (tabFromUrl === null) {
      setActiveTab('dashboard');
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [agentFromUrl, tabFromUrl, agents.length]);

  useEffect(() => {
    if (status === 'guest') {
      router.replace('/login');
    } else if (status === 'pending_mfa') {
      router.replace('/mfa');
    }
  }, [status, router]);

  useEffect(() => {
    if (selectedAgent && !agents.includes(selectedAgent)) {
      setSelectedAgent(null);
    }
  }, [agents, selectedAgent]);

  useEffect(() => {
    const params = new URLSearchParams();
    if (selectedAgent) {
      params.set('agent', selectedAgent.replace(/-id$/, ''));
      if (activeTab !== 'dashboard') params.set('tab', activeTab);
    }
    const next = params.toString();
    const current = searchParams.toString();
    if (next !== current) {
      router.replace(next ? `/?${next}` : '/', { scroll: false });
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [selectedAgent, activeTab]);

  if (status !== 'authed') {
    return (
      <div className="center-screen">
        <Loader2Icon className="w-6 h-6 animate-spin" style={{ color: 'var(--fg-2)' }} />
      </div>
    );
  }

  const agentLabel = selectedAgent?.replace(/-id$/, '');
  const tabsToShow = TAB_DEFS.filter((t) => t.id !== 'backups' || backupsEnabled);

  // Breadcrumb
  let crumbs: string[];
  if (selectedAgent) {
    crumbs = ['fleet', agentLabel || '', activeTab === 'dashboard' ? 'overview' : activeTab];
  } else {
    crumbs = ['fleet', 'overview'];
  }

  function toggleTheme() {
    const cur = document.documentElement.getAttribute('data-theme') || 'dark';
    document.documentElement.setAttribute('data-theme', cur === 'dark' ? 'light' : 'dark');
  }
  function toggleDensity() {
    const cur = document.documentElement.getAttribute('data-density') || 'dense';
    document.documentElement.setAttribute('data-density', cur === 'dense' ? 'comfy' : 'dense');
  }

  return (
    <div className="app-shell">
      {sidebarOpen && (
        <div className="sidebar-backdrop" onClick={() => setSidebarOpen(false)} />
      )}

      <aside className={`sidebar ${sidebarOpen ? 'open' : ''}`}>
        <div className="brand">
          <div className="brand-row">
            <div className="brand-name">
              <span className="tilde">~/</span>sys-manager
            </div>
            <span className={`pill ${isConnected ? 'live' : 'err'}`}>
              <span className={`dot ${isConnected ? 'pulse' : ''}`} />
              {isConnected ? 'LIVE' : 'OFFLINE'}
            </span>
          </div>
          <div className="brand-meta">
            <span>v0.4.2</span>
            <span className="muted">·</span>
            <span>ws://sys-api</span>
            <button
              type="button"
              onClick={() => setSidebarOpen(false)}
              aria-label="Close menu"
              className="icon-btn mobile-only"
              style={{ marginLeft: 'auto' }}
            >
              <XIcon className="w-4 h-4" />
            </button>
          </div>
        </div>

        <div className="nav-section">CONTROL</div>
        <div className="nav-list">
          <button
            type="button"
            className={`nav-item ${selectedAgent === null ? 'active' : ''}`}
            onClick={() => {
              setSelectedAgent(null);
              router.push('/');
            }}
          >
            <span className="ico">▤</span>
            <span>Fleet overview</span>
          </button>
          <button type="button" className="nav-item" onClick={() => router.push('/fan-out')}>
            <span className="ico">↗</span>
            <span>Fan-out</span>
          </button>
          <button type="button" className="nav-item" onClick={() => router.push('/activity')}>
            <span className="ico">≡</span>
            <span>Activity</span>
          </button>
          <button
            type="button"
            className="nav-item"
            onClick={() => router.push('/notifications')}
          >
            <span className="ico">◇</span>
            <span>Notifications</span>
            {unreadCount > 0 && (
              <span className="nav-badge">{unreadCount > 99 ? '99+' : unreadCount}</span>
            )}
          </button>
        </div>

        <div className="nav-section">ACCOUNT</div>
        <div className="nav-list">
          <button type="button" className="nav-item" onClick={() => router.push('/security')}>
            <span className="ico">⌘</span>
            <span>Account &amp; 2FA</span>
            {!mfaEnabled && (
              <span
                className="nav-badge"
                title="Two-factor authentication is not enabled"
                style={{ background: 'var(--warn-bg, #2a2310)', color: 'var(--warn, #d8b65a)' }}
              >
                !
              </span>
            )}
          </button>
        </div>

        <div className="nav-section">ADMIN</div>
        <div className="nav-list">
          <button type="button" className="nav-item" onClick={() => router.push('/tokens')}>
            <span className="ico">⚿</span>
            <span>Tokens</span>
          </button>
          <button type="button" className="nav-item" onClick={() => router.push('/device')}>
            <span className="ico">＋</span>
            <span>Connect agent</span>
          </button>
          {role === 'admin' && (
            <button type="button" className="nav-item" onClick={() => router.push('/admin')}>
              <span className="ico">⌬</span>
              <span>Users &amp; seats</span>
            </button>
          )}
        </div>

        <div className="nav-section">
          <span>AGENTS</span>
          <span className="count">{agents.length} online</span>
        </div>
        <div className="agents-list">
          <AgentList selectedAgent={selectedAgent} onSelectAgent={setSelectedAgent} />
        </div>

        <div className="user-bar">
          <div className="who">
            <div className="label">SIGNED IN AS</div>
            <div
              className="name"
              title={user ?? ''}
              style={{ display: 'flex', alignItems: 'center', gap: 6 }}
            >
              <span style={{ overflow: 'hidden', textOverflow: 'ellipsis' }}>
                {user ?? '—'}
              </span>
              {role && (
                <span
                  className="chip"
                  style={{
                    fontSize: 10,
                    color:
                      role === 'admin' ? 'var(--accent)' : 'var(--fg-2)',
                    border: '1px solid var(--line)',
                    padding: '0 5px',
                    borderRadius: 3,
                  }}
                >
                  {role}
                </span>
              )}
            </div>
          </div>
          <button
            type="button"
            onClick={logout}
            className="icon-btn"
            title="Sign out"
          >
            ⏻
          </button>
        </div>
      </aside>

      <main className="main">
        <div className="topbar">
          <button
            type="button"
            onClick={() => setSidebarOpen(true)}
            aria-label="Open menu"
            className="icon-btn mobile-only"
          >
            <MenuIcon className="w-4 h-4" />
          </button>
          <div className="breadcrumb">
            <span className="prompt">$</span>
            {crumbs.map((c, i) => (
              <span key={i} style={{ display: 'inline-flex', alignItems: 'center', gap: 8 }}>
                {i > 0 && <span className="sep">/</span>}
                <span className={i === crumbs.length - 1 ? 'here' : ''}>{c}</span>
              </span>
            ))}
          </div>
          <div className="topbar-actions">
            <PaletteTrigger />
            <button
              type="button"
              className="btn ghost icon"
              title="Toggle theme"
              onClick={toggleTheme}
            >
              ◐
            </button>
            <button
              type="button"
              className="btn ghost icon"
              title="Toggle density"
              onClick={toggleDensity}
            >
              ⇿
            </button>
          </div>
        </div>

        {!mfaEnabled && (
          <div
            role="status"
            style={{
              display: 'flex',
              alignItems: 'center',
              gap: 12,
              padding: '8px 14px',
              background: 'var(--warn-bg, #2a2310)',
              borderBottom: '1px solid var(--warn-bd, #4a3a18)',
              color: 'var(--warn, #d8b65a)',
              fontFamily: 'var(--mono)',
              fontSize: 12,
            }}
          >
            <span aria-hidden>⚠</span>
            <span style={{ flex: 1 }}>
              two-factor authentication is not enabled on your account.
              add a second factor in <strong>Account &amp; 2FA</strong>.
            </span>
            <button
              type="button"
              className="btn"
              style={{ height: 26, padding: '0 10px', fontSize: 11 }}
              onClick={() => router.push('/security')}
            >
              set up 2FA →
            </button>
          </div>
        )}

        {selectedAgent ? (
          <>
            <div className="agent-header">
              <div className="agent-title">
                <span className="glyph">▢</span>
                <h2>
                  <span className="user">root</span>
                  <span className="at">@</span>
                  <span className="host">{agentLabel}</span>
                </h2>
                <span className="pill live">
                  <span className="dot pulse" />
                  connected
                </span>
                <div className="label-row" style={{ marginLeft: 8 }}>
                  <AgentLabels agentId={selectedAgent} />
                </div>
              </div>
              <div className="tabs">
                {tabsToShow.map((t, i) => (
                  <button
                    key={t.id}
                    type="button"
                    className={`tab ${activeTab === t.id ? 'active' : ''}`}
                    onClick={() => setActiveTab(t.id)}
                  >
                    <span className="num">{String(i + 1).padStart(2, '0')}</span>
                    <span>{t.label}</span>
                  </button>
                ))}
              </div>
            </div>

            <div className="scroll" style={{ display: 'flex', flexDirection: 'column' }}>
              {activeTab === 'dashboard' ? (
                <div
                  style={{
                    flex: 1,
                    display: 'grid',
                    gridTemplateColumns: '1fr 1fr',
                    minHeight: 0,
                    overflow: 'hidden',
                  }}
                  className="agent-overview-grid"
                >
                  <div
                    style={{
                      display: 'flex',
                      flexDirection: 'column',
                      borderRight: '1px solid var(--line)',
                      minHeight: 0,
                      overflow: 'hidden',
                    }}
                  >
                    <div style={{ padding: 'var(--pad)', borderBottom: '1px solid var(--line)' }}>
                      <SystemStats agentId={selectedAgent} />
                    </div>
                    <div style={{ flex: 1, padding: 'var(--pad)', overflowY: 'auto' }}>
                      <ServiceList agentId={selectedAgent} />
                    </div>
                  </div>
                  <div
                    style={{
                      display: 'flex',
                      flexDirection: 'column',
                      minHeight: 0,
                      background: '#06090b',
                    }}
                  >
                    <Terminal agentId={selectedAgent} />
                  </div>
                </div>
              ) : activeTab === 'containers' ? (
                <Containers agentId={selectedAgent} />
              ) : activeTab === 'images' ? (
                <ContainerImages agentId={selectedAgent} />
              ) : activeTab === 'networks' ? (
                <ContainerNetworks agentId={selectedAgent} />
              ) : activeTab === 'volumes' ? (
                <ContainerVolumes agentId={selectedAgent} />
              ) : activeTab === 'stacks' ? (
                <SwarmStacks agentId={selectedAgent} />
              ) : activeTab === 'stats' ? (
                <ContainerStats agentId={selectedAgent} />
              ) : activeTab === 'metrics' ? (
                <Metrics agentId={selectedAgent} />
              ) : activeTab === 'journal' ? (
                <JournalStream agentId={selectedAgent} />
              ) : activeTab === 'prune' ? (
                <SystemPrune agentId={selectedAgent} />
              ) : activeTab === 'deploy' ? (
                <Deploy agentId={selectedAgent} />
              ) : activeTab === 'updates' ? (
                <AptManager agentId={selectedAgent} />
              ) : activeTab === 'health' ? (
                <HealthProbes agentId={selectedAgent} />
              ) : activeTab === 'backups' && backupsEnabled ? (
                <Backups agentId={selectedAgent} />
              ) : (
                <ConfigEditor agentId={selectedAgent} />
              )}
            </div>
          </>
        ) : (
          <div className="scroll">
            {!isConnected && agents.length === 0 ? (
              <ReconnectingState />
            ) : (
              <FleetOverview onSelectAgent={setSelectedAgent} />
            )}
          </div>
        )}
      </main>

      <CommandPalette onSelectAgent={setSelectedAgent} />

      <style jsx>{`
        @media (max-width: 900px) {
          .agent-overview-grid {
            grid-template-columns: 1fr !important;
          }
        }
      `}</style>
    </div>
  );
}

function PaletteTrigger() {
  return (
    <button
      type="button"
      className="cmd"
      onClick={() => {
        // Synthesize Ctrl+K so CommandPalette's listener handles it.
        const ev = new KeyboardEvent('keydown', { key: 'k', ctrlKey: true, bubbles: true });
        document.dispatchEvent(ev);
      }}
    >
      <span style={{ color: 'var(--accent)' }}>⌕</span>
      <span>Search hosts, services, containers…</span>
      <span className="kbd">⌘ K</span>
    </button>
  );
}

function ReconnectingState() {
  return (
    <div
      className="empty"
      style={{ padding: 64, display: 'flex', flexDirection: 'column', alignItems: 'center' }}
    >
      <Loader2Icon className="w-6 h-6 animate-spin" style={{ marginBottom: 12 }} />
      <span>Reconnecting to the server…</span>
    </div>
  );
}
