'use client';

import { useWebSocket } from '@/components/providers/WebSocketProvider';
import { useState } from 'react';
import AgentList from '@/components/AgentList';
import ServiceList from '@/components/ServiceList';
import Terminal from '@/components/Terminal';
import ConfigEditor from '@/components/ConfigEditor';
import { LayoutDashboardIcon, FileCode2Icon } from 'lucide-react';

export default function Home() {
  const { isConnected } = useWebSocket();
  const [selectedAgent, setSelectedAgent] = useState<string | null>(null);
  const [activeTab, setActiveTab] = useState<'dashboard' | 'config'>('dashboard');

  return (
    <div className="flex h-screen overflow-hidden">
      {/* Sidebar for Agents */}
      <div className="w-64 bg-slate-900 text-slate-100 flex flex-col shadow-lg z-10">
        <div className="p-4 border-b border-slate-800">
          <h1 className="text-xl font-bold">Sys Manager</h1>
          <div className="flex items-center mt-2 text-sm text-slate-400">
            <span className={`w-2 h-2 rounded-full mr-2 ${isConnected ? 'bg-green-500' : 'bg-red-500'}`}></span>
            {isConnected ? 'Connected' : 'Disconnected'}
          </div>
        </div>
        <div className="flex-1 overflow-y-auto">
          <AgentList selectedAgent={selectedAgent} onSelectAgent={setSelectedAgent} />
        </div>
      </div>

      {/* Main Content Area */}
      <div className="flex-1 flex flex-col bg-white overflow-hidden">
        {selectedAgent ? (
          <div className="flex-1 flex flex-col h-full overflow-hidden">
            {/* Header and Tabs */}
            <div className="p-0 border-b bg-slate-50 flex flex-col">
              <div className="px-6 py-4">
                <h2 className="text-2xl font-semibold text-slate-800">{selectedAgent.replace('-id', '')}</h2>
              </div>
              <div className="flex px-4 space-x-2 border-t border-slate-200">
                <button
                  onClick={() => setActiveTab('dashboard')}
                  className={`px-4 py-2 text-sm font-medium flex items-center border-b-2 transition-colors ${
                    activeTab === 'dashboard' 
                      ? 'border-blue-600 text-blue-600' 
                      : 'border-transparent text-slate-500 hover:text-slate-700 hover:border-slate-300'
                  }`}
                >
                  <LayoutDashboardIcon className="w-4 h-4 mr-2" />
                  Dashboard & Terminal
                </button>
                <button
                  onClick={() => setActiveTab('config')}
                  className={`px-4 py-2 text-sm font-medium flex items-center border-b-2 transition-colors ${
                    activeTab === 'config' 
                      ? 'border-blue-600 text-blue-600' 
                      : 'border-transparent text-slate-500 hover:text-slate-700 hover:border-slate-300'
                  }`}
                >
                  <FileCode2Icon className="w-4 h-4 mr-2" />
                  Config Editor
                </button>
              </div>
            </div>
            
            {/* Tab Content */}
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
          <div className="flex-1 flex flex-col items-center justify-center text-slate-400">
            {!isConnected ? (
              <div className="text-center">
                <p className="mb-4">You are disconnected. You may need to authenticate.</p>
                <a 
                  href="/login" 
                  className="inline-flex items-center px-4 py-2 bg-blue-600 hover:bg-blue-700 text-white text-sm font-medium rounded-md shadow-sm transition-colors"
                >
                  Go to Login
                </a>
              </div>
            ) : (
              <p>Select an agent from the sidebar to manage it.</p>
            )}
          </div>
        )}
      </div>
    </div>
  );
}
