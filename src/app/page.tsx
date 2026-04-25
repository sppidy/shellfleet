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
import AptManager from '@/components/AptManager';
import FleetOverview from '@/components/FleetOverview';
import Deploy from '@/components/Deploy';
import HealthProbes from '@/components/HealthProbes';
import Backups from '@/components/Backups';
import AgentLabels from '@/components/AgentLabels';
import {
  LayoutDashboardIcon,
  FileCode2Icon,
  PlusIcon,
  LogOutIcon,
  Loader2Icon,
  ServerIcon,
  KeyIcon,
  BoxIcon,
  GaugeIcon,
  PackageIcon,
  RocketIcon,
  ActivityIcon,
  ActivitySquareIcon,
  BellIcon,
  ArchiveIcon,
  MenuIcon,
  XIcon,
  ImageIcon,
  NetworkIcon,
  HardDriveIcon,
  LayersIcon,
} from 'lucide-react';

type Tab = 'dashboard' | 'containers' | 'images' | 'networks' | 'volumes' | 'stacks' | 'deploy' | 'updates' | 'health' | 'backups' | 'config';

const TABS: Tab[] = ['dashboard', 'containers', 'images', 'networks', 'volumes', 'stacks', 'deploy', 'updates', 'health', 'backups', 'config'];

