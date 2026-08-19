import React from 'react';
import { Terminal } from 'lucide-react';
import { User, Workspace } from '../types';
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
  onOpenAuth,
  onNavigate,
}) => {
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
        </button>

        {/* Right Action Buttons */}
        <div className="flex items-center gap-3">
          <button
            onClick={() => {
              navigator.clipboard.writeText('strata login');
              toast.success('Copied CLI Command', 'Run "strata login" in terminal.');
            }}
            className="hidden sm:flex items-center gap-1.5 px-2.5 py-1.5 rounded-md bg-zinc-900 border border-zinc-800 hover:border-zinc-700 text-zinc-300 hover:text-white font-mono text-xs btn-pressable"
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
