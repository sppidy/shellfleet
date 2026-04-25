'use client';

import { useEffect, useState } from 'react';
import { useWebSocket } from './providers/WebSocketProvider';
import Editor from '@monaco-editor/react';
import { SaveIcon, FileTextIcon, AlertCircleIcon, CheckCircleIcon, Loader2Icon } from 'lucide-react';

export default function ConfigEditor({ agentId }: { agentId: string }) {
  const { sendToAgent, onAgentMessage } = useWebSocket();
  const [filePath, setFilePath] = useState('');
  const [fileContent, setFileContent] = useState('');
  const [pending, setPending] = useState<'read' | 'save' | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [successMsg, setSuccessMsg] = useState<string | null>(null);

  useEffect(() => {
    const unsubscribe = onAgentMessage(agentId, (msg) => {
      if (msg.type === 'ReadConfigResponse') {
        setPending(null);
        if (msg.payload.error) {
          setError(`Failed to read file: ${msg.payload.error}`);
          setFileContent('');
          setSuccessMsg(null);
        } else {
          setFileContent(msg.payload.content);
          setError(null);
          setSuccessMsg(`Loaded ${msg.payload.path}`);
        }
      } else if (msg.type === 'WriteConfigResponse') {
        setPending(null);
        if (msg.payload.error) {
          setError(`Failed to save file: ${msg.payload.error}`);
          setSuccessMsg(null);
        } else {
          setError(null);
          setSuccessMsg(`Saved ${msg.payload.path}`);
        }
      }
    });
    return unsubscribe;
  }, [agentId, onAgentMessage]);

  useEffect(() => {
    if (!successMsg) return;
    const t = setTimeout(() => setSuccessMsg(null), 3000);
    return () => clearTimeout(t);
  }, [successMsg]);

  const handleReadFile = (e: React.FormEvent) => {
    e.preventDefault();
    if (!filePath.trim()) return;
    setPending('read');
    setError(null);
    setSuccessMsg(null);
    sendToAgent(agentId, {
      type: 'ReadConfigRequest',
      payload: { path: filePath.trim() },
    });
  };

  const handleSaveFile = () => {
    if (!filePath.trim()) return;
    setPending('save');
    setError(null);
    setSuccessMsg(null);
    sendToAgent(agentId, {
      type: 'WriteConfigRequest',
      payload: { path: filePath.trim(), content: fileContent },
    });
  };

  const getLanguage = (path: string) => {
    const ext = path.split('.').pop()?.toLowerCase();
    switch (ext) {
      case 'json': return 'json';
      case 'yaml': case 'yml': return 'yaml';
      case 'toml': return 'toml';
      case 'xml': return 'xml';
      case 'ini': case 'conf': return 'ini';
      case 'sh': case 'bash': return 'shell';
      case 'py': return 'python';
      case 'js': case 'ts': return 'javascript';
      case 'html': return 'html';
      case 'css': return 'css';
      default: return 'plaintext';
    }
  };

  return (
    <div className="flex flex-col h-full bg-slate-950">
      <div className="p-4 border-b border-slate-800 bg-slate-900 flex items-center justify-between">
        <form onSubmit={handleReadFile} className="flex flex-1 items-center max-w-2xl">
          <FileTextIcon className="w-5 h-5 text-slate-500 mr-2" />
          <input
            type="text"
            value={filePath}
            onChange={(e) => setFilePath(e.target.value)}
            placeholder="Enter absolute file path (e.g. /etc/nginx/nginx.conf)"
            spellCheck={false}
            className="flex-1 bg-slate-950 border border-slate-700 rounded-md px-3 py-1.5 text-sm text-slate-100 placeholder:text-slate-500 focus:outline-none focus:ring-1 focus:ring-blue-500 focus:border-blue-500"
          />
          <button
            type="submit"
            disabled={pending !== null || !filePath.trim()}
            className="ml-3 px-4 py-1.5 bg-slate-700 text-white text-sm font-medium rounded-md hover:bg-slate-600 disabled:opacity-50 disabled:cursor-not-allowed transition-colors inline-flex items-center gap-2"
          >
            {pending === 'read' && <Loader2Icon className="w-3.5 h-3.5 animate-spin" />}
            Read
          </button>
        </form>

        <button
          onClick={handleSaveFile}
          disabled={pending !== null || !fileContent}
          className="ml-4 flex items-center px-4 py-1.5 bg-blue-600 text-white text-sm font-medium rounded-md hover:bg-blue-500 disabled:opacity-50 disabled:cursor-not-allowed transition-colors shadow-sm"
        >
          {pending === 'save' ? (
            <Loader2Icon className="w-4 h-4 mr-2 animate-spin" />
          ) : (
            <SaveIcon className="w-4 h-4 mr-2" />
          )}
          Save
        </button>
      </div>

      {(error || successMsg) && (
        <div
          className={`px-4 py-2 text-sm flex items-center border-b ${
            error
              ? 'bg-red-500/10 text-red-300 border-red-500/30'
              : 'bg-emerald-500/10 text-emerald-300 border-emerald-500/30'
          }`}
        >
          {error ? (
            <AlertCircleIcon className="w-4 h-4 mr-2" />
          ) : (
            <CheckCircleIcon className="w-4 h-4 mr-2" />
          )}
          {error || successMsg}
        </div>
      )}

      <div className="flex-1 border-b border-slate-800 relative">
        <Editor
          height="100%"
          language={getLanguage(filePath)}
          theme="vs-dark"
          value={fileContent}
          onChange={(value) => setFileContent(value || '')}
          options={{
            minimap: { enabled: false },
            fontSize: 14,
            wordWrap: 'on',
            scrollBeyondLastLine: false,
            smoothScrolling: true,
            padding: { top: 16, bottom: 16 },
          }}
        />
        {!fileContent && !filePath && (
          <div className="absolute inset-0 flex items-center justify-center pointer-events-none bg-slate-950/80 z-10">
            <div className="text-slate-500 flex flex-col items-center">
              <FileTextIcon className="w-12 h-12 mb-2 opacity-50" />
              <p>Enter a file path above to view or edit</p>
            </div>
          </div>
        )}
      </div>
    </div>
  );
}
