'use client';

import { useEffect, useState } from 'react';
import { useRouter } from 'next/navigation';
import { useWebSocket } from '@/components/providers/WebSocketProvider';
import { useSession } from '@/components/providers/SessionProvider';
import AgentList from '@/components/AgentList';
import ServiceList from '@/components/ServiceList';
import Terminal from '@/components/Terminal';
import ConfigEditor from '@/components/ConfigEditor';
import {
  LayoutDashboardIcon,
  FileCode2Icon,
  PlusIcon,
  LogOutIcon,
  Loader2Icon,
  ServerIcon,
  KeyIcon,
} from 'lucide-react';

export default function Home() {
  const router = useRouter();
  const { isConnected, agents } = useWebSocket();
  const { user, status, logout } = useSession();
  const [selectedAgent, setSelectedAgent] = useState<string | null>(null);
  const [activeTab, setActiveTab] = useState<'dashboard' | 'config'>('dashboard');

  useEffect(() => {
    if (status === 'guest') {
      router.replace('/login');
    }
  }, [status, router]);

  // If the selected agent disconnects, drop the selection so the empty
  // state shows up instead of a panel pointing at nothing.
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
    <div className="flex h-screen overflow-hidden">
      <aside className="w-72 bg-slate-900 text-slate-100 flex flex-col shadow-lg z-10">
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
            onClick={() => router.push('/device')}
            className="mt-3 w-full inline-flex items-center justify-center gap-1.5 text-xs font-medium py-2 px-3 rounded-md bg-blue-600 hover:bg-blue-500 text-white transition-colors"
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
            <div className="truncate text-slate-200" title={user ?? ''}>
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

      <main className="flex-1 flex flex-col bg-white overflow-hidden">
        {selectedAgent ? (
          <div className="flex-1 flex flex-col h-full overflow-hidden">
            <div className="border-b bg-slate-50 flex flex-col">
              <div className="px-6 py-4 flex items-center justify-between">
                <div className="flex items-center gap-3">
                  <ServerIcon className="w-5 h-5 text-slate-500" />
                  <h2 className="text-xl font-semibold text-slate-800">{agentLabel}</h2>
                </div>
                <span className="inline-flex items-center gap-1.5 text-xs text-emerald-700 bg-emerald-100 border border-emerald-200 rounded-full px-2 py-0.5">
                  <span className="w-1.5 h-1.5 rounded-full bg-emerald-500" />
                  Connected
                </span>
              </div>
              <div className="flex px-4 space-x-2 border-t border-slate-200">
                <TabButton
                  active={activeTab === 'dashboard'}
                  onClick={() => setActiveTab('dashboard')}
                  icon={<LayoutDashboardIcon className="w-4 h-4 mr-2" />}
                  label="Dashboard & Terminal"
                />
                <TabButton
                  active={activeTab === 'config'}
                  onClick={() => setActiveTab('config')}
                  icon={<FileCode2Icon className="w-4 h-4 mr-2" />}
                  label="Config Editor"
                />
              </div>
            </div>

            <div className="flex-1 overflow-hidden flex flex-col">
              {activeTab === 'dashboard' ? (
                <div className="flex-1 flex overflow-hidden">
                  <div className="w-1/2 p-4 overflow-y-auto border-r border-slate-200">
                    <ServiceList agentId={selectedAgent} />
                  </div>
                  <div className="w-1/2 bg-slate-950">
                    <Terminal agentId={selectedAgent} />
                  </div>
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
          ? 'border-blue-600 text-blue-600'
          : 'border-transparent text-slate-500 hover:text-slate-700 hover:border-slate-300'
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
}: {
  isConnected: boolean;
  agentCount: number;
  onAddAgent: () => void;
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
          <div className="inline-flex w-12 h-12 items-center justify-center rounded-lg bg-blue-50 text-blue-500 mb-4">
            <ServerIcon className="w-6 h-6" />
          </div>
          <h3 className="text-base font-semibold text-slate-800 mb-1">No agents connected</h3>
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
    <div className="flex-1 flex items-center justify-center text-slate-400">
      <p className="text-sm">Select an agent from the sidebar to manage it.</p>
    </div>
  );
}
