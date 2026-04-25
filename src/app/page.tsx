'use client';

import { useEffect, useState } from 'react';
import { useRouter } from 'next/navigation';
import { useWebSocket } from '@/components/providers/WebSocketProvider';
import { useSession } from '@/components/providers/SessionProvider';
import AgentList from '@/components/AgentList';
import ServiceList from '@/components/ServiceList';
import Terminal from '@/components/Terminal';
import ConfigEditor from '@/components/ConfigEditor';
import SystemStats from '@/components/SystemStats';
import Containers from '@/components/Containers';
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
} from 'lucide-react';

type Tab = 'dashboard' | 'containers' | 'config';

export default function Home() {
  const router = useRouter();
  const { isConnected, agents } = useWebSocket();
  const { user, status, logout } = useSession();
  const [selectedAgent, setSelectedAgent] = useState<string | null>(null);
  const [activeTab, setActiveTab] = useState<Tab>('dashboard');

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

  if (status === 'loading' || status === 'guest') {
    return (
      <div className="flex-1 flex items-center justify-center text-slate-500">
        <Loader2Icon className="w-6 h-6 animate-spin" />
      </div>
    );
  }

  const agentLabel = selectedAgent?.replace(/-id$/, '');

  return (
    <div className="flex h-screen overflow-hidden bg-slate-950 text-slate-100">
      <aside className="w-72 bg-slate-900 border-r border-slate-800 flex flex-col z-10">
        <div className="p-4 border-b border-slate-800">
          <div className="flex items-center justify-between">
            <h1 className="text-lg font-semibold">Sys Manager</h1>
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
          </div>
          <button
            type="button"
            onClick={() => router.push('/overview')}
            className="mt-3 w-full inline-flex items-center justify-center gap-1.5 text-xs font-medium py-2 px-3 rounded-md bg-slate-800 hover:bg-slate-700 text-slate-100 border border-slate-700 transition-colors"
          >
            <GaugeIcon className="w-3.5 h-3.5" />
            Fleet overview
          </button>
          <button
            type="button"
            onClick={() => router.push('/device')}
            className="mt-2 w-full inline-flex items-center justify-center gap-1.5 text-xs font-medium py-2 px-3 rounded-md bg-blue-600 hover:bg-blue-500 text-white transition-colors"
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

      <main className="flex-1 flex flex-col overflow-hidden">
        {selectedAgent ? (
          <div className="flex-1 flex flex-col h-full overflow-hidden">
            <div className="border-b border-slate-800 bg-slate-900 flex flex-col">
              <div className="px-6 py-4 flex items-center justify-between">
                <div className="flex items-center gap-3">
                  <ServerIcon className="w-5 h-5 text-slate-500" />
                  <h2 className="text-xl font-semibold text-slate-100">{agentLabel}</h2>
                </div>
                <span className="inline-flex items-center gap-1.5 text-xs text-emerald-300 bg-emerald-500/10 border border-emerald-500/30 rounded-full px-2 py-0.5">
                  <span className="w-1.5 h-1.5 rounded-full bg-emerald-400" />
                  Connected
                </span>
              </div>
              <div className="flex px-4 space-x-2 border-t border-slate-800">
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
                  active={activeTab === 'config'}
                  onClick={() => setActiveTab('config')}
                  icon={<FileCode2Icon className="w-4 h-4 mr-2" />}
                  label="Config Editor"
                />
              </div>
            </div>

            <div className="flex-1 overflow-hidden flex flex-col bg-slate-950">
              {activeTab === 'dashboard' ? (
                <div className="flex-1 flex overflow-hidden">
                  <div className="w-1/2 flex flex-col border-r border-slate-800 overflow-hidden">
                    <div className="p-4 border-b border-slate-800">
                      <SystemStats agentId={selectedAgent} />
                    </div>
                    <div className="flex-1 p-4 overflow-y-auto relative">
                      <ServiceList agentId={selectedAgent} />
                    </div>
                  </div>
                  <div className="w-1/2 bg-slate-950">
                    <Terminal agentId={selectedAgent} />
                  </div>
                </div>
              ) : activeTab === 'containers' ? (
                <div className="flex-1 overflow-y-auto p-6">
                  <Containers agentId={selectedAgent} />
                </div>
              ) : (
                <div className="flex-1 overflow-hidden">
                  <ConfigEditor agentId={selectedAgent} />
                </div>
              )}
            </div>
          </div>
        ) : (
          <EmptyState
            isConnected={isConnected}
            agentCount={agents.length}
            onAddAgent={() => router.push('/device')}
            onOverview={() => router.push('/overview')}
          />
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

function EmptyState({
  isConnected,
  agentCount,
  onAddAgent,
  onOverview,
}: {
  isConnected: boolean;
  agentCount: number;
  onAddAgent: () => void;
  onOverview: () => void;
}) {
  if (!isConnected) {
    return (
      <div className="flex-1 flex items-center justify-center text-slate-500">
        <div className="text-center max-w-sm">
          <Loader2Icon className="w-6 h-6 animate-spin mx-auto mb-3 text-slate-400" />
          <p className="text-sm">Reconnecting to the server…</p>
        </div>
      </div>
    );
  }
  if (agentCount === 0) {
    return (
      <div className="flex-1 flex items-center justify-center text-slate-500">
        <div className="text-center max-w-sm px-6">
          <div className="inline-flex w-12 h-12 items-center justify-center rounded-lg bg-blue-500/10 text-blue-400 mb-4">
            <ServerIcon className="w-6 h-6" />
          </div>
          <h3 className="text-base font-semibold text-slate-100 mb-1">No agents connected</h3>
          <p className="text-sm text-slate-500 mb-4">
            Run the sys-manager-agent on a host, then approve its pairing code below.
          </p>
          <button
            type="button"
            onClick={onAddAgent}
            className="inline-flex items-center gap-1.5 text-sm font-medium py-2 px-3 rounded-md bg-blue-600 hover:bg-blue-500 text-white transition-colors"
          >
            <PlusIcon className="w-4 h-4" />
            Connect agent
          </button>
        </div>
      </div>
    );
  }
  return (
    <div className="flex-1 flex items-center justify-center text-slate-500">
      <div className="text-center">
        <p className="text-sm mb-3">Select an agent from the sidebar to manage it.</p>
        <button
          type="button"
          onClick={onOverview}
          className="inline-flex items-center gap-1.5 text-xs font-medium py-1.5 px-3 rounded-md border border-slate-700 text-slate-300 hover:bg-slate-800 transition-colors"
        >
          <GaugeIcon className="w-3.5 h-3.5" />
          Or open the fleet overview
        </button>
      </div>
    </div>
  );
}
