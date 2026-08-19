import React, { useState } from 'react';
import { PlayCircle, Sparkles, Send, Brain, GitBranch, ShieldCheck, CheckCircle2, ArrowRight } from 'lucide-react';
import { toast } from './Toast';

export const AgentPlayground: React.FC = () => {
  const [prompt, setPrompt] = useState('How should we handle database connections and SSL certificates on Supabase?');
  const [loading, setLoading] = useState(false);
  const [result, setResult] = useState<any | null>(null);

  const handleSimulate = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!prompt.trim()) return;

    setLoading(true);
    setResult(null);

    // Simulate vector search + JTMS arbitration pipeline
    setTimeout(() => {
      setResult({
        query: prompt,
        latency_ms: 1.4,
        retrieved_memories: [
          {
            title: 'PostgreSQL Supabase Transaction Pooler Port 6543 TLS Handshake',
            similarity: 0.942,
            type: 'semantic_fact',
            scope: 'crates/strata-server/storage',
            status: 'verified',
            content: 'Supabase transaction pooler on port 6543 uses wildcard SANs (*.supabase.co). Client must utilize AcceptAnyServerCertVerifier unless sslmode=verify-full is explicitly requested.',
          },
          {
            title: 'Anti-Pattern: OpenSSL Dynamic Linking on Distroless Containers',
            similarity: 0.825,
            type: 'failure_pattern',
            scope: 'global',
            status: 'verified',
            content: 'Do not use native OpenSSL bindings in server dependencies. Always prefer pure Rust TLS with rustls + ring and webpki-roots.',
          },
        ],
        jtms_arbitration: {
          conflicts_detected: 0,
          belief_state: 'CONSISTENT',
          recommended_directive: 'Apply AcceptAnyServerCertVerifier with rustls::crypto::ring provider to guarantee zero handshake failures.',
        },
      });
      setLoading(false);
      toast.success('Simulation Completed', 'Retrieved 2 high-confidence memories in 1.4ms.');
    }, 450);
  };

  return (
    <div className="space-y-6 max-w-4xl">
      <div className="glass-panel p-6 rounded-2xl border border-border">
        <div className="flex items-center gap-3 mb-2">
          <div className="p-2 rounded-xl bg-purple-500/10 border border-purple-500/20 text-purple-400">
            <PlayCircle className="w-5 h-5" />
          </div>
          <div>
            <h3 className="text-base font-bold text-white">Agent Memory Simulator</h3>
            <p className="text-xs text-slate-400">
              Test how Claude Code or Cursor queries Strata's semantic memory graph and JTMS belief engine.
            </p>
          </div>
        </div>

        <form onSubmit={handleSimulate} className="mt-4 space-y-3">
          <div>
            <label className="text-[10px] font-mono uppercase tracking-wider text-slate-400 mb-1 block">
              Agent Coding Prompt
            </label>
            <textarea
              rows={2}
              value={prompt}
              onChange={(e) => setPrompt(e.target.value)}
              placeholder="Enter a task or architectural query..."
              className="w-full px-4 py-3 rounded-xl bg-[#080c16] border border-border text-slate-100 placeholder:text-slate-500 focus:outline-none focus:border-primary text-xs font-mono resize-none leading-relaxed"
            />
          </div>

          <button
            type="submit"
            disabled={loading}
            className="px-6 py-2.5 rounded-xl bg-primary hover:bg-primary-hover text-black font-bold text-xs flex items-center justify-center gap-2 btn-pressable shadow-glow disabled:opacity-50"
          >
            <Sparkles className="w-4 h-4" />
            <span>{loading ? 'Retrieving Memories...' : 'Simulate Agent Recall'}</span>
          </button>
        </form>
      </div>

      {result && (
        <div className="glass-panel-glow p-6 rounded-2xl border border-border space-y-5 animate-in fade-in zoom-in-95">
          <div className="flex items-center justify-between border-b border-border/80 pb-3 text-xs font-mono">
            <span className="text-slate-400">
              Query Latency: <strong className="text-emerald-400">{result.latency_ms}ms</strong>
            </span>
            <span className="text-purple-400 font-bold">
              JTMS State: {result.jtms_arbitration.belief_state}
            </span>
          </div>

          <div>
            <h4 className="text-xs font-bold font-mono uppercase tracking-wider text-slate-400 mb-3">
              Retrieved Semantic Memories ({result.retrieved_memories.length})
            </h4>

            <div className="space-y-3">
              {result.retrieved_memories.map((m: any, idx: number) => (
                <div key={idx} className="p-4 rounded-xl bg-card border border-border">
                  <div className="flex items-center justify-between text-xs mb-1.5 font-mono">
                    <span className="font-bold text-white">{m.title}</span>
                    <span className="text-primary font-bold">
                      Cosine: {(m.similarity * 100).toFixed(1)}%
                    </span>
                  </div>
                  <p className="text-xs text-slate-300 font-mono leading-relaxed">{m.content}</p>
                </div>
              ))}
            </div>
          </div>

          <div className="p-4 rounded-xl bg-purple-950/30 border border-purple-500/30 text-xs font-mono">
            <span className="text-purple-300 font-bold block mb-1">🤖 Injected Prompt Context for Agent:</span>
            <p className="text-slate-200">{result.jtms_arbitration.recommended_directive}</p>
          </div>
        </div>
      )}
    </div>
  );
};
