# Auth and policy handoff

The dispatcher evaluates each call before limits, cache or provider code. All seven schemas stay visible. Modules receive only the frozen CallContext and typed arguments. They must still enforce confirmation ownership, expiry and binding generation inside the booking implementation.

Use Dispatcher::with_security with Security. App installs it automatically. Device integration must supply the same IdentityResolver used by /device/bind through Security.identity; absent a resolver, devices remain unbound. Resolution cannot change azp, auth kind, device ID or add scopes.

## Fallback for the 8 PM gate

Set auth.mode: no_login in school.yaml and restart. Missing credentials become guest; supplied invalid credentials still receive 401. Guests may read Public tools, but cannot propose, confirm or access Sensitive data. Anonymous callers cannot be approved. Inspector can use a valid synthetic student token issued to an approved pre-registered confidential client. Sensitive also requires sensitive_enabled: true and its enabled allow rule. DCR/CIMD/device registrations cannot gain approval.

Offline synthetic signing-key and policy tests pass independently of Keycloak. Actual Public vendor responses depend on merging the module branches; this branch still contains foundation module placeholders. A successful policy decision can therefore return unsupported_source until those implementations arrive.

Live Claude/ChatGPT login, Inspector runs, real-realm discovery/JWKS evidence and voice compatibility are human checks still outstanding. Do not treat offline tests as evidence of those outcomes. Setup instructions and environment names are in deploy/keycloak/README.md and .env.example. The resource server needs KAMPUS_TEMPLE_AUDIT_SALT and configured issuer/audience; it never needs a user's login token as an upstream credential.
