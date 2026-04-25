'use client';

import { useEffect, useRef, useState } from 'react';
import { useWebSocket } from './providers/WebSocketProvider';
import { ContainerSpec, ServiceSpec, SwarmRole } from '@/lib/types';
import {
  AlertCircleIcon,
  BoxIcon,
  CheckCircleIcon,
  Loader2Icon,
  NetworkIcon,
  RocketIcon,
} from 'lucide-react';

type Mode = 'container' | 'service';
type Outcome =
  | { kind: 'success'; id: string | null; log: string }
  | { kind: 'error'; log: string; error: string | null }
  | null;

const linesFrom = (s: string) =>
  s
    .split('\n')
    .map((l) => l.trim())
    .filter(Boolean);

export default function Deploy({ agentId }: { agentId: string }) {
  const { sendToAgent, onAgentMessage } = useWebSocket();
  const [mode, setMode] = useState<Mode>('container');
  const [swarmRole, setSwarmRole] = useState<SwarmRole | null>(null);

  // Shared fields
  const [image, setImage] = useState('');
  const [name, setName] = useState('');
  const [ports, setPorts] = useState('');
  const [env, setEnv] = useState('');
  const [command, setCommand] = useState('');

  // Container-only
  const [volumes, setVolumes] = useState('');
  const [network, setNetwork] = useState('');
  const [restartPolicy, setRestartPolicy] = useState<'no' | 'always' | 'unless-stopped' | 'on-failure'>('unless-stopped');
  const [pull, setPull] = useState(false);

  // Service-only
  const [replicas, setReplicas] = useState('1');
  const [serviceMode, setServiceMode] = useState<'replicated' | 'global'>('replicated');
  const [mounts, setMounts] = useState('');
  const [constraints, setConstraints] = useState('');
  const [networks, setNetworks] = useState('');
  const [restartCondition, setRestartCondition] = useState<'any' | 'on-failure' | 'none'>('any');

  const [submitting, setSubmitting] = useState(false);
  const [outcome, setOutcome] = useState<Outcome>(null);
  const submittingRef = useRef(false);

  // Discover swarm role so we can light up the service tab.
  useEffect(() => {
    setOutcome(null);
    setSubmitting(false);
    submittingRef.current = false;
    setSwarmRole(null);

    const unsub = onAgentMessage(agentId, (msg) => {
      if (msg.type === 'DockerListResponse') {
        setSwarmRole(msg.payload.swarm_role);
      } else if (msg.type === 'DockerCreateContainerResponse') {
        if (!submittingRef.current) return;
        submittingRef.current = false;
        setSubmitting(false);
        if (msg.payload.success) {
          setOutcome({
            kind: 'success',
            id: msg.payload.container_id,
            log: msg.payload.log,
          });
        } else {
          setOutcome({
            kind: 'error',
            log: msg.payload.log,
            error: msg.payload.error,
          });
        }
      } else if (msg.type === 'SwarmCreateServiceResponse') {
        if (!submittingRef.current) return;
        submittingRef.current = false;
        setSubmitting(false);
        if (msg.payload.success) {
          setOutcome({
            kind: 'success',
            id: msg.payload.service_id,
            log: msg.payload.log,
          });
        } else {
          setOutcome({
            kind: 'error',
            log: msg.payload.log,
            error: msg.payload.error,
          });
        }
      }
    });
    sendToAgent(agentId, { type: 'DockerListRequest' });
    return unsub;
  }, [agentId, sendToAgent, onAgentMessage]);

  // If user switches agents, fall back to container mode if the new host
  // isn't a swarm manager.
  useEffect(() => {
    if (mode === 'service' && swarmRole !== null && swarmRole !== 'manager') {
      setMode('container');
    }
  }, [swarmRole, mode]);

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault();
    if (submitting) return;
    setOutcome(null);
    submittingRef.current = true;
    setSubmitting(true);

    if (mode === 'container') {
      const spec: ContainerSpec = {
        image: image.trim(),
        name: name.trim() || null,
        ports: linesFrom(ports),
        env: linesFrom(env),
        volumes: linesFrom(volumes),
        restart_policy: restartPolicy,
        command: command.trim() || null,
        network: network.trim() || null,
        detached: true,
        pull,
      };
      sendToAgent(agentId, { type: 'DockerCreateContainerRequest', payload: { spec } });
    } else {
      const spec: ServiceSpec = {
        image: image.trim(),
        name: name.trim(),
        replicas: serviceMode === 'replicated' ? Math.max(0, Number.parseInt(replicas, 10) || 1) : null,
        mode: serviceMode,
        ports: linesFrom(ports),
        env: linesFrom(env),
        mounts: linesFrom(mounts),
        constraints: linesFrom(constraints),
        command: command.trim() || null,
        networks: linesFrom(networks),
        restart_condition: restartCondition,
      };
      sendToAgent(agentId, { type: 'SwarmCreateServiceRequest', payload: { spec } });
    }
  };

  const canSubmitService = swarmRole === 'manager';

  return (
    <div className="max-w-3xl mx-auto w-full">
      <div className="flex items-center gap-2 mb-1">
        <RocketIcon className="w-5 h-5 text-slate-400" />
        <h2 className="text-base font-semibold">Deploy a service</h2>
      </div>
      <p className="text-sm text-slate-500 mb-4">
        Create a standalone Docker container, or — on a swarm manager — a
        replicated swarm service. The agent shells out to the docker CLI and
        echoes the full output back.
      </p>

      <div className="inline-flex bg-slate-900 border border-slate-800 rounded-md p-0.5 text-xs mb-4">
        <ModeButton
          active={mode === 'container'}
          onClick={() => setMode('container')}
          icon={<BoxIcon className="w-3.5 h-3.5" />}
          label="Container"
        />
        <ModeButton
          active={mode === 'service'}
          disabled={!canSubmitService}
          onClick={() => setMode('service')}
          icon={<NetworkIcon className="w-3.5 h-3.5" />}
          label={canSubmitService ? 'Swarm service' : 'Swarm service (manager only)'}
        />
      </div>

      <form onSubmit={handleSubmit} className="space-y-4">
        <div className="grid grid-cols-1 md:grid-cols-2 gap-3">
          <Field label="Image" required>
            <input
              type="text"
              required
              value={image}
              onChange={(e) => setImage(e.target.value)}
              placeholder="nginx:1.27-alpine"
              spellCheck={false}
              className={inputClass}
            />
          </Field>
          <Field label={mode === 'service' ? 'Service name' : 'Container name (optional)'} required={mode === 'service'}>
            <input
              type="text"
              value={name}
              onChange={(e) => setName(e.target.value)}
              required={mode === 'service'}
              placeholder={mode === 'service' ? 'my-app' : '(auto-generated)'}
              spellCheck={false}
              className={inputClass}
            />
          </Field>
        </div>

        <Field label="Ports" hint="One per line. host:container[/proto]">
          <textarea
            rows={2}
            value={ports}
            onChange={(e) => setPorts(e.target.value)}
            placeholder={'80:80\n8080:8080/tcp'}
            className={textareaClass}
          />
        </Field>

        <Field label="Environment" hint="One KEY=value per line">
          <textarea
            rows={2}
            value={env}
            onChange={(e) => setEnv(e.target.value)}
            placeholder={'TZ=UTC\nLOG_LEVEL=info'}
            className={textareaClass}
          />
        </Field>

        {mode === 'container' ? (
          <>
            <div className="grid grid-cols-1 md:grid-cols-2 gap-3">
              <Field label="Volumes" hint="One per line. host:container[:ro]">
                <textarea
                  rows={2}
                  value={volumes}
                  onChange={(e) => setVolumes(e.target.value)}
                  placeholder="/var/log:/logs:ro"
                  className={textareaClass}
                />
              </Field>
              <Field label="Network">
                <input
                  type="text"
                  value={network}
                  onChange={(e) => setNetwork(e.target.value)}
                  placeholder="bridge"
                  className={inputClass}
                />
              </Field>
            </div>
            <div className="grid grid-cols-1 md:grid-cols-2 gap-3">
              <Field label="Restart policy">
                <select
                  value={restartPolicy}
                  onChange={(e) => setRestartPolicy(e.target.value as typeof restartPolicy)}
                  className={inputClass}
                >
                  <option value="no">no</option>
                  <option value="always">always</option>
                  <option value="unless-stopped">unless-stopped</option>
                  <option value="on-failure">on-failure</option>
                </select>
              </Field>
              <label className="flex items-center gap-2 self-end pb-1.5 text-sm text-slate-300">
                <input
                  type="checkbox"
                  checked={pull}
                  onChange={(e) => setPull(e.target.checked)}
                  className="accent-blue-500"
                />
                <code className="text-xs">--pull always</code>
              </label>
            </div>
          </>
        ) : (
          <>
            <div className="grid grid-cols-1 md:grid-cols-3 gap-3">
              <Field label="Mode">
                <select
                  value={serviceMode}
                  onChange={(e) => setServiceMode(e.target.value as typeof serviceMode)}
                  className={inputClass}
                >
                  <option value="replicated">replicated</option>
                  <option value="global">global</option>
                </select>
              </Field>
              <Field label="Replicas" hint={serviceMode === 'global' ? '(ignored)' : ''}>
                <input
                  type="number"
                  min={0}
                  value={replicas}
                  onChange={(e) => setReplicas(e.target.value)}
                  disabled={serviceMode === 'global'}
                  className={inputClass}
                />
              </Field>
              <Field label="Restart condition">
                <select
                  value={restartCondition}
                  onChange={(e) => setRestartCondition(e.target.value as typeof restartCondition)}
                  className={inputClass}
                >
                  <option value="any">any</option>
                  <option value="on-failure">on-failure</option>
                  <option value="none">none</option>
                </select>
              </Field>
            </div>
            <Field label="Mounts" hint="One per line. type=bind,source=...,target=... or src=...,dst=...">
              <textarea
                rows={2}
                value={mounts}
                onChange={(e) => setMounts(e.target.value)}
                placeholder="type=volume,source=app-data,target=/data"
                className={textareaClass}
              />
            </Field>
            <Field label="Networks" hint="One overlay network name per line">
              <textarea
                rows={2}
                value={networks}
                onChange={(e) => setNetworks(e.target.value)}
                placeholder="ingress"
                className={textareaClass}
              />
            </Field>
            <Field label="Placement constraints" hint='One per line, e.g. node.role==manager'>
              <textarea
                rows={2}
                value={constraints}
                onChange={(e) => setConstraints(e.target.value)}
                placeholder="node.role==worker"
                className={textareaClass}
              />
            </Field>
          </>
        )}

        <Field label="Command (optional)" hint="Overrides the image's CMD. Quotes preserved.">
          <input
            type="text"
            value={command}
            onChange={(e) => setCommand(e.target.value)}
            placeholder="--help"
            spellCheck={false}
            className={inputClass}
          />
        </Field>

        <div className="flex items-center gap-3">
          <button
            type="submit"
            disabled={submitting || !image.trim() || (mode === 'service' && !canSubmitService)}
            className="inline-flex items-center gap-2 px-4 py-2 rounded-md bg-blue-600 hover:bg-blue-500 disabled:bg-slate-700 disabled:cursor-not-allowed text-white text-sm font-medium transition-colors"
          >
            {submitting ? <Loader2Icon className="w-4 h-4 animate-spin" /> : <RocketIcon className="w-4 h-4" />}
            {submitting
              ? mode === 'container'
                ? 'Creating container…'
                : 'Creating service…'
              : mode === 'container'
                ? 'Create container'
                : 'Create service'}
          </button>
          {mode === 'service' && !canSubmitService && (
            <span className="text-xs text-amber-300">
              Switch to a swarm manager (this host is{' '}
              {swarmRole ?? 'unknown'}) to create a service.
            </span>
          )}
        </div>
      </form>

      {outcome && <OutcomePanel outcome={outcome} mode={mode} />}
    </div>
  );
}

