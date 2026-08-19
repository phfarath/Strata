import React, { useState, useEffect } from 'react';
import {
  Sparkles,
  Terminal,
  Brain,
  Zap,
  ShieldCheck,
  Cpu,
  Layers,
  ArrowRight,
  Database,
  Radio,
  CheckCircle2,
  Lock,
  GitBranch,
  RefreshCw,
  Clock,
  Code2,
  Copy,
  Check,
} from 'lucide-react';
import { CodeBlock } from './CodeBlock';
import { toast } from './Toast';
import { PingResponse } from '../types';
import { api } from '../api';

interface LandingPageProps {
  onOpenAuth: () => void;
}

export const LandingPage: React.FC<LandingPageProps> = ({ onOpenAuth }) => {
  const [ping, setPing] = useState<PingResponse | null>(null);
  const [copiedInstall, setCopiedInstall] = useState(false);
  const [activeTab, setActiveTab] = useState<'episodic' | 'semantic' | 'jtms' | 'decay'>('semantic');

  useEffect(() => {
    api.ping().then(setPing).catch(() => {});
  }, []);

  const handleCopyInstall = () => {
    navigator.clipboard.writeText('cargo install strata-cli && strata login');
    setCopiedInstall(true);
    toast.success('Command Copied', 'Run in your terminal to initialize Strata.');
    setTimeout(() => setCopiedInstall(false), 2000);
  };

  return (
    <div className="relative min-h-screen hero-glow subtle-grid overflow-hidden">
      {/* Top Banner / Announcement */}
      <div className="border-b border-border/60 bg-primary/5 py-2 px-4 text-center text-xs font-mono text-slate-300">
        <span className="inline-flex items-center gap-2">
          <span className="w-2 h-2 rounded-full bg-emerald-400 animate-pulse" />
          <span>Strata Cloud Core v0.1.0 is live on <strong>strata.pedrofarath.me</strong></span>
          <span className="text-slate-500">•</span>
          <span className="text-primary font-semibold">PostgreSQL & pgvector active</span>
        </span>
      </div>

      {/* Hero Section */}
      <section className="max-w-6xl mx-auto px-4 sm:px-6 pt-16 pb-20 text-center">
        {/* Badge */}
        <div className="inline-flex items-center gap-2 px-3.5 py-1.5 rounded-full glass-panel border border-primary/30 text-primary text-xs font-semibold mb-8 shadow-glow">
          <Sparkles className="w-3.5 h-3.5" />
          <span>The Memory Infrastructure for AI Coding Agents</span>
        </div>

        {/* Title */}
        <h1 className="text-4xl sm:text-6xl lg:text-7xl font-extrabold tracking-tight text-white max-w-4xl mx-auto leading-[1.08] mb-6">
          Never let your coding agents{' '}
          <span className="text-transparent bg-clip-text bg-gradient-to-r from-primary via-sky-300 to-accent-purple">
            start from zero again.
          </span>
        </h1>

        {/* Subtitle */}
        <p className="text-lg sm:text-xl text-slate-400 max-w-2xl mx-auto mb-10 leading-relaxed">
          Coding agents in Cursor, Claude Code, and Gemini forget architectural decisions when sessions close. 
          <strong> Strata</strong> is a high-speed, offline-first cognitive memory engine in pure Rust with causal graphs and realtime cloud sync.
        </p>

        {/* Action Buttons */}
        <div className="flex flex-col sm:flex-row items-center justify-center gap-4 max-w-md mx-auto mb-14">
          <button
            onClick={onOpenAuth}
            className="w-full sm:w-auto px-8 py-3.5 rounded-xl bg-primary hover:bg-primary-hover text-black font-bold text-sm flex items-center justify-center gap-2 shadow-glow-lg btn-pressable"
          >
            <span>Open Cloud Console</span>
            <ArrowRight className="w-4 h-4" />
          </button>

          <button
            onClick={handleCopyInstall}
            className="w-full sm:w-auto px-6 py-3.5 rounded-xl glass-panel border border-border hover:border-primary/50 text-slate-200 hover:text-white font-mono text-xs flex items-center justify-center gap-2 btn-pressable"
          >
            {copiedInstall ? <Check className="w-4 h-4 text-emerald-400" /> : <Terminal className="w-4 h-4 text-primary" />}
            <span>strata login</span>
          </button>
        </div>

        {/* Live Terminal Demo */}
        <div className="max-w-3xl mx-auto text-left glass-panel-glow rounded-2xl border border-border shadow-2xl overflow-hidden animate-in fade-in slide-in-from-bottom-6 duration-300">
          <div className="flex items-center justify-between px-4 py-3 bg-[#0d121f] border-b border-border/80 text-xs font-mono text-slate-400">
            <div className="flex items-center gap-2">
              <span className="w-3 h-3 rounded-full bg-rose-500/80 inline-block" />
              <span className="w-3 h-3 rounded-full bg-amber-500/80 inline-block" />
              <span className="w-3 h-3 rounded-full bg-emerald-500/80 inline-block" />
              <span className="ml-2 text-slate-300 font-semibold">terminal — strata sync & recall</span>
            </div>
            <div className="flex items-center gap-1 text-[11px] text-emerald-400">
              <span className="w-1.5 h-1.5 rounded-full bg-emerald-400 animate-ping" />
              <span>LIVE CLUSTER</span>
            </div>
          </div>

          <div className="p-5 font-mono text-xs leading-relaxed space-y-3 bg-[#080b14] text-slate-200">
            <div>
              <span className="text-slate-500">$ </span>
              <span className="text-primary font-bold">strata login</span>
            </div>
            <div className="text-slate-400 pl-4 border-l border-border">
              🌐 Opening browser at <span className="text-sky-300 underline">https://strata.pedrofarath.me/auth/cli</span>...<br />
              ✓ Authenticated! Machine API key generated and stored at <code className="text-slate-300">~/.strata/config.toml</code>
            </div>

            <div>
              <span className="text-slate-500">$ </span>
              <span className="text-primary font-bold">claude "Refactor auth middleware according to Strata architecture rule"</span>
            </div>
            <div className="text-emerald-400 pl-4 border-l border-emerald-500/40">
              ⚡ [Strata Memory Gateway] Retrieved 3 high-confidence semantic facts (decay: 0.98, S: 864000s):<br />
              <span className="text-slate-300">  1. [VerifiedFact] "Strict Security Headers Middleware: HSTS 2y + CSP"</span><br />
              <span className="text-slate-300">  2. [AntiPattern] "Do not use OpenSSL native bindings; prefer rustls + ring"</span><br />
              <span className="text-slate-300">  3. [JTMS Belief] "Postgres Transaction Pooler on port 6543 requires flexible cert verifier"</span>
            </div>

            <div>
              <span className="text-slate-500">$ </span>
              <span className="text-primary font-bold">strata sync status</span>
            </div>
            <div className="text-slate-400 pl-4 border-l border-border flex items-center justify-between">
              <span>Workspace: <strong>phfarath/Strata</strong> | Total Deltas: <strong>2,418</strong> | Remotes Synced: <strong>100%</strong></span>
              <span className="text-emerald-400 font-bold">CONNECTED (0.8ms)</span>
            </div>
          </div>
        </div>
      </section>

      {/* 4 Core Pillars Interactive Tabs */}
      <section className="max-w-6xl mx-auto px-4 sm:px-6 py-16 border-t border-border/80">
        <div className="text-center max-w-2xl mx-auto mb-12">
          <h2 className="text-2xl sm:text-4xl font-extrabold text-white tracking-tight mb-3">
            Engineered for Cognitive Continuity
          </h2>
          <p className="text-sm text-slate-400">
            A state-of-the-art memory pipeline combining mathematical decay, causal truth revision, and multi-tenant cloud sync.
          </p>
        </div>

        {/* Tab Selector */}
        <div className="flex flex-wrap justify-center gap-2 mb-10 max-w-2xl mx-auto">
          <button
            onClick={() => setActiveTab('semantic')}
            className={`px-4 py-2 rounded-xl text-xs font-semibold flex items-center gap-2 btn-pressable transition-all ${
              activeTab === 'semantic'
                ? 'bg-primary text-black shadow-glow'
                : 'glass-panel text-slate-400 hover:text-slate-200'
            }`}
          >
            <Brain className="w-4 h-4" />
            <span>Semantic & Episodic Graph</span>
          </button>

          <button
            onClick={() => setActiveTab('jtms')}
            className={`px-4 py-2 rounded-xl text-xs font-semibold flex items-center gap-2 btn-pressable transition-all ${
              activeTab === 'jtms'
                ? 'bg-primary text-black shadow-glow'
                : 'glass-panel text-slate-400 hover:text-slate-200'
            }`}
          >
            <GitBranch className="w-4 h-4" />
            <span>JTMS Belief Revision</span>
          </button>

          <button
            onClick={() => setActiveTab('decay')}
            className={`px-4 py-2 rounded-xl text-xs font-semibold flex items-center gap-2 btn-pressable transition-all ${
              activeTab === 'decay'
                ? 'bg-primary text-black shadow-glow'
                : 'glass-panel text-slate-400 hover:text-slate-200'
            }`}
          >
            <Clock className="w-4 h-4" />
            <span>ACT-R Mathematical Decay</span>
          </button>

          <button
            onClick={() => setActiveTab('episodic')}
            className={`px-4 py-2 rounded-xl text-xs font-semibold flex items-center gap-2 btn-pressable transition-all ${
              activeTab === 'episodic'
                ? 'bg-primary text-black shadow-glow'
                : 'glass-panel text-slate-400 hover:text-slate-200'
            }`}
          >
            <Database className="w-4 h-4" />
            <span>Offline-First CDC Engine</span>
          </button>
        </div>

        {/* Tab Content Cards */}
        <div className="grid grid-cols-1 md:grid-cols-3 gap-6">
          <div className="glass-panel p-6 rounded-2xl border border-border card-interactive">
            <div className="w-10 h-10 rounded-xl bg-primary/10 border border-primary/20 flex items-center justify-center text-primary mb-4">
              <Zap className="w-5 h-5" />
            </div>
            <h3 className="text-lg font-bold text-white mb-2">Zero-Latency Local Runtime</h3>
            <p className="text-sm text-slate-400 leading-relaxed">
              Every memory read and write executes on local embedded SQLite in microseconds. When online, deltas sync seamlessly to Supabase PostgreSQL.
            </p>
          </div>

          <div className="glass-panel p-6 rounded-2xl border border-border card-interactive">
            <div className="w-10 h-10 rounded-xl bg-purple-500/10 border border-purple-500/20 flex items-center justify-center text-purple-400 mb-4">
              <GitBranch className="w-5 h-5" />
            </div>
            <h3 className="text-lg font-bold text-white mb-2">Epistemic Truth Maintenance</h3>
            <p className="text-sm text-slate-400 leading-relaxed">
              No hallucinatory memory overwrites. JTMS tracks evidence trees. When an assumption is refuted, all dependent conclusions automatically update.
            </p>
          </div>

          <div className="glass-panel p-6 rounded-2xl border border-border card-interactive">
            <div className="w-10 h-10 rounded-xl bg-emerald-500/10 border border-emerald-500/20 flex items-center justify-center text-emerald-400 mb-4">
              <ShieldCheck className="w-5 h-5" />
            </div>
            <h3 className="text-lg font-bold text-white mb-2">Continuous LoRA Distillation</h3>
            <p className="text-sm text-slate-400 leading-relaxed">
              Strata mines implicit developer feedback (accepted suggestions vs reverts) directly into DPO, KTO, and SFT datasets ready for Unsloth and Ollama fine-tuning.
            </p>
          </div>
        </div>
      </section>

      {/* Supported Coding Agents */}
      <section className="max-w-6xl mx-auto px-4 sm:px-6 py-16 border-t border-border/80">
        <div className="text-center max-w-xl mx-auto mb-10">
          <h2 className="text-xl sm:text-2xl font-bold text-white mb-2">
            Universal Native Integration
          </h2>
          <p className="text-xs text-slate-400">
            Works across all modern coding agents through the standard MCP protocol and native CLI hooks.
          </p>
        </div>

        <div className="grid grid-cols-2 sm:grid-cols-4 gap-4 max-w-4xl mx-auto font-mono text-xs">
          <div className="p-4 rounded-xl glass-panel border border-border flex items-center gap-3">
            <div className="w-2.5 h-2.5 rounded-full bg-primary" />
            <span className="text-slate-200 font-semibold">Cursor IDE</span>
          </div>
          <div className="p-4 rounded-xl glass-panel border border-border flex items-center gap-3">
            <div className="w-2.5 h-2.5 rounded-full bg-accent-purple" />
            <span className="text-slate-200 font-semibold">Claude Code</span>
          </div>
          <div className="p-4 rounded-xl glass-panel border border-border flex items-center gap-3">
            <div className="w-2.5 h-2.5 rounded-full bg-emerald-400" />
            <span className="text-slate-200 font-semibold">Gemini CLI</span>
          </div>
          <div className="p-4 rounded-xl glass-panel border border-border flex items-center gap-3">
            <div className="w-2.5 h-2.5 rounded-full bg-amber-400" />
            <span className="text-slate-200 font-semibold">OpenAI Codex</span>
          </div>
        </div>
      </section>

      {/* CTA Section */}
      <section className="max-w-4xl mx-auto px-4 sm:px-6 py-20 text-center">
        <div className="glass-panel-glow p-10 rounded-3xl border border-primary/30 relative overflow-hidden">
          <div className="relative z-10">
            <h2 className="text-3xl sm:text-4xl font-extrabold text-white mb-4">
              Give your coding agents permanent memory today.
            </h2>
            <p className="text-sm text-slate-400 max-w-xl mx-auto mb-8">
              Open the web console or connect directly from your command line in 5 seconds.
            </p>
            <button
              onClick={onOpenAuth}
              className="px-8 py-3.5 rounded-xl bg-primary hover:bg-primary-hover text-black font-bold text-sm shadow-glow-lg btn-pressable inline-flex items-center gap-2"
            >
              <span>Get Started with Strata Cloud</span>
              <ArrowRight className="w-4 h-4" />
            </button>
          </div>
        </div>
      </section>

      {/* Footer */}
      <footer className="border-t border-border/80 py-8 text-center text-xs text-slate-500 font-mono">
        <p>Strata Cognitive Architecture • Pure Rust Engine • <a href="https://github.com/phfarath/Strata" target="_blank" rel="noreferrer" className="text-primary hover:underline">GitHub</a></p>
      </footer>
    </div>
  );
};
