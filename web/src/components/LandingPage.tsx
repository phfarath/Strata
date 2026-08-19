import React, { useState, useEffect } from 'react';
import {
  Terminal,
  Layers,
  Shield,
  ArrowRight,
  Database,
  Check,
  Cpu,
  GitBranch,
} from 'lucide-react';
import { toast } from './Toast';
import { PingResponse } from '../types';
import { api } from '../api';

interface LandingPageProps {
  onOpenAuth: () => void;
}

export const LandingPage: React.FC<LandingPageProps> = ({ onOpenAuth }) => {
  const [ping, setPing] = useState<PingResponse | null>(null);
  const [copiedInstall, setCopiedInstall] = useState(false);

  useEffect(() => {
    api.ping().then(setPing).catch(() => {});
  }, []);

  const handleCopyInstall = () => {
    navigator.clipboard.writeText('cargo install strata-cli && strata login');
    setCopiedInstall(true);
    toast.success('Command copied', 'Run in terminal to initialize Strata.');
    setTimeout(() => setCopiedInstall(false), 2000);
  };

  return (
    <div className="min-h-screen bg-[#09090b] text-zinc-100 font-sans">
      {/* Top Telemetry Strip */}
      <div className="border-b border-[#27272a] bg-[#121215] py-2 px-4 text-center text-xs font-mono text-zinc-400">
        <div className="max-w-6xl mx-auto flex items-center justify-between">
          <div className="flex items-center gap-2">
            <span className="w-1.5 h-1.5 rounded-full bg-emerald-500" />
            <span>strata.pedrofarath.me</span>
            <span className="text-zinc-600">/</span>
            <span className="text-zinc-300">PostgreSQL + pgvector (Supabase)</span>
          </div>
          <div className="hidden sm:flex items-center gap-3">
            <span>Protocol: <code className="text-zinc-200">v1.0-cdc</code></span>
            <span>Uptime: <code className="text-zinc-200">{ping ? `${Math.round(ping.uptime_secs / 60)}m` : 'active'}</code></span>
          </div>
        </div>
      </div>

      {/* Hero Section */}
      <section className="max-w-5xl mx-auto px-4 sm:px-6 pt-20 pb-16 text-center">
        <div className="inline-flex items-center gap-2 px-3 py-1 rounded-md bg-zinc-900 border border-zinc-800 text-zinc-300 text-xs font-mono mb-8">
          <span>Engineered for Claude Code, Cursor & Windsurf</span>
        </div>

        <h1 className="text-4xl sm:text-6xl font-bold tracking-tight text-white max-w-3xl mx-auto leading-tight mb-6">
          Persistent cognitive memory layer for coding agents.
        </h1>

        <p className="text-base sm:text-lg text-zinc-400 max-w-2xl mx-auto mb-10 leading-relaxed">
          AI coding agents lose context when sessions close. Strata provides zero-latency local memory in embedded SQLite with causal belief tracking and atomic cloud synchronization to PostgreSQL.
        </p>

        <div className="flex flex-col sm:flex-row items-center justify-center gap-3 max-w-md mx-auto mb-16">
          <button
            onClick={onOpenAuth}
            className="w-full sm:w-auto px-6 py-2.5 rounded-lg bg-white text-black font-semibold text-sm flex items-center justify-center gap-2 hover:bg-zinc-200 btn-pressable"
          >
            <span>Open Console</span>
            <ArrowRight className="w-4 h-4" />
          </button>

          <button
            onClick={handleCopyInstall}
            className="w-full sm:w-auto px-5 py-2.5 rounded-lg bg-zinc-900 border border-zinc-800 hover:border-zinc-700 text-zinc-200 font-mono text-xs flex items-center justify-center gap-2 btn-pressable"
          >
            {copiedInstall ? <Check className="w-3.5 h-3.5 text-emerald-400" /> : <Terminal className="w-3.5 h-3.5 text-zinc-400" />}
            <span>strata login</span>
          </button>
        </div>

        {/* Technical Terminal Snippet */}
        <div className="max-w-3xl mx-auto text-left rounded-xl border border-zinc-800 bg-[#0d0d10] overflow-hidden shadow-2xl">
          <div className="flex items-center justify-between px-4 py-2.5 bg-zinc-900/80 border-b border-zinc-800 text-xs font-mono text-zinc-400">
            <div className="flex items-center gap-2">
              <span className="w-2.5 h-2.5 rounded-full bg-zinc-700 inline-block" />
              <span className="w-2.5 h-2.5 rounded-full bg-zinc-700 inline-block" />
              <span className="w-2.5 h-2.5 rounded-full bg-zinc-700 inline-block" />
              <span className="ml-2 text-zinc-300">strata-cli — sync & memory recall</span>
            </div>
            <span className="text-emerald-400 text-[11px]">TCP 127.0.0.1</span>
          </div>

          <div className="p-5 font-mono text-xs leading-relaxed space-y-3 bg-[#09090c] text-zinc-300">
            <div>
              <span className="text-zinc-600">$ </span>
              <span className="text-zinc-100 font-medium">strata login</span>
            </div>
            <div className="text-zinc-400 pl-3 border-l border-zinc-800">
              Authenticated via browser loopback on https://strata.pedrofarath.me.<br />
              Generated machine API key: <code className="text-zinc-200">strata_live_8f92a...</code>
            </div>

            <div>
              <span className="text-zinc-600">$ </span>
              <span className="text-zinc-100 font-medium">strata recall "TLS config for PostgreSQL pooler"</span>
            </div>
            <div className="text-zinc-400 pl-3 border-l border-emerald-800/60 text-zinc-300">
              [Memory Match 0.94] "Supabase transaction pooler requires AcceptAnyServerCertVerifier on port 6543, or port 5432 session pooler."<br />
              <span className="text-zinc-500 text-[11px]">Causal JTMS: 0 contradictions | ACT-R Decay: 0.98 (active)</span>
            </div>
          </div>
        </div>
      </section>

      {/* Technical Architecture Table */}
      <section className="max-w-5xl mx-auto px-4 sm:px-6 py-16 border-t border-[#27272a]">
        <div className="mb-8">
          <h2 className="text-xl font-bold text-white mb-1">
            System Architecture Specifications
          </h2>
          <p className="text-xs text-zinc-400">
            Engineered in pure Rust with strict atomic consistency and deterministic offline execution.
          </p>
        </div>

        <div className="rounded-xl border border-zinc-800 bg-[#0d0d10] overflow-hidden">
          <table className="w-full text-left text-xs font-mono">
            <thead>
              <tr className="border-b border-zinc-800 bg-zinc-900/50 text-zinc-400">
                <th className="py-3 px-4 font-semibold">Subsystem</th>
                <th className="py-3 px-4 font-semibold">Implementation</th>
                <th className="py-3 px-4 font-semibold">Latency / Guarantee</th>
              </tr>
            </thead>
            <tbody className="divide-y divide-zinc-800/60 text-zinc-300">
              <tr>
                <td className="py-3.5 px-4 font-medium text-white">Local Storage Engine</td>
                <td className="py-3.5 px-4 text-zinc-400">Embedded SQLite (WAL mode, memory-mapped)</td>
                <td className="py-3.5 px-4 text-emerald-400">&lt; 0.5 ms read/write</td>
              </tr>
              <tr>
                <td className="py-3.5 px-4 font-medium text-white">Cloud Sync Backend</td>
                <td className="py-3.5 px-4 text-zinc-400">PostgreSQL + pgvector (Supabase Managed)</td>
                <td className="py-3.5 px-4 text-zinc-300">Monotonic sequence CDC</td>
              </tr>
              <tr>
                <td className="py-3.5 px-4 font-medium text-white">Belief Maintenance</td>
                <td className="py-3.5 px-4 text-zinc-400">Justification-based Truth Maintenance (JTMS)</td>
                <td className="py-3.5 px-4 text-zinc-300">Deterministic dependency graph</td>
              </tr>
              <tr>
                <td className="py-3.5 px-4 font-medium text-white">Decay Algorithm</td>
                <td className="py-3.5 px-4 text-zinc-400">ACT-R cognitive model: R = e^(-t/S)</td>
                <td className="py-3.5 px-4 text-zinc-300">Continuous half-life adjustment</td>
              </tr>
              <tr>
                <td className="py-3.5 px-4 font-medium text-white">Client Protocol</td>
                <td className="py-3.5 px-4 text-zinc-400">Model Context Protocol (MCP) + CLI Gateway</td>
                <td className="py-3.5 px-4 text-zinc-300">Universal multi-IDE support</td>
              </tr>
            </tbody>
          </table>
        </div>
      </section>

      {/* Footer */}
      <footer className="border-t border-[#27272a] py-8 text-center text-xs text-zinc-600 font-mono">
        <p>Strata Cognitive Engine • Open Source • <a href="https://github.com/phfarath/Strata" target="_blank" rel="noreferrer" className="text-zinc-400 hover:underline">GitHub</a></p>
      </footer>
    </div>
  );
};
