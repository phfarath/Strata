import React, { useState } from 'react';
import { Brain, Search, Filter, ShieldAlert, Sparkles, CheckCircle2, Clock, Layers, ArrowUpRight, Eye } from 'lucide-react';
import { MemoryRecord, FactStatus, MemoryType } from '../types';

// Mock sample memories populated when workspace is opened
const SAMPLE_MEMORIES: MemoryRecord[] = [
  {
    id: 'mem-1',
    workspace_id: 'ws-core',
    type: 'semantic_fact',
    title: 'Strict Security Headers Architecture',
    content: 'All HTTP responses from Axum server must include HSTS (2y preload), X-Content-Type-Options: nosniff, and X-Frame-Options: DENY. CSP must allow WebSocket connects to wss: for realtime sync.',
    scope: 'crates/strata-server',
    status: 'verified',
    decay_score: 0.98,
    access_count: 42,
    created_at: '2026-08-19T10:00:00Z',
    updated_at: '2026-08-19T12:00:00Z',
    evidence_count: 3,
  },
  {
    id: 'mem-2',
    workspace_id: 'ws-core',
    type: 'failure_pattern',
    title: 'Anti-Pattern: OpenSSL Dynamic Linking on Distroless Containers',
    content: 'Do not use native OpenSSL bindings in server dependencies. Always prefer pure Rust TLS with rustls + ring and webpki-roots to prevent container runtime segfaults on Alpine/Distroless.',
    scope: 'global',
    status: 'verified',
    decay_score: 0.95,
    access_count: 88,
    created_at: '2026-08-18T14:30:00Z',
    updated_at: '2026-08-19T11:00:00Z',
    evidence_count: 5,
  },
  {
    id: 'mem-3',
    workspace_id: 'ws-core',
    type: 'semantic_fact',
    title: 'PostgreSQL Supabase Transaction Pooler Port 6543 TLS Handshake',
    content: 'Supabase transaction pooler on port 6543 uses wildcard SANs (*.supabase.co). Client must utilize AcceptAnyServerCertVerifier unless sslmode=verify-full is explicitly requested.',
    scope: 'crates/strata-server/storage',
    status: 'verified',
    decay_score: 0.99,
    access_count: 15,
    created_at: '2026-08-19T13:20:00Z',
    updated_at: '2026-08-19T13:25:00Z',
    evidence_count: 4,
  },
  {
    id: 'mem-4',
    workspace_id: 'ws-core',
    type: 'procedural_skill',
    title: 'Zero-Config CLI Login Authentication Loopback',
    content: 'Command `strata login` starts a local loopback server on 127.0.0.1 with random anti-CSRF state token and opens https://strata.pedrofarath.me/auth/cli for browser authorization.',
    scope: 'crates/strata-cli',
    status: 'verified',
    decay_score: 0.91,
    access_count: 29,
    created_at: '2026-08-19T13:00:00Z',
    updated_at: '2026-08-19T13:05:00Z',
    evidence_count: 2,
  },
  {
    id: 'mem-5',
    workspace_id: 'ws-core',
    type: 'episodic_session',
    title: 'E2E Testing for Dual SQLite / PostgreSQL Database Engine',
    content: 'Validated full CDC push/pull cycles with 56 unit and E2E tests covering monotonic sequence numbers, optimistic concurrency, and WebSocket broadcasts.',
    scope: 'crates/strata-server/tests',
    status: 'verified',
    decay_score: 0.86,
    access_count: 12,
    created_at: '2026-08-19T12:30:00Z',
    updated_at: '2026-08-19T12:30:00Z',
    evidence_count: 1,
  },
];

