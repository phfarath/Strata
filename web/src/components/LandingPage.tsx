import React, { useState } from 'react';
import {
  Terminal,
  ArrowRight,
  Check,
} from 'lucide-react';
import { toast } from './Toast';

interface LandingPageProps {
  onOpenAuth: () => void;
}

export const LandingPage: React.FC<LandingPageProps> = ({ onOpenAuth }) => {
  const [copiedInstall, setCopiedInstall] = useState(false);

  const handleCopyInstall = () => {
    navigator.clipboard.writeText('cargo install strata-cli && strata login');
    setCopiedInstall(true);
    toast.success('Command copied', 'Run in terminal to initialize Strata.');
    setTimeout(() => setCopiedInstall(false), 2000);
  };

  return (
    <div className="min-h-screen bg-[#090a0d] text-zinc-100 font-sans">
      {/* Hero Section */}
      <section className="max-w-5xl mx-auto px-4 sm:px-6 pt-24 pb-16 text-center">
        <div className="inline-flex items-center gap-2 px-3 py-1 rounded-md bg-amber-300/10 border border-amber-300/20 text-amber-200 text-xs font-mono mb-8">
          <span className="w-1.5 h-1.5 rounded-full bg-amber-300 inline-block opacity-80" />
          <span>Engineered for Claude Code, Cursor & Windsurf</span>
        </div>

        <h1 className="text-4xl sm:text-6xl font-bold tracking-tight text-white max-w-3xl mx-auto leading-tight mb-6 font-sans">
          Persistent cognitive memory layer for coding agents.
        </h1>

        <p className="text-base sm:text-lg text-zinc-400 max-w-2xl mx-auto mb-10 leading-relaxed font-sans">
          AI coding agents lose context when sessions close. Strata provides zero-latency local memory in embedded SQLite with causal belief tracking and atomic cloud synchronization to PostgreSQL.
        </p>

        <div className="flex flex-col sm:flex-row items-center justify-center gap-3 max-w-md mx-auto mb-16">
          <button
            onClick={onOpenAuth}
            className="w-full sm:w-auto px-6 py-2.5 rounded-lg bg-amber-300 text-zinc-950 font-bold text-xs flex items-center justify-center gap-2 hover:bg-amber-200 btn-pressable sweep-hover shadow-lg shadow-amber-300/5"
          >
            <span>Open Console</span>
            <ArrowRight className="w-4 h-4" />
          </button>

          <button
            onClick={handleCopyInstall}
            className="w-full sm:w-auto px-5 py-2.5 rounded-lg bg-[#15171d] border border-[#23262f] hover:border-[#343846] text-zinc-200 font-mono text-xs flex items-center justify-center gap-2 btn-pressable sweep-hover"
          >
            {copiedInstall ? <Check className="w-3.5 h-3.5 text-emerald-400" /> : <Terminal className="w-3.5 h-3.5 text-amber-300/90" />}
            <span>strata login</span>
          </button>
        </div>

        {/* Technical Terminal Snippet */}
        <div className="max-w-3xl mx-auto text-left rounded-xl border border-[#23262f] bg-[#0f1115] overflow-hidden shadow-2xl">
          <div className="flex items-center justify-between px-4 py-2.5 bg-[#15171d] border-b border-[#23262f] text-xs font-mono text-zinc-400">
            <div className="flex items-center gap-2">
              <span className="w-2.5 h-2.5 rounded-full bg-[#23262f] inline-block" />
              <span className="w-2.5 h-2.5 rounded-full bg-[#23262f] inline-block" />
              <span className="w-2.5 h-2.5 rounded-full bg-[#23262f] inline-block" />
              <span className="ml-2 text-zinc-300">strata-cli — sync & memory recall</span>
            </div>
            <span className="text-zinc-500 text-[11px]">TCP 127.0.0.1</span>
          </div>

          <div className="p-5 font-mono text-xs leading-relaxed space-y-3 bg-[#090a0d] text-zinc-300">
            <div>
              <span className="text-zinc-600">$ </span>
              <span className="text-zinc-100 font-medium">strata login</span>
            </div>
            <div className="text-zinc-400 pl-3 border-l border-[#23262f]">
              Authenticated via browser loopback on https://strata.pedrofarath.me.<br />
              Generated machine API key: <code className="text-amber-200">strata_live_8f92a...</code>
            </div>

            <div>
              <span className="text-zinc-600">$ </span>
              <span className="text-zinc-100 font-medium">strata recall "TLS config for PostgreSQL pooler"</span>
            </div>
            <div className="text-zinc-400 pl-3 border-l border-amber-300/40 text-zinc-200">
              [Memory Match 0.94] "Supabase transaction pooler requires AcceptAnyServerCertVerifier on port 6543, or port 5432 session pooler."<br />
              <span className="text-zinc-500 text-[11px]">Causal JTMS: 0 contradictions | ACT-R Decay: 0.98</span>
            </div>
          </div>
        </div>
      </section>

      {/* Technical Architecture Table */}
      <section className="max-w-5xl mx-auto px-4 sm:px-6 py-16 border-t border-[#23262f]">
        <div className="mb-8">
          <h2 className="text-xl font-bold text-white mb-1">
            System Architecture Specifications
          </h2>
          <p className="text-xs text-zinc-400">
            Engineered in pure Rust with strict atomic consistency and deterministic offline execution.
          </p>
        </div>

        <div className="rounded-xl border border-[#23262f] bg-[#0f1115] overflow-hidden">
          <table className="w-full text-left text-xs font-mono">
            <thead>
              <tr className="border-b border-[#23262f] bg-[#15171d] text-zinc-400">
                <th className="py-3 px-4 font-semibold">Subsystem</th>
                <th className="py-3 px-4 font-semibold">Implementation</th>
                <th className="py-3 px-4 font-semibold">Latency / Guarantee</th>
              </tr>
            </thead>
            <tbody className="divide-y divide-[#23262f] text-zinc-300">
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
      <footer className="border-t border-[#23262f] py-8 text-center text-xs text-zinc-600 font-mono">
        <p>Strata Cognitive Engine • Open Source • <a href="https://github.com/phfarath/Strata" target="_blank" rel="noreferrer" className="text-zinc-400 hover:underline">GitHub</a></p>
      </footer>
    </div>
  );
};
