import React, { useState } from 'react';
import { Sliders, ArrowRight } from 'lucide-react';
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

    setTimeout(() => {
      setResult({
        query: prompt,
        latency_ms: 0.8,
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
      toast.success('Recall Completed', '2 memories retrieved in 0.8ms');
    }, 300);
  };

  return (
    <div className="space-y-4 max-w-4xl font-sans">
      <div className="p-4 rounded-xl border border-[#23262f] bg-[#15171d]">
        <div className="flex items-center gap-2 mb-1">
          <Sliders className="w-4 h-4 text-amber-300/90" />
          <h3 className="text-sm font-semibold text-white">Agent Memory Recall Simulator</h3>
        </div>
        <p className="text-xs text-zinc-400 mb-3">
          Simulate how Claude Code, Cursor or Codex queries the semantic graph and JTMS belief engine.
        </p>

        <form onSubmit={handleSimulate} className="space-y-2.5">
          <textarea
            rows={2}
            value={prompt}
            onChange={(e) => setPrompt(e.target.value)}
            placeholder="Enter an architectural prompt or query..."
            className="w-full px-3 py-2 rounded-lg bg-[#0f1115] border border-[#23262f] text-zinc-200 placeholder:text-zinc-500 focus:outline-none focus:border-amber-300/40 text-xs font-mono resize-none leading-relaxed"
          />

          <button
            type="submit"
            disabled={loading}
            className="px-4 py-2 rounded-lg bg-amber-300 text-zinc-950 font-bold text-xs flex items-center justify-center gap-1.5 hover:bg-amber-200 btn-pressable sweep-hover disabled:opacity-50"
          >
            <span>{loading ? 'Querying...' : 'Execute Simulator Query'}</span>
            <ArrowRight className="w-3.5 h-3.5" />
          </button>
        </form>
      </div>

      {result && (
        <div className="p-5 rounded-xl border border-[#23262f] bg-[#15171d] space-y-4 font-mono text-xs animate-in fade-in">
          <div className="flex items-center justify-between border-b border-[#23262f] pb-2.5 text-zinc-400">
            <span>
              Latency: <strong className="text-emerald-400">{result.latency_ms} ms</strong>
            </span>
            <span>
              JTMS State: <strong className="text-amber-200">{result.jtms_arbitration.belief_state}</strong>
            </span>
          </div>

          <div className="space-y-2">
            <div className="text-zinc-500 text-[11px]">Retrieved Semantic Facts ({result.retrieved_memories.length})</div>
            {result.retrieved_memories.map((m: any, idx: number) => (
              <div key={idx} className="p-3 rounded-lg bg-[#0f1115] border border-[#23262f] space-y-1 sweep-hover">
                <div className="flex items-center justify-between">
                  <span className="font-semibold text-zinc-200">{m.title}</span>
                  <span className="text-amber-200 font-mono text-[11px]">
                    Score: {(m.similarity * 100).toFixed(1)}%
                  </span>
                </div>
                <p className="text-zinc-400 text-xs">{m.content}</p>
              </div>
            ))}
          </div>

          <div className="p-3 rounded-lg bg-[#0f1115] border border-[#23262f] text-zinc-300">
            <span className="text-amber-300 text-[11px] block mb-0.5 font-semibold">Injected Agent Directive:</span>
            <p className="text-zinc-200 text-xs">{result.jtms_arbitration.recommended_directive}</p>
          </div>
        </div>
      )}
    </div>
  );
};