const inputClass =
  'w-full bg-slate-950 border border-slate-700 rounded-md px-3 py-1.5 text-sm text-slate-100 placeholder:text-slate-600 focus:outline-none focus:ring-1 focus:ring-blue-500 focus:border-blue-500';

const textareaClass =
  'w-full bg-slate-950 border border-slate-700 rounded-md px-3 py-1.5 text-sm font-mono text-slate-100 placeholder:text-slate-600 focus:outline-none focus:ring-1 focus:ring-blue-500 focus:border-blue-500';

function Field({
  label,
  hint,
  required,
  children,
}: {
  label: string;
  hint?: string;
  required?: boolean;
  children: React.ReactNode;
}) {
  return (
    <label className="flex flex-col gap-1">
      <span className="text-[11px] uppercase tracking-wide text-slate-500">
        {label}
        {required && <span className="text-blue-400 ml-1">*</span>}
        {hint && <span className="ml-1 normal-case tracking-normal text-slate-600">— {hint}</span>}
      </span>
      {children}
    </label>
  );
}

function ModeButton({
  active,
  onClick,
  icon,
  label,
  disabled,
}: {
  active: boolean;
  onClick: () => void;
  icon: React.ReactNode;
  label: string;
  disabled?: boolean;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      disabled={disabled}
      className={`px-3 py-1.5 rounded-md flex items-center gap-1.5 transition-colors ${
        active
          ? 'bg-slate-700 text-slate-100'
          : 'text-slate-400 hover:text-slate-100 disabled:opacity-50 disabled:cursor-not-allowed'
      }`}
    >
      {icon}
      {label}
    </button>
  );
}

