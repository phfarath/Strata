import React, { useState, useEffect, useRef } from 'react';
import { Radio, Play, Pause, Trash2, ShieldCheck, Activity, Terminal } from 'lucide-react';
import { api } from '../api';
import { RealtimeWsEvent } from '../types';

export const RealtimeStream: React.FC = () => {
  const [events, setEvents] = useState<RealtimeWsEvent[]>([]);
  const [isConnected, setIsConnected] = useState(false);
  const [isPaused, setIsPaused] = useState(false);
  const scrollRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    let ws: WebSocket | null = null;

    try {
      ws = api.createWebSocket(
        (data) => {
          if (!isPaused) {
            setEvents((prev) => [
              ...prev.slice(-100),
              {
                event: data.event || 'delta_stream',
                workspace_id: data.workspace_id,
                max_seq: data.max_seq,
                delta_count: data.delta_count,
                timestamp: new Date().toISOString(),
                raw: data,
              },
            ]);
          }
        },
        () => setIsConnected(true),
        () => setIsConnected(false)
      );
    } catch {
      setIsConnected(false);
    }

    return () => {
      if (ws) ws.close();
    };
  }, [isPaused]);

  useEffect(() => {
    if (scrollRef.current && !isPaused) {
      scrollRef.current.scrollTop = scrollRef.current.scrollHeight;
    }
  }, [events, isPaused]);

  return (
    <div className="space-y-4 max-w-5xl">
      {/* Header with connection status and controls */}
      <div className="glass-panel p-4 rounded-2xl border border-border flex flex-col sm:flex-row items-start sm:items-center justify-between gap-4">
        <div className="flex items-center gap-3">
          <div className="p-2 rounded-xl bg-amber-500/10 border border-amber-500/20 text-amber-400">
            <Radio className="w-5 h-5 animate-pulse" />
          </div>
          <div>
            <h3 className="text-base font-bold text-white flex items-center gap-2">
              <span>Change Data Capture (CDC) Realtime Stream</span>
              <span
                className={`text-[10px] font-mono px-2 py-0.5 rounded-full border ${
                  isConnected
                    ? 'bg-emerald-500/20 text-emerald-400 border-emerald-500/30'
                    : 'bg-rose-500/20 text-rose-400 border-rose-500/30'
                }`}
              >
                {isConnected ? 'LIVE WS' : 'CONNECTING'}
              </span>
            </h3>
            <p className="text-xs text-slate-400">
              Live broadcast channel broadcasting memory mutations across multi-device agents.
            </p>
          </div>
        </div>

        <div className="flex items-center gap-2">
          <button
            onClick={() => setIsPaused(!isPaused)}
            className={`px-3 py-1.5 rounded-xl border text-xs font-semibold flex items-center gap-1.5 btn-pressable ${
              isPaused
                ? 'border-emerald-500/40 bg-emerald-950/40 text-emerald-300'
                : 'border-border bg-card text-slate-300 hover:text-white'
            }`}
          >
            {isPaused ? <Play className="w-3.5 h-3.5" /> : <Pause className="w-3.5 h-3.5" />}
            <span>{isPaused ? 'Resume' : 'Pause'}</span>
          </button>

          <button
            onClick={() => setEvents([])}
            className="p-1.5 rounded-xl border border-border bg-card text-slate-400 hover:text-rose-400 btn-pressable"
            title="Clear terminal log"
          >
            <Trash2 className="w-4 h-4" />
          </button>
        </div>
      </div>

      {/* Stream Terminal Window */}
      <div className="glass-panel rounded-2xl border border-border overflow-hidden">
        <div className="px-4 py-2.5 bg-[#0a0f1d] border-b border-border/80 flex items-center justify-between text-xs font-mono text-slate-400">
          <div className="flex items-center gap-2">
            <Terminal className="w-4 h-4 text-primary" />
            <span>ws://strata.pedrofarath.me/api/v1/sync/ws</span>
          </div>
          <span>{events.length} events captured</span>
        </div>

        <div
          ref={scrollRef}
          className="p-4 h-[420px] overflow-y-auto font-mono text-xs space-y-2 bg-[#060810]"
        >
          {events.length === 0 ? (
            <div className="h-full flex flex-col items-center justify-center text-slate-600 space-y-2">
              <Activity className="w-8 h-8 animate-pulse text-slate-700" />
              <p>Listening for incoming synchronization deltas...</p>
              <p className="text-[11px] text-slate-700">Run `strata sync push` in your terminal to trigger live events.</p>
            </div>
          ) : (
            events.map((evt, idx) => (
              <div
                key={idx}
                className="p-2.5 rounded-xl bg-card/60 border border-border/60 hover:border-primary/40 transition-colors"
              >
                <div className="flex items-center justify-between text-[11px] text-slate-500 mb-1">
                  <span className="text-primary font-bold">{evt.event}</span>
                  <span>{new Date(evt.timestamp).toLocaleTimeString()}</span>
                </div>
                <pre className="text-slate-300 text-[11px] overflow-x-auto">
                  {JSON.stringify(evt.raw, null, 2)}
                </pre>
              </div>
            ))
          )}
        </div>
      </div>
    </div>
  );
};
