import React, { useState, useEffect } from 'react';
import { Key, Plus, Trash2, Copy, Check, ShieldCheck, AlertCircle, Clock } from 'lucide-react';
import { ApiKey, ApiKeyCreated, Workspace } from '../types';
import { api } from '../api';
import { toast } from './Toast';

interface ApiKeyManagerProps {
  workspace: Workspace;
}

export const ApiKeyManager: React.FC<ApiKeyManagerProps> = ({ workspace }) => {
  const [keys, setKeys] = useState<ApiKey[]>([]);
  const [loading, setLoading] = useState(true);
  const [newKeyName, setNewKeyName] = useState('');
  const [createdKey, setCreatedKey] = useState<ApiKeyCreated | null>(null);
  const [copiedKeyId, setCopiedKeyId] = useState<string | null>(null);

  const fetchKeys = async () => {
    try {
      setLoading(true);
      const data = await api.listApiKeys(workspace.id);
      setKeys(data);
    } catch (err: any) {
      toast.error('Failed to load API keys', err.message);
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    fetchKeys();
  }, [workspace.id]);

  const handleCreate = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!newKeyName.trim()) return;

    try {
      const resp = await api.createApiKey(workspace.id, newKeyName.trim());
      setCreatedKey(resp);
      setNewKeyName('');
      toast.success('API Key generated!', 'Save your secret key now; it will not be shown again.');
      fetchKeys();
    } catch (err: any) {
      toast.error('Failed to create API key', err.message);
    }
  };

  const handleRevoke = async (keyId: string) => {
    if (!confirm('Are you sure you want to revoke this API key? Agents using it will lose access immediately.')) {
      return;
    }

    try {
      await api.revokeApiKey(keyId);
      toast.success('Key revoked');
      setKeys((prev) => prev.filter((k) => k.id !== keyId));
    } catch (err: any) {
      toast.error('Failed to revoke key', err.message);
    }
  };

  const handleCopy = (keyText: string, id: string) => {
    navigator.clipboard.writeText(keyText);
    setCopiedKeyId(id);
    toast.success('API Key copied to clipboard');
    setTimeout(() => setCopiedKeyId(null), 2000);
  };

  return (
    <div className="space-y-6 max-w-4xl">
      {/* Creation Card */}
      <div className="glass-panel p-6 rounded-2xl border border-border">
        <div className="flex items-center gap-3 mb-2">
          <div className="p-2 rounded-xl bg-primary/10 border border-primary/20 text-primary">
            <Key className="w-5 h-5" />
          </div>
          <div>
            <h3 className="text-base font-bold text-white">Generate Machine API Key</h3>
            <p className="text-xs text-slate-400">
              Provide this key to Cursor, Claude Code, Windsurf or Codex for persistent memory synchronization.
            </p>
          </div>
        </div>

        <form onSubmit={handleCreate} className="mt-4 flex flex-col sm:flex-row gap-3">
          <input
            type="text"
            required
            placeholder="Key Name (e.g. Cursor MacBook Pro, Claude Desktop)"
            value={newKeyName}
            onChange={(e) => setNewKeyName(e.target.value)}
            className="flex-1 px-4 py-2.5 rounded-xl bg-[#080c16] border border-border text-slate-100 placeholder:text-slate-500 focus:outline-none focus:border-primary text-xs"
          />

          <button
            type="submit"
            className="px-5 py-2.5 rounded-xl bg-primary hover:bg-primary-hover text-black font-bold text-xs flex items-center justify-center gap-2 btn-pressable shadow-glow"
          >
            <Plus className="w-4 h-4" />
            <span>Generate Key</span>
          </button>
        </form>
      </div>

      {/* Newly Created Key Banner */}
      {createdKey && (
        <div className="p-5 rounded-2xl bg-emerald-950/40 border border-emerald-500/40 glass-panel animate-in zoom-in-95 space-y-2">
          <div className="flex items-center justify-between text-xs font-bold text-emerald-300">
            <span className="flex items-center gap-1.5">
              <ShieldCheck className="w-4 h-4 text-emerald-400" />
              <span>Copy Your Secret Key Now</span>
            </span>
            <button
              onClick={() => setCreatedKey(null)}
              className="text-xs text-slate-400 hover:text-white"
            >
              Dismiss
            </button>
          </div>

          <div className="flex items-center gap-2 bg-black/60 p-3 rounded-xl border border-emerald-500/20 font-mono text-xs text-slate-100">
            <span className="flex-1 truncate">{createdKey.key}</span>
            <button
              onClick={() => handleCopy(createdKey.key, 'newly-created')}
              className="p-1.5 rounded-lg bg-emerald-500 text-black hover:bg-emerald-400 btn-pressable"
            >
              {copiedKeyId === 'newly-created' ? <Check className="w-4 h-4" /> : <Copy className="w-4 h-4" />}
            </button>
          </div>
        </div>
      )}

      {/* Keys List */}
      <div className="glass-panel rounded-2xl border border-border overflow-hidden">
        <div className="px-6 py-4 border-b border-border/80 flex items-center justify-between">
          <h4 className="text-sm font-bold text-white">Active Machine Keys ({keys.length})</h4>
          <span className="text-xs text-slate-500 font-mono">Workspace: {workspace.name}</span>
        </div>

        {loading ? (
          <div className="p-8 text-center text-xs text-slate-500 font-mono">Loading API keys...</div>
        ) : keys.length === 0 ? (
          <div className="p-8 text-center text-xs text-slate-500 font-mono">
            No API keys issued yet. Generate one above to connect an IDE.
          </div>
        ) : (
          <div className="divide-y divide-border/60">
            {keys.map((k) => (
              <div key={k.id} className="px-6 py-4 flex items-center justify-between gap-4 hover:bg-card/40 transition-colors">
                <div className="space-y-1">
                  <div className="flex items-center gap-2">
                    <span className="font-bold text-xs text-slate-200">{k.name}</span>
                    <span className="px-2 py-0.5 rounded-md bg-card border border-border text-[10px] font-mono text-slate-400">
                      {k.key_prefix}...
                    </span>
                  </div>
                  <div className="text-[11px] text-slate-500 font-mono flex items-center gap-3">
                    <span>Created: {new Date(k.created_at).toLocaleDateString()}</span>
                    <span>Scopes: {k.scopes.join(', ')}</span>
                  </div>
                </div>

                <div className="flex items-center gap-2">
                  <button
                    onClick={() => handleRevoke(k.id)}
                    className="p-2 rounded-lg text-slate-500 hover:text-rose-400 hover:bg-rose-950/30 btn-pressable transition-colors"
                    title="Revoke API key"
                  >
                    <Trash2 className="w-4 h-4" />
                  </button>
                </div>
              </div>
            ))}
          </div>
        )}
      </div>
    </div>
  );
};
