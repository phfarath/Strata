import {
  AuthResponse,
  HealthResponse,
  PingResponse,
  StatusResponse,
  Workspace,
  ApiKey,
  ApiKeyCreated,
  MemoryRecord,
} from './types';

// Default production endpoint
const DEFAULT_API_URL = 'https://strata.pedrofarath.me';

export class StrataApiClient {
  private baseUrl: string;

  constructor() {
    this.baseUrl = localStorage.getItem('strata_api_url') || DEFAULT_API_URL;
  }

  public getBaseUrl(): string {
    return this.baseUrl;
  }

  public setBaseUrl(url: string) {
    this.baseUrl = url.trim().replace(/\/$/, '');
    localStorage.setItem('strata_api_url', this.baseUrl);
  }

  public getToken(): string | null {
    return localStorage.getItem('strata_token');
  }

  public setToken(token: string) {
    localStorage.setItem('strata_token', token);
  }

  public clearToken() {
    localStorage.removeItem('strata_token');
    localStorage.removeItem('strata_user');
  }

  private async request<T>(path: string, options: RequestInit = {}): Promise<T> {
    const headers = new Headers(options.headers || {});
    headers.set('Content-Type', 'application/json');

    const token = this.getToken();
    if (token) {
      headers.set('Authorization', `Bearer ${token}`);
    }

    const url = `${this.baseUrl}${path}`;
    const resp = await fetch(url, {
      ...options,
      headers,
    });

    if (!resp.ok) {
      let errorMessage = `HTTP Error ${resp.status}`;
      try {
        const errorData = await resp.json();
        if (errorData.error) errorMessage = errorData.error;
      } catch {
        errorMessage = await resp.text() || errorMessage;
      }
      throw new Error(errorMessage);
    }

    return resp.json();
  }

  public async ping(): Promise<PingResponse> {
    return this.request<PingResponse>('/api/v1/ping');
  }

  public async health(): Promise<HealthResponse> {
    return this.request<HealthResponse>('/api/v1/health');
  }

  public async signup(email: string, password: string, fullName: string, workspaceName?: string): Promise<AuthResponse> {
    const data = await this.request<AuthResponse>('/api/v1/auth/signup', {
      method: 'POST',
      body: JSON.stringify({
        email,
        password,
        full_name: fullName,
        workspace_name: workspaceName,
      }),
    });
    this.setToken(data.token);
    localStorage.setItem('strata_user', JSON.stringify(data.user));
    return data;
  }

  public async login(email: string, password: string): Promise<AuthResponse> {
    const data = await this.request<AuthResponse>('/api/v1/auth/login', {
      method: 'POST',
      body: JSON.stringify({ email, password }),
    });
    this.setToken(data.token);
    localStorage.setItem('strata_user', JSON.stringify(data.user));
    return data;
  }

  public async getMe(): Promise<AuthResponse> {
    return this.request<AuthResponse>('/api/v1/auth/me');
  }

  public async listWorkspaces(): Promise<Workspace[]> {
    return this.request<Workspace[]>('/api/v1/workspaces');
  }

  public async createWorkspace(name: string, slug?: string): Promise<Workspace> {
    return this.request<Workspace>('/api/v1/workspaces', {
      method: 'POST',
      body: JSON.stringify({ name, slug }),
    });
  }

  public async listApiKeys(workspaceId: string): Promise<ApiKey[]> {
    return this.request<ApiKey[]>(`/api/v1/keys?workspace_id=${encodeURIComponent(workspaceId)}`);
  }

  public async createApiKey(workspaceId: string, name: string, scopes?: string[], expiresDays?: number): Promise<ApiKeyCreated> {
    return this.request<ApiKeyCreated>('/api/v1/keys', {
      method: 'POST',
      body: JSON.stringify({
        workspace_id: workspaceId,
        name,
        scopes: scopes || ['sync:read', 'sync:write'],
        expires_days: expiresDays,
      }),
    });
  }

  public async revokeApiKey(keyId: string): Promise<{ status: string; message: string }> {
    return this.request<{ status: string; message: string }>(`/api/v1/keys/${keyId}`, {
      method: 'DELETE',
    });
  }

  public async getStatus(workspaceId: string): Promise<StatusResponse> {
    return this.request<StatusResponse>(`/api/v1/sync/status?workspace_id=${encodeURIComponent(workspaceId)}`);
  }

  public createWebSocket(onMessage: (event: any) => void, onOpen?: () => void, onClose?: () => void): WebSocket {
    const token = this.getToken() || '';
    const wsProto = this.baseUrl.startsWith('https') ? 'wss:' : 'ws:';
    const host = this.baseUrl.replace(/^https?:\/\//, '');
    const wsUrl = `${wsProto}//${host}/api/v1/sync/ws?token=${encodeURIComponent(token)}`;

    const ws = new WebSocket(wsUrl);
    ws.onopen = () => {
      if (onOpen) onOpen();
    };
    ws.onmessage = (evt) => {
      try {
        const parsed = JSON.parse(evt.data);
        onMessage(parsed);
      } catch {
        onMessage({ event: 'raw', data: evt.data, timestamp: new Date().toISOString() });
      }
    };
    ws.onclose = () => {
      if (onClose) onClose();
    };
    return ws;
  }
}

export const api = new StrataApiClient();
