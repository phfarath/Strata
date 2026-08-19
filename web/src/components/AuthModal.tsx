import React, { useState } from 'react';
import { X, Sparkles, Lock, Mail, User as UserIcon, ArrowRight, ShieldCheck } from 'lucide-react';
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
          toast.error('Invalid fields', 'Password must have at least 8 characters.');
          setLoading(false);
          return;
        }
        const resp = await api.signup(email, password, fullName || email.split('@')[0], workspaceName);
        toast.success('Account created!', `Welcome to Strata, ${resp.user.full_name}`);
        onSuccess(resp.user, resp.workspaces);
      } else {
        const resp = await api.login(email, password);
        toast.success('Welcome back!', `Logged in as ${resp.user.email}`);
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
    <div className="fixed inset-0 z-50 flex items-center justify-center p-4 bg-black/70 backdrop-blur-md animate-in fade-in duration-200">
      <div 
        className="relative w-full max-w-md p-8 rounded-2xl glass-panel-glow border border-border bg-[#0a0f1d] shadow-2xl text-slate-100 animate-in zoom-in-95 duration-200"
        style={{ transformOrigin: 'center' }}
      >
        <button
          onClick={onClose}
          className="absolute top-5 right-5 p-1.5 rounded-lg text-slate-400 hover:text-white hover:bg-slate-800/60 btn-pressable"
        >
          <X className="w-5 h-5" />
        </button>

        <div className="flex items-center gap-2 text-primary font-bold text-lg mb-2">
          <div className="p-1.5 rounded-lg bg-primary/10 border border-primary/20">
            <Sparkles className="w-4 h-4 text-primary" />
          </div>
          <span>Strata Cloud Console</span>
        </div>

        <h2 className="text-2xl font-extrabold tracking-tight text-white mb-1">
          {mode === 'login' ? 'Sign in to your account' : 'Create developer account'}
        </h2>
        <p className="text-sm text-slate-400 mb-6">
          {mode === 'login'
            ? 'Access your coding agent memories, decay graphs & API keys.'
            : 'Start syncing episodic and semantic memories across IDEs.'}
        </p>

        {/* Tab switch */}
        <div className="flex rounded-xl bg-card border border-border p-1 mb-6">
          <button
            type="button"
            onClick={() => setMode('login')}
            className={`flex-1 py-2 text-sm font-semibold rounded-lg transition-all ${
              mode === 'login'
                ? 'bg-primary text-black shadow-md'
                : 'text-slate-400 hover:text-slate-200'
            }`}
          >
            Sign In
          </button>
          <button
            type="button"
            onClick={() => setMode('signup')}
            className={`flex-1 py-2 text-sm font-semibold rounded-lg transition-all ${
              mode === 'signup'
                ? 'bg-primary text-black shadow-md'
                : 'text-slate-400 hover:text-slate-200'
            }`}
          >
            Sign Up
          </button>
        </div>

        <form onSubmit={handleSubmit} className="space-y-4">
          {mode === 'signup' && (
            <div>
              <label className="block text-xs font-semibold uppercase tracking-wider text-slate-400 mb-1">
                Full Name
              </label>
              <div className="relative">
                <UserIcon className="absolute left-3.5 top-3 w-4 h-4 text-slate-500" />
                <input
                  type="text"
                  required
                  placeholder="Pedro Farath"
                  value={fullName}
                  onChange={(e) => setFullName(e.target.value)}
                  className="w-full pl-10 pr-4 py-2.5 rounded-xl bg-[#080c16] border border-border text-slate-100 placeholder:text-slate-600 focus:outline-none focus:border-primary transition-colors text-sm"
                />
              </div>
            </div>
          )}

          <div>
            <label className="block text-xs font-semibold uppercase tracking-wider text-slate-400 mb-1">
              Email Address
            </label>
            <div className="relative">
              <Mail className="absolute left-3.5 top-3 w-4 h-4 text-slate-500" />
              <input
                type="email"
                required
                placeholder="developer@strata.pedrofarath.me"
                value={email}
                onChange={(e) => setEmail(e.target.value)}
                className="w-full pl-10 pr-4 py-2.5 rounded-xl bg-[#080c16] border border-border text-slate-100 placeholder:text-slate-600 focus:outline-none focus:border-primary transition-colors text-sm"
              />
            </div>
          </div>

          <div>
            <label className="block text-xs font-semibold uppercase tracking-wider text-slate-400 mb-1">
              Password
            </label>
            <div className="relative">
              <Lock className="absolute left-3.5 top-3 w-4 h-4 text-slate-500" />
              <input
                type="password"
                required
                placeholder="••••••••••••"
                value={password}
                onChange={(e) => setPassword(e.target.value)}
                className="w-full pl-10 pr-4 py-2.5 rounded-xl bg-[#080c16] border border-border text-slate-100 placeholder:text-slate-600 focus:outline-none focus:border-primary transition-colors text-sm"
              />
            </div>
          </div>

          {mode === 'signup' && (
            <div>
              <label className="block text-xs font-semibold uppercase tracking-wider text-slate-400 mb-1">
                Default Workspace Name (Optional)
              </label>
              <input
                type="text"
                placeholder="My Core Team"
                value={workspaceName}
                onChange={(e) => setWorkspaceName(e.target.value)}
                className="w-full px-4 py-2.5 rounded-xl bg-[#080c16] border border-border text-slate-100 placeholder:text-slate-600 focus:outline-none focus:border-primary transition-colors text-sm"
              />
            </div>
          )}

          <button
            type="submit"
            disabled={loading}
            className="w-full py-3 px-4 rounded-xl bg-primary hover:bg-primary-hover text-black font-bold flex items-center justify-center gap-2 btn-pressable shadow-glow mt-2 text-sm disabled:opacity-50"
          >
            <span>{loading ? 'Authenticating...' : mode === 'login' ? 'Sign In to Dashboard' : 'Create & Launch'}</span>
            {!loading && <ArrowRight className="w-4 h-4" />}
          </button>
        </form>

        <div className="mt-6 pt-4 border-t border-border/70 flex items-center justify-center gap-2 text-xs text-slate-500">
          <ShieldCheck className="w-3.5 h-3.5 text-emerald-400" />
          <span>Unified with <code>strata login</code> CLI credentials</span>
        </div>
      </div>
    </div>
  );
};
