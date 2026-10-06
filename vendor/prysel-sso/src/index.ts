export interface PryselSSOConfig {
  appId: string;
  clientSecret: string;
  authBaseUrl: string;
  redirectUri: string;
  scopes?: string;
}

export interface PryselUser {
  id: string;
  email: string;
  name?: string | null;
  username?: string | null;
  nickname?: string | null;
  picture?: string | null;
  mobile?: string | null;
  role?: string | null;
  emailVerified?: boolean;
}

export interface AuthorizeSession {
  session_id: string;
  token: string;
  authorize_url: string;
  expires_at: string;
}

export class PryselSSO {
  private readonly config: PryselSSOConfig;

  constructor(config: PryselSSOConfig) {
    this.config = {
      scopes: 'profile email phone',
      ...config,
    };
  }

  /** Create an authorization session and return the URL to redirect the user to */
  async createAuthorizeUrl(state?: string): Promise<{ url: string; token: string }> {
    const response = await fetch(`${this.config.authBaseUrl}/api/sso/sessions`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        app_id: this.config.appId,
        client_secret: this.config.clientSecret,
        redirect_uri: this.config.redirectUri,
        state,
        scopes: this.config.scopes,
      }),
    });

    if (!response.ok) {
      const error = await response.json().catch(() => ({}));
      throw new Error((error as { message?: string }).message ?? 'Failed to create authorization session');
    }

    const data = (await response.json()) as AuthorizeSession;
    return { url: data.authorize_url, token: data.token };
  }

  /** Redirect the user to Prysel Auth for login and consent */
  async login(state?: string): Promise<void> {
    const { url } = await this.createAuthorizeUrl(state);
    if (typeof window !== 'undefined') {
      window.location.href = url;
    }
  }

  /** Exchange authorization code for user data (call from your callback route) */
  async exchangeCode(code: string): Promise<{ user: PryselUser; scopes: string }> {
    const response = await fetch(`${this.config.authBaseUrl}/api/sso/token`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        code,
        app_id: this.config.appId,
        client_secret: this.config.clientSecret,
        redirect_uri: this.config.redirectUri,
      }),
    });

    if (!response.ok) {
      const error = await response.json().catch(() => ({}));
      throw new Error((error as { error?: string }).error ?? 'Token exchange failed');
    }

    return response.json() as Promise<{ user: PryselUser; scopes: string }>;
  }

  /** Parse callback URL search params */
  static parseCallback(url: string): { code?: string; state?: string; error?: string } {
    const params = new URL(url).searchParams;
    return {
      code: params.get('code') ?? undefined,
      state: params.get('state') ?? undefined,
      error: params.get('error') ?? undefined,
    };
  }
}

export default PryselSSO;
