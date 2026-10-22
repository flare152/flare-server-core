//! The Core admin surface as a route contract shared by both of its ends (v4 R3).
//!
//! The Social gateway admin partition resolves `/admin/v1/apps/{app}/<path>` with this table to
//! sign the route's permission code into the Staff context it forwards; Core `admin-gateway`
//! resolves `/api/v1/admin/<path>` with the same table to check that code. One table, so the two
//! cannot drift: a route either exists here with its code, or it is not an admin route.

/// Core-side prefix of the admin surface (`admin-gateway`).
pub const ADMIN_ROOT: &str = "/api/v1/admin";

/// Permission codes of the Core admin surface, as the platform's permission catalog names them.
pub mod permission {
    pub const CONVERSATIONS_READ: &str = "core:conversations:read";
    pub const MESSAGES_READ: &str = "core:messages:read";
    pub const MESSAGES_EXPORT: &str = "core:messages:export";
    pub const USERS_READ: &str = "core:users:read";
    pub const USERS_KICK: &str = "core:users:kick";
    pub const TENANT_READ: &str = "core:tenant:read";
    pub const TENANT_KICK: &str = "core:tenant:kick";
    pub const CAPABILITIES_READ: &str = "core:capabilities:read";
    pub const CAPABILITIES_WRITE: &str = "core:capabilities:write";
    pub const PLUGINS_READ: &str = "core:plugins:read";
    pub const PLUGINS_WRITE: &str = "core:plugins:write";
    pub const HOOKS_READ: &str = "core:hooks:read";
    pub const HOOKS_WRITE: &str = "core:hooks:write";
    pub const MEDIAS_READ: &str = "core:medias:read";
    pub const MEDIAS_WRITE: &str = "core:medias:write";
    pub const GATEWAY_READ: &str = "core:gateway:read";

    /// Every code, for catalog seeding and bundle checks.
    pub const ALL: &[&str] = &[
        CONVERSATIONS_READ,
        MESSAGES_READ,
        MESSAGES_EXPORT,
        USERS_READ,
        USERS_KICK,
        TENANT_READ,
        TENANT_KICK,
        CAPABILITIES_READ,
        CAPABILITIES_WRITE,
        PLUGINS_READ,
        PLUGINS_WRITE,
        HOOKS_READ,
        HOOKS_WRITE,
        MEDIAS_READ,
        MEDIAS_WRITE,
        GATEWAY_READ,
    ];
}

/// Template of the per-plugin staff permission `plugin:<pluginId>:admin` (the plugin protocol's
/// `plugin_admin_permission`). A route with it requires the code built from its `{plugin_id}`
/// parameter, see [`route_permission`]. It is not a `core:` code and not in [`permission::ALL`]:
/// the platform grants it per plugin.
pub const PLUGIN_ADMIN_PERMISSION_TEMPLATE: &str = "plugin:{plugin_id}:admin";
/// Permission codes are at most this long across the signed context, the platform catalog and audit.
pub const MAX_PERMISSION_CODE_BYTES: usize = 128;

/// One admin route: the path pattern is relative to the admin root, `{name}` is one segment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CoreAdminRoute {
    pub method: &'static str,
    pub pattern: &'static str,
    pub permission: &'static str,
    /// Mutates state: the partition requires an audit reason and a correlation key, and the
    /// effect is audited.
    pub write: bool,
    /// Only platform staff (no organization) may call it.
    pub platform_only: bool,
}

const fn read(
    method: &'static str,
    pattern: &'static str,
    permission: &'static str,
) -> CoreAdminRoute {
    CoreAdminRoute {
        method,
        pattern,
        permission,
        write: false,
        platform_only: false,
    }
}

const fn write(
    method: &'static str,
    pattern: &'static str,
    permission: &'static str,
) -> CoreAdminRoute {
    CoreAdminRoute {
        method,
        pattern,
        permission,
        write: true,
        platform_only: false,
    }
}

