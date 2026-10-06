import { PryselSSO } from '@prysel/sso';
import { NextResponse } from 'next/server';

const prysel = new PryselSSO({
  appId: process.env.PRYSEL_APP_ID!,
  clientSecret: process.env.PRYSEL_CLIENT_SECRET!,
  authBaseUrl: process.env.PRYSEL_AUTH_URL!,
  redirectUri: process.env.PRYSEL_REDIRECT_URI!,
  scopes: 'profile email phone',
});

export async function GET() {
  const { url } = await prysel.createAuthorizeUrl();
  return NextResponse.redirect(url);
}
