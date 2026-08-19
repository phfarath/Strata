import React, { useState } from 'react';
import {
  Layers,
  LayoutDashboard,
  Key,
  Terminal,
  Activity,
  ChevronDown,
  Plus,
  LogOut,
  Sliders,
  Check,
} from 'lucide-react';
import { User, Workspace } from '../types';
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
  const [copiedCli, setCopiedCli] = useState(false);

  const handleCopyCli = () => {
    navigator.clipboard.writeText('strata login');
    setCopiedCli(true);
    toast.success('Copied CLI command', 'Run "strata login" in terminal.');
    setTimeout(() => setCopiedCli(false), 2000);
  };

  interface NavItem {
    id: 'overview' | 'explorer' | 'keys' | 'stream' | 'playground';
    label: string;
    icon: React.ElementType;
  }

  const navItems: NavItem[] = [
    { id: 'overview', label: 'Overview', icon: LayoutDashboard },
    { id: 'explorer', label: 'Memory Explorer', icon: Layers },
    { id: 'keys', label: 'API Keys & Agents', icon: Key },
    { id: 'stream', label: 'CDC Delta Stream', icon: Activity },
    { id: 'playground', label: 'Simulator', icon: Sliders },
  ];

  return (
    <aside className="w-60 h-screen flex flex-col bg-[#0d0d10] border-r border-[#27272a] shrink-0 select-none text-zinc-300">
      {/* 1. Header */}
      <div className="h-14 px-4 border-b border-[#27272a] flex items-center justify-between">
        <div className="flex items-center gap-2.5">
          <div className="w-6 h-6 rounded-md bg-zinc-800 border border-zinc-700 flex items-center justify-center text-zinc-100 font-mono text-xs font-bold">
            S
          </div>
          <span className="font-semibold text-sm tracking-tight text-zinc-100">
            Strata
          </span>
        </div>
      </div>

      {/* 2. Workspace Selector */}
      <div className="p-3 border-b border-[#27272a]">
        <div className="relative">
          <button
            onClick={() => setWsDropdownOpen(!wsDropdownOpen)}
            className="w-full flex items-center justify-between px-3 py-2 rounded-lg bg-zinc-900 border border-zinc-800 hover:border-zinc-700 text-xs font-medium text-zinc-200 btn-pressable"
          >
            <span className="truncate">{activeWorkspace.name}</span>
            <ChevronDown className="w-3.5 h-3.5 text-zinc-500 shrink-0 ml-1" />
          </button>

          {wsDropdownOpen && (
            <div className="absolute left-0 right-0 top-full mt-1 rounded-lg bg-zinc-900 border border-zinc-800 p-1.5 shadow-xl z-50 animate-in fade-in zoom-in-95">
              <div className="text-[10px] font-medium text-zinc-500 px-2 py-1">
                Workspaces
              </div>
              {workspaces.map((ws) => (
                <button
                  key={ws.id}
                  onClick={() => {
                    onSelectWorkspace(ws);
                    setWsDropdownOpen(false);
                  }}
                  className={`w-full flex items-center justify-between px-2.5 py-1.5 rounded-md text-xs text-left transition-colors ${
                    activeWorkspace.id === ws.id
                      ? 'bg-zinc-800 text-zinc-100 font-medium'
                      : 'text-zinc-400 hover:bg-zinc-800/60 hover:text-zinc-200'
                  }`}
                >
                  <span className="truncate">{ws.name}</span>
                  {activeWorkspace.id === ws.id && <Check className="w-3 h-3 text-zinc-400" />}
                </button>
              ))}

              <div className="border-t border-zinc-800 mt-1 pt-1">
                <button
                  onClick={() => {
                    setWsDropdownOpen(false);
                    onCreateWorkspaceClick();
                  }}
                  className="w-full flex items-center gap-1.5 px-2.5 py-1.5 rounded-md text-xs text-zinc-300 hover:bg-zinc-800 transition-colors"
                >
                  <Plus className="w-3 h-3" />
                  <span>New Workspace</span>
                </button>
              </div>
            </div>
          )}
        </div>
      </div>

      {/* 3. Navigation List */}
      <nav className="flex-1 p-3 space-y-0.5 overflow-y-auto">
        {navItems.map((item) => {
          const Icon = item.icon;
          const isActive = currentTab === item.id;
          return (
            <button
              key={item.id}
              onClick={() => onTabChange(item.id)}
              className={`w-full flex items-center justify-between px-3 py-2 rounded-lg text-xs font-medium btn-pressable transition-colors ${
                isActive
                  ? 'bg-zinc-800 text-white'
                  : 'text-zinc-400 hover:text-zinc-200 hover:bg-zinc-900'
              }`}
            >
              <div className="flex items-center gap-2.5">
                <Icon className={`w-4 h-4 ${isActive ? 'text-white' : 'text-zinc-400'}`} />
                <span>{item.label}</span>
              </div>
            </button>
          );
        })}
      </nav>

      {/* 4. Footer */}
      <div className="p-3 border-t border-[#27272a] space-y-2 bg-[#09090c]">
        {/* CLI Command */}
        <button
          onClick={handleCopyCli}
          className="w-full flex items-center justify-between px-2.5 py-1.5 rounded-md bg-zinc-900 border border-zinc-800/80 hover:border-zinc-700 text-zinc-300 hover:text-white font-mono text-[11px] btn-pressable"
          title="Copy CLI login command"
        >
          <div className="flex items-center gap-2">
            <Terminal className="w-3 h-3 text-zinc-400" />
            <span>strata login</span>
          </div>
          {copiedCli ? <Check className="w-3 h-3 text-emerald-400" /> : <span className="text-zinc-600 text-[10px]">copy</span>}
        </button>

        {/* User Card */}
        <div className="pt-2 border-t border-zinc-800 flex items-center justify-between gap-2">
          <div className="truncate text-left">
            <div className="text-xs font-medium text-zinc-200 truncate">
              {user.full_name || 'Developer'}
            </div>
            <div className="text-[11px] text-zinc-500 truncate font-mono">
              {user.email}
            </div>
          </div>

          <button
            onClick={onLogout}
            className="p-1.5 rounded-md text-zinc-500 hover:text-zinc-200 hover:bg-zinc-800 btn-pressable transition-colors"
            title="Sign Out"
          >
            <LogOut className="w-3.5 h-3.5" />
          </button>
        </div>
      </div>
    </aside>
  );
};