const fn platform_write(
    method: &'static str,
    pattern: &'static str,
    permission: &'static str,
) -> CoreAdminRoute {
    CoreAdminRoute {
        method,
        pattern,
        permission,
        write: true,
        platform_only: true,
    }
}

use permission::*;

/// The Core admin surface. Query POSTs are reads by contract (the body is a filter).
pub const ROUTES: &[CoreAdminRoute] = &[
    read("GET", "/auth/check", GATEWAY_READ),
    read("GET", "/capabilities", GATEWAY_READ),
    read("GET", "/gateway/health", GATEWAY_READ),
    read("GET", "/gateway/upstreams", GATEWAY_READ),
    read("GET", "/gateway/routes", GATEWAY_READ),
    read("GET", "/gateway/config", GATEWAY_READ),
    read("POST", "/messages/query", MESSAGES_READ),
    read("POST", "/messages/write-ledger/query", MESSAGES_READ),
    write("POST", "/messages/export", MESSAGES_EXPORT),
    read("GET", "/messages/{message_id}", MESSAGES_READ),
    read("GET", "/messages/{message_id}/events", MESSAGES_READ),
    read("GET", "/conversations", CONVERSATIONS_READ),
    read(
        "GET",
        "/conversations/{conversation_id}",
        CONVERSATIONS_READ,
    ),
    read(
        "GET",
        "/conversations/{conversation_id}/participants",
        CONVERSATIONS_READ,
    ),
    read("GET", "/users/{user_id}/devices", USERS_READ),
    read("GET", "/users/{user_id}/presence", USERS_READ),
    write("POST", "/users/{user_id}/kick", USERS_KICK),
    write(
        "POST",
        "/users/{user_id}/devices/{device_id}/kick",
        USERS_KICK,
    ),
    write("POST", "/tenant/kick-all", TENANT_KICK),
    read("GET", "/tenant/runtime", TENANT_READ),
    read("GET", "/capabilities/catalog", CAPABILITIES_READ),
    read("GET", "/capabilities/tenant", CAPABILITIES_READ),
    write(
        "PUT",
        "/capabilities/tenant/{capability_id}",
        CAPABILITIES_WRITE,
    ),
    read("GET", "/plugins", PLUGINS_READ),
    platform_write("POST", "/plugins/register", PLUGINS_WRITE),
    platform_write("POST", "/plugins/deregister", PLUGINS_WRITE),
    read("GET", "/hooks", HOOKS_READ),
    write("POST", "/hooks", HOOKS_WRITE),
    read("GET", "/hooks/{hook_id}", HOOKS_READ),
    write("PUT", "/hooks/{hook_id}", HOOKS_WRITE),
    write("DELETE", "/hooks/{hook_id}", HOOKS_WRITE),
    write("PUT", "/hooks/{hook_id}/status", HOOKS_WRITE),
    read("GET", "/hooks/{hook_id}/statistics", HOOKS_READ),
    read("GET", "/hooks/{hook_id}/executions", HOOKS_READ),
    read("GET", "/medias/objects", MEDIAS_READ),
    read("GET", "/medias/bucket", MEDIAS_READ),
    write("POST", "/medias/cleanup-orphaned-assets", MEDIAS_WRITE),
    write("POST", "/medias/object-acl", MEDIAS_WRITE),
    // Plugin admin pages (Capability `dispatch_admin`): reads (rows sources, details) and actions.
    read(
        "POST",
        "/plugin-admin/{plugin_id}/query",
        PLUGIN_ADMIN_PERMISSION_TEMPLATE,
    ),
    write(
        "POST",
        "/plugin-admin/{plugin_id}/dispatch",
        PLUGIN_ADMIN_PERMISSION_TEMPLATE,
    ),
];

/// Path parameters are object ids: ASCII letters, digits, `_`, `-`, `.`, `:`; at most 128 bytes.
pub fn valid_param(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.' | b':'))
}

