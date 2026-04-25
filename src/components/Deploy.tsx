'use client';

import { useEffect, useRef, useState } from 'react';
import { useWebSocket } from './providers/WebSocketProvider';
import { ContainerSpec, ServiceSpec, SwarmRole } from '@/lib/types';

type Mode = 'container' | 'service' | 'stack';
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

  // Shared
  const [image, setImage] = useState('');
  const [name, setName] = useState('');
  const [ports, setPorts] = useState('');
  const [env, setEnv] = useState('');
  const [command, setCommand] = useState('');

  // Container-only
  const [volumes, setVolumes] = useState('');
  const [network, setNetwork] = useState('');
  const [restartPolicy, setRestartPolicy] = useState<
    'no' | 'always' | 'unless-stopped' | 'on-failure'
  >('unless-stopped');
  const [pull, setPull] = useState(false);

  // Service-only
  const [replicas, setReplicas] = useState('1');
  const [serviceMode, setServiceMode] = useState<'replicated' | 'global'>('replicated');
  const [mounts, setMounts] = useState('');
  const [constraints, setConstraints] = useState('');
  const [networks, setNetworks] = useState('');
  const [restartCondition, setRestartCondition] = useState<'any' | 'on-failure' | 'none'>('any');

  // Stack-only
  const [stackName, setStackName] = useState('');
  const [composeYaml, setComposeYaml] = useState(
    'version: "3.9"\n\nservices:\n  web:\n    image: nginx:1.27-alpine\n    ports:\n      - "8080:80"\n',
  );
  const [stackPrune, setStackPrune] = useState(false);

  const [submitting, setSubmitting] = useState(false);
  const [outcome, setOutcome] = useState<Outcome>(null);
  const submittingRef = useRef(false);

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
          setOutcome({ kind: 'success', id: msg.payload.container_id, log: msg.payload.log });
        } else {
          setOutcome({ kind: 'error', log: msg.payload.log, error: msg.payload.error });
        }
      } else if (msg.type === 'SwarmCreateServiceResponse') {
        if (!submittingRef.current) return;
        submittingRef.current = false;
        setSubmitting(false);
        if (msg.payload.success) {
          setOutcome({ kind: 'success', id: msg.payload.service_id, log: msg.payload.log });
        } else {
          setOutcome({ kind: 'error', log: msg.payload.log, error: msg.payload.error });
        }
      } else if (msg.type === 'SwarmStackDeployResponse') {
        if (!submittingRef.current) return;
        submittingRef.current = false;
        setSubmitting(false);
        if (msg.payload.success) {
          setOutcome({ kind: 'success', id: msg.payload.stack_name, log: msg.payload.log });
        } else {
          setOutcome({ kind: 'error', log: msg.payload.log, error: msg.payload.error });
        }
      }
    });

    sendToAgent(agentId, { type: 'DockerListRequest' });
    return unsub;
  }, [agentId, sendToAgent, onAgentMessage]);

  useEffect(() => {
    if ((mode === 'service' || mode === 'stack') && swarmRole !== null && swarmRole !== 'manager') {
      setMode('container');
    }
  }, [swarmRole, mode]);

  const canSubmitService = swarmRole === 'manager';

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault();
    if (submitting) return;
    setOutcome(null);
    submittingRef.current = true;
    setSubmitting(true);

    if (mode === 'stack') {
      sendToAgent(agentId, {
        type: 'SwarmStackDeployRequest',
        payload: {
          stack_name: stackName.trim(),
          compose_yaml: composeYaml,
          prune: stackPrune,
        },
      });
      return;
    }

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
        replicas:
          serviceMode === 'replicated' ? Math.max(0, Number.parseInt(replicas, 10) || 1) : null,
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

  return (
    <div className="pane">
      <div className="panel">
        <div className="panel-head">
          <div className="panel-title">
            <span className="ico">▷</span> DEPLOY
          </div>
          <div className="panel-actions">
            <div className="seg">
              <button
                className={mode === 'container' ? 'on' : ''}
                onClick={() => setMode('container')}
              >
                container
              </button>
              <button
                className={mode === 'service' ? 'on' : ''}
                disabled={!canSubmitService}
                onClick={() => setMode('service')}
              >
                swarm service
              </button>
              <button
                className={mode === 'stack' ? 'on' : ''}
                disabled={!canSubmitService}
                onClick={() => setMode('stack')}
              >
                stack
              </button>
            </div>
          </div>
        </div>
        <form
          onSubmit={handleSubmit}
          className="panel-body"
          style={{ display: 'flex', flexDirection: 'column', gap: 12 }}
        >
          {mode === 'stack' ? (
            <>
              <div className="grid-2">
                <div className="field">
                  <label>stack name</label>
                  <input
                    className="input"
                    type="text"
                    required
                    value={stackName}
                    onChange={(e) => setStackName(e.target.value)}
                    placeholder="my-app"
                    spellCheck={false}
                  />
                </div>
                <div className="field">
                  <label>options</label>
                  <label
                    className="row"
                    style={{ gap: 6, fontSize: 12, height: 28, alignItems: 'center' }}
                  >
                    <input
                      type="checkbox"
                      checked={stackPrune}
                      onChange={(e) => setStackPrune(e.target.checked)}
                    />
                    prune services not in compose
                  </label>
                </div>
              </div>
              <div className="field">
                <label>compose yaml</label>
                <textarea
                  className="textarea"
                  rows={16}
                  value={composeYaml}
                  onChange={(e) => setComposeYaml(e.target.value)}
                  spellCheck={false}
                  style={{ minHeight: 320, fontFamily: 'var(--mono)' }}
                />
              </div>
              <div className="row between">
                <div className="kbd-hint">
                  <kbd>⌘</kbd>+<kbd>⏎</kbd> deploy
                </div>
                <button
                  type="submit"
                  className="btn primary"
                  disabled={
                    submitting ||
                    !canSubmitService ||
                    !stackName.trim() ||
                    !composeYaml.trim()
                  }
                >
                  {submitting ? '…' : '▷ deploy stack'}
                </button>
              </div>
              {!canSubmitService && (
                <div className="warn-c" style={{ fontSize: 11, fontFamily: 'var(--mono)' }}>
                  ⚠ this host is {swarmRole ?? 'unknown'}, not a swarm manager.
                </div>
              )}
            </>
          ) : (
            <>
              <div className="grid-2">
                <div className="field">
                  <label>image</label>
                  <input
                    className="input"
                    type="text"
                    required
                    value={image}
                    onChange={(e) => setImage(e.target.value)}
                    placeholder="nginx:1.27-alpine"
                    spellCheck={false}
                  />
                </div>
                <div className="field">
                  <label>{mode === 'service' ? 'service name' : 'container name (optional)'}</label>
                  <input
                    className="input"
                    type="text"
                    value={name}
                    onChange={(e) => setName(e.target.value)}
                    required={mode === 'service'}
                    placeholder={mode === 'service' ? 'my-app' : '(auto-generated)'}
                    spellCheck={false}
                  />
                </div>
              </div>

              <div className="field">
                <label>ports — host:container[/proto], one per line</label>
                <textarea
                  className="textarea"
                  rows={2}
                  value={ports}
                  onChange={(e) => setPorts(e.target.value)}
                  placeholder={'80:80\n8080:8080/tcp'}
                />
              </div>

              <div className="field">
                <label>environment — KEY=value, one per line</label>
                <textarea
                  className="textarea"
                  rows={2}
                  value={env}
                  onChange={(e) => setEnv(e.target.value)}
                  placeholder={'TZ=UTC\nLOG_LEVEL=info'}
                />
              </div>

              {mode === 'container' ? (
                <>
                  <div className="grid-2">
                    <div className="field">
                      <label>volumes — host:container[:ro]</label>
                      <textarea
                        className="textarea"
                        rows={2}
                        value={volumes}
                        onChange={(e) => setVolumes(e.target.value)}
                        placeholder="/var/log:/logs:ro"
                      />
                    </div>
                    <div className="field">
                      <label>network</label>
                      <input
                        className="input"
                        type="text"
                        value={network}
                        onChange={(e) => setNetwork(e.target.value)}
                        placeholder="bridge"
                      />
                    </div>
                  </div>
                  <div className="grid-2">
                    <div className="field">
                      <label>restart policy</label>
                      <select
                        className="select"
                        value={restartPolicy}
                        onChange={(e) =>
                          setRestartPolicy(e.target.value as typeof restartPolicy)
                        }
                      >
                        <option value="no">no</option>
                        <option value="always">always</option>
                        <option value="unless-stopped">unless-stopped</option>
                        <option value="on-failure">on-failure</option>
                      </select>
                    </div>
                    <div className="field">
                      <label>options</label>
                      <label
                        className="row"
                        style={{ gap: 6, fontSize: 12, height: 28, alignItems: 'center' }}
                      >
                        <input
                          type="checkbox"
                          checked={pull}
                          onChange={(e) => setPull(e.target.checked)}
                        />
                        --pull always
                      </label>
                    </div>
                  </div>
                </>
              ) : (
                <>
                  <div className="grid-3">
                    <div className="field">
                      <label>mode</label>
                      <select
                        className="select"
                        value={serviceMode}
                        onChange={(e) =>
                          setServiceMode(e.target.value as typeof serviceMode)
                        }
                      >
                        <option value="replicated">replicated</option>
                        <option value="global">global</option>
                      </select>
                    </div>
                    <div className="field">
                      <label>replicas</label>
                      <input
                        className="input"
                        type="number"
                        min={0}
                        value={replicas}
                        onChange={(e) => setReplicas(e.target.value)}
                        disabled={serviceMode === 'global'}
                      />
                    </div>
                    <div className="field">
                      <label>restart condition</label>
                      <select
                        className="select"
                        value={restartCondition}
                        onChange={(e) =>
                          setRestartCondition(e.target.value as typeof restartCondition)
                        }
                      >
                        <option value="any">any</option>
                        <option value="on-failure">on-failure</option>
                        <option value="none">none</option>
                      </select>
                    </div>
                  </div>
                  <div className="field">
                    <label>mounts — type=…,source=…,target=…</label>
                    <textarea
                      className="textarea"
                      rows={2}
                      value={mounts}
                      onChange={(e) => setMounts(e.target.value)}
                      placeholder="type=volume,source=app-data,target=/data"
                    />
                  </div>
                  <div className="field">
                    <label>networks — overlay names, one per line</label>
                    <textarea
                      className="textarea"
                      rows={2}
                      value={networks}
                      onChange={(e) => setNetworks(e.target.value)}
                      placeholder="ingress"
                    />
                  </div>
                  <div className="field">
                    <label>placement constraints</label>
                    <textarea
                      className="textarea"
                      rows={2}
                      value={constraints}
                      onChange={(e) => setConstraints(e.target.value)}
                      placeholder="node.role==worker"
                    />
                  </div>
                </>
              )}

              <div className="field">
                <label>command (optional)</label>
                <input
                  className="input"
                  type="text"
                  value={command}
                  onChange={(e) => setCommand(e.target.value)}
                  placeholder="--help"
                  spellCheck={false}
                />
              </div>

              <div className="row between">
                {mode === 'service' && !canSubmitService ? (
                  <div className="warn-c" style={{ fontSize: 11, fontFamily: 'var(--mono)' }}>
                    ⚠ this host is {swarmRole ?? 'unknown'}.
                  </div>
                ) : (
                  <div />
                )}
                <button
                  type="submit"
                  className="btn primary"
                  disabled={
                    submitting || !image.trim() || (mode === 'service' && !canSubmitService)
                  }
                >
                  {submitting
                    ? '…'
                    : mode === 'container'
                      ? '▷ create container'
                      : '▷ create service'}
                </button>
              </div>
            </>
          )}
        </form>
      </div>

      {outcome && <OutcomePanel outcome={outcome} mode={mode} />}
    </div>
  );
}

