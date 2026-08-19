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
    <header className="sticky top-0 z-40 w-full border-b border-[#23262f] bg-[#0f1115]">
      <div className="max-w-6xl mx-auto px-4 sm:px-6 h-14 flex items-center justify-between gap-4 font-sans">
        {/* Brand */}
        <button
          onClick={() => onNavigate(user ? 'overview' : 'landing')}
          className="flex items-center gap-2.5 text-left btn-pressable"
        >
          <div className="w-6 h-6 rounded-md bg-[#1c1f26] border border-[#343846] flex items-center justify-center text-amber-400 font-mono text-xs font-bold shadow-inner">
            S
          </div>
          <span className="font-semibold text-sm tracking-tight text-white font-sans">
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
            className="hidden sm:flex items-center gap-1.5 px-2.5 py-1.5 rounded-md bg-[#15171d] border border-[#23262f] hover:border-[#343846] text-zinc-300 hover:text-white font-mono text-xs btn-pressable sweep-hover"
            title="Copy command to connect CLI"
          >
            <Terminal className="w-3.5 h-3.5 text-amber-500" />
            <span>strata login</span>
          </button>

          {user ? (
            <button
              onClick={() => onNavigate('overview')}
              className="px-3.5 py-1.5 rounded-md bg-amber-500 text-black font-bold text-xs btn-pressable sweep-hover hover:bg-amber-400"
            >
              Open Console
            </button>
          ) : (
            <button
              onClick={onOpenAuth}
              className="px-3.5 py-1.5 rounded-md bg-amber-500 text-black font-bold text-xs btn-pressable sweep-hover hover:bg-amber-400"
            >
              Sign In
            </button>
          )}
        </div>
      </div>
    </header>
  );
};
