import React, { useState, useEffect } from 'react';
import { User, Workspace } from './types';
import { api } from './api';
import { Navbar } from './components/Navbar';
import { Sidebar } from './components/Sidebar';
import { LandingPage } from './components/LandingPage';
import { Dashboard } from './components/Dashboard';
import { AuthModal } from './components/AuthModal';
import { Toaster, toast } from './components/Toast';
import { X } from 'lucide-react';

export function App() {
  const [user, setUser] = useState<User | null>(null);
  const [workspaces, setWorkspaces] = useState<Workspace[]>([]);
  const [activeWorkspace, setActiveWorkspace] = useState<Workspace | null>(null);
  const [authModalOpen, setAuthModalOpen] = useState(false);
  const [createWsModalOpen, setCreateWsModalOpen] = useState(false);
  const [newWsName, setNewWsName] = useState('');
  const [currentTab, setCurrentTab] = useState<'overview' | 'explorer' | 'keys' | 'stream' | 'playground'>('overview');
  const [landingViewOpen, setLandingViewOpen] = useState(false);

  // Helper to construct a safe fallback workspace
  const getSafeWorkspace = (u: User, wsList: Workspace[]): Workspace => {
    if (wsList.length > 0) return wsList[0];
    return {
      id: u.id,
      owner_id: u.id,
      name: `${u.full_name || 'My'} Organization`,
      slug: 'default-workspace',
      created_at: new Date().toISOString(),
    };
  };

  // Check existing session
  useEffect(() => {
    const token = api.getToken();
    if (token) {
      api.getMe()
        .then(async (resp) => {
          setUser(resp.user);
          let wsList = resp.workspaces;
          if (wsList.length === 0) {
            try {
              const defaultWs = await api.createWorkspace(`${resp.user.full_name || 'My'} Workspace`);
              wsList = [defaultWs];
            } catch {
              // ignore
            }
          }
          setWorkspaces(wsList);
          if (wsList.length > 0) {
            setActiveWorkspace(wsList[0]);
          } else {
            setActiveWorkspace(getSafeWorkspace(resp.user, []));
          }
        })
        .catch(() => {
          api.clearToken();
          setUser(null);
        });
    }
  }, []);

  const handleAuthSuccess = async (u: User, wsList: Workspace[]) => {
    setUser(u);
    let effectiveWsList = wsList;
    if (effectiveWsList.length === 0) {
      try {
        const defaultWs = await api.createWorkspace(`${u.full_name || 'My'} Workspace`);
        effectiveWsList = [defaultWs];
      } catch {
        // ignore
      }
    }
    setWorkspaces(effectiveWsList);
    if (effectiveWsList.length > 0) {
      setActiveWorkspace(effectiveWsList[0]);
    } else {
      setActiveWorkspace(getSafeWorkspace(u, []));
    }
    setLandingViewOpen(false);
    setCurrentTab('overview');
  };

  const handleLogout = () => {
    api.clearToken();
    setUser(null);
    setWorkspaces([]);
    setActiveWorkspace(null);
    setLandingViewOpen(false);
    toast.info('Signed out');
  };

  const handleCreateWorkspace = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!newWsName.trim()) return;

    try {
      const created = await api.createWorkspace(newWsName.trim());
      setWorkspaces((prev) => [...prev, created]);
      setActiveWorkspace(created);
      setNewWsName('');
      setCreateWsModalOpen(false);
      toast.success('Workspace Created', created.name);
    } catch (err: any) {
      toast.error('Failed to create workspace', err.message);
    }
  };

  // If not logged in or explicitly viewing landing page
  if (!user || landingViewOpen) {
    return (
      <div className="min-h-screen bg-background text-slate-100 flex flex-col selection:bg-primary/20 selection:text-primary">
        <Navbar
          user={user}
          workspaces={workspaces}
          activeWorkspace={activeWorkspace}
          onSelectWorkspace={(ws) => {
            setActiveWorkspace(ws);
            setLandingViewOpen(false);
          }}
          onCreateWorkspaceClick={() => setCreateWsModalOpen(true)}
          onOpenAuth={() => setAuthModalOpen(true)}
          onLogout={handleLogout}
          currentView={landingViewOpen ? 'landing' : 'overview'}
          onNavigate={(view) => {
            if (view === 'landing') {
              setLandingViewOpen(true);
            } else {
              setLandingViewOpen(false);
              setCurrentTab(view as any);
            }
          }}
        />

        <main className="flex-1">
          <LandingPage onOpenAuth={() => setAuthModalOpen(true)} />
        </main>

        <AuthModal
          isOpen={authModalOpen}
          onClose={() => setAuthModalOpen(false)}
          onSuccess={handleAuthSuccess}
        />

        <Toaster />
      </div>
    );
  }

  // Active workspace guaranteed
  const safeCurrentWorkspace = activeWorkspace || getSafeWorkspace(user, workspaces);

  return (
    <div className="flex h-screen w-screen overflow-hidden bg-background text-slate-100 selection:bg-primary/20 selection:text-primary">
      {/* Sleek App Sidebar */}
      <Sidebar
        user={user}
        workspaces={workspaces.length > 0 ? workspaces : [safeCurrentWorkspace]}
        activeWorkspace={safeCurrentWorkspace}
        onSelectWorkspace={(ws) => setActiveWorkspace(ws)}
        onCreateWorkspaceClick={() => setCreateWsModalOpen(true)}
        onLogout={handleLogout}
        currentTab={currentTab}
        onTabChange={(tab) => setCurrentTab(tab)}
      />

      {/* Main Dashboard Area */}
      <main className="flex-1 h-screen overflow-y-auto bg-[#07090e]">
        <Dashboard
          user={user}
          workspace={safeCurrentWorkspace}
          currentTab={currentTab}
          onTabChange={(tab) => setCurrentTab(tab)}
          onRefreshWorkspaces={() => {
            api.listWorkspaces().then(setWorkspaces).catch(() => {});
          }}
        />
      </main>

      {/* Create Workspace Modal */}
      {createWsModalOpen && (
        <div className="fixed inset-0 z-50 flex items-center justify-center p-4 bg-black/70 backdrop-blur-md animate-in fade-in duration-200">
          <div className="relative w-full max-w-md p-6 rounded-2xl glass-panel-glow border border-border bg-[#0a0f1d] shadow-2xl text-slate-100 animate-in zoom-in-95">
            <button
              onClick={() => setCreateWsModalOpen(false)}
              className="absolute top-5 right-5 p-1.5 rounded-lg text-slate-400 hover:text-white btn-pressable"
            >
              <X className="w-5 h-5" />
            </button>

            <h3 className="text-lg font-bold text-white mb-1">Create New Workspace</h3>
            <p className="text-xs text-slate-400 mb-4">
              Group memories and API keys by project or team repository.
            </p>

            <form onSubmit={handleCreateWorkspace} className="space-y-4">
              <div>
                <label className="block text-xs font-semibold uppercase tracking-wider text-slate-400 mb-1">
                  Workspace Name
                </label>
                <input
                  type="text"
                  required
                  placeholder="e.g. Mobile App Team, AI Agent Research"
                  value={newWsName}
                  onChange={(e) => setNewWsName(e.target.value)}
                  className="w-full px-4 py-2.5 rounded-xl bg-[#080c16] border border-border text-slate-100 placeholder:text-slate-600 focus:outline-none focus:border-primary text-xs"
                />
              </div>

              <button
                type="submit"
                className="w-full py-2.5 rounded-xl bg-primary hover:bg-primary-hover text-black font-bold text-xs btn-pressable shadow-glow"
              >
                Create Workspace
              </button>
            </form>
          </div>
        </div>
      )}

      {/* Sonner-style Toast Notifications */}
      <Toaster />
    </div>
  );
}

export default App;