function OutcomePanel({ outcome, mode }: { outcome: NonNullable<Outcome>; mode: Mode }) {
  const verb =
    mode === 'container' ? 'Container' : mode === 'service' ? 'Service' : 'Stack';
  return (
    <div
      className="panel"
      style={{
        borderColor: outcome.kind === 'success' ? 'var(--accent-bd)' : 'var(--err-bd)',
      }}
    >
      <div
        className="panel-head"
        style={{ background: outcome.kind === 'success' ? 'var(--accent-bg)' : 'var(--err-bg)' }}
      >
        <div
          className="panel-title"
          style={{ color: outcome.kind === 'success' ? 'var(--accent)' : 'var(--err)' }}
        >
          {outcome.kind === 'success' ? '✓' : '×'} {verb}{' '}
          {mode === 'stack' ? 'deployed' : 'created'}
          {outcome.kind === 'success' && outcome.id && (
            <span className="meta">
              {outcome.id.length > 32 ? `${outcome.id.slice(0, 12)}…` : outcome.id}
            </span>
          )}
          {outcome.kind === 'error' && outcome.error && (
            <span className="meta">{outcome.error}</span>
          )}
        </div>
      </div>
      {outcome.log && (
        <pre className="code" style={{ margin: 0, borderRadius: 0, border: 0, maxHeight: 240 }}>
          {outcome.log}
        </pre>
      )}
    </div>
  );
}
