import React, { useState, useEffect, useRef } from 'react';
import { Play, Pause, Trash2, Activity, Terminal } from 'lucide-react';
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
    <div className="space-y-4 max-w-5xl font-sans">
      {/* Stream Controls */}
      <div className="p-4 rounded-xl border border-zinc-800 bg-[#111114] flex flex-col sm:flex-row items-start sm:items-center justify-between gap-4">
        <div>
          <div className="flex items-center gap-2">
            <h3 className="text-sm font-semibold text-white">CDC Realtime Delta Stream</h3>
          </div>
          <p className="text-xs text-zinc-400 mt-0.5">
            Broadcast channel streaming memory insertions, updates, and causal invalidations.
          </p>
        </div>

        <div className="flex items-center gap-2">
          <button
            onClick={() => setIsPaused(!isPaused)}
            className={`px-3 py-1.5 rounded-lg border text-xs font-medium flex items-center gap-1.5 btn-pressable ${
              isPaused
                ? 'border-zinc-700 bg-zinc-800 text-zinc-200'
                : 'border-zinc-800 bg-zinc-900 text-zinc-300 hover:text-white'
            }`}
          >
            {isPaused ? <Play className="w-3.5 h-3.5" /> : <Pause className="w-3.5 h-3.5" />}
            <span>{isPaused ? 'Resume' : 'Pause'}</span>
          </button>

          <button
            onClick={() => setEvents([])}
            className="p-1.5 rounded-lg border border-zinc-800 bg-zinc-900 text-zinc-400 hover:text-zinc-200 btn-pressable"
            title="Clear stream log"
          >
            <Trash2 className="w-3.5 h-3.5" />
          </button>
        </div>
      </div>

      {/* Terminal View */}
      <div className="rounded-xl border border-zinc-800 bg-[#0c0c0f] overflow-hidden">
        <div className="px-4 py-2 bg-zinc-900/60 border-b border-zinc-800 flex items-center justify-between text-xs font-mono text-zinc-500">
          <span>ws://strata.pedrofarath.me/api/v1/sync/ws</span>
          <span>{events.length} frames</span>
        </div>

        <div
          ref={scrollRef}
          className="p-4 h-[400px] overflow-y-auto font-mono text-xs space-y-2 bg-[#09090c]"
        >
          {events.length === 0 ? (
            <div className="h-full flex flex-col items-center justify-center text-zinc-600 space-y-1">
              <Activity className="w-6 h-6 text-zinc-700" />
              <p className="text-xs">Listening for incoming CDC deltas...</p>
              <p className="text-[11px] text-zinc-700">Run `strata sync push` in terminal to push events.</p>
            </div>
          ) : (
            events.map((evt, idx) => (
              <div
                key={idx}
                className="p-2.5 rounded-md bg-zinc-900/60 border border-zinc-800/80 text-zinc-300"
              >
                <div className="flex items-center justify-between text-[11px] text-zinc-500 mb-1">
                  <span className="font-semibold text-zinc-300">{evt.event}</span>
                  <span>{new Date(evt.timestamp).toLocaleTimeString()}</span>
                </div>
                <pre className="text-zinc-400 text-[11px] overflow-x-auto">
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
