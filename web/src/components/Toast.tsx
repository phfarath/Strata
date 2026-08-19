import React, { useState, useEffect } from 'react';
import { CheckCircle2, AlertCircle, Info, X } from 'lucide-react';

export interface ToastMessage {
  id: string;
  type: 'success' | 'error' | 'info';
  title: string;
  description?: string;
}

let toastListener: ((t: ToastMessage) => void) | null = null;

export const toast = {
  success: (title: string, description?: string) => {
    if (toastListener) {
      toastListener({ id: Math.random().toString(), type: 'success', title, description });
    }
  },
  error: (title: string, description?: string) => {
    if (toastListener) {
      toastListener({ id: Math.random().toString(), type: 'error', title, description });
    }
  },
  info: (title: string, description?: string) => {
    if (toastListener) {
      toastListener({ id: Math.random().toString(), type: 'info', title, description });
    }
  },
};

export const Toaster: React.FC = () => {
  const [toasts, setToasts] = useState<ToastMessage[]>([]);
  const [paused, setPaused] = useState(false);

  useEffect(() => {
    toastListener = (newToast) => {
      setToasts((prev) => [...prev.slice(-3), newToast]);
    };
    return () => {
      toastListener = null;
    };
  }, []);

  useEffect(() => {
    if (toasts.length === 0 || paused) return;

    const timer = setTimeout(() => {
      setToasts((prev) => prev.slice(1));
    }, 4000);

    return () => clearTimeout(timer);
  }, [toasts, paused]);

  if (toasts.length === 0) return null;

  return (
    <div
      className="fixed bottom-6 right-6 z-50 flex flex-col gap-2 pointer-events-none max-w-sm w-full"
      onMouseEnter={() => setPaused(true)}
      onMouseLeave={() => setPaused(false)}
    >
      {toasts.map((t) => (
        <div
          key={t.id}
          className={`pointer-events-auto flex items-start gap-3 p-4 rounded-xl border glass-panel shadow-xl transform transition-all duration-200 ease-out animate-in fade-in slide-in-from-bottom-3 ${
            t.type === 'success'
              ? 'border-emerald-500/30 bg-emerald-950/40 text-emerald-100'
              : t.type === 'error'
              ? 'border-rose-500/30 bg-rose-950/40 text-rose-100'
              : 'border-sky-500/30 bg-sky-950/40 text-sky-100'
          }`}
          style={{ transformOrigin: 'bottom center' }}
        >
          {t.type === 'success' && <CheckCircle2 className="w-5 h-5 text-emerald-400 shrink-0 mt-0.5" />}
          {t.type === 'error' && <AlertCircle className="w-5 h-5 text-rose-400 shrink-0 mt-0.5" />}
          {t.type === 'info' && <Info className="w-5 h-5 text-sky-400 shrink-0 mt-0.5" />}

          <div className="flex-1 text-sm">
            <div className="font-semibold">{t.title}</div>
            {t.description && <div className="text-xs text-slate-400 mt-0.5">{t.description}</div>}
          </div>

          <button
            onClick={() => setToasts((prev) => prev.filter((item) => item.id !== t.id))}
            className="text-slate-400 hover:text-slate-200 p-0.5 rounded transition-colors"
          >
            <X className="w-4 h-4" />
          </button>
        </div>
      ))}
    </div>
  );
};