export default function Home() {
  return (
    <Suspense
      fallback={
        <div className="flex-1 flex items-center justify-center text-slate-500">
          <Loader2Icon className="w-6 h-6 animate-spin" />
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
  const { user, status, logout } = useSession();

  const agentFromUrl = searchParams.get('agent');
  const tabFromUrl = searchParams.get('tab');
  const initialAgent =
    agentFromUrl && agents.includes(agentFromUrl)
      ? agentFromUrl
      : agentFromUrl && agents.includes(`${agentFromUrl}-id`)
        ? `${agentFromUrl}-id`
        : null;
  // initialTab uses the full TABS list (we don't yet know whether
  // backups is gated on; the effect below corrects activeTab if it
  // landed on `backups` while the feature flag is off).
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
        /* ignore — leave disabled */
      }
    };
    void load();
  }, []);

  // Auto-close mobile drawer when the user picks an agent / tab.
  useEffect(() => {
    setSidebarOpen(false);
  }, [selectedAgent, activeTab]);

  // If the operator deep-linked to ?tab=backups but backups is gated
  // off on the server, fall back to the dashboard so they don't see
  // an empty tab.
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

  // When URL params change (e.g. via browser back), pull them into state.
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
    }
  }, [status, router]);

  useEffect(() => {
    if (selectedAgent && !agents.includes(selectedAgent)) {
      setSelectedAgent(null);
    }
  }, [agents, selectedAgent]);

  // Keep the URL in sync with the current selection so it's bookmarkable.
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
    // searchParams changing here would loop; rely on selectedAgent/activeTab.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [selectedAgent, activeTab]);

  if (status === 'loading' || status === 'guest') {
    return (
      <div className="flex-1 flex items-center justify-center text-slate-500">
        <Loader2Icon className="w-6 h-6 animate-spin" />
      </div>
    );
  }

  const agentLabel = selectedAgent?.replace(/-id$/, '');

  return (
    <div className="flex h-screen overflow-hidden bg-slate-950 text-slate-100 relative">
      {/* Mobile-only top bar with hamburger. Hidden at md+. */}
      <div className="md:hidden absolute top-0 left-0 right-0 h-12 bg-slate-900 border-b border-slate-800 flex items-center justify-between px-3 z-20">
        <button
          type="button"
          onClick={() => setSidebarOpen(true)}
          aria-label="Open menu"
          className="p-1.5 rounded-md text-slate-300 hover:bg-slate-800"
        >
          <MenuIcon className="w-5 h-5" />
        </button>
        <h1 className="text-sm font-semibold text-slate-100 truncate">
          {selectedAgent ? agentLabel : 'Sys Manager'}
        </h1>
        <span
          className={`inline-flex items-center gap-1 text-[10px] px-1.5 py-0.5 rounded-full border ${
            isConnected
              ? 'border-emerald-500/30 text-emerald-400 bg-emerald-500/5'
              : 'border-red-500/30 text-red-400 bg-red-500/5'
          }`}
        >
          <span
            className={`w-1.5 h-1.5 rounded-full ${
              isConnected ? 'bg-emerald-400' : 'bg-red-400'
            }`}
          />
          {isConnected ? 'Live' : 'Offline'}
        </span>
      </div>

      {/* Drawer backdrop for mobile. */}
      {sidebarOpen && (
        <div
          className="md:hidden fixed inset-0 bg-slate-950/70 backdrop-blur-sm z-30"
          onClick={() => setSidebarOpen(false)}
        />
      )}

      <aside
        className={`fixed md:static inset-y-0 left-0 w-72 bg-slate-900 border-r border-slate-800 flex flex-col z-40 transform transition-transform md:transform-none ${
          sidebarOpen ? 'translate-x-0' : '-translate-x-full md:translate-x-0'
        }`}
      >
        <div className="p-4 border-b border-slate-800">
          <div className="flex items-center justify-between">
            <h1 className="text-lg font-semibold">Sys Manager</h1>
            <div className="flex items-center gap-2">
              <span
                className={`inline-flex items-center gap-1.5 text-xs px-2 py-0.5 rounded-full border ${
                  isConnected
                    ? 'border-emerald-500/30 text-emerald-400 bg-emerald-500/5'
                    : 'border-red-500/30 text-red-400 bg-red-500/5'
                }`}
                title={isConnected ? 'WebSocket connected' : 'WebSocket disconnected'}
              >
                <span
                  className={`w-1.5 h-1.5 rounded-full ${
                    isConnected ? 'bg-emerald-400' : 'bg-red-400'
                  }`}
                />
                {isConnected ? 'Live' : 'Offline'}
              </span>
              <button
                type="button"
                onClick={() => setSidebarOpen(false)}
                aria-label="Close menu"
                className="md:hidden p-1 rounded-md text-slate-400 hover:bg-slate-800"
              >
                <XIcon className="w-4 h-4" />
              </button>
            </div>
          </div>
          <button
            type="button"
            onClick={() => setSelectedAgent(null)}
            className={`mt-3 w-full inline-flex items-center justify-center gap-1.5 text-xs font-medium py-2 px-3 rounded-md transition-colors ${
              selectedAgent === null
                ? 'bg-blue-600 hover:bg-blue-500 text-white'
                : 'bg-slate-800 hover:bg-slate-700 text-slate-100 border border-slate-700'
            }`}
          >
            <GaugeIcon className="w-3.5 h-3.5" />
            Fleet overview
          </button>
          <button
            type="button"
            onClick={() => router.push('/device')}
            className="mt-2 w-full inline-flex items-center justify-center gap-1.5 text-xs font-medium py-2 px-3 rounded-md border border-slate-700 text-slate-300 hover:bg-slate-800 transition-colors"
          >
            <PlusIcon className="w-3.5 h-3.5" />
            Connect agent
          </button>
          <button
            type="button"
            onClick={() => router.push('/tokens')}
            className="mt-2 w-full inline-flex items-center justify-center gap-1.5 text-xs font-medium py-1.5 px-3 rounded-md border border-slate-700 text-slate-300 hover:bg-slate-800 transition-colors"
          >
            <KeyIcon className="w-3.5 h-3.5" />
            Manage tokens
          </button>
          <button
            type="button"
            onClick={() => router.push('/activity')}
            className="mt-2 w-full inline-flex items-center justify-center gap-1.5 text-xs font-medium py-1.5 px-3 rounded-md border border-slate-700 text-slate-300 hover:bg-slate-800 transition-colors"
          >
            <ActivityIcon className="w-3.5 h-3.5" />
            Activity
          </button>
          <button
            type="button"
            onClick={() => router.push('/notifications')}
            className="mt-2 w-full relative inline-flex items-center justify-center gap-1.5 text-xs font-medium py-1.5 px-3 rounded-md border border-slate-700 text-slate-300 hover:bg-slate-800 transition-colors"
          >
            <BellIcon className="w-3.5 h-3.5" />
            Notifications
            {unreadCount > 0 && (
              <span className="absolute right-2 top-1/2 -translate-y-1/2 inline-flex items-center justify-center min-w-[1.25rem] h-5 px-1.5 text-[10px] font-semibold rounded-full bg-blue-600 text-white">
                {unreadCount > 99 ? '99+' : unreadCount}
              </span>
            )}
          </button>
          <button
            type="button"
            onClick={() => router.push('/fan-out')}
            className="mt-2 w-full inline-flex items-center justify-center gap-1.5 text-xs font-medium py-1.5 px-3 rounded-md border border-slate-700 text-slate-300 hover:bg-slate-800 transition-colors"
          >
            <RocketIcon className="w-3.5 h-3.5" />
            Fan-out
          </button>
        </div>

        <div className="px-4 py-3 border-b border-slate-800 text-xs uppercase tracking-wide text-slate-500 flex items-center justify-between">
          <span>Agents</span>
          <span className="text-slate-400 normal-case tracking-normal">
            {agents.length} online
          </span>
        </div>
        <div className="flex-1 overflow-y-auto">
          <AgentList selectedAgent={selectedAgent} onSelectAgent={setSelectedAgent} />
        </div>

        <div className="p-3 border-t border-slate-800 flex items-center justify-between">
          <div className="text-xs text-slate-400 truncate">
            <div className="text-slate-500 uppercase tracking-wide text-[10px]">Signed in as</div>
            <div className="truncate text-slate-100" title={user ?? ''}>
              {user ?? '—'}
            </div>
          </div>
          <button
            type="button"
            onClick={logout}
            title="Sign out"
            className="ml-2 p-1.5 rounded-md text-slate-400 hover:text-slate-100 hover:bg-slate-800 transition-colors"
          >
            <LogOutIcon className="w-4 h-4" />
          </button>
        </div>
      </aside>

      <main className="flex-1 flex flex-col overflow-hidden pt-12 md:pt-0">
        {selectedAgent ? (
          <div className="flex-1 flex flex-col h-full overflow-hidden">
            <div className="border-b border-slate-800 bg-slate-900 flex flex-col">
              <div className="px-6 py-4 flex items-center justify-between gap-4 flex-wrap">
                <div className="flex items-center gap-3 min-w-0">
                  <ServerIcon className="w-5 h-5 text-slate-500 shrink-0" />
                  <h2 className="text-xl font-semibold text-slate-100 truncate">{agentLabel}</h2>
                  <AgentLabels agentId={selectedAgent} />
                </div>
                <span className="inline-flex items-center gap-1.5 text-xs text-emerald-300 bg-emerald-500/10 border border-emerald-500/30 rounded-full px-2 py-0.5 shrink-0">
                  <span className="w-1.5 h-1.5 rounded-full bg-emerald-400" />
                  Connected
                </span>
              </div>
              <div className="flex px-4 space-x-2 border-t border-slate-800 overflow-x-auto">
                <TabButton
                  active={activeTab === 'dashboard'}
                  onClick={() => setActiveTab('dashboard')}
                  icon={<LayoutDashboardIcon className="w-4 h-4 mr-2" />}
                  label="Overview"
                />
                <TabButton
                  active={activeTab === 'containers'}
                  onClick={() => setActiveTab('containers')}
                  icon={<BoxIcon className="w-4 h-4 mr-2" />}
                  label="Containers"
                />
                <TabButton
                  active={activeTab === 'images'}
                  onClick={() => setActiveTab('images')}
                  icon={<ImageIcon className="w-4 h-4 mr-2" />}
                  label="Images"
                />
                <TabButton
                  active={activeTab === 'networks'}
                  onClick={() => setActiveTab('networks')}
                  icon={<NetworkIcon className="w-4 h-4 mr-2" />}
                  label="Networks"
                />
                <TabButton
                  active={activeTab === 'volumes'}
                  onClick={() => setActiveTab('volumes')}
                  icon={<HardDriveIcon className="w-4 h-4 mr-2" />}
                  label="Volumes"
                />
                <TabButton
                  active={activeTab === 'stacks'}
                  onClick={() => setActiveTab('stacks')}
                  icon={<LayersIcon className="w-4 h-4 mr-2" />}
                  label="Stacks"
                />
                <TabButton
                  active={activeTab === 'deploy'}
                  onClick={() => setActiveTab('deploy')}
                  icon={<RocketIcon className="w-4 h-4 mr-2" />}
                  label="Deploy"
                />
                <TabButton
                  active={activeTab === 'updates'}
                  onClick={() => setActiveTab('updates')}
                  icon={<PackageIcon className="w-4 h-4 mr-2" />}
                  label="Updates"
                />
                <TabButton
                  active={activeTab === 'health'}
                  onClick={() => setActiveTab('health')}
                  icon={<ActivitySquareIcon className="w-4 h-4 mr-2" />}
                  label="Health"
                />
                {backupsEnabled && (
                  <TabButton
                    active={activeTab === 'backups'}
                    onClick={() => setActiveTab('backups')}
                    icon={<ArchiveIcon className="w-4 h-4 mr-2" />}
                    label="Backups"
                  />
                )}
                <TabButton
                  active={activeTab === 'config'}
                  onClick={() => setActiveTab('config')}
                  icon={<FileCode2Icon className="w-4 h-4 mr-2" />}
                  label="Config Editor"
                />
              </div>
            </div>

            <div className="flex-1 overflow-hidden flex flex-col bg-slate-950">
              {activeTab === 'dashboard' ? (
                <div className="flex-1 flex flex-col md:flex-row overflow-hidden">
                  <div className="w-full md:w-1/2 flex flex-col md:border-r border-slate-800 overflow-hidden">
                    <div className="p-4 border-b border-slate-800">
                      <SystemStats agentId={selectedAgent} />
                    </div>
                    <div className="flex-1 p-4 overflow-y-auto relative">
                      <ServiceList agentId={selectedAgent} />
                    </div>
                  </div>
                  <div className="w-full md:w-1/2 bg-slate-950 min-h-[18rem]">
                    <Terminal agentId={selectedAgent} />
                  </div>
                </div>
              ) : activeTab === 'containers' ? (
                <div className="flex-1 overflow-y-auto p-6">
                  <Containers agentId={selectedAgent} />
                </div>
              ) : activeTab === 'images' ? (
                <div className="flex-1 overflow-y-auto p-6 max-w-5xl mx-auto w-full">
                  <ContainerImages agentId={selectedAgent} />
                </div>
              ) : activeTab === 'networks' ? (
                <div className="flex-1 overflow-y-auto p-6 max-w-5xl mx-auto w-full">
                  <ContainerNetworks agentId={selectedAgent} />
                </div>
              ) : activeTab === 'volumes' ? (
                <div className="flex-1 overflow-y-auto p-6 max-w-5xl mx-auto w-full">
                  <ContainerVolumes agentId={selectedAgent} />
                </div>
              ) : activeTab === 'stacks' ? (
                <div className="flex-1 overflow-y-auto p-6 max-w-5xl mx-auto w-full">
                  <SwarmStacks agentId={selectedAgent} />
                </div>
              ) : activeTab === 'deploy' ? (
                <div className="flex-1 overflow-y-auto p-6">
                  <Deploy agentId={selectedAgent} />
                </div>
              ) : activeTab === 'updates' ? (
                <div className="flex-1 overflow-y-auto p-6 max-w-4xl mx-auto w-full">
                  <AptManager agentId={selectedAgent} />
                </div>
              ) : activeTab === 'health' ? (
                <div className="flex-1 overflow-y-auto p-6 max-w-4xl mx-auto w-full">
                  <HealthProbes agentId={selectedAgent} />
                </div>
              ) : activeTab === 'backups' && backupsEnabled ? (
                <div className="flex-1 overflow-y-auto p-6 max-w-4xl mx-auto w-full">
                  <Backups agentId={selectedAgent} />
                </div>
              ) : (
                <div className="flex-1 overflow-hidden">
                  <ConfigEditor agentId={selectedAgent} />
                </div>
              )}
            </div>
          </div>
        ) : (
          <div className="flex-1 overflow-y-auto">
            {!isConnected && agents.length === 0 ? (
              <ReconnectingState />
            ) : (
              <FleetOverview onSelectAgent={setSelectedAgent} />
            )}
          </div>
        )}
      </main>
    </div>
  );
}

function TabButton({
  active,
  onClick,
  icon,
  label,
}: {
  active: boolean;
  onClick: () => void;
  icon: React.ReactNode;
  label: string;
}) {
  return (
    <button
      onClick={onClick}
      className={`px-4 py-2 text-sm font-medium flex items-center border-b-2 transition-colors ${
        active
          ? 'border-blue-500 text-blue-400'
          : 'border-transparent text-slate-400 hover:text-slate-100 hover:border-slate-600'
      }`}
    >
      {icon}
      {label}
    </button>
  );
}

function ReconnectingState() {
  return (
    <div className="h-full flex items-center justify-center text-slate-500">
      <div className="text-center">
        <Loader2Icon className="w-6 h-6 animate-spin mx-auto mb-3 text-slate-400" />
        <p className="text-sm">Reconnecting to the server…</p>
      </div>
    </div>
  );
}