/// The route for `method` and `path` (relative to the admin root, with or without a leading
/// slash). `None` is the surface's 404: unknown route, unsafe path or an invalid parameter.
pub fn resolve(method: &str, path: &str) -> Option<&'static CoreAdminRoute> {
    if path.len() > 2048
        || !path.is_ascii()
        || path.contains("//")
        || path.contains('%')
        || path.contains('\\')
        || path.contains('?')
    {
        return None;
    }
    let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    if segments.is_empty() {
        return None;
    }
    ROUTES.iter().find(|route| {
        route.method.eq_ignore_ascii_case(method) && matches(route.pattern, &segments)
    })
}

/// The permission code `route` requires for `path` (a path it resolves): its `permission`, or for a
/// per-plugin route `plugin:<plugin_id>:admin` with the path's plugin id. `None` when the path does
/// not match the route or the code would exceed [`MAX_PERMISSION_CODE_BYTES`]. Both ends of the admin
/// surface sign and check this code, never the template.
pub fn route_permission(route: &CoreAdminRoute, path: &str) -> Option<String> {
    let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    if !matches(route.pattern, &segments) {
        return None;
    }
    if route.permission != PLUGIN_ADMIN_PERMISSION_TEMPLATE {
        return Some(route.permission.to_owned());
    }
    let index = route
        .pattern
        .split('/')
        .filter(|s| !s.is_empty())
        .position(|part| part == "{plugin_id}")?;
    let plugin_id = segments.get(index)?;
    let code = format!("plugin:{plugin_id}:admin");
    (!plugin_id.contains(':') && code.len() <= MAX_PERMISSION_CODE_BYTES).then_some(code)
}

/// [`resolve`] with the code the route requires for this path ([`route_permission`]).
pub fn resolve_with_permission(
    method: &str,
    path: &str,
) -> Option<(&'static CoreAdminRoute, String)> {
    let route = resolve(method, path)?;
    Some((route, route_permission(route, path)?))
}

