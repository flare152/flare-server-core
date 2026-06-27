use std::collections::HashMap;

use flare_core_infra::auth::{AuthenticatedPrincipal, TokenClaims};

#[test]
fn principal_from_token_claims_is_business_neutral() {
    let claims = TokenClaims {
        sub: "user-a".to_string(),
        iss: "flare-im-core".to_string(),
        exp: 100,
        iat: 1,
        jti: "jwt-id".to_string(),
        device_id: Some("device-a".to_string()),
        tenant_id: Some("tenant-a".to_string()),
    };

    let principal = AuthenticatedPrincipal::from_token_claims(claims);

    assert_eq!(principal.user_id, "user-a");
    assert_eq!(principal.tenant_id.as_deref(), Some("tenant-a"));
    assert_eq!(principal.device_id.as_deref(), Some("device-a"));
    assert_eq!(principal.app_id, None);
    assert_eq!(principal.expires_at, Some(100));
    assert!(principal.scopes.is_empty());
}

#[test]
fn principal_scope_matching_supports_admin_prefix_grants() {
    let mut principal = AuthenticatedPrincipal {
        user_id: "admin-a".to_string(),
        tenant_id: Some("tenant-a".to_string()),
        device_id: None,
        app_id: Some("console-a".to_string()),
        expires_at: None,
        scopes: vec!["core_gateway:admin:*".to_string()],
        metadata: HashMap::new(),
    };

    assert!(principal.has_scope("core_gateway:admin:messages"));
    assert!(principal.has_gateway_admin_scope());

    principal.scopes = vec!["admin_gateway:admin:*".to_string()];
    assert!(principal.has_scope("admin_gateway:admin:messages"));
    assert!(principal.has_gateway_admin_scope());

    principal.scopes = vec!["message:send".to_string()];
    assert!(!principal.has_gateway_admin_scope());
}
