import React, { useState } from 'react';
import { Search } from 'lucide-react';
import { MemoryRecord } from '../types';

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
    <div className="grid grid-cols-1 lg:grid-cols-12 gap-6 font-sans">
      {/* Left Column: Search & List */}
      <div className="lg:col-span-7 space-y-3">
        {/* Search and Filters */}
        <div className="flex flex-col sm:flex-row gap-2">
          <div className="relative flex-1">
            <Search className="absolute left-3 top-2.5 w-3.5 h-3.5 text-zinc-500" />
            <input
              type="text"
              placeholder="Search memories, anti-patterns, scopes..."
              value={search}
              onChange={(e) => setSearch(e.target.value)}
              className="w-full pl-9 pr-3 py-2 rounded-lg bg-[#15171d] border border-[#23262f] text-zinc-200 placeholder:text-zinc-500 focus:outline-none focus:border-amber-300/40 text-xs font-mono"
            />
          </div>

          <div className="flex items-center gap-1 bg-[#15171d] border border-[#23262f] p-1 rounded-lg">
            <button
              onClick={() => setTypeFilter('all')}
              className={`px-2.5 py-1 rounded text-xs font-medium btn-pressable ${
                typeFilter === 'all' ? 'bg-[#23262f] text-amber-200 font-bold' : 'text-zinc-400 hover:text-zinc-200'
              }`}
            >
              All
            </button>
            <button
              onClick={() => setTypeFilter('semantic_fact')}
              className={`px-2.5 py-1 rounded text-xs font-medium btn-pressable ${
                typeFilter === 'semantic_fact' ? 'bg-[#23262f] text-amber-200 font-bold' : 'text-zinc-400 hover:text-zinc-200'
              }`}
            >
              Facts
            </button>
            <button
              onClick={() => setTypeFilter('failure_pattern')}
              className={`px-2.5 py-1 rounded text-xs font-medium btn-pressable ${
                typeFilter === 'failure_pattern' ? 'bg-[#23262f] text-amber-200 font-bold' : 'text-zinc-400 hover:text-zinc-200'
              }`}
            >
              Anti-Patterns
            </button>
            <button
              onClick={() => setTypeFilter('procedural_skill')}
              className={`px-2.5 py-1 rounded text-xs font-medium btn-pressable ${
                typeFilter === 'procedural_skill' ? 'bg-[#23262f] text-amber-200 font-bold' : 'text-zinc-400 hover:text-zinc-200'
              }`}
            >
              Skills
            </button>
          </div>
        </div>

        {/* Memory Items with Instant Hover */}
        <div className="space-y-2">
          {filteredMemories.map((mem) => {
            const isSelected = selectedMemory?.id === mem.id;
            return (
              <div
                key={mem.id}
                onClick={() => setSelectedMemory(mem)}
                className={`p-3.5 rounded-lg border cursor-pointer btn-pressable ${
                  isSelected
                    ? 'border-amber-300/50 bg-[#1a1d24] shadow-sm'
                    : 'border-[#23262f] bg-[#15171d] hover:border-amber-300/40 hover:bg-[#1b1e26]'
                }`}
              >
                <div className="flex items-center justify-between gap-2 mb-1">
                  <div className="flex items-center gap-2">
                    <span className="px-1.5 py-0.5 rounded text-[10px] font-mono font-medium bg-[#0f1115] text-amber-200/90 border border-[#23262f]">
                      {mem.type.replace('_', ' ')}
                    </span>
                    <span className="text-[11px] font-mono text-zinc-500 truncate max-w-[200px]">
                      {mem.scope}
                    </span>
                  </div>

                  <span className="text-[11px] font-mono text-zinc-400">
                    Retention: <strong className="text-amber-200">{(mem.decay_score * 100).toFixed(0)}%</strong>
                  </span>
                </div>

                <h4 className="font-semibold text-xs text-zinc-100 mb-0.5">{mem.title}</h4>
                <p className="text-xs text-zinc-400 line-clamp-2 leading-relaxed">{mem.content}</p>
              </div>
            );
          })}
        </div>
      </div>

      {/* Right Column: Detailed Inspector */}
      <div className="lg:col-span-5">
        {selectedMemory ? (
          <div className="p-5 rounded-xl border border-[#23262f] bg-[#15171d] sticky top-6 space-y-4 font-mono text-xs shadow-xl">
            <div className="border-b border-[#23262f] pb-3 flex items-center justify-between">
              <div>
                <div className="text-amber-300 text-[11px] font-semibold">:: Memory Inspector</div>
                <h3 className="text-sm font-bold text-white mt-1">{selectedMemory.title}</h3>
              </div>
            </div>

            <div className="grid grid-cols-2 gap-2 text-[11px]">
              <div className="p-2.5 rounded-md bg-[#0f1115] border border-[#23262f]">
                <span className="text-zinc-500 block">Status</span>
                <span className="font-semibold text-emerald-400 mt-0.5 block">{selectedMemory.status}</span>
              </div>
              <div className="p-2.5 rounded-md bg-[#0f1115] border border-[#23262f]">
                <span className="text-zinc-500 block">ACT-R Retention</span>
                <span className="font-semibold text-amber-200 mt-0.5 block">{(selectedMemory.decay_score * 100).toFixed(1)}%</span>
              </div>
            </div>

            <div>
              <div className="text-zinc-500 text-[11px] mb-1">Statement Bedrock</div>
              <div className="p-3 rounded-md bg-[#090a0d] border border-[#23262f] text-zinc-200 text-xs leading-relaxed">
                {selectedMemory.content}
              </div>
            </div>

            <div className="space-y-1.5 pt-2 border-t border-[#23262f] text-[11px]">
              <div className="flex justify-between">
                <span className="text-zinc-500">Scope:</span>
                <span className="text-sky-300">{selectedMemory.scope}</span>
              </div>
              <div className="flex justify-between">
                <span className="text-zinc-500">Evidence Graph:</span>
                <span className="text-zinc-300">{selectedMemory.evidence_count} nodes</span>
              </div>
              <div className="flex justify-between">
                <span className="text-zinc-500">Access Count:</span>
                <span className="text-zinc-300">{selectedMemory.access_count} reads</span>
              </div>
            </div>
          </div>
        ) : null}
      </div>
    </div>
  );
};
