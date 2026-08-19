import React, { useState } from 'react';
import { Check, Copy } from 'lucide-react';
import { toast } from './Toast';

interface CodeBlockProps {
  code: string;
  language?: string;
  title?: string;
}

export const CodeBlock: React.FC<CodeBlockProps> = ({ code, language = 'bash', title }) => {
  const [copied, setCopied] = useState(false);

  const handleCopy = async () => {
    try {
      await navigator.clipboard.writeText(code);
      setCopied(true);
      toast.success('Copied to clipboard', code);
      setTimeout(() => setCopied(false), 2000);
    } catch {
      toast.error('Failed to copy');
    }
  };

  return (
    <div className="relative group rounded-xl border border-border bg-[#0a0e17] overflow-hidden">
      {title && (
        <div className="flex items-center justify-between px-4 py-2 bg-card border-b border-border/70 text-xs font-mono text-slate-400">
          <span>{title}</span>
          <span className="text-slate-500 uppercase">{language}</span>
        </div>
      )}

      <div className="relative p-4 font-mono text-sm leading-relaxed overflow-x-auto text-slate-200">
        <pre>{code}</pre>

        <button
          onClick={handleCopy}
          aria-label="Copy code"
          className={`absolute top-3 right-3 p-2 rounded-lg border border-border bg-card/80 text-slate-400 hover:text-white hover:border-primary/50 hover:bg-primary/10 btn-pressable backdrop-blur-md opacity-80 group-hover:opacity-100 transition-all ${
            copied ? 'text-emerald-400 border-emerald-500/50 bg-emerald-950/30' : ''
          }`}
        >
          {copied ? <Check className="w-4 h-4" /> : <Copy className="w-4 h-4" />}
        </button>
      </div>
    </div>
  );
};
