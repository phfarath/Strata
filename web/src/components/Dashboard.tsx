import React, { useState, useEffect } from 'react';
import { User, Workspace, StatusResponse } from '../types';
import { api } from '../api';
import { MemoryExplorer } from './MemoryExplorer';
import { ApiKeyManager } from './ApiKeyManager';
import { RealtimeStream } from './RealtimeStream';
import { AgentPlayground } from './AgentPlayground';
import { CodeBlock } from './CodeBlock';

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
}) => {
  const [status, setStatus] = useState<StatusResponse | null>(null);

  useEffect(() => {
    api.getStatus(workspace.id).then(setStatus).catch(() => {});
  }, [workspace.id]);

  return (
    <div className="max-w-6xl mx-auto p-6 space-y-6">
      {/* Workspace Header */}
      <div className="flex items-center justify-between p-4 rounded-xl border border-[#23262f] bg-[#0f1115]">
        <div>
          <div className="flex items-center gap-2 text-xs font-mono text-zinc-500 mb-0.5">
            <span>Workspace</span>
            <span>/</span>
            <span className="text-amber-400 font-semibold">{workspace.slug}</span>
          </div>
          <h1 className="text-xl font-bold text-white tracking-tight font-sans">
            {workspace.name}
          </h1>
        </div>
      </div>

      {/* Tab 1: Overview */}
      {currentTab === 'overview' && (
        <div className="space-y-6 animate-in fade-in duration-150">
          {/* Telemetry Metrics */}
          <div className="grid grid-cols-1 md:grid-cols-3 gap-4 font-mono text-xs">
            <div className="p-4 rounded-xl border border-[#23262f] bg-[#15171d] space-y-1 sweep-hover">
              <div className="text-zinc-500">Total Memories</div>
              <div className="text-2xl font-bold text-amber-400">1,842</div>
              <div className="text-zinc-500 text-[11px] pt-1">
                Semantic facts, failure patterns & skills
              </div>
            </div>

            <div className="p-4 rounded-xl border border-[#23262f] bg-[#15171d] space-y-1 sweep-hover">
              <div className="text-zinc-500">CDC Sequence</div>
              <div className="text-2xl font-bold text-zinc-200">
                #{status?.total_deltas || 2418}
              </div>
              <div className="text-zinc-500 text-[11px] pt-1">
                Monotonic version counter
              </div>
            </div>

            <div className="p-4 rounded-xl border border-[#23262f] bg-[#15171d] space-y-1 sweep-hover">
              <div className="text-zinc-500">Connected Agents</div>
              <div className="text-2xl font-bold text-zinc-200">3</div>
              <div className="text-zinc-500 text-[11px] pt-1">
                Cursor, Claude Code, Gemini CLI
              </div>
            </div>
          </div>

          {/* Quick Setup */}
          <div className="p-5 rounded-xl border border-[#23262f] bg-[#0f1115] space-y-4">
            <div>
              <h3 className="text-sm font-semibold text-white">Link Local Repository</h3>
              <p className="text-xs text-zinc-400 mt-0.5">
                Run these commands in your project root to start syncing agent memories.
              </p>
            </div>

            <CodeBlock
              title="CLI Setup"
              language="bash"
              code={`# 1. Login once\nstrata login\n\n# 2. Push initial codebase memories\nstrata sync push\n\n# 3. Query memories\nstrata recall "PostgreSQL connection requirements"`}
            />
          </div>
        </div>
      )}

      {/* Tab 2: Memory Explorer */}
      {currentTab === 'explorer' && <MemoryExplorer />}

      {/* Tab 3: API Keys */}
      {currentTab === 'keys' && <ApiKeyManager workspace={workspace} />}

      {/* Tab 4: CDC Delta Stream */}
      {currentTab === 'stream' && <RealtimeStream />}

      {/* Tab 5: Agent Simulator */}
      {currentTab === 'playground' && <AgentPlayground />}
    </div>
  );
};
