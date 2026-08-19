export interface User {
  id: string;
  email: string;
  full_name: string;
  role: string;
  created_at: string;
}

export interface Workspace {
  id: string;
  owner_id: string;
  name: string;
  slug: string;
  created_at: string;
}

export interface ApiKey {
  id: string;
  workspace_id: string;
  name: string;
  key_prefix: string;
  scopes: string[];
  expires_at?: string | null;
  created_at: string;
}

export interface ApiKeyCreated extends ApiKey {
  key: string;
}

export interface AuthResponse {
  user: User;
  workspaces: Workspace[];
  token: string;
}

export interface PingResponse {
  status: string;
  timestamp: string;
  epoch_ms: number;
  protocol: string;
  custom_domain?: string | null;
  is_postgres: boolean;
  has_pgvector: boolean;
  uptime_secs: number;
}

export interface HealthResponse {
  status: string;
  version: string;
  uptime_secs: number;
  workspaces_count: number;
  is_postgres: boolean;
  has_pgvector: boolean;
  custom_domain?: string | null;
}

export interface StatusResponse {
  workspace_id: string;
  total_deltas: number;
  max_seq: number;
  is_postgres: boolean;
  has_pgvector: boolean;
}

export type MemoryType = 'semantic_fact' | 'episodic_session' | 'procedural_skill' | 'failure_pattern';
export type FactStatus = 'verified' | 'under_review' | 'refuted' | 'tentative';

export interface MemoryRecord {
  id: string;
  workspace_id: string;
  type: MemoryType;
  title: string;
  content: string;
  scope: string;
  status: FactStatus;
  decay_score: number; // 0.0 to 1.0
  access_count: number;
  created_at: string;
  updated_at: string;
  evidence_count: number;
}

export interface SyncDeltaItem {
  id: string;
  entity_type: string;
  operation: 'INSERT' | 'UPDATE' | 'DELETE';
  version: number;
  timestamp: string;
  author_host: string;
}

export interface RealtimeWsEvent {
  event: string;
  workspace_id?: string;
  max_seq?: number;
  delta_count?: number;
  timestamp: string;
  raw?: any;
}
