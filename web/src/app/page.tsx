'use client';

import { Suspense, useEffect, useState } from 'react';
import { apiFetch } from '@/lib/api';
import {
  dockerAvailable as capDockerAvailable,
  swarmAvailable as capSwarmAvailable,
  k8sAvailable as capK8sAvailable,
  systemdAvailable as capSystemdAvailable,
} from '@/lib/capabilities';
import { useRouter, useSearchParams } from 'next/navigation';
import { useWebSocket } from '@/components/providers/WebSocketProvider';
import { useSession } from '@/components/providers/SessionProvider';
import AgentList from '@/components/AgentList';
import ServiceList from '@/components/ServiceList';
import Terminal from '@/components/Terminal';
import ConfigEditor from '@/components/ConfigEditor';
import SystemStats from '@/components/SystemStats';
import DockerHub, { DOCKER_SUBTABS, type DockerSubtab } from '@/components/DockerHub';
import KubernetesHub, { K8S_SUBTABS, type K8sSubtab } from '@/components/KubernetesHub';
import AptManager from '@/components/AptManager';
import Metrics from '@/components/Metrics';
import JournalStream from '@/components/JournalStream';
import HSplitter from '@/components/HSplitter';
import FleetOverview from '@/components/FleetOverview';
import HealthProbes from '@/components/HealthProbes';
import Backups from '@/components/Backups';
import AgentLabels from '@/components/AgentLabels';
import CommandPalette from '@/components/CommandPalette';
import AiAnalysis from '@/components/AiAnalysis';
import EeUpsell from '@/components/EeUpsell';
import { EE_TAB_FEATURE, hasFeature, type LicenseStatus } from '@/lib/eeFeatures';
import LicenseBanner from '@/components/LicenseBanner';
import { Loader2Icon, MenuIcon, XIcon } from 'lucide-react';

type Tab =
  | 'dashboard'
  | 'docker'
  | 'kubernetes'
  | 'metrics'
  | 'journal'
  | 'updates'
  | 'health'
  | 'backups'
  | 'config'
  | 'ai';

const TABS: Tab[] = [
  'dashboard',
  'docker',
  'kubernetes',
  'metrics',
  'journal',
  'updates',
  'health',
  'backups',
  'config',
  'ai',
];

const TAB_DEFS: { id: Tab; label: string; badge?: () => string | null }[] = [
  { id: 'dashboard', label: 'overview' },
  { id: 'docker', label: 'docker' },
  { id: 'kubernetes', label: 'k8s' },
  { id: 'metrics', label: 'metrics' },
  { id: 'journal', label: 'journal' },
  { id: 'updates', label: 'updates' },
  { id: 'health', label: 'health' },
  { id: 'backups', label: 'backups' },
  { id: 'config', label: 'config' },
  { id: 'ai', label: 'ai ✦' },
];

