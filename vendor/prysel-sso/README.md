# @prysel/sso

Prysel SSO SDK for Next.js, Node.js, and other applications.

## Installation

```bash
npm install @prysel/sso
```

Or copy `packages/prysel-sso` from this repository.

## Quick Start (Next.js App Router)

### 1. Configure environment variables

```env
PRYSEL_APP_ID=demo-nextjs-app
PRYSEL_CLIENT_SECRET=demo-secret-change-in-production
PRYSEL_AUTH_URL=http://localhost:4200
PRYSEL_REDIRECT_URI=http://localhost:3001/api/auth/callback
```

### 2. Login route (`app/api/auth/login/route.ts`)

```typescript
import { PryselSSO } from '@prysel/sso';
import { NextResponse } from 'next/server';

const prysel = new PryselSSO({
  appId: process.env.PRYSEL_APP_ID!,
  clientSecret: process.env.PRYSEL_CLIENT_SECRET!,
  authBaseUrl: process.env.PRYSEL_AUTH_URL!,
  redirectUri: process.env.PRYSEL_REDIRECT_URI!,
});

export async function GET() {
  const { url } = await prysel.createAuthorizeUrl();
  return NextResponse.redirect(url);
}
```

### 3. Callback route (`app/api/auth/callback/route.ts`)

```typescript
import { PryselSSO } from '@prysel/sso';
import { NextRequest, NextResponse } from 'next/server';

const prysel = new PryselSSO({
  appId: process.env.PRYSEL_APP_ID!,
  clientSecret: process.env.PRYSEL_CLIENT_SECRET!,
  authBaseUrl: process.env.PRYSEL_AUTH_URL!,
  redirectUri: process.env.PRYSEL_REDIRECT_URI!,
});

export async function GET(request: NextRequest) {
  const { code, error } = PryselSSO.parseCallback(request.url);

  if (error || !code) {
    return NextResponse.redirect(new URL('/login?error=access_denied', request.url));
  }

  const { user } = await prysel.exchangeCode(code);

  // Store user in session/cookie — example uses redirect with query (use secure session in production)
  const response = NextResponse.redirect(new URL('/dashboard', request.url));
  response.cookies.set('prysel_user', JSON.stringify(user), { httpOnly: true, secure: true });
  return response;
}
```

## SSO Flow

```
Your App                    Prysel Auth                    User
   |                            |                            |
   |-- POST /api/sso/sessions ->|                            |
   |<- authorize_url -----------|                            |
   |-- redirect to /company/{appId}/{token} ----------------->|
   |                            |<-- login (if needed) -------|
   |                            |<-- approve/decline ---------|
   |<- redirect with ?code=xxx -|                            |
   |-- POST /api/sso/token ---->|                            |
   |<- { user: { name, email, picture } }                     |
```

## User Data Returned

After successful authorization, `exchangeCode()` returns:

```json
{
  "user": {
    "id": "uuid",
    "email": "user@example.com",
    "emailVerified": true,
    "name": "John Doe",
    "username": "johndoe",
    "picture": "https://...",
    "mobile": "01700000000"
  },
  "scopes": "profile email phone"
}
```

Ask for `role` in `scopes` when the app also needs the account role (`admin`, `editor`, or `user`).

Copy-paste starters live in `examples/nextjs` and `examples/node-callback.mjs`.

## Authorization URL Pattern

```
https://auth.prysel.com/company/{appId}/{token}
```

Users see the app name, logo, and approve/decline buttons before data is shared.
