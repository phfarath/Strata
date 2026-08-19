import React, { useState, useEffect } from 'react';
import { Terminal, ChevronDown, Plus, LogOut, Key, User as UserIcon, LayoutDashboard, Layers, Activity, Sliders } from 'lucide-react';
import { User, Workspace, PingResponse } from '../types';
import { api } from '../api';
import { toast } from './Toast';

interface NavbarProps {
  user: User | null;
  workspaces: Workspace[];
  activeWorkspace: Workspace | null;
  onSelectWorkspace: (ws: Workspace) => void;
  onCreateWorkspaceClick: () => void;
  onOpenAuth: () => void;
  onLogout: () => void;
  currentView: 'landing' | 'overview' | 'explorer' | 'keys' | 'stream' | 'playground';
  onNavigate: (view: 'landing' | 'overview' | 'explorer' | 'keys' | 'stream' | 'playground') => void;
}

export const Navbar: React.FC<NavbarProps> = ({
  user,
  workspaces,
  activeWorkspace,
  onSelectWorkspace,
  onCreateWorkspaceClick,
  onOpenAuth,
  onLogout,
  currentView,
  onNavigate,
}) => {
  const [pingData, setPingData] = useState<PingResponse | null>(null);
  const [latency, setLatency] = useState<number | null>(null);

  useEffect(() => {
    let mounted = true;
    const checkPing = async () => {
      const start = performance.now();
      try {
        const resp = await api.ping();
        const took = Math.round(performance.now() - start);
        if (mounted) {
          setPingData(resp);
          setLatency(took);
        }
      } catch {
        if (mounted) {
          setPingData(null);
          setLatency(null);
        }
      }
    };

    checkPing();
    const interval = setInterval(checkPing, 15000);
    return () => {
      mounted = false;
      clearInterval(interval);
    };
  }, []);

  return (
    <header className="sticky top-0 z-40 w-full border-b border-[#27272a] bg-[#0d0d10]">
      <div className="max-w-6xl mx-auto px-4 sm:px-6 h-14 flex items-center justify-between gap-4 font-sans">
        {/* Brand */}
        <button
          onClick={() => onNavigate(user ? 'overview' : 'landing')}
          className="flex items-center gap-2.5 text-left btn-pressable"
        >
          <div className="w-6 h-6 rounded-md bg-zinc-800 border border-zinc-700 flex items-center justify-center text-zinc-100 font-mono text-xs font-bold">
            S
          </div>
          <span className="font-semibold text-sm tracking-tight text-white">
            Strata
          </span>
          <span className="text-[11px] font-mono text-zinc-500">Cognitive Memory</span>
        </button>

        {/* Right Action Buttons */}
        <div className="flex items-center gap-3">
          <div className="hidden sm:flex items-center gap-2 px-2.5 py-1 rounded-md bg-zinc-900 border border-zinc-800 text-xs font-mono">
            <span
              className={`w-1.5 h-1.5 rounded-full ${
                pingData ? 'bg-emerald-500' : 'bg-rose-500'
              }`}
            />
            <span className="text-zinc-400">
              {pingData ? (
                <>
                  <span>Online</span>
                  {latency !== null && <span className="text-zinc-500 ml-1">({latency}ms)</span>}
                </>
              ) : (
                'Connecting'
              )}
            </span>
          </div>

          <button
            onClick={() => {
              navigator.clipboard.writeText('strata login');
              toast.success('Copied CLI Command', 'Run "strata login" in terminal.');
            }}
            className="hidden sm:flex items-center gap-1.5 px-2.5 py-1 rounded-md bg-zinc-900 border border-zinc-800 hover:border-zinc-700 text-zinc-300 hover:text-white font-mono text-xs btn-pressable"
            title="Copy command to connect CLI"
          >
            <Terminal className="w-3.5 h-3.5 text-zinc-400" />
            <span>strata login</span>
          </button>

          {user ? (
            <button
              onClick={() => onNavigate('overview')}
              className="px-3.5 py-1.5 rounded-md bg-white text-black font-semibold text-xs btn-pressable hover:bg-zinc-200"
            >
              Open Console
            </button>
          ) : (
            <button
              onClick={onOpenAuth}
              className="px-3.5 py-1.5 rounded-md bg-white text-black font-semibold text-xs btn-pressable hover:bg-zinc-200"
            >
              Sign In
            </button>
          )}
        </div>
      </div>
    </header>
  );
};
