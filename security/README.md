# AEGIS Security Module

Production-grade security layer providing authentication, authorization, rate limiting, and encrypted communication for distributed AI inference systems.

## Features

### 1. API Key Authentication

- Generate cryptographically secure API keys
- Hash-based storage (SHA256)
- Key metadata tracking (created_at, expires_at, revoked)
- Permission-based access control
- API key rotation mechanism
- Per-key quota enforcement

```rust
let provider = ApiKeyProvider::new();
let (key, metadata) = provider.generate_key(
    "user1".to_string(),
    "org1".to_string(),
    vec!["read".to_string(), "write".to_string()],
    Some(30), // expires in 30 days
)?;
```

### 2. JWT Token Validation

- Create and validate JWT tokens
- Configurable expiration
- Token refresh capability
- Clock skew tolerance
- Custom claims support

```rust
let jwt = JwtProvider::new(JwtConfig {
    secret: "your-secret".to_string(),
    issuer: "aegis".to_string(),
    expiration_secs: 3600,
    clock_skew_secs: 60,
})?;

let token = jwt.issue_token(&principal)?;
let validated = jwt.validate_token(&token)?;
```

### 3. Rate Limiting

Token bucket rate limiter with three levels:

- **Global**: Limit total requests to system
- **Per-API-Key**: Limit requests from specific API key
- **Per-IP**: Limit requests from specific IP address

```rust
let limiter = RateLimiter::new(RateLimiterConfig {
    global_rps: 10000,
    per_key_rps: 1000,
    per_ip_rps: 100,
    burst_size: 100,
    enable_per_key: true,
    enable_per_ip: true,
});

// Check before processing request
limiter.check(Some(&api_key_id), Some(&ip_address))?;
```

### 4. TLS/mTLS Support

- TLS server configuration
- Mutual TLS (mTLS) for client authentication
- Certificate validation
- Certificate expiration checking
- Secure communication endpoints

```rust
let tls_config = TlsConfig::new(
    PathBuf::from("/etc/ssl/cert.pem"),
    PathBuf::from("/etc/ssl/key.pem"),
).with_mtls(PathBuf::from("/etc/ssl/ca.pem"));

let server_config = TlsServerConfig::load(&tls_config)?;
```

### 5. Authorization & Permissions

Role-based access control (RBAC) via permission checks:

```rust
let principal = Principal {
    id: "user1".to_string(),
    permissions: vec!["read".to_string(), "write".to_string()],
    org_id: Some("org1".to_string()),
    // ...
};

// Single permission check
if !principal.has_permission("read") {
    return Err(SecurityError::InsufficientPermissions);
}

// Multiple permission checks
if !principal.has_all_permissions(&["read", "write"]) {
    return Err(SecurityError::InsufficientPermissions);
}
```

## Architecture

```
┌─────────────────────────────────────┐
│  API Gateway / HTTP Handler         │
├─────────────────────────────────────┤
│  Security Middleware                │
│  ├─ Extract Credentials             │
│  ├─ Authenticate (API Key / JWT)    │
│  ├─ Check Rate Limit                │
│  └─ Verify Permissions              │
├─────────────────────────────────────┤
│  TLS Layer                          │
│  ├─ Server Certificate              │
│  ├─ mTLS Client Verification        │
│  └─ Encrypted Communication         │
├─────────────────────────────────────┤
│  Application Code                   │
│  (with authenticated principal)     │
└─────────────────────────────────────┘
```

## Usage Patterns

### Pattern 1: API Key-Based Access

```rust
use security::{ApiKeyProvider, Credential};

// Setup
let api_key_provider = ApiKeyProvider::new();
let (key, metadata) = api_key_provider.generate_key(
    "service1".to_string(),
    "prod".to_string(),
    vec!["infer".to_string()],
    Some(365),
)?;

println!("API Key: {}", key);
println!("Key ID: {}", metadata.id);

// Usage in request
let credential = Credential::ApiKey(key);
let principal = api_key_provider.authenticate(&credential).await?;

// Verify permissions
if principal.has_permission("infer") {
    // Proceed with inference
}
```

### Pattern 2: JWT Token-Based Access

