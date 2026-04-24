'use client';

import { useState, useEffect } from 'react';
import { useWebSocket } from './providers/WebSocketProvider';
import Editor from '@monaco-editor/react';
import { SaveIcon, FileTextIcon, AlertCircleIcon } from 'lucide-react';

export default function ConfigEditor({ agentId }: { agentId: string }) {
  const { sendToAgent, lastAgentMessage } = useWebSocket();
  const [filePath, setFilePath] = useState('');
  const [fileContent, setFileContent] = useState('');
  const [isLoading, setIsLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [successMsg, setSuccessMsg] = useState<string | null>(null);

  // Listen for messages from this agent
  useEffect(() => {
    if (lastAgentMessage && lastAgentMessage.agentId === agentId) {
      const { message } = lastAgentMessage;
      
      if (message.type === 'ReadConfigResponse') {
        setIsLoading(false);
        if (message.payload.error) {
          setError(`Failed to read file: ${message.payload.error}`);
          setFileContent('');
        } else {
          setFileContent(message.payload.content);
          setError(null);
          setSuccessMsg(`Loaded ${message.payload.path}`);
          setTimeout(() => setSuccessMsg(null), 3000);
        }
      } else if (message.type === 'WriteConfigResponse') {
        setIsLoading(false);
        if (message.payload.error) {
          setError(`Failed to save file: ${message.payload.error}`);
        } else {
          setError(null);
          setSuccessMsg(`Successfully saved ${message.payload.path}`);
          setTimeout(() => setSuccessMsg(null), 3000);
        }
      }
    }
  }, [lastAgentMessage, agentId]);

  const handleReadFile = (e: React.FormEvent) => {
    e.preventDefault();
    if (!filePath.trim()) return;
    
    setIsLoading(true);
    setError(null);
    setSuccessMsg(null);
    sendToAgent(agentId, {
      type: 'ReadConfigRequest',
      payload: { path: filePath.trim() }
    });
  };

  const handleSaveFile = () => {
    if (!filePath.trim()) return;
    
    setIsLoading(true);
    setError(null);
    setSuccessMsg(null);
    sendToAgent(agentId, {
      type: 'WriteConfigRequest',
      payload: { path: filePath.trim(), content: fileContent }
    });
  };

  // Determine language based on file extension
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
    <div className="flex flex-col h-full bg-white">
      {/* Top Bar for File Path & Actions */}
      <div className="p-4 border-b border-slate-200 bg-slate-50 flex items-center justify-between">
        <form onSubmit={handleReadFile} className="flex flex-1 items-center max-w-2xl">
          <FileTextIcon className="w-5 h-5 text-slate-400 mr-2" />
          <input
            type="text"
            value={filePath}
            onChange={(e) => setFilePath(e.target.value)}
            placeholder="Enter absolute file path (e.g. /etc/nginx/nginx.conf, C:\config.json)"
            className="flex-1 bg-white border border-slate-300 rounded-md px-3 py-1.5 text-sm focus:outline-none focus:ring-2 focus:ring-blue-500 focus:border-blue-500"
          />
          <button
            type="submit"
            disabled={isLoading || !filePath.trim()}
            className="ml-3 px-4 py-1.5 bg-slate-800 text-white text-sm font-medium rounded-md hover:bg-slate-700 disabled:opacity-50 disabled:cursor-not-allowed transition-colors"
          >
            Read
          </button>
        </form>
        
        <button
          onClick={handleSaveFile}
          disabled={isLoading || !fileContent}
          className="ml-4 flex items-center px-4 py-1.5 bg-blue-600 text-white text-sm font-medium rounded-md hover:bg-blue-700 disabled:opacity-50 disabled:cursor-not-allowed transition-colors shadow-sm"
        >
          <SaveIcon className="w-4 h-4 mr-2" />
          Save Changes
        </button>
      </div>

      {/* Status Bar */}
      {(error || successMsg || isLoading) && (
        <div className={`px-4 py-2 text-sm flex items-center ${
          error ? 'bg-red-50 text-red-600 border-b border-red-100' : 
          successMsg ? 'bg-green-50 text-green-600 border-b border-green-100' : 
          'bg-blue-50 text-blue-600 border-b border-blue-100'
        }`}>
          {error && <AlertCircleIcon className="w-4 h-4 mr-2" />}
          {error || successMsg || 'Processing...'}
        </div>
      )}

      {/* Editor Area */}
      <div className="flex-1 border-b border-slate-200 relative">
        <Editor
          height="100%"
          language={getLanguage(filePath)}
          theme="light"
          value={fileContent}
          onChange={(value) => setFileContent(value || '')}
          options={{
            minimap: { enabled: false },
            fontSize: 14,
            wordWrap: 'on',
            scrollBeyondLastLine: false,
            smoothScrolling: true,
            padding: { top: 16, bottom: 16 }
          }}
        />
        {/* Placeholder when no content */}
        {!fileContent && !filePath && (
          <div className="absolute inset-0 flex items-center justify-center pointer-events-none bg-slate-50 bg-opacity-80 z-10">
            <div className="text-slate-400 flex flex-col items-center">
              <FileTextIcon className="w-12 h-12 mb-2 opacity-50" />
              <p>Enter a file path above to view or edit</p>
            </div>
          </div>
        )}
      </div>
    </div>
  );
}
