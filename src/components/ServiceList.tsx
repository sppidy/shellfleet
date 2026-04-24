'use client';

import { useEffect, useState } from 'react';
import { useWebSocket } from './providers/WebSocketProvider';
import { ServiceInfo } from '@/lib/types';
import { PlayIcon, SquareIcon, RefreshCwIcon } from 'lucide-react';

export default function ServiceList({ agentId }: { agentId: string }) {
  const { sendToAgent, lastAgentMessage } = useWebSocket();
  const [services, setServices] = useState<ServiceInfo[]>([]);
  const [loading, setLoading] = useState(true);

  // Fetch services when agent changes
  useEffect(() => {
    // eslint-disable-next-line react-hooks/set-state-in-effect
    setLoading(true);
    sendToAgent(agentId, { type: 'ListServicesRequest' });
  }, [agentId, sendToAgent]);

  // Listen for messages from this agent
  useEffect(() => {
    if (lastAgentMessage && lastAgentMessage.agentId === agentId) {
      const { message } = lastAgentMessage;
      if (message.type === 'ListServicesResponse') {
        // eslint-disable-next-line react-hooks/set-state-in-effect
        setServices(message.payload.services);
        // eslint-disable-next-line react-hooks/set-state-in-effect
        setLoading(false);
      } else if (message.type === 'ControlServiceResponse') {
        // Refresh services after a successful control action
        if (message.payload.success) {
          sendToAgent(agentId, { type: 'ListServicesRequest' });
        } else {
          alert(`Failed to control service: ${message.payload.error}`);
        }
      }
    }
  }, [lastAgentMessage, agentId, sendToAgent]);

  const handleControl = (name: string, action: string) => {
    sendToAgent(agentId, {
      type: 'ControlServiceRequest',
      payload: { name, action }
    });
  };

  if (loading) {
    return <div className="text-slate-500 animate-pulse">Loading services...</div>;
  }

  return (
    <div>
      <div className="flex justify-between items-center mb-4">
        <h3 className="text-lg font-medium text-slate-800">System Services</h3>
        <button 
          onClick={() => sendToAgent(agentId, { type: 'ListServicesRequest' })}
          className="text-sm flex items-center px-3 py-1.5 bg-slate-100 hover:bg-slate-200 text-slate-700 rounded-md transition-colors"
        >
          <RefreshCwIcon className="w-4 h-4 mr-2" />
          Refresh
        </button>
      </div>

      <div className="space-y-2">
        {services.slice(0, 100).map((service) => ( // Limiting to 100 for UI performance
          <div key={service.name} className="flex items-center justify-between p-3 bg-white border border-slate-200 rounded-lg shadow-sm">
            <div className="overflow-hidden mr-4">
              <div className="font-medium text-slate-900 truncate" title={service.name}>
                {service.name}
              </div>
              <div className="text-xs text-slate-500 truncate mt-0.5" title={service.description}>
                {service.description}
              </div>
              <div className="flex items-center mt-1 space-x-2">
                <span className={`inline-flex items-center px-2 py-0.5 rounded text-xs font-medium ${
                  service.active_state === 'active' ? 'bg-green-100 text-green-800' : 'bg-slate-100 text-slate-800'
                }`}>
                  {service.active_state}
                </span>
                <span className="text-xs text-slate-400">{service.status}</span>
              </div>
            </div>
            
            <div className="flex space-x-1 flex-shrink-0">
              <button
                title="Start"
                onClick={() => handleControl(service.name, 'start')}
                className="p-1.5 text-slate-500 hover:text-green-600 hover:bg-green-50 rounded"
              >
                <PlayIcon className="w-4 h-4" />
              </button>
              <button
                title="Stop"
                onClick={() => handleControl(service.name, 'stop')}
                className="p-1.5 text-slate-500 hover:text-red-600 hover:bg-red-50 rounded"
              >
                <SquareIcon className="w-4 h-4" />
              </button>
              <button
                title="Restart"
                onClick={() => handleControl(service.name, 'restart')}
                className="p-1.5 text-slate-500 hover:text-blue-600 hover:bg-blue-50 rounded"
              >
                <RefreshCwIcon className="w-4 h-4" />
              </button>
            </div>
          </div>
        ))}
      </div>
    </div>
  );
}