```rust
use security::{JwtProvider, Credential, JwtConfig};

// Setup
let jwt = JwtProvider::new(JwtConfig::default())?;

// Issue token
let token = jwt.issue_token(&principal)?;

// Later, validate token
let credential = Credential::Bearer(token);
let authenticated = jwt.authenticate(&credential).await?;

// Refresh token
let new_token = jwt.refresh_token(&token)?;
```

### Pattern 3: Multi-Auth (API Key + JWT)

```rust
use security::MultiAuthProvider;

let api_key = Arc::new(ApiKeyProvider::new());
let jwt = Arc::new(JwtProvider::new(JwtConfig::default())?);

let multi_auth = MultiAuthProvider::new()
    .with_api_key(api_key)
    .with_jwt(jwt);

// Try to authenticate - will work with either API key or JWT
let principal = multi_auth.authenticate(&credential).await?;
```

### Pattern 4: Rate Limiting in Middleware

```rust
use security::RateLimiter;

let limiter = RateLimiter::new(RateLimiterConfig::default());

// In request handler
async fn handle_inference(
    api_key_id: String,
    ip_address: String,
    request: InferenceRequest,
) -> Result<InferenceResponse> {
    // Check rate limits
    limiter.check(Some(&api_key_id), Some(&ip_address))?;

    // Process inference
    // ...
}

// View statistics
let stats = limiter.stats();
println!("Rejected: {}", stats.rejected_requests);
```

### Pattern 5: Permission-Based Access Control

```rust
// Protect sensitive operations
async fn delete_model(principal: &Principal, model_id: &str) -> Result<()> {
    // Require admin permission
    if !principal.has_permission("admin") {
        return Err(SecurityError::InsufficientPermissions);
    }

    // Proceed with deletion
    // ...

    Ok(())
}
```

## Configuration

### API Key Configuration

No configuration needed - keys are generated at runtime and stored in memory (extend to database for production).

### JWT Configuration

```rust
let config = JwtConfig {
    secret: std::env::var("JWT_SECRET")?,
    issuer: "aegis".to_string(),
    expiration_secs: 3600,       // 1 hour
    clock_skew_secs: 60,
};
```

### Rate Limiter Configuration

```yaml
# security.yaml
rate_limiter:
  global_rps: 10000           # 10k requests/sec globally
  per_key_rps: 1000           # 1k requests/sec per API key
  per_ip_rps: 100             # 100 requests/sec per IP
  burst_size: 100             # Allow 100 burst tokens
  enable_per_key: true
  enable_per_ip: true
```

### TLS Configuration

```yaml
# tls.yaml
tls:
  cert_path: /etc/ssl/aegis/cert.pem
  key_path: /etc/ssl/aegis/key.pem
  mutual_tls: true
  ca_cert_path: /etc/ssl/aegis/ca.pem
  min_version: "1.2"
  cipher_suites: []            # Use defaults
```

## API Key Management

### Generate Key

```rust
let (key, metadata) = api_key_provider.generate_key(
    "user@example.com".to_string(),
    "production".to_string(),
    vec!["read".to_string(), "write".to_string()],
    Some(90), // 90 days
)?;
```

### Validate Key

```rust
let metadata = api_key_provider.validate_key(&key)?;
println!("Valid key for: {}", metadata.owner);
```

### Revoke Key

```rust
api_key_provider.revoke_key(&key_id, Some("Compromised".to_string()))?;
```

### Rotate Key

```rust
let (new_key, new_metadata) = api_key_provider.rotate_key(
    &old_key_id,
    Some(vec!["read".to_string()]), // Optionally update permissions
)?;
```

### List Keys

```rust
let keys = api_key_provider.list_keys("production");
for key in keys {
    println!("{}: {}", key.id, key.name);
}
```

## Rate Limiter Metrics

```rust
let stats = limiter.stats();
println!("Global requests: {}", stats.global_requests);
println!("Active key limiters: {}", stats.key_limiters_count);
println!("Active IP limiters: {}", stats.ip_limiters_count);
println!("Rejected requests: {}", stats.rejected_requests);

// Per-key stats
if let Some(key_stats) = limiter.key_stats(&api_key_id) {
    println!("Key requests: {}", key_stats.requests);
}
```

