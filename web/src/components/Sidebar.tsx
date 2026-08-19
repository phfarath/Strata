import React, { useState, useEffect } from 'react';
import {
  Brain,
  LayoutDashboard,
  Key,
  Radio,
  PlayCircle,
  ChevronDown,
  Plus,
  LogOut,
  Terminal,
  Activity,
  Server,
  User as UserIcon,
  Check,
  Sparkles,
} from 'lucide-react';
import { User, Workspace, PingResponse } from '../types';
import { api } from '../api';
import { toast } from './Toast';

interface SidebarProps {
  user: User;
  workspaces: Workspace[];
  activeWorkspace: Workspace;
  onSelectWorkspace: (ws: Workspace) => void;
  onCreateWorkspaceClick: () => void;
  onLogout: () => void;
  currentTab: 'overview' | 'explorer' | 'keys' | 'stream' | 'playground';
  onTabChange: (tab: 'overview' | 'explorer' | 'keys' | 'stream' | 'playground') => void;
}

export const Sidebar: React.FC<SidebarProps> = ({
  user,
  workspaces,
  activeWorkspace,
  onSelectWorkspace,
  onCreateWorkspaceClick,
  onLogout,
  currentTab,
  onTabChange,
}) => {
  const [wsDropdownOpen, setWsDropdownOpen] = useState(false);
  const [pingData, setPingData] = useState<PingResponse | null>(null);
  const [latency, setLatency] = useState<number | null>(null);
  const [copiedCli, setCopiedCli] = useState(false);

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

  const handleCopyCli = () => {
    navigator.clipboard.writeText('strata login');
    setCopiedCli(true);
    toast.success('Copied CLI Command', 'Run "strata login" in your terminal.');
    setTimeout(() => setCopiedCli(false), 2000);
  };

  interface NavItem {
    id: 'overview' | 'explorer' | 'keys' | 'stream' | 'playground';
    label: string;
    icon: React.ElementType;
    badge: string | null;
    isLive?: boolean;
  }

  const navItems: NavItem[] = [
    { id: 'overview', label: 'Overview', icon: LayoutDashboard, badge: null },
    { id: 'explorer', label: 'Memory Explorer', icon: Brain, badge: '1.8k' },
    { id: 'keys', label: 'API Keys & Agents', icon: Key, badge: null },
    { id: 'stream', label: 'CDC Realtime Stream', icon: Radio, badge: 'Live', isLive: true },
    { id: 'playground', label: 'Agent Simulator', icon: PlayCircle, badge: null },
  ];

  return (
    <aside className="w-64 h-screen flex flex-col bg-[#07090f] border-r border-border/80 shrink-0 select-none">
      {/* 1. Header: Brand Logo & Title */}
      <div className="p-4 border-b border-border/80 flex items-center justify-between">
        <div className="flex items-center gap-2.5">
          <div className="w-8 h-8 rounded-xl bg-primary/10 border border-primary/30 flex items-center justify-center text-primary shadow-glow">
            <Brain className="w-4 h-4" />
          </div>
          <div>
            <div className="flex items-center gap-1.5 font-extrabold text-sm tracking-tight text-white leading-none">
              <span>STRATA</span>
              <span className="text-[9px] px-1.5 py-0.5 rounded-full bg-primary/10 text-primary border border-primary/20 font-mono">
                CLOUD
              </span>
            </div>
            <span className="text-[10px] text-slate-400 font-mono leading-none">Cognitive Runtime</span>
          </div>
        </div>
      </div>

      {/* 2. Workspace Selector */}
      <div className="p-3 border-b border-border/60">
        <div className="relative">
          <button
            onClick={() => setWsDropdownOpen(!wsDropdownOpen)}
            className="w-full flex items-center justify-between p-2 rounded-xl bg-card border border-border hover:border-border-light text-xs font-semibold text-slate-200 btn-pressable"
          >
            <div className="flex items-center gap-2 truncate">
              <span className="w-2 h-2 rounded-full bg-primary shrink-0" />
              <span className="truncate">{activeWorkspace.name}</span>
            </div>
            <ChevronDown className="w-3.5 h-3.5 text-slate-400 shrink-0 ml-1" />
          </button>

          {wsDropdownOpen && (
            <div className="absolute left-0 right-0 top-full mt-1.5 rounded-2xl glass-panel-glow border border-border p-2 shadow-2xl z-50 animate-in fade-in zoom-in-95 bg-[#0a0f1d]">
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
                    activeWorkspace.id === ws.id
                      ? 'bg-primary/20 text-primary font-bold'
                      : 'text-slate-300 hover:bg-card-hover'
                  }`}
                >
                  <span className="truncate">{ws.name}</span>
                  {activeWorkspace.id === ws.id && <span className="text-[10px]">✓</span>}
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
      </div>

      {/* 3. Navigation List */}
      <nav className="flex-1 p-3 space-y-1 overflow-y-auto">
        <div className="text-[10px] font-mono uppercase tracking-wider text-slate-500 px-3 py-1.5 font-bold">
          Navigation
        </div>

        {navItems.map((item) => {
          const Icon = item.icon;
          const isActive = currentTab === item.id;
          return (
            <button
              key={item.id}
              onClick={() => onTabChange(item.id)}
              className={`w-full flex items-center justify-between px-3 py-2.5 rounded-xl text-xs font-semibold btn-pressable transition-all ${
                isActive
                  ? 'bg-primary text-black shadow-glow font-bold'
                  : 'text-slate-400 hover:text-slate-200 hover:bg-card/70'
              }`}
            >
              <div className="flex items-center gap-2.5">
                <Icon className={`w-4 h-4 ${isActive ? 'text-black' : 'text-slate-400'}`} />
                <span>{item.label}</span>
              </div>

              {item.badge && (
                <span
                  className={`text-[10px] px-1.5 py-0.5 rounded-full font-mono font-medium ${
                    isActive
                      ? 'bg-black/20 text-black'
                      : item.isLive
                      ? 'bg-amber-500/20 text-amber-300 border border-amber-500/30 animate-pulse'
                      : 'bg-card border border-border text-slate-400'
                  }`}
                >
                  {item.badge}
                </span>
              )}
            </button>
          );
        })}
      </nav>

      {/* 4. Footer Section: Cloud Telemetry, CLI Copy & Profile */}
      <div className="p-3 border-t border-border/80 space-y-2 bg-[#06080e]">
        {/* Live Cluster Ping Badge */}
        <div className="p-2.5 rounded-xl bg-card border border-border/80 flex items-center justify-between text-[11px] font-mono">
          <div className="flex items-center gap-2">
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
              {pingData ? 'Supabase Postgres' : 'Connecting...'}
            </span>
          </div>
          {latency !== null && <span className="text-emerald-400 font-bold">{latency}ms</span>}
        </div>

        {/* Quick CLI Login */}
        <button
          onClick={handleCopyCli}
          className="w-full flex items-center justify-between p-2 rounded-xl bg-[#0a0f1d] border border-border hover:border-primary/40 text-slate-300 hover:text-white font-mono text-[11px] btn-pressable"
          title="Click to copy CLI login command"
        >
          <div className="flex items-center gap-1.5">
            <Terminal className="w-3.5 h-3.5 text-primary" />
            <span>strata login</span>
          </div>
          {copiedCli ? <Check className="w-3.5 h-3.5 text-emerald-400" /> : <span className="text-slate-500 text-[10px]">copy</span>}
        </button>

        {/* User Card & Sign Out */}
        <div className="pt-2 border-t border-border/60 flex items-center justify-between gap-2">
          <div className="flex items-center gap-2 truncate">
            <div className="w-7 h-7 rounded-lg bg-primary/20 border border-primary/30 flex items-center justify-center text-primary text-xs font-bold shrink-0">
              {user.full_name ? user.full_name.charAt(0).toUpperCase() : 'U'}
            </div>
            <div className="truncate">
              <div className="text-xs font-semibold text-white truncate leading-none">
                {user.full_name || 'Developer'}
              </div>
              <div className="text-[10px] text-slate-400 truncate leading-none mt-1">
                {user.email}
              </div>
            </div>
          </div>

          <button
            onClick={onLogout}
            className="p-1.5 rounded-lg text-slate-400 hover:text-rose-400 hover:bg-rose-950/30 btn-pressable transition-colors shrink-0"
            title="Sign Out"
          >
            <LogOut className="w-4 h-4" />
          </button>
        </div>
      </div>
    </aside>
  );
};