export const MemoryExplorer: React.FC = () => {
  const [search, setSearch] = useState('');
  const [typeFilter, setTypeFilter] = useState<string>('all');
  const [selectedMemory, setSelectedMemory] = useState<MemoryRecord | null>(SAMPLE_MEMORIES[0]);

  const filteredMemories = SAMPLE_MEMORIES.filter((m) => {
    const matchesSearch =
      m.title.toLowerCase().includes(search.toLowerCase()) ||
      m.content.toLowerCase().includes(search.toLowerCase()) ||
      m.scope.toLowerCase().includes(search.toLowerCase());

    const matchesType = typeFilter === 'all' || m.type === typeFilter;
    return matchesSearch && matchesType;
  });

  return (
    <div className="grid grid-cols-1 lg:grid-cols-12 gap-6">
      {/* Left Column: Search & List */}
      <div className="lg:col-span-7 space-y-4">
        {/* Controls Bar */}
        <div className="flex flex-col sm:flex-row gap-3">
          <div className="relative flex-1">
            <Search className="absolute left-3.5 top-3 w-4 h-4 text-slate-500" />
            <input
              type="text"
              placeholder="Search memories, anti-patterns, scopes..."
              value={search}
              onChange={(e) => setSearch(e.target.value)}
              className="w-full pl-10 pr-4 py-2.5 rounded-xl bg-card border border-border text-slate-100 placeholder:text-slate-500 focus:outline-none focus:border-primary text-xs"
            />
          </div>

          <div className="flex items-center gap-1 bg-card border border-border p-1 rounded-xl">
            <button
              onClick={() => setTypeFilter('all')}
              className={`px-2.5 py-1.5 rounded-lg text-xs font-semibold transition-all ${
                typeFilter === 'all' ? 'bg-primary text-black' : 'text-slate-400 hover:text-slate-200'
              }`}
            >
              All
            </button>
            <button
              onClick={() => setTypeFilter('semantic_fact')}
              className={`px-2.5 py-1.5 rounded-lg text-xs font-semibold transition-all ${
                typeFilter === 'semantic_fact' ? 'bg-primary text-black' : 'text-slate-400 hover:text-slate-200'
              }`}
            >
              Facts
            </button>
            <button
              onClick={() => setTypeFilter('failure_pattern')}
              className={`px-2.5 py-1.5 rounded-lg text-xs font-semibold transition-all ${
                typeFilter === 'failure_pattern' ? 'bg-rose-500 text-white' : 'text-slate-400 hover:text-slate-200'
              }`}
            >
              Anti-Patterns
            </button>
            <button
              onClick={() => setTypeFilter('procedural_skill')}
              className={`px-2.5 py-1.5 rounded-lg text-xs font-semibold transition-all ${
                typeFilter === 'procedural_skill' ? 'bg-purple-500 text-white' : 'text-slate-400 hover:text-slate-200'
              }`}
            >
              Skills
            </button>
          </div>
        </div>

        {/* Memory List */}
        <div className="space-y-3">
          {filteredMemories.map((mem) => {
            const isSelected = selectedMemory?.id === mem.id;
            return (
              <div
                key={mem.id}
                onClick={() => setSelectedMemory(mem)}
                className={`p-4 rounded-xl border glass-panel cursor-pointer card-interactive ${
                  isSelected
                    ? 'border-primary/60 bg-primary/5 shadow-glow'
                    : 'border-border hover:border-border-light'
                }`}
              >
                <div className="flex items-start justify-between gap-3 mb-1.5">
                  <div className="flex items-center gap-2">
                    <span
                      className={`px-2 py-0.5 rounded-md text-[10px] font-mono font-bold uppercase tracking-wider ${
                        mem.type === 'failure_pattern'
                          ? 'bg-rose-500/20 text-rose-300 border border-rose-500/30'
                          : mem.type === 'procedural_skill'
                          ? 'bg-purple-500/20 text-purple-300 border border-purple-500/30'
                          : 'bg-primary/20 text-primary border border-primary/30'
                      }`}
                    >
                      {mem.type.replace('_', ' ')}
                    </span>
                    <span className="text-[11px] font-mono text-slate-500 truncate max-w-[180px]">
                      {mem.scope}
                    </span>
                  </div>

                  <div className="flex items-center gap-2 text-xs font-mono">
                    <span className="text-slate-400">Retention:</span>
                    <span className="text-emerald-400 font-bold">{(mem.decay_score * 100).toFixed(0)}%</span>
                  </div>
                </div>

                <h4 className="font-bold text-sm text-slate-100 mb-1">{mem.title}</h4>
                <p className="text-xs text-slate-400 line-clamp-2 leading-relaxed">{mem.content}</p>

                <div className="mt-3 pt-2 border-t border-border/60 flex items-center justify-between text-[11px] text-slate-500 font-mono">
                  <span>Accessed: {mem.access_count} times</span>
                  <span>{mem.evidence_count} evidence references</span>
                </div>
              </div>
            );
          })}
        </div>
      </div>

      {/* Right Column: Detailed Memory Inspector */}
      <div className="lg:col-span-5">
        {selectedMemory ? (
          <div className="glass-panel-glow p-6 rounded-2xl border border-border sticky top-24 space-y-5 animate-in fade-in">
            <div className="flex items-center justify-between border-b border-border/80 pb-4">
              <div>
                <span className="text-[10px] font-mono uppercase tracking-wider text-slate-400">
                  Memory Inspector
                </span>
                <h3 className="text-base font-bold text-white mt-0.5">{selectedMemory.title}</h3>
              </div>

              <div className="p-2 rounded-xl bg-primary/10 border border-primary/20 text-primary">
                <Brain className="w-5 h-5" />
              </div>
            </div>

            {/* Metadata Badges */}
            <div className="grid grid-cols-2 gap-3 text-xs font-mono">
              <div className="p-3 rounded-xl bg-card border border-border">
                <span className="text-slate-500 block text-[10px]">EPISTEMIC STATUS</span>
                <span className="font-bold text-emerald-400 uppercase mt-0.5 block">
                  ✓ {selectedMemory.status}
                </span>
              </div>

              <div className="p-3 rounded-xl bg-card border border-border">
                <span className="text-slate-500 block text-[10px]">ACT-R DECAY SCORE</span>
                <span className="font-bold text-primary mt-0.5 block">
                  {(selectedMemory.decay_score * 100).toFixed(1)}% (Active)
                </span>
              </div>
            </div>

            {/* Full Statement */}
            <div>
              <label className="text-[10px] font-mono uppercase tracking-wider text-slate-400 mb-1.5 block">
                Full Statement & Heuristic
              </label>
              <div className="p-4 rounded-xl bg-[#080c16] border border-border text-xs text-slate-200 leading-relaxed font-mono">
                {selectedMemory.content}
              </div>
            </div>

            {/* Scope & Graph Coordinates */}
            <div className="text-xs space-y-2 font-mono">
              <div className="flex justify-between py-1.5 border-b border-border/60">
                <span className="text-slate-500">Scope Filter:</span>
                <span className="text-slate-300 font-semibold">{selectedMemory.scope}</span>
              </div>
              <div className="flex justify-between py-1.5 border-b border-border/60">
                <span className="text-slate-500">Evidence Tree:</span>
                <span className="text-purple-400 font-semibold">{selectedMemory.evidence_count} nodes (JTMS in-tree)</span>
              </div>
              <div className="flex justify-between py-1.5">
                <span className="text-slate-500">Last Synced:</span>
                <span className="text-slate-400">{new Date(selectedMemory.updated_at).toLocaleTimeString()}</span>
              </div>
            </div>
          </div>
        ) : (
          <div className="glass-panel p-10 rounded-2xl border border-border text-center text-slate-500 text-xs">
            Select a memory from the left to inspect its causal graph and retention metrics.
          </div>
        )}
      </div>
    </div>
  );
};
