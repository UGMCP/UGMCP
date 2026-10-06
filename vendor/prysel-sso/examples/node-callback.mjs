import { PryselSSO } from '@prysel/sso';

const prysel = new PryselSSO({
  appId: process.env.PRYSEL_APP_ID,
  clientSecret: process.env.PRYSEL_CLIENT_SECRET,
  authBaseUrl: process.env.PRYSEL_AUTH_URL,
  redirectUri: process.env.PRYSEL_REDIRECT_URI,
  scopes: 'profile email phone',
});

const { url } = await prysel.createAuthorizeUrl();
console.log('Send the user to', url);

const callbackUrl = process.argv[2];
if (callbackUrl) {
  const { code, error } = PryselSSO.parseCallback(callbackUrl);
  if (error || !code) {
    throw new Error(error ?? 'missing code');
  }
  const { user } = await prysel.exchangeCode(code);
  console.log({
    name: user.name,
    photo: user.picture,
    email: user.email,
    username: user.username,
    mobile: user.mobile,
  });
}