fn matches(pattern: &str, segments: &[&str]) -> bool {
    let parts: Vec<&str> = pattern.split('/').filter(|s| !s.is_empty()).collect();
    parts.len() == segments.len()
        && parts.iter().zip(segments).all(|(part, segment)| {
            if part.starts_with('{') {
                valid_param(segment)
            } else {
                part == segment
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_path(pattern: &str) -> String {
        pattern
            .split('/')
            .map(|part| if part.starts_with('{') { "x-1" } else { part })
            .collect::<Vec<_>>()
            .join("/")
    }

    #[test]
    fn every_route_resolves_to_itself_and_only_with_its_method() {
        for route in ROUTES {
            let path = sample_path(route.pattern);
            assert_eq!(resolve(route.method, &path), Some(route), "{path}");
            assert_eq!(
                resolve(route.method, path.trim_start_matches('/')),
                Some(route)
            );
            assert_eq!(resolve(route.method, &format!("{path}/")), Some(route));
            assert!(
                resolve("PATCH", &path).is_none(),
                "{path} must not resolve for PATCH"
            );
        }
        assert_eq!(
            resolve("get", "/conversations").unwrap().pattern,
            "/conversations"
        );
    }

    #[test]
    fn unknown_unsafe_and_invalid_parameters_do_not_resolve() {
        for (method, path) in [
            ("GET", ""),
            ("GET", "/"),
            ("GET", "/users"),
            ("GET", "/users/u1"),
            ("GET", "/users/u1/devices/d1"),
            ("GET", "/conversations/c1/participants/extra"),
            ("GET", "/conversations//participants"),
            ("GET", "/messages/%2e%2e/events"),
            ("GET", "/messages/a\\b"),
            ("GET", "/messages/m1?x=1"),
            ("GET", "/users/ /devices"),
            ("GET", "/users/u 1/devices"),
            ("GET", "/users/u/1/devices"),
            ("DELETE", "/conversations/c1"),
            ("POST", "/plugins"),
        ] {
            assert!(resolve(method, path).is_none(), "{method} {path}");
        }
        let long = "x".repeat(129);
        assert!(resolve("GET", &format!("/users/{long}/devices")).is_none());
        let max = "x".repeat(128);
        assert!(resolve("GET", &format!("/users/{max}/devices")).is_some());
    }

    #[test]
    fn write_and_platform_flags_follow_the_contract() {
        for route in ROUTES {
            if route.method == "GET" {
                assert!(!route.write, "{}", route.pattern);
            }
            if route.pattern.ends_with("/query") {
                assert!(!route.write, "{} is a read by contract", route.pattern);
            }
            assert_eq!(
                route.platform_only,
                route.pattern.starts_with("/plugins/"),
                "{}",
                route.pattern
            );
            if route.platform_only {
                assert!(route.write);
            }
        }
        assert!(resolve("POST", "/messages/export").unwrap().write);
        assert!(
            resolve("PUT", "/capabilities/tenant/message.pin")
                .unwrap()
                .write
        );
        assert!(resolve("POST", "/plugins/register").unwrap().platform_only);
    }

    #[test]
    fn every_permission_code_is_used_and_unique() {
        let mut seen = std::collections::BTreeSet::new();
        for code in permission::ALL {
            assert!(seen.insert(*code), "duplicate {code}");
            assert!(code.starts_with("core:"), "{code}");
            assert!(
                ROUTES.iter().any(|route| route.permission == *code),
                "{code} is not used by any route"
            );
        }
        for route in ROUTES {
            assert!(
                permission::ALL.contains(&route.permission)
                    || route.permission == PLUGIN_ADMIN_PERMISSION_TEMPLATE,
                "{}",
                route.pattern
            );
        }
        let mut routes = std::collections::BTreeSet::new();
        for route in ROUTES {
            assert!(
                routes.insert((route.method, route.pattern)),
                "duplicate {} {}",
                route.method,
                route.pattern
            );
        }
    }

    #[test]
    fn plugin_admin_routes_require_the_plugins_own_code() {
        let (route, code) =
            resolve_with_permission("POST", "/plugin-admin/flare.poll/query").unwrap();
        assert_eq!(route.permission, PLUGIN_ADMIN_PERMISSION_TEMPLATE);
        assert!(!route.write && !route.platform_only);
        assert_eq!(code, "plugin:flare.poll:admin");
        let (route, code) =
            resolve_with_permission("POST", "plugin-admin/flare.poll/dispatch/").unwrap();
        assert!(route.write && !route.platform_only);
        assert_eq!(code, "plugin:flare.poll:admin");
        // Ordinary routes keep their code.
        assert_eq!(
            resolve_with_permission("GET", "/plugins").unwrap().1,
            permission::PLUGINS_READ
        );
        // A plugin id the code cannot carry: `:` or a code over 128 bytes.
        assert!(resolve_with_permission("POST", "/plugin-admin/a:b/query").is_none());
        let longest = "p".repeat(MAX_PERMISSION_CODE_BYTES - "plugin::admin".len());
        assert!(
            resolve_with_permission("POST", &format!("/plugin-admin/{longest}/query")).is_some()
        );
        assert!(
            resolve_with_permission("POST", &format!("/plugin-admin/{longest}x/query")).is_none()
        );
        for (method, path) in [
            ("GET", "/plugin-admin/flare.poll/query"),
            ("POST", "/plugin-admin/flare.poll"),
            ("POST", "/plugin-admin//query"),
        ] {
            assert!(
                resolve_with_permission(method, path).is_none(),
                "{method} {path}"
            );
        }
        // The route table never lets the template itself through as a code.
        let route = resolve("POST", "/plugin-admin/flare.poll/query").unwrap();
        assert!(route_permission(route, "/plugin-admin/other/dispatch").is_none());
    }
}
