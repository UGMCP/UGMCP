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

  const response = NextResponse.redirect(new URL('/dashboard', request.url));
  response.cookies.set(
    'prysel_user',
    JSON.stringify({
      id: user.id,
      name: user.name,
      email: user.email,
      picture: user.picture,
      username: user.username,
      mobile: user.mobile,
    }),
    { httpOnly: true, secure: true, path: '/' },
  );
  return response;
}