## Security Best Practices

1. **Secret Management**
   - Store JWT secrets in environment variables
   - Rotate secrets periodically
   - Use strong secrets (32+ random bytes)

2. **Key Rotation**
   - Rotate API keys every 90 days
   - Use automated rotation for service accounts
   - Revoke leaked keys immediately

3. **TLS/mTLS**
   - Use TLS 1.2+ for all communication
   - Enable mTLS for service-to-service communication
   - Monitor certificate expiration
   - Keep certificates fresh (<30 days before expiration)

4. **Rate Limiting**
   - Set reasonable per-key limits based on expected usage
   - Monitor rejection rates for DDoS detection
   - Increase limits for privileged operations gradually
   - Use IP-based limits as secondary defense

5. **Permissions**
   - Follow principle of least privilege
   - Grant minimum required permissions
   - Review permissions periodically
   - Audit permission changes

6. **Error Handling**
   - Don't reveal auth failure reasons to clients
   - Log detailed error info server-side only
   - Return generic 401/403 errors to clients
   - Monitor auth failure patterns for attacks

## Integration Points

### With API Gateway

```rust
// In gateway middleware
async fn auth_middleware(request: HttpRequest) -> Result<Principal> {
    // Extract credential from header
    let credential = extract_credential(&request)?;

    // Authenticate
    let principal = multi_auth.authenticate(&credential).await?;

    // Check rate limit
    let ip = request.remote_addr();
    limiter.check(principal.api_key_id.as_deref(), Some(ip))?;

    Ok(principal)
}
```

### With Observability

```rust
// Record security metrics
METRICS.record_authentication_attempt("api_key", success);
METRICS.record_rate_limit_rejection(api_key_id);
METRICS.record_authorization_denied(principal, resource);
```

### With Inference Backend

```rust
// Verify access before inference
async fn infer(principal: Principal, request: InferenceRequest) -> Result<Response> {
    // Check permission
    if !principal.has_permission("infer") {
        return Err(SecurityError::InsufficientPermissions);
    }

    // Check rate limit
    limiter.check(principal.api_key_id.as_deref(), None)?;

    // Proceed with inference
    backend.infer(&request).await
}
```

## Testing

```bash
# Run tests
cargo test --release

# Test API key generation
cargo test test_api_key_generation -- --nocapture

# Test JWT validation
cargo test test_jwt_creation_and_validation -- --nocapture

# Test rate limiting
cargo test test_rate_limiter -- --nocapture
```

## Performance

- **API Key Validation**: ~0.1ms (hash lookup)
- **JWT Validation**: ~0.5-1ms (signature verification)
- **Rate Limit Check**: ~0.05ms (atomic operation)
- **Authorization Check**: ~0.01ms (vector search)

## Troubleshooting

### "Invalid API key"
- Verify key is correct and not revoked
- Check key hasn't expired
- Ensure key is for correct organization

### "Token expired"
- Token has exceeded expiration time
- Use `refresh()` to get new token
- Check system clock synchronization

### "Rate limit exceeded"
- Request rate exceeds configured limit
- Wait before retrying (backoff recommended)
- Check if limit needs adjustment
- Contact support for quota increase

### "Insufficient permissions"
- Principal doesn't have required permission
- Request new permission from admin
- Check if using correct API key/token

## Next Steps

1. Set up API key management service
2. Implement audit logging for security events
3. Add OAuth/OpenID Connect support
4. Implement API key scopes for fine-grained access
5. Add IP whitelisting/blacklisting
6. Implement two-factor authentication
7. Set up security alerts and monitoring
8. Create certificate rotation automation

## References

- [OWASP Authentication Cheat Sheet](https://cheatsheetseries.owasp.org/cheatsheets/Authentication_Cheat_Sheet.html)
- [JWT Best Practices](https://tools.ietf.org/html/rfc8725)
- [TLS 1.3 Specifications](https://tools.ietf.org/html/rfc8446)
- [NIST Cybersecurity Framework](https://www.nist.gov/cyberframework)
