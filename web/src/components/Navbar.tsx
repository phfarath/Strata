import React, { useState, useEffect } from 'react';
import { Sparkles, Terminal, Activity, ChevronDown, Plus, LogOut, Key, User as UserIcon, LayoutDashboard, Brain, Radio, PlayCircle } from 'lucide-react';
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
  const [wsDropdownOpen, setWsDropdownOpen] = useState(false);
  const [userDropdownOpen, setUserDropdownOpen] = useState(false);

  // Periodic health check
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
    <header className="sticky top-0 z-40 w-full border-b border-border/80 glass-panel backdrop-blur-xl">
      <div className="max-w-7xl mx-auto px-4 sm:px-6 h-16 flex items-center justify-between gap-4">
        {/* Left: Brand */}
        <div className="flex items-center gap-6">
          <button
            onClick={() => onNavigate(user ? 'overview' : 'landing')}
            className="flex items-center gap-2.5 text-left group btn-pressable"
          >
            <div className="w-9 h-9 rounded-xl bg-gradient-to-tr from-primary/30 to-sky-400/10 border border-primary/40 flex items-center justify-center text-primary shadow-glow group-hover:scale-105 transition-transform">
              <Brain className="w-5 h-5" />
            </div>
            <div>
              <div className="flex items-center gap-1.5 font-extrabold text-base tracking-tight text-white leading-none">
                <span>STRATA</span>
                <span className="text-[10px] px-1.5 py-0.5 rounded-full bg-primary/10 text-primary border border-primary/20 font-mono font-medium">
                  MEMORY
                </span>
              </div>
              <span className="text-[11px] text-slate-400 leading-none">Cognitive Layer for Code Agents</span>
            </div>
          </button>

          {/* Navigation Items (when logged in or on dashboard) */}
          {user && (
            <nav className="hidden md:flex items-center gap-1 bg-card/60 border border-border/60 p-1 rounded-xl">
              <button
                onClick={() => onNavigate('overview')}
                className={`px-3 py-1.5 rounded-lg text-xs font-semibold flex items-center gap-1.5 btn-pressable transition-all ${
                  currentView === 'overview'
                    ? 'bg-primary text-black shadow-sm'
                    : 'text-slate-400 hover:text-slate-200'
                }`}
              >
                <LayoutDashboard className="w-3.5 h-3.5" />
                <span>Overview</span>
              </button>

              <button
                onClick={() => onNavigate('explorer')}
                className={`px-3 py-1.5 rounded-lg text-xs font-semibold flex items-center gap-1.5 btn-pressable transition-all ${
                  currentView === 'explorer'
                    ? 'bg-primary text-black shadow-sm'
                    : 'text-slate-400 hover:text-slate-200'
                }`}
              >
                <Brain className="w-3.5 h-3.5" />
                <span>Memory Explorer</span>
              </button>

              <button
                onClick={() => onNavigate('keys')}
                className={`px-3 py-1.5 rounded-lg text-xs font-semibold flex items-center gap-1.5 btn-pressable transition-all ${
                  currentView === 'keys'
                    ? 'bg-primary text-black shadow-sm'
                    : 'text-slate-400 hover:text-slate-200'
                }`}
              >
                <Key className="w-3.5 h-3.5" />
                <span>API Keys</span>
              </button>

              <button
                onClick={() => onNavigate('stream')}
                className={`px-3 py-1.5 rounded-lg text-xs font-semibold flex items-center gap-1.5 btn-pressable transition-all ${
                  currentView === 'stream'
                    ? 'bg-primary text-black shadow-sm'
                    : 'text-slate-400 hover:text-slate-200'
                }`}
              >
                <Radio className="w-3.5 h-3.5 text-amber-400" />
                <span>CDC Stream</span>
              </button>

              <button
                onClick={() => onNavigate('playground')}
                className={`px-3 py-1.5 rounded-lg text-xs font-semibold flex items-center gap-1.5 btn-pressable transition-all ${
                  currentView === 'playground'
                    ? 'bg-primary text-black shadow-sm'
                    : 'text-slate-400 hover:text-slate-200'
                }`}
              >
                <PlayCircle className="w-3.5 h-3.5 text-purple-400" />
                <span>Agent Simulator</span>
              </button>
            </nav>
          )}
        </div>

        {/* Right: Cloud Status, Workspace, Profile */}
        <div className="flex items-center gap-3">
          {/* Live Ping Status Indicator */}
          <div className="hidden lg:flex items-center gap-2 px-3 py-1.5 rounded-full bg-card border border-border text-xs font-mono">
            <span className="relative flex h-2 w-2">
              <span
                className={`animate-ping absolute inline-flex h-full w-full rounded-full opacity-75 ${
                  pingData ? 'bg-emerald-400' : 'bg-rose-400'
                }`}
              />
              <span
                className={`relative inline-flex rounded-full h-2 w-2 ${
                  pingData ? 'bg-emerald-500' : 'bg-rose-500'
                }`}
              />
            </span>
            <span className="text-slate-300">
              {pingData ? (
                <>
                  <span className="text-emerald-400 font-semibold">Online</span>
                  {latency !== null && <span className="text-slate-500 ml-1.5">{latency}ms</span>}
                </>
              ) : (
                <span className="text-rose-400">Reconnecting</span>
              )}
            </span>
          </div>

          {/* CLI quick login copy */}
          <button
            onClick={() => {
              navigator.clipboard.writeText('strata login');
              toast.success('Copied CLI Command', 'Run "strata login" in your terminal.');
            }}
            className="hidden sm:flex items-center gap-1.5 px-3 py-1.5 rounded-xl bg-card border border-border hover:border-primary/40 text-slate-300 hover:text-white font-mono text-xs btn-pressable"
            title="Copy command to connect your CLI"
          >
            <Terminal className="w-3.5 h-3.5 text-primary" />
            <span>strata login</span>
          </button>

          {/* User Logged In Controls */}
          {user ? (
            <>
              {/* Workspace Selector */}
              <div className="relative">
                <button
                  onClick={() => setWsDropdownOpen(!wsDropdownOpen)}
                  className="flex items-center gap-2 px-3 py-1.5 rounded-xl bg-card border border-border hover:border-border-light text-xs font-semibold text-slate-200 btn-pressable"
                >
                  <span className="w-2 h-2 rounded-full bg-primary" />
                  <span className="max-w-[120px] truncate">{activeWorkspace?.name || 'Workspace'}</span>
                  <ChevronDown className="w-3.5 h-3.5 text-slate-400" />
                </button>

                {wsDropdownOpen && (
                  <div className="absolute right-0 mt-2 w-56 rounded-2xl glass-panel-glow border border-border p-2 shadow-2xl z-50 animate-in fade-in zoom-in-95">
                    <div className="text-[10px] uppercase font-bold text-slate-400 px-3 py-1 tracking-wider">
                      Workspaces
                    </div>
                    {workspaces.map((ws) => (
                      <button
                        key={ws.id}
                        onClick={() => {
                          onSelectWorkspace(ws);
                          setWsDropdownOpen(false);
                        }}
                        className={`w-full flex items-center justify-between px-3 py-2 rounded-xl text-xs text-left transition-colors ${
                          activeWorkspace?.id === ws.id
                            ? 'bg-primary/20 text-primary font-bold'
                            : 'text-slate-300 hover:bg-card-hover'
                        }`}
                      >
                        <span className="truncate">{ws.name}</span>
                        {activeWorkspace?.id === ws.id && <span className="text-[10px]">✓</span>}
                      </button>
                    ))}

                    <div className="border-t border-border/80 mt-1 pt-1">
                      <button
                        onClick={() => {
                          setWsDropdownOpen(false);
                          onCreateWorkspaceClick();
                        }}
                        className="w-full flex items-center gap-2 px-3 py-2 rounded-xl text-xs text-primary hover:bg-primary/10 transition-colors font-semibold"
                      >
                        <Plus className="w-3.5 h-3.5" />
                        <span>Create Workspace</span>
                      </button>
                    </div>
                  </div>
                )}
              </div>

              {/* User Dropdown */}
              <div className="relative">
                <button
                  onClick={() => setUserDropdownOpen(!userDropdownOpen)}
                  className="w-9 h-9 rounded-xl bg-card border border-border flex items-center justify-center text-slate-300 hover:text-white hover:border-primary/40 btn-pressable"
                >
                  <UserIcon className="w-4 h-4" />
                </button>

                {userDropdownOpen && (
                  <div className="absolute right-0 mt-2 w-56 rounded-2xl glass-panel-glow border border-border p-2 shadow-2xl z-50 animate-in fade-in zoom-in-95">
                    <div className="px-3 py-2 border-b border-border/80">
                      <div className="font-semibold text-xs text-white truncate">{user.full_name}</div>
                      <div className="text-[11px] text-slate-400 truncate">{user.email}</div>
                    </div>

                    <button
                      onClick={() => {
                        setUserDropdownOpen(false);
                        onLogout();
                      }}
                      className="w-full mt-1 flex items-center gap-2 px-3 py-2 rounded-xl text-xs text-rose-400 hover:bg-rose-950/30 transition-colors"
                    >
                      <LogOut className="w-3.5 h-3.5" />
                      <span>Sign Out</span>
                    </button>
                  </div>
                )}
              </div>
            </>
          ) : (
            <button
              onClick={onOpenAuth}
              className="px-4 py-2 rounded-xl bg-primary hover:bg-primary-hover text-black font-bold text-xs flex items-center gap-1.5 shadow-glow btn-pressable"
            >
              <Sparkles className="w-3.5 h-3.5" />
              <span>Console Sign In</span>
            </button>
          )}
        </div>
      </div>
    </header>
  );
};
