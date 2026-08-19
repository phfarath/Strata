import React, { useState, useEffect } from 'react';
import { Key, Plus, Trash2, Copy, Check, Shield } from 'lucide-react';
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
      <div className="p-4 rounded-xl border border-zinc-800 bg-[#111114]">
        <div className="flex items-center gap-2 mb-1">
          <Key className="w-4 h-4 text-zinc-400" />
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
            className="flex-1 px-3 py-2 rounded-lg bg-zinc-900 border border-zinc-800 text-zinc-100 placeholder:text-zinc-500 focus:outline-none focus:border-zinc-700 text-xs font-mono"
          />

          <button
            type="submit"
            className="px-4 py-2 rounded-lg bg-white text-black font-semibold text-xs flex items-center justify-center gap-1.5 hover:bg-zinc-200 btn-pressable"
          >
            <Plus className="w-3.5 h-3.5" />
            <span>Generate Key</span>
          </button>
        </form>
      </div>

      {/* Newly Created Key Alert */}
      {createdKey && (
        <div className="p-4 rounded-xl bg-zinc-900 border border-zinc-700 space-y-2">
          <div className="flex items-center justify-between text-xs text-zinc-200 font-medium">
            <span>Copy Secret API Key (Shown only once)</span>
            <button
              onClick={() => setCreatedKey(null)}
              className="text-zinc-500 hover:text-zinc-300 text-[11px]"
            >
              Dismiss
            </button>
          </div>

          <div className="flex items-center gap-2 bg-[#09090c] p-2.5 rounded-lg border border-zinc-800 font-mono text-xs text-zinc-200">
            <span className="flex-1 truncate">{createdKey.key}</span>
            <button
              onClick={() => handleCopy(createdKey.key, 'newly-created')}
              className="p-1 rounded bg-zinc-800 text-zinc-300 hover:text-white btn-pressable"
            >
              {copiedKeyId === 'newly-created' ? <Check className="w-3.5 h-3.5 text-emerald-400" /> : <Copy className="w-3.5 h-3.5" />}
            </button>
          </div>
        </div>
      )}

      {/* Keys Table */}
      <div className="rounded-xl border border-zinc-800 bg-[#111114] overflow-hidden">
        <div className="px-4 py-3 border-b border-zinc-800 flex items-center justify-between">
          <h4 className="text-xs font-semibold text-white">Active Keys ({keys.length})</h4>
          <span className="text-xs text-zinc-500 font-mono">Workspace: {workspace.slug}</span>
        </div>

        {loading ? (
          <div className="p-6 text-center text-xs text-zinc-500 font-mono">Loading keys...</div>
        ) : keys.length === 0 ? (
          <div className="p-6 text-center text-xs text-zinc-500 font-mono">
            No machine keys issued.
          </div>
        ) : (
          <div className="divide-y divide-zinc-800/80 font-mono text-xs">
            {keys.map((k) => (
              <div key={k.id} className="px-4 py-3 flex items-center justify-between gap-4 hover:bg-zinc-900/40">
                <div className="space-y-0.5">
                  <div className="flex items-center gap-2">
                    <span className="font-medium text-zinc-200">{k.name}</span>
                    <span className="px-1.5 py-0.5 rounded bg-zinc-900 border border-zinc-800 text-[10px] text-zinc-400">
                      {k.key_prefix}...
                    </span>
                  </div>
                  <div className="text-[11px] text-zinc-500">
                    Created: {new Date(k.created_at).toLocaleDateString()} • Scopes: {k.scopes.join(', ')}
                  </div>
                </div>

                <button
                  onClick={() => handleRevoke(k.id)}
                  className="p-1.5 rounded text-zinc-500 hover:text-rose-400 hover:bg-zinc-900 btn-pressable"
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