// Old top-level tabs that have moved under "docker". Keep the names
// recognised so deep-links + bookmarks land on the right subtab
// instead of falling back to overview.
const LEGACY_DOCKER_TABS: Record<string, DockerSubtab> = {
  containers: 'containers',
  stats: 'stats',
  images: 'images',
  networks: 'networks',
  volumes: 'volumes',
  stacks: 'stacks',
  deploy: 'deploy',
  prune: 'prune',
};

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
  const { isConnected, agents, agentCapabilities } = useWebSocket();
  const { user, role, mfaEnabled, status, logout } = useSession();

  const agentFromUrl = searchParams.get('agent');
  const tabFromUrl = searchParams.get('tab');
  const dockerFromUrl = searchParams.get('docker');
  const k8sFromUrl = searchParams.get('k8s');
  const initialAgent =
    agentFromUrl && agents.includes(agentFromUrl)
      ? agentFromUrl
      : agentFromUrl && agents.includes(`${agentFromUrl}-id`)
        ? `${agentFromUrl}-id`
        : null;
  // Resolve `?tab=` against current tabs; redirect legacy docker subtabs
  // (containers/images/...) to the new `docker` parent + the right
  // subtab below.
  const legacyDockerSub = tabFromUrl && LEGACY_DOCKER_TABS[tabFromUrl];
  const initialTab: Tab = legacyDockerSub
    ? 'docker'
    : TABS.includes(tabFromUrl as Tab)
      ? (tabFromUrl as Tab)
      : 'dashboard';
  const initialDockerSub: DockerSubtab = legacyDockerSub
    ? legacyDockerSub
    : DOCKER_SUBTABS.includes(dockerFromUrl as DockerSubtab)
      ? (dockerFromUrl as DockerSubtab)
      : 'containers';
  const initialK8sSub: K8sSubtab = K8S_SUBTABS.includes(k8sFromUrl as K8sSubtab)
    ? (k8sFromUrl as K8sSubtab)
    : 'pods';

  const [selectedAgent, setSelectedAgent] = useState<string | null>(initialAgent);
  const [activeTab, setActiveTab] = useState<Tab>(initialTab);
  const [dockerSubtab, setDockerSubtab] = useState<DockerSubtab>(initialDockerSub);
  const [k8sSubtab, setK8sSubtab] = useState<K8sSubtab>(initialK8sSub);

  // Per-agent capabilities reported on Register. Pre-v15 agents don't
  // have an entry here; treat absence as "show every tab" so legacy
  // hosts still work. Once an entry exists, a missing capability hides
  // the matching tab. Derived early so the redirect useEffects below
  // can reference it.
  const caps = (selectedAgent && agentCapabilities[selectedAgent]) || null;
  const dockerAvailable = capDockerAvailable(caps);
  const swarmAvailable = capSwarmAvailable(caps);
  const k8sAvailable = capK8sAvailable(caps);
  // systemd gates the apt-update, journalctl, and service-list surfaces.
  // Pre-v15 agents (caps null) keep showing them — only k8s-only Pod
  // agents that explicitly advertise without "systemd" hide the
  // matching UI.
  const systemdAvailable = capSystemdAvailable(caps);
  const [unreadCount, setUnreadCount] = useState<number>(0);
  const [sidebarOpen, setSidebarOpen] = useState(false);
  const [backupsEnabled, setBackupsEnabled] = useState(false);
  const [eeActive, setEeActive] = useState(false);
  // Features the EE license advertises (empty when EE is down or unhealthy).
  // Drives which EE surfaces are enabled vs shown disabled with an upsell.
  const [eeFeatures, setEeFeatures] = useState<string[]>([]);

  useEffect(() => {
    const cancelled = false;
    const load = async () => {
      try {
        const res = await apiFetch('/api/features');
        if (!res.ok) return;
        const data: { backups_enabled: boolean } = await res.json();
        if (!cancelled) setBackupsEnabled(data.backups_enabled);
      } catch {
        /* ignore */
      }
      try {
        // license/status is ungated — it's both the "is EE up?" probe and the
        // source of the advertised feature set.
        const res = await apiFetch('/api/ee/license/status');
        if (!cancelled && res.ok) {
          const data: LicenseStatus = await res.json();
          setEeActive(true);
          setEeFeatures(Array.isArray(data.features) ? data.features : []);
        }
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

  // If the operator deep-links into the Docker tab on an agent that
  // doesn't have docker (e.g. a future k8s-only agent), bounce them
  // to overview rather than rendering an empty rail.
  useEffect(() => {
    if (activeTab === 'docker' && caps !== null && !dockerAvailable) {
      setActiveTab('dashboard');
    }
  }, [activeTab, caps, dockerAvailable]);

  // Same fallback for the Kubernetes tab when the agent doesn't
  // advertise k8s.
  useEffect(() => {
    if (activeTab === 'kubernetes' && caps !== null && !k8sAvailable) {
      setActiveTab('dashboard');
    }
  }, [activeTab, caps, k8sAvailable]);

  // Same fallback for the apt-update / journal tabs on a no-systemd
  // (k8s-only Pod) agent.
  useEffect(() => {
    if (
      (activeTab === 'updates' || activeTab === 'journal') &&
      caps !== null &&
      !systemdAvailable
    ) {
      setActiveTab('dashboard');
    }
  }, [activeTab, caps, systemdAvailable]);

  // If swarm goes away while the operator is on the stacks subtab,
  // fall back to containers instead of rendering an empty pane.
  useEffect(() => {
    if (
      activeTab === 'docker' &&
      dockerSubtab === 'stacks' &&
      caps !== null &&
      !swarmAvailable
    ) {
      setDockerSubtab('containers');
    }
  }, [activeTab, dockerSubtab, caps, swarmAvailable]);

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
    const legacy = tabFromUrl && LEGACY_DOCKER_TABS[tabFromUrl];
    if (legacy) {
      setActiveTab('docker');
      setDockerSubtab(legacy);
    } else if (TABS.includes(tabFromUrl as Tab)) {
      setActiveTab(tabFromUrl as Tab);
    } else if (tabFromUrl === null) {
      setActiveTab('dashboard');
    }
    if (dockerFromUrl && DOCKER_SUBTABS.includes(dockerFromUrl as DockerSubtab)) {
      setDockerSubtab(dockerFromUrl as DockerSubtab);
    }
    if (k8sFromUrl && K8S_SUBTABS.includes(k8sFromUrl as K8sSubtab)) {
      setK8sSubtab(k8sFromUrl as K8sSubtab);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [agentFromUrl, tabFromUrl, dockerFromUrl, k8sFromUrl, agents.length]);

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
      // Persist the docker subtab only when relevant — keeps the URL
      // tidy for non-docker tabs and drops "?docker=containers" since
      // that's the default.
      if (activeTab === 'docker' && dockerSubtab !== 'containers') {
        params.set('docker', dockerSubtab);
      }
      if (activeTab === 'kubernetes' && k8sSubtab !== 'pods') {
        params.set('k8s', k8sSubtab);
      }
    }
    const next = params.toString();
    const current = searchParams.toString();
    if (next !== current) {
      router.replace(next ? `/?${next}` : '/', { scroll: false });
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [selectedAgent, activeTab, dockerSubtab, k8sSubtab]);

  if (status !== 'authed') {
    return (
      <div className="center-screen">
        <Loader2Icon className="w-6 h-6 animate-spin" style={{ color: 'var(--fg-2)' }} />
      </div>
    );
  }

  const agentLabel = selectedAgent?.replace(/-id$/, '');
  const tabsToShow = TAB_DEFS.filter((t) => {
    if (t.id === 'backups' && !backupsEnabled) return false;
    if (t.id === 'docker' && !dockerAvailable) return false;
    // Kubernetes tab only shows for agents that explicitly advertise
    // the "k8s" capability — i.e. the shellfleet-agent-k8s package.
    // Pre-v15 agents (caps null) still shouldn't see it; this is the
    // one tab where "show on absence" would be wrong.
    if (t.id === 'kubernetes' && !k8sAvailable) return false;
    // systemd-driven surfaces: apt update window + journalctl
    // streaming. K8s-only Pod agents skip these.
    if ((t.id === 'updates' || t.id === 'journal') && !systemdAvailable) return false;
    // EE-gated tabs: hidden entirely when EE is down; when EE is up but the
    // feature isn't licensed they stay visible but locked (disabled + upsell).
    if (EE_TAB_FEATURE[t.id] && !eeActive) return false;
    return true;
  }).map((t) => ({
    ...t,
    locked: !!EE_TAB_FEATURE[t.id] && !hasFeature(eeFeatures, EE_TAB_FEATURE[t.id].feature),
  }));

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
      <LicenseBanner />
      {sidebarOpen && (
        <div className="sidebar-backdrop" onClick={() => setSidebarOpen(false)} />
      )}

      <aside className={`sidebar ${sidebarOpen ? 'open' : ''}`}>
        <div className="brand">
          <div className="brand-row">
            <div className="brand-name">
              <span className="tilde">~/</span>shellfleet
            </div>
            <span className={`pill ${isConnected ? 'live' : 'err'}`}>
              <span className={`dot ${isConnected ? 'pulse' : ''}`} />
              {isConnected ? 'LIVE' : 'OFFLINE'}
            </span>
          </div>
          <div className="brand-meta">
            <span>{agents.length} agents</span>
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
          <button type="button" className="nav-item" onClick={() => router.push('/terminal')}>
            <span className="ico">›_</span>
            <span>Terminal</span>
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
          {eeActive && (
            <button type="button" className={`nav-item ${hasFeature(eeFeatures, 'webauthn') ? '' : 'nav-locked'}`} onClick={() => router.push('/passkeys')} title={hasFeature(eeFeatures, 'webauthn') ? '' : 'Not included in your EE license'}>
              <span className="ico">⚷</span>
              <span>Passkeys</span>
              {!hasFeature(eeFeatures, 'webauthn') && <span className="nav-badge" style={{ background: 'transparent' }}>⊘</span>}
            </button>
          )}
          {eeActive && (
            <button type="button" className={`nav-item ${hasFeature(eeFeatures, 'api-keys') ? '' : 'nav-locked'}`} onClick={() => router.push('/api-keys')} title={hasFeature(eeFeatures, 'api-keys') ? '' : 'Not included in your EE license'}>
              <span className="ico">⚿</span>
              <span>API Keys</span>
              {!hasFeature(eeFeatures, 'api-keys') && <span className="nav-badge" style={{ background: 'transparent' }}>⊘</span>}
            </button>
          )}
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
          {role === 'admin' && (
            <button type="button" className="nav-item" onClick={() => router.push('/policy')}>
              <span className="ico">⛨</span>
              <span>Access policy</span>
            </button>
          )}
        </div>

        {eeActive && role === 'admin' && (
          <>
            <div className="nav-section">ENTERPRISE</div>
            <div className="nav-list">
              {[
                { path: '/recordings', label: 'Recordings', feat: 'recordings', icon: '◉' },
                { path: '/runbooks', label: 'Runbooks', feat: 'runbooks', icon: '▦' },
                { path: '/approvals', label: 'Approvals', feat: 'approvals', icon: '◷' },
                { path: '/break-glass', label: 'Break-glass', feat: 'break-glass', icon: '⊗' },
                { path: '/drift', label: 'Drift', feat: 'drift', icon: '◷' },
                { path: '/cost', label: 'Cost', feat: 'cost', icon: '⊞' },
                { path: '/sla', label: 'SLA & uptime', feat: 'sla', icon: '▲' },
                { path: '/ip-allowlist', label: 'IP allow-list', feat: 'ip-allowlist', icon: '⊟' },
                { path: '/iam', label: 'IAM Policies', feat: 'acl', icon: '⊡' },
                { path: '/audit', label: 'Audit log', feat: 'audit-long', icon: '≣' },
                { path: '/metrics-ee', label: 'EE metrics', feat: 'metrics-multi', icon: '▤' },
                { path: '/scim', label: 'SCIM', feat: 'scim', icon: '⇄' },
              ].map((n) => {
                const locked = !hasFeature(eeFeatures, n.feat);
                return (
                  <button
                    key={n.path}
                    type="button"
                    className={`nav-item ${locked ? 'nav-locked' : ''}`}
                    onClick={() => router.push(n.path)}
                    title={locked ? 'Not included in your EE license' : undefined}
                  >
                    <span className="ico">{n.icon}</span>
                    <span>{n.label}</span>
                    {locked && <span className="nav-badge" style={{ background: 'transparent' }}>⊘</span>}
                  </button>
                );
              })}
            </div>
          </>
        )}

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
                    className={`tab ${activeTab === t.id ? 'active' : ''} ${t.locked ? 'tab-locked' : ''}`}
                    onClick={() => setActiveTab(t.id)}
                    title={t.locked ? 'Not included in your EE license' : undefined}
                  >
                    <span className="num">{String(i + 1).padStart(2, '0')}</span>
                    <span>{t.label}{t.locked ? ' ⊘' : ''}</span>
                  </button>
                ))}
              </div>
            </div>

            <div className="scroll" style={{ display: 'flex', flexDirection: 'column' }}>
              {activeTab === 'dashboard' ? (
                <HSplitter
                  storageKey="shellfleet.agent-overview.split"
                  defaultLeftPct={50}
                  minLeftPct={25}
                  maxLeftPct={80}
                  left={
                    <>
                      <div style={{ padding: 'var(--pad)', borderBottom: '1px solid var(--line)' }}>
                        <SystemStats agentId={selectedAgent} />
                      </div>
                      {systemdAvailable && (
                        <div style={{ flex: 1, padding: 'var(--pad)', overflowY: 'auto' }}>
                          <ServiceList agentId={selectedAgent} />
                        </div>
                      )}
                    </>
                  }
                  right={
                    <div
                      style={{
                        display: 'flex',
                        flexDirection: 'column',
                        flex: 1,
                        minHeight: 0,
                        background: '#06090b',
                      }}
                    >
                      <Terminal agentId={selectedAgent} />
                    </div>
                  }
                />
              ) : activeTab === 'docker' ? (
                <DockerHub
                  agentId={selectedAgent}
                  subtab={dockerSubtab}
                  onSubtabChange={setDockerSubtab}
                  swarmAvailable={swarmAvailable}
                />
              ) : activeTab === 'kubernetes' ? (
                <KubernetesHub
                  agentId={selectedAgent}
                  subtab={k8sSubtab}
                  onSubtabChange={setK8sSubtab}
                />
              ) : activeTab === 'metrics' ? (
                <Metrics agentId={selectedAgent} />
              ) : activeTab === 'journal' ? (
                <JournalStream agentId={selectedAgent} />
              ) : activeTab === 'updates' ? (
                <AptManager agentId={selectedAgent} />
              ) : activeTab === 'health' ? (
                <HealthProbes agentId={selectedAgent} />
              ) : activeTab === 'backups' && backupsEnabled ? (
                <Backups agentId={selectedAgent} />
              ) : activeTab === 'ai' ? (
                hasFeature(eeFeatures, EE_TAB_FEATURE.ai.feature) ? (
                  <AiAnalysis agentId={selectedAgent} />
                ) : (
                  <EeUpsell feature={EE_TAB_FEATURE.ai.feature} label={EE_TAB_FEATURE.ai.label} />
                )
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
