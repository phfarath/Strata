import React, { useState } from 'react';
import { X, Lock, Mail, User as UserIcon, ArrowRight } from 'lucide-react';
import { api } from '../api';
import { toast } from './Toast';
import { User, Workspace } from '../types';

interface AuthModalProps {
  isOpen: boolean;
  onClose: () => void;
  onSuccess: (user: User, workspaces: Workspace[]) => void;
}

export const AuthModal: React.FC<AuthModalProps> = ({ isOpen, onClose, onSuccess }) => {
  const [mode, setMode] = useState<'login' | 'signup'>('login');
  const [email, setEmail] = useState('');
  const [password, setPassword] = useState('');
  const [fullName, setFullName] = useState('');
  const [workspaceName, setWorkspaceName] = useState('');
  const [loading, setLoading] = useState(false);

  if (!isOpen) return null;

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    setLoading(true);

    try {
      if (mode === 'signup') {
        if (!email.includes('@') || password.length < 8) {
          toast.error('Validation error', 'Password must have at least 8 characters.');
          setLoading(false);
          return;
        }
        const resp = await api.signup(email, password, fullName || email.split('@')[0], workspaceName);
        toast.success('Account created', `Welcome, ${resp.user.full_name}`);
        onSuccess(resp.user, resp.workspaces);
      } else {
        const resp = await api.login(email, password);
        toast.success('Signed in', resp.user.email);
        onSuccess(resp.user, resp.workspaces);
      }
      onClose();
    } catch (err: any) {
      toast.error('Authentication failed', err.message || 'Check your credentials.');
    } finally {
      setLoading(false);
    }
  };

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center p-4 bg-black/80 backdrop-blur-sm animate-in fade-in duration-150">
      <div 
        className="relative w-full max-w-sm p-6 rounded-xl border border-[#23262f] bg-[#15171d] shadow-2xl text-zinc-100 animate-in zoom-in-95 duration-150 font-sans"
        style={{ transformOrigin: 'center' }}
      >
        <button
          onClick={onClose}
          className="absolute top-4 right-4 p-1.5 rounded-md text-zinc-500 hover:text-zinc-200 hover:bg-[#23262f] btn-pressable"
        >
          <X className="w-4 h-4" />
        </button>

        <div className="mb-4">
          <h2 className="text-lg font-bold tracking-tight text-white">
            {mode === 'login' ? 'Sign In to Console' : 'Create Developer Account'}
          </h2>
          <p className="text-xs text-zinc-400 mt-0.5">
            {mode === 'login'
              ? 'Enter your credentials to access your memory workspace.'
              : 'Sign up with unified credentials for CLI and Web Console.'}
          </p>
        </div>

        {/* Tab switch */}
        <div className="flex rounded-lg bg-[#0f1115] border border-[#23262f] p-1 mb-4">
          <button
            type="button"
            onClick={() => setMode('login')}
            className={`flex-1 py-1.5 text-xs font-medium rounded-md transition-colors btn-pressable ${
              mode === 'login'
                ? 'bg-[#23262f] text-amber-200 font-bold'
                : 'text-zinc-400 hover:text-zinc-200'
            }`}
          >
            Sign In
          </button>
          <button
            type="button"
            onClick={() => setMode('signup')}
            className={`flex-1 py-1.5 text-xs font-medium rounded-md transition-colors btn-pressable ${
              mode === 'signup'
                ? 'bg-[#23262f] text-amber-200 font-bold'
                : 'text-zinc-400 hover:text-zinc-200'
            }`}
          >
            Sign Up
          </button>
        </div>

        <form onSubmit={handleSubmit} className="space-y-3">
          {mode === 'signup' && (
            <div>
              <label className="block text-xs font-medium text-zinc-400 mb-1">
                Full Name
              </label>
              <div className="relative">
                <UserIcon className="absolute left-3 top-2.5 w-3.5 h-3.5 text-zinc-500" />
                <input
                  type="text"
                  required
                  placeholder="Pedro Farath"
                  value={fullName}
                  onChange={(e) => setFullName(e.target.value)}
                  className="w-full pl-9 pr-3 py-2 rounded-lg bg-[#0f1115] border border-[#23262f] text-zinc-100 placeholder:text-zinc-600 focus:outline-none focus:border-amber-300/40 text-xs"
                />
              </div>
            </div>
          )}

          <div>
            <label className="block text-xs font-medium text-zinc-400 mb-1">
              Email Address
            </label>
            <div className="relative">
              <Mail className="absolute left-3 top-2.5 w-3.5 h-3.5 text-zinc-500" />
              <input
                type="email"
                required
                placeholder="developer@strata.pedrofarath.me"
                value={email}
                onChange={(e) => setEmail(e.target.value)}
                className="w-full pl-9 pr-3 py-2 rounded-lg bg-[#0f1115] border border-[#23262f] text-zinc-100 placeholder:text-zinc-600 focus:outline-none focus:border-amber-300/40 text-xs font-mono"
              />
            </div>
          </div>

          <div>
            <label className="block text-xs font-medium text-zinc-400 mb-1">
              Password
            </label>
            <div className="relative">
              <Lock className="absolute left-3 top-2.5 w-3.5 h-3.5 text-zinc-500" />
              <input
                type="password"
                required
                placeholder="••••••••••••"
                value={password}
                onChange={(e) => setPassword(e.target.value)}
                className="w-full pl-9 pr-3 py-2 rounded-lg bg-[#0f1115] border border-[#23262f] text-zinc-100 placeholder:text-zinc-600 focus:outline-none focus:border-amber-300/40 text-xs font-mono"
              />
            </div>
          </div>

          {mode === 'signup' && (
            <div>
              <label className="block text-xs font-medium text-zinc-400 mb-1">
                Workspace Name (Optional)
              </label>
              <input
                type="text"
                placeholder="My Core Team"
                value={workspaceName}
                onChange={(e) => setWorkspaceName(e.target.value)}
                className="w-full px-3 py-2 rounded-lg bg-[#0f1115] border border-[#23262f] text-zinc-100 placeholder:text-zinc-600 focus:outline-none focus:border-amber-300/40 text-xs"
              />
            </div>
          )}

          <button
            type="submit"
            disabled={loading}
            className="w-full py-2.5 px-4 rounded-lg bg-amber-300 text-zinc-950 font-bold flex items-center justify-center gap-1.5 hover:bg-amber-200 btn-pressable sweep-hover mt-2 text-xs disabled:opacity-50 shadow-sm"
          >
            <span>{loading ? 'Authenticating...' : mode === 'login' ? 'Sign In' : 'Create Account'}</span>
            {!loading && <ArrowRight className="w-3.5 h-3.5" />}
          </button>
        </form>

        <div className="mt-4 pt-3 border-t border-[#23262f] text-center text-[11px] text-zinc-500 font-mono">
          Unified with <code className="text-amber-200">strata login</code>
        </div>
      </div>
    </div>
  );
};