function OutcomePanel({ outcome, mode }: { outcome: NonNullable<Outcome>; mode: Mode }) {
  return (
    <div
      className={`mt-4 rounded-md border ${
        outcome.kind === 'success'
          ? 'border-emerald-500/30 bg-emerald-500/5'
          : 'border-red-500/30 bg-red-500/5'
      }`}
    >
      <div
        className={`flex items-start gap-2 px-3 py-2 text-sm ${
          outcome.kind === 'success' ? 'text-emerald-200' : 'text-red-200'
        }`}
      >
        {outcome.kind === 'success' ? (
          <CheckCircleIcon className="w-4 h-4 mt-0.5 shrink-0" />
        ) : (
          <AlertCircleIcon className="w-4 h-4 mt-0.5 shrink-0" />
        )}
        <div className="min-w-0">
          {outcome.kind === 'success' ? (
            <>
              <div>
                {mode === 'container' ? 'Container' : 'Service'} created.
                {outcome.id && (
                  <>
                    {' '}
                    ID <code className="text-slate-200">{outcome.id.slice(0, 12)}</code>
                  </>
                )}
              </div>
            </>
          ) : (
            <div>
              {outcome.error ?? 'Docker rejected the request. See log below.'}
            </div>
          )}
        </div>
      </div>
      {outcome.log && (
        <pre className="text-[11px] bg-slate-950 text-slate-300 px-3 py-2 overflow-x-auto whitespace-pre-wrap max-h-64 border-t border-slate-800">
          {outcome.log}
        </pre>
      )}
    </div>
  );
}
