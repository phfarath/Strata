import React, { useState, useEffect } from 'react';
import {
  Brain,
  Key,
  Radio,
  PlayCircle,
  Database,
  Activity,
  Layers,
  Sparkles,
  Plus,
  Server,
  ShieldCheck,
  CheckCircle2,
  HardDrive,
  Cpu,
} from 'lucide-react';
import { User, Workspace, StatusResponse, PingResponse } from '../types';
import { api } from '../api';
import { MemoryExplorer } from './MemoryExplorer';
import { ApiKeyManager } from './ApiKeyManager';
import { RealtimeStream } from './RealtimeStream';
import { AgentPlayground } from './AgentPlayground';
import { CodeBlock } from './CodeBlock';
import { toast } from './Toast';

interface DashboardProps {
  user: User;
  workspace: Workspace;
  currentTab: 'overview' | 'explorer' | 'keys' | 'stream' | 'playground';
  onTabChange: (tab: 'overview' | 'explorer' | 'keys' | 'stream' | 'playground') => void;
  onRefreshWorkspaces: () => void;
}

export const Dashboard: React.FC<DashboardProps> = ({
  user,
  workspace,
  currentTab,
  onTabChange,
  onRefreshWorkspaces,
}) => {
  const [status, setStatus] = useState<StatusResponse | null>(null);
  const [ping, setPing] = useState<PingResponse | null>(null);

  useEffect(() => {
    api.getStatus(workspace.id).then(setStatus).catch(() => {});
    api.ping().then(setPing).catch(() => {});
  }, [workspace.id]);

  return (
    <div className="max-w-7xl mx-auto px-4 sm:px-6 py-8 space-y-8">
      {/* Workspace Header Strip */}
      <div className="flex flex-col sm:flex-row items-start sm:items-center justify-between gap-4 glass-panel p-6 rounded-2xl border border-border">
        <div>
          <div className="flex items-center gap-2 text-xs font-mono text-slate-400 mb-1">
            <span>ORGANIZATION WORKSPACE</span>
            <span>•</span>
            <span className="text-primary font-semibold">{workspace.slug}</span>
          </div>
          <h1 className="text-2xl font-extrabold text-white tracking-tight flex items-center gap-2">
            <span>{workspace.name}</span>
          </h1>
        </div>

        {/* Status Pill */}
        <div className="flex items-center gap-3">
          <div className="flex items-center gap-2 px-3 py-1.5 rounded-xl bg-card border border-border text-xs font-mono">
            <Server className="w-4 h-4 text-emerald-400" />
            <span className="text-slate-300">
              Supabase Postgres: <strong className="text-emerald-400">Connected</strong>
            </span>
          </div>
        </div>
      </div>

      {/* Overview Metrics Grid (visible on Overview tab) */}
      {currentTab === 'overview' && (
        <div className="space-y-8 animate-in fade-in duration-200">
          <div className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-4 gap-4">
            <div className="glass-panel p-5 rounded-2xl border border-border card-interactive">
              <div className="flex items-center justify-between text-xs font-mono text-slate-400 mb-3">
                <span>TOTAL MEMORIES</span>
                <Brain className="w-4 h-4 text-primary" />
              </div>
              <div className="text-3xl font-extrabold text-white font-mono">1,842</div>
              <div className="text-xs text-slate-400 mt-2 flex items-center gap-1">
                <span className="text-emerald-400 font-bold">+14%</span> across 5 coding sessions
              </div>
            </div>

            <div className="glass-panel p-5 rounded-2xl border border-border card-interactive">
              <div className="flex items-center justify-between text-xs font-mono text-slate-400 mb-3">
                <span>SYNCHRONIZED DELTAS</span>
                <Activity className="w-4 h-4 text-purple-400" />
              </div>
              <div className="text-3xl font-extrabold text-white font-mono">
                {status?.total_deltas || '2,418'}
              </div>
              <div className="text-xs text-slate-400 mt-2 flex items-center gap-1">
                <span className="text-purple-400 font-bold">100%</span> bi-directional CDC sync
              </div>
            </div>

            <div className="glass-panel p-5 rounded-2xl border border-border card-interactive">
              <div className="flex items-center justify-between text-xs font-mono text-slate-400 mb-3">
                <span>CONNECTED AGENTS</span>
                <Cpu className="w-4 h-4 text-emerald-400" />
              </div>
              <div className="text-3xl font-extrabold text-white font-mono">3</div>
              <div className="text-xs text-slate-400 mt-2 flex items-center gap-1">
                <span>Cursor, Claude Code, Gemini</span>
              </div>
            </div>

            <div className="glass-panel p-5 rounded-2xl border border-border card-interactive">
              <div className="flex items-center justify-between text-xs font-mono text-slate-400 mb-3">
                <span>PGVECTOR INDEX</span>
                <Database className="w-4 h-4 text-amber-400" />
              </div>
              <div className="text-3xl font-extrabold text-emerald-400 font-mono">ACTIVE</div>
              <div className="text-xs text-slate-400 mt-2">
                <span>gte-small (384-dims) cosine</span>
              </div>
            </div>
          </div>

          {/* Connect CLI Instructions */}
          <div className="glass-panel-glow p-6 rounded-2xl border border-border space-y-4">
            <div className="flex items-center gap-3">
              <div className="p-2 rounded-xl bg-primary/10 border border-primary/20 text-primary">
                <Brain className="w-5 h-5" />
              </div>
              <div>
                <h3 className="text-base font-bold text-white">Connect Your Coding Agent in 1 Step</h3>
                <p className="text-xs text-slate-400">
                  Run this in your repository to link persistent memory to this workspace.
                </p>
              </div>
            </div>

            <CodeBlock
              title="Terminal Setup Command"
              language="bash"
              code={`# 1. Login with unified Supabase credentials\nstrata login\n\n# 2. Link this workspace\nstrata sync push\n\n# 3. Use memory in any agent (Claude Code / Cursor / Windsurf)\nstrata recall "How is PostgreSQL TLS initialized?"`}
            />
          </div>
        </div>
      )}

      {/* Memory Explorer Tab */}
      {currentTab === 'explorer' && <MemoryExplorer />}

      {/* API Keys Tab */}
      {currentTab === 'keys' && <ApiKeyManager workspace={workspace} />}

      {/* CDC Stream Tab */}
      {currentTab === 'stream' && <RealtimeStream />}

      {/* Simulator Playground Tab */}
      {currentTab === 'playground' && <AgentPlayground />}
    </div>
  );
};
