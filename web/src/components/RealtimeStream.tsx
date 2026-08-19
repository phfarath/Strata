import React, { useState, useEffect, useRef } from 'react';
import { Play, Pause, Trash2, Activity } from 'lucide-react';
import { api } from '../api';
import { RealtimeWsEvent } from '../types';

export const RealtimeStream: React.FC = () => {
  const [events, setEvents] = useState<RealtimeWsEvent[]>([]);
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
        () => {},
        () => {}
      );
    } catch {
      // ignore
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
      <div className="p-4 rounded-xl border border-[#23262f] bg-[#15171d] flex flex-col sm:flex-row items-start sm:items-center justify-between gap-4">
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
            className={`px-3 py-1.5 rounded-lg border text-xs font-medium flex items-center gap-1.5 btn-pressable sweep-hover ${
              isPaused
                ? 'border-[#343846] bg-[#23262f] text-zinc-200'
                : 'border-[#23262f] bg-[#0f1115] text-zinc-300 hover:text-white'
            }`}
          >
            {isPaused ? <Play className="w-3.5 h-3.5" /> : <Pause className="w-3.5 h-3.5" />}
            <span>{isPaused ? 'Resume' : 'Pause'}</span>
          </button>

          <button
            onClick={() => setEvents([])}
            className="p-1.5 rounded-lg border border-[#23262f] bg-[#0f1115] text-zinc-400 hover:text-zinc-200 btn-pressable sweep-hover"
            title="Clear stream log"
          >
            <Trash2 className="w-3.5 h-3.5" />
          </button>
        </div>
      </div>

      {/* Terminal View */}
      <div className="rounded-xl border border-[#23262f] bg-[#0f1115] overflow-hidden shadow-xl">
        <div className="px-4 py-2 bg-[#15171d] border-b border-[#23262f] flex items-center justify-between text-xs font-mono text-zinc-500">
          <span>ws://strata.pedrofarath.me/api/v1/sync/ws</span>
          <span className="text-amber-500 font-semibold">{events.length} frames</span>
        </div>

        <div
          ref={scrollRef}
          className="p-4 h-[400px] overflow-y-auto font-mono text-xs space-y-2 bg-[#090a0d]"
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
                className="p-2.5 rounded-md bg-[#15171d] border border-[#23262f] text-zinc-300 sweep-hover"
              >
                <div className="flex items-center justify-between text-[11px] text-zinc-500 mb-1">
                  <span className="font-semibold text-amber-400">{evt.event}</span>
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
