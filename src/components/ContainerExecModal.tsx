'use client';

import Terminal from './Terminal';
import { XIcon, InfoIcon } from 'lucide-react';

export default function ContainerExecModal({
  agentId,
  containerId,
  containerName,
  shell,
  onClose,
}: {
  agentId: string;
  containerId: string;
  containerName: string;
  shell?: string;
  onClose: () => void;
}) {
  return (
    <div
      className="fixed inset-0 z-50 bg-slate-950/70 backdrop-blur-sm flex items-center justify-center p-4"
      onClick={onClose}
    >
      <div
        className="bg-slate-900 border border-slate-800 rounded-lg shadow-2xl max-w-4xl w-full h-[80vh] flex flex-col"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="px-4 py-3 border-b border-slate-800 flex items-center justify-between">
          <div className="min-w-0">
            <h3 className="text-sm font-semibold text-slate-100 truncate">
              docker exec — {containerName}
            </h3>
            <p className="text-[11px] text-slate-500 mt-0.5 truncate">
              container <code>{containerId.slice(0, 12)}</code> · shell{' '}
              <code>{shell ?? 'sh'}</code>
            </p>
          </div>
          <button
            type="button"
            onClick={onClose}
            aria-label="Close"
            className="p-1 rounded text-slate-400 hover:bg-slate-800"
          >
            <XIcon className="w-4 h-4" />
          </button>
        </div>
        <div className="px-3 py-2 border-b border-slate-800 text-[11px] text-slate-400 flex items-start gap-2">
          <InfoIcon className="w-3.5 h-3.5 mt-0.5 text-slate-500 shrink-0" />
          <span>
            One exec session per host at a time. Closing this modal kills the
            PTY on the agent — nothing keeps running in the background.
          </span>
        </div>
        <div className="flex-1 min-h-0 bg-slate-950">
          <Terminal
            agentId={agentId}
            containerId={containerId}
            shell={shell}
            title={`exec ${containerName}`}
          />
        </div>
      </div>
    </div>
  );
}
