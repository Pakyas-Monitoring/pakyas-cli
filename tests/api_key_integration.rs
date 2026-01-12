//! Integration tests for API key commands
//! Tests that response types correctly deserialize server response formats

mod common;

use chrono::{DateTime, Utc};
use common::{TEST_API_KEY, create_test_client};
use serde::Deserialize;
use uuid::Uuid;
use wiremock::matchers::{header, method, path, path_regex};
use wiremock::{Mock, MockServer, ResponseTemplate};

// ============================================================================
// Response types (must match server format)
// ============================================================================

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
enum ApiKeyScope {
    Read,
    Write,
    Manage,
}

#[derive(Debug, Deserialize)]
struct ListApiKeysResponse {
    api_keys: Vec<ApiKeyResponse>,
    total: usize,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct ApiKeyResponse {
    id: Uuid,
    name: String,
    key_prefix: String,
    scopes: Vec<ApiKeyScope>,
    project_access: String,
    allowed_project_ids: Vec<Uuid>,
    expires_at: Option<DateTime<Utc>>,
    last_used_at: Option<DateTime<Utc>>,
    created_at: DateTime<Utc>,
    is_expired: bool,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct ApiKeyCreated {
    id: Uuid,
    name: String,
    key: String,
    key_prefix: String,
    scopes: Vec<ApiKeyScope>,
    expires_at: Option<DateTime<Utc>>,
    created_at: DateTime<Utc>,
}

// ============================================================================
// List API Keys Tests
// ============================================================================

#[tokio::test]
async fn test_list_api_keys_deserializes_correctly() {
    let mock_server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path_regex("/api/v1/api-keys.*"))
        .and(header("Authorization", format!("Bearer {}", TEST_API_KEY)))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "api_keys": [{
                "id": "550e8400-e29b-41d4-a716-446655440000",
                "name": "Test Key",
                "key_prefix": "a1b2c3d4",
                "scopes": ["read", "write"],
                "project_access": "all",
                "allowed_project_ids": [],
                "expires_at": null,
                "last_used_at": "2024-01-15T10:30:00Z",
                "created_at": "2024-01-01T00:00:00Z",
                "is_expired": false
            }],
            "total": 1
        })))
        .expect(1)
        .mount(&mock_server)
        .await;

    let client = create_test_client(&mock_server.uri(), TEST_API_KEY);
    let result: ListApiKeysResponse = client
        .get("/api/v1/api-keys?org_id=550e8400-e29b-41d4-a716-446655440000")
        .await
        .unwrap();

    assert_eq!(result.total, 1);
    assert_eq!(result.api_keys.len(), 1);

    let key = &result.api_keys[0];
    assert_eq!(key.name, "Test Key");
    assert_eq!(key.key_prefix, "a1b2c3d4");
    assert_eq!(key.scopes, vec![ApiKeyScope::Read, ApiKeyScope::Write]);
    assert_eq!(key.project_access, "all");
    assert!(key.allowed_project_ids.is_empty());
    assert!(!key.is_expired);
}

#[tokio::test]
async fn test_list_api_keys_with_all_scopes() {
    let mock_server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path_regex("/api/v1/api-keys.*"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "api_keys": [{
                "id": "550e8400-e29b-41d4-a716-446655440001",
                "name": "Full Access Key",
                "key_prefix": "e5f6g7h8",
                "scopes": ["read", "write", "manage"],
                "project_access": "all",
                "allowed_project_ids": [],
                "expires_at": "2025-01-01T00:00:00Z",
                "last_used_at": null,
                "created_at": "2024-01-01T00:00:00Z",
                "is_expired": false
            }],
            "total": 1
        })))
        .mount(&mock_server)
        .await;

    let client = create_test_client(&mock_server.uri(), TEST_API_KEY);
    let result: ListApiKeysResponse = client
        .get("/api/v1/api-keys?org_id=550e8400-e29b-41d4-a716-446655440000")
        .await
        .unwrap();

    let key = &result.api_keys[0];
    assert_eq!(
        key.scopes,
        vec![ApiKeyScope::Read, ApiKeyScope::Write, ApiKeyScope::Manage]
    );
    assert!(key.expires_at.is_some());
    assert!(key.last_used_at.is_none());
}

#[tokio::test]
async fn test_list_api_keys_with_project_restrictions() {
    let mock_server = MockServer::start().await;

    let project_id_1 = "660e8400-e29b-41d4-a716-446655440001";
    let project_id_2 = "660e8400-e29b-41d4-a716-446655440002";

    Mock::given(method("GET"))
        .and(path_regex("/api/v1/api-keys.*"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "api_keys": [{
                "id": "550e8400-e29b-41d4-a716-446655440002",
                "name": "Limited Key",
                "key_prefix": "i9j0k1l2",
                "scopes": ["read"],
                "project_access": "selected",
                "allowed_project_ids": [project_id_1, project_id_2],
                "expires_at": null,
                "last_used_at": "2024-06-15T12:00:00Z",
                "created_at": "2024-01-01T00:00:00Z",
                "is_expired": false
            }],
            "total": 1
        })))
        .mount(&mock_server)
        .await;

    let client = create_test_client(&mock_server.uri(), TEST_API_KEY);
    let result: ListApiKeysResponse = client
        .get("/api/v1/api-keys?org_id=550e8400-e29b-41d4-a716-446655440000")
        .await
        .unwrap();

    let key = &result.api_keys[0];
    assert_eq!(key.project_access, "selected");
    assert_eq!(key.allowed_project_ids.len(), 2);
}

