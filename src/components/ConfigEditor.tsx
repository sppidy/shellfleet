'use client';

import { useEffect, useState } from 'react';
import { useWebSocket } from './providers/WebSocketProvider';
import Editor from '@monaco-editor/react';
import { Loader2Icon } from 'lucide-react';

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
    <div className="pane" style={{ flex: 1, height: '100%' }}>
      <div
        className="panel"
        style={{ flex: 1, display: 'flex', flexDirection: 'column' }}
      >
        <div className="panel-head">
          <div className="panel-title">
            <span className="ico">▤</span> CONFIG EDITOR
            {filePath && <span className="meta">{filePath}</span>}
          </div>
          <div className="panel-actions">
            <form
              onSubmit={handleReadFile}
              style={{ display: 'flex', alignItems: 'center', gap: 6 }}
            >
              <input
                className="input"
                type="text"
                value={filePath}
                onChange={(e) => setFilePath(e.target.value)}
                placeholder="/etc/nginx/nginx.conf"
                spellCheck={false}
                style={{ width: 320, height: 26 }}
              />
              <button
                type="submit"
                className="btn"
                disabled={pending !== null || !filePath.trim()}
              >
                {pending === 'read' ? '…' : '↺ read'}
              </button>
            </form>
            <button
              className="btn primary"
              onClick={handleSaveFile}
              disabled={pending !== null || !fileContent}
            >
              {pending === 'save' ? '…' : '▼ write'}
            </button>
          </div>
        </div>

        {(error || successMsg) && (
          <div
            style={{
              padding: '6px 12px',
              fontSize: 11.5,
              fontFamily: 'var(--mono)',
              borderBottom: '1px solid var(--line)',
              background: error ? 'var(--err-bg)' : 'var(--accent-bg)',
              color: error ? 'var(--err)' : 'var(--accent)',
            }}
          >
            {error ? `× ${error}` : `✓ ${successMsg}`}
          </div>
        )}

        <div
          className="panel-body"
          style={{ flex: 1, padding: 0, position: 'relative', minHeight: 0 }}
        >
          <Editor
            height="100%"
            language={getLanguage(filePath)}
            theme="vs-dark"
            value={fileContent}
            onChange={(value) => setFileContent(value || '')}
            options={{
              minimap: { enabled: false },
              fontSize: 13,
              wordWrap: 'on',
              scrollBeyondLastLine: false,
              smoothScrolling: true,
              padding: { top: 12, bottom: 12 },
              fontFamily: "JetBrains Mono, ui-monospace, 'SF Mono', Menlo, monospace",
            }}
          />
          {!fileContent && !filePath && (
            <div
              style={{
                position: 'absolute',
                inset: 0,
                display: 'flex',
                alignItems: 'center',
                justifyContent: 'center',
                pointerEvents: 'none',
                background: 'rgba(0,0,0,0.5)',
              }}
            >
              <div
                className="empty"
                style={{
                  background: 'var(--bg-1)',
                  border: '1px solid var(--line)',
                  borderRadius: 'var(--r-lg)',
                  padding: 32,
                }}
              >
                <pre style={{ margin: 0, color: 'var(--fg-3)' }}>
                  {`┌──────────────────────────┐
│  enter a file path above │
│  to view or edit         │
└──────────────────────────┘`}
                </pre>
              </div>
            </div>
          )}
          {pending === 'read' && !fileContent && (
            <div
              style={{
                position: 'absolute',
                inset: 0,
                display: 'flex',
                alignItems: 'center',
                justifyContent: 'center',
              }}
            >
              <Loader2Icon className="w-5 h-5 animate-spin" />
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
