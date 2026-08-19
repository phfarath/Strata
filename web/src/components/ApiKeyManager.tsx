import React, { useState, useEffect } from 'react';
import { Key, Plus, Trash2, Copy, Check } from 'lucide-react';
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
    if (!workspace?.id || workspace.id === 'default' || workspace.id.length < 10) {
      setLoading(false);
      return;
    }
    try {
      setLoading(true);
      const data = await api.listApiKeys(workspace.id);
      setKeys(data);
    } catch (err: any) {
      console.warn('API keys fetch:', err.message);
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
      toast.success('API Key generated', 'Save your secret key; it will not be shown again.');
      fetchKeys();
    } catch (err: any) {
      toast.error('Failed to create API key', err.message);
    }
  };

  const handleRevoke = async (keyId: string) => {
    if (!confirm('Revoke this API key? Connected agents will lose access.')) {
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
    toast.success('Key copied to clipboard');
    setTimeout(() => setCopiedKeyId(null), 2000);
  };

  return (
    <div className="space-y-4 max-w-4xl font-sans">
      {/* Create Key Card */}
      <div className="p-4 rounded-xl border border-[#23262f] bg-[#15171d]">
        <div className="flex items-center gap-2 mb-1">
          <Key className="w-4 h-4 text-amber-500" />
          <h3 className="text-sm font-semibold text-white">Generate Machine API Key</h3>
        </div>
        <p className="text-xs text-zinc-400 mb-3">
          Configure this key in Cursor, Claude Code or Windsurf configuration to sync memories.
        </p>

        <form onSubmit={handleCreate} className="flex flex-col sm:flex-row gap-2">
          <input
            type="text"
            required
            placeholder="Key Description (e.g. Cursor MacBook Pro)"
            value={newKeyName}
            onChange={(e) => setNewKeyName(e.target.value)}
            className="flex-1 px-3 py-2 rounded-lg bg-[#0f1115] border border-[#23262f] text-zinc-100 placeholder:text-zinc-500 focus:outline-none focus:border-amber-500 text-xs font-mono"
          />

          <button
            type="submit"
            className="px-4 py-2 rounded-lg bg-amber-500 text-black font-bold text-xs flex items-center justify-center gap-1.5 hover:bg-amber-400 btn-pressable sweep-hover"
          >
            <Plus className="w-3.5 h-3.5" />
            <span>Generate Key</span>
          </button>
        </form>
      </div>

      {/* Newly Created Key Alert */}
      {createdKey && (
        <div className="p-4 rounded-xl bg-[#1c1913] border border-amber-500/40 space-y-2">
          <div className="flex items-center justify-between text-xs text-amber-300 font-medium">
            <span>Copy Secret API Key (Shown only once)</span>
            <button
              onClick={() => setCreatedKey(null)}
              className="text-zinc-500 hover:text-zinc-300 text-[11px]"
            >
              Dismiss
            </button>
          </div>

          <div className="flex items-center gap-2 bg-[#090a0d] p-2.5 rounded-lg border border-[#23262f] font-mono text-xs text-zinc-200">
            <span className="flex-1 truncate text-amber-300">{createdKey.key}</span>
            <button
              onClick={() => handleCopy(createdKey.key, 'newly-created')}
              className="p-1.5 rounded bg-[#1f232b] text-zinc-300 hover:text-white btn-pressable sweep-hover"
            >
              {copiedKeyId === 'newly-created' ? <Check className="w-3.5 h-3.5 text-emerald-400" /> : <Copy className="w-3.5 h-3.5" />}
            </button>
          </div>
        </div>
      )}

      {/* Keys Table */}
      <div className="rounded-xl border border-[#23262f] bg-[#15171d] overflow-hidden">
        <div className="px-4 py-3 border-b border-[#23262f] flex items-center justify-between">
          <h4 className="text-xs font-semibold text-white">Active Keys ({keys.length})</h4>
          <span className="text-xs text-amber-500 font-mono">Workspace: {workspace.slug}</span>
        </div>

        {loading ? (
          <div className="p-6 text-center text-xs text-zinc-500 font-mono">Loading keys...</div>
        ) : keys.length === 0 ? (
          <div className="p-6 text-center text-xs text-zinc-500 font-mono">
            No machine keys issued.
          </div>
        ) : (
          <div className="divide-y divide-[#23262f] font-mono text-xs">
            {keys.map((k) => (
              <div key={k.id} className="px-4 py-3 flex items-center justify-between gap-4 hover:bg-[#1b1e26] transition-colors sweep-hover">
                <div className="space-y-0.5">
                  <div className="flex items-center gap-2">
                    <span className="font-medium text-zinc-200">{k.name}</span>
                    <span className="px-1.5 py-0.5 rounded bg-[#0f1115] border border-[#23262f] text-[10px] text-amber-400">
                      {k.key_prefix}...
                    </span>
                  </div>
                  <div className="text-[11px] text-zinc-500">
                    Created: {new Date(k.created_at).toLocaleDateString()} • Scopes: {k.scopes.join(', ')}
                  </div>
                </div>

                <button
                  onClick={() => handleRevoke(k.id)}
                  className="p-1.5 rounded text-zinc-500 hover:text-rose-400 hover:bg-[#23262f] btn-pressable"
                  title="Revoke key"
                >
                  <Trash2 className="w-3.5 h-3.5" />
                </button>
              </div>
            ))}
          </div>
        )}
      </div>
    </div>
  );
};