#[tokio::test]
async fn test_list_api_keys_empty_list() {
    let mock_server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path_regex("/api/v1/api-keys.*"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "api_keys": [],
            "total": 0
        })))
        .mount(&mock_server)
        .await;

    let client = create_test_client(&mock_server.uri(), TEST_API_KEY);
    let result: ListApiKeysResponse = client
        .get("/api/v1/api-keys?org_id=550e8400-e29b-41d4-a716-446655440000")
        .await
        .unwrap();

    assert_eq!(result.total, 0);
    assert!(result.api_keys.is_empty());
}

// ============================================================================
// Create API Key Tests
// ============================================================================

#[tokio::test]
async fn test_create_api_key_deserializes_correctly() {
    let mock_server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/api/v1/api-keys"))
        .and(header("Authorization", format!("Bearer {}", TEST_API_KEY)))
        .and(header("Content-Type", "application/json"))
        .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({
            "id": "550e8400-e29b-41d4-a716-446655440003",
            "name": "New API Key",
            "key": "pk_live_abcdefghijklmnopqrstuvwxyz123456",
            "key_prefix": "abcdefgh",
            "scopes": ["read", "write"],
            "expires_at": "2025-06-01T00:00:00Z",
            "created_at": "2024-01-15T10:30:00Z"
        })))
        .expect(1)
        .mount(&mock_server)
        .await;

    let client = create_test_client(&mock_server.uri(), TEST_API_KEY);

    #[derive(serde::Serialize)]
    struct CreateRequest {
        org_id: Uuid,
        name: String,
        scopes: Vec<String>,
        expires_in_days: Option<i64>,
    }

    let request = CreateRequest {
        org_id: Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap(),
        name: "New API Key".to_string(),
        scopes: vec!["read".to_string(), "write".to_string()],
        expires_in_days: Some(30),
    };

    let result: ApiKeyCreated = client.post("/api/v1/api-keys", &request).await.unwrap();

    assert_eq!(result.name, "New API Key");
    assert_eq!(result.key, "pk_live_abcdefghijklmnopqrstuvwxyz123456");
    assert_eq!(result.key_prefix, "abcdefgh");
    assert_eq!(result.scopes, vec![ApiKeyScope::Read, ApiKeyScope::Write]);
    assert!(result.expires_at.is_some());
}

#[tokio::test]
async fn test_create_api_key_no_expiration() {
    let mock_server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/api/v1/api-keys"))
        .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({
            "id": "550e8400-e29b-41d4-a716-446655440004",
            "name": "Non-Expiring Key",
            "key": "pk_live_xyz789",
            "key_prefix": "xyz78900",
            "scopes": ["read"],
            "expires_at": null,
            "created_at": "2024-01-15T10:30:00Z"
        })))
        .mount(&mock_server)
        .await;

    let client = create_test_client(&mock_server.uri(), TEST_API_KEY);

    #[derive(serde::Serialize)]
    struct CreateRequest {
        org_id: Uuid,
        name: String,
        scopes: Vec<String>,
    }

    let request = CreateRequest {
        org_id: Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap(),
        name: "Non-Expiring Key".to_string(),
        scopes: vec!["read".to_string()],
    };

    let result: ApiKeyCreated = client.post("/api/v1/api-keys", &request).await.unwrap();

    assert_eq!(result.name, "Non-Expiring Key");
    assert!(result.expires_at.is_none());
}

// ============================================================================
// Revoke API Key Tests
// ============================================================================

#[tokio::test]
async fn test_revoke_api_key_returns_no_content() {
    let mock_server = MockServer::start().await;

    Mock::given(method("DELETE"))
        .and(path(
            "/api/v1/api-keys/550e8400-e29b-41d4-a716-446655440000",
        ))
        .and(header("Authorization", format!("Bearer {}", TEST_API_KEY)))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&mock_server)
        .await;

    let client = create_test_client(&mock_server.uri(), TEST_API_KEY);
    let result = client
        .delete("/api/v1/api-keys/550e8400-e29b-41d4-a716-446655440000")
        .await;

    assert!(result.is_ok());
}

// ============================================================================
// Edge Cases
// ============================================================================

#[tokio::test]
async fn test_api_key_with_expired_status() {
    let mock_server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path_regex("/api/v1/api-keys.*"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "api_keys": [{
                "id": "550e8400-e29b-41d4-a716-446655440005",
                "name": "Expired Key",
                "key_prefix": "expired1",
                "scopes": ["read"],
                "project_access": "all",
                "allowed_project_ids": [],
                "expires_at": "2023-01-01T00:00:00Z",
                "last_used_at": "2022-12-31T23:59:59Z",
                "created_at": "2022-01-01T00:00:00Z",
                "is_expired": true
            }],
            "total": 1
        })))
        .mount(&mock_server)
        .await;

    let client = create_test_client(&mock_server.uri(), TEST_API_KEY);
    let result: ListApiKeysResponse = client
        .get("/api/v1/api-keys?org_id=550e8400-e29b-41d4-a716-446655440000")
        .await
        .unwrap();

    let key = &result.api_keys[0];
    assert!(key.is_expired);
    assert!(key.expires_at.is_some());
}
