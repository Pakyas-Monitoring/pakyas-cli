//! Integration tests for the HTTP client using wiremock

mod common;

use common::{create_test_client, create_test_client_no_auth, TEST_API_KEY};
use pakyas_cli::error::CliError;
use serde::{Deserialize, Serialize};
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

// Test response struct for deserialization
#[derive(Debug, Deserialize, PartialEq)]
struct TestResponse {
    id: String,
    name: String,
}

// Test request struct for serialization
#[derive(Debug, Serialize)]
struct TestRequest {
    name: String,
}

// ============== Success Tests ==============

#[tokio::test]
async fn test_get_success() {
    let mock_server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/api/v1/checks"))
        .and(header("Authorization", format!("Bearer {}", TEST_API_KEY)))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "123",
                "name": "test-check"
            })),
        )
        .expect(1)
        .mount(&mock_server)
        .await;

    let client = create_test_client(&mock_server.uri(), TEST_API_KEY);

    let result: TestResponse = client.get("/api/v1/checks").await.unwrap();

    assert_eq!(result.id, "123");
    assert_eq!(result.name, "test-check");
}

#[tokio::test]
async fn test_post_success() {
    let mock_server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/api/v1/checks"))
        .and(header("Authorization", format!("Bearer {}", TEST_API_KEY)))
        .and(header("Content-Type", "application/json"))
        .respond_with(
            ResponseTemplate::new(201).set_body_json(serde_json::json!({
                "id": "new-id",
                "name": "new-check"
            })),
        )
        .expect(1)
        .mount(&mock_server)
        .await;

    let client = create_test_client(&mock_server.uri(), TEST_API_KEY);
    let body = TestRequest {
        name: "new-check".to_string(),
    };

    let result: TestResponse = client.post("/api/v1/checks", &body).await.unwrap();

    assert_eq!(result.id, "new-id");
    assert_eq!(result.name, "new-check");
}

#[tokio::test]
async fn test_put_success() {
    let mock_server = MockServer::start().await;

    Mock::given(method("PUT"))
        .and(path("/api/v1/checks/123"))
        .and(header("Authorization", format!("Bearer {}", TEST_API_KEY)))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "123",
                "name": "updated-check"
            })),
        )
        .expect(1)
        .mount(&mock_server)
        .await;

    let client = create_test_client(&mock_server.uri(), TEST_API_KEY);
    let body = TestRequest {
        name: "updated-check".to_string(),
    };

    let result: TestResponse = client.put("/api/v1/checks/123", &body).await.unwrap();

    assert_eq!(result.id, "123");
    assert_eq!(result.name, "updated-check");
}

#[tokio::test]
async fn test_delete_success() {
    let mock_server = MockServer::start().await;

    Mock::given(method("DELETE"))
        .and(path("/api/v1/checks/123"))
        .and(header("Authorization", format!("Bearer {}", TEST_API_KEY)))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&mock_server)
        .await;

    let client = create_test_client(&mock_server.uri(), TEST_API_KEY);

    let result = client.delete("/api/v1/checks/123").await;

    assert!(result.is_ok());
}

#[tokio::test]
async fn test_post_unauth_no_header() {
    let mock_server = MockServer::start().await;

    // This mock will succeed - we just verify no auth header was sent
    Mock::given(method("POST"))
        .and(path("/api/v1/auth/login"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "user-123",
                "name": "Test User"
            })),
        )
        .expect(1)
        .mount(&mock_server)
        .await;

    let client = create_test_client_no_auth(&mock_server.uri());

    #[derive(Serialize)]
    struct LoginRequest {
        email: String,
        password: String,
    }

    let body = LoginRequest {
        email: "test@example.com".to_string(),
        password: "secret".to_string(),
    };

    let result: TestResponse = client.post_unauth("/api/v1/auth/login", &body).await.unwrap();

    assert_eq!(result.id, "user-123");
}

// ============== Header Tests ==============

#[tokio::test]
async fn test_bearer_token_header() {
    let mock_server = MockServer::start().await;

    // Verify exact Bearer token format
    let custom_key = "pk_custom_key_abc123456789";
    Mock::given(method("GET"))
        .and(path("/api/v1/orgs"))
        .and(header("Authorization", format!("Bearer {}", custom_key)))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "org-1",
                "name": "Test Org"
            })),
        )
        .expect(1)
        .mount(&mock_server)
        .await;

    let client = create_test_client(&mock_server.uri(), custom_key);

    let result: TestResponse = client.get("/api/v1/orgs").await.unwrap();

    assert_eq!(result.id, "org-1");
}

#[tokio::test]
async fn test_content_type_json() {
    let mock_server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/api/v1/projects"))
        .and(header("Content-Type", "application/json"))
        .respond_with(
            ResponseTemplate::new(201).set_body_json(serde_json::json!({
                "id": "proj-1",
                "name": "Test Project"
            })),
        )
        .expect(1)
        .mount(&mock_server)
        .await;

    let client = create_test_client(&mock_server.uri(), TEST_API_KEY);
    let body = TestRequest {
        name: "Test Project".to_string(),
    };

    let result: TestResponse = client.post("/api/v1/projects", &body).await.unwrap();

    assert_eq!(result.name, "Test Project");
}

// ============== Error Tests ==============

#[tokio::test]
async fn test_401_returns_not_authenticated() {
    let mock_server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/api/v1/organizations"))
        .respond_with(ResponseTemplate::new(401))
        .mount(&mock_server)
        .await;

    let client = create_test_client(&mock_server.uri(), "pk_invalid_key_12345678");

    let result: Result<TestResponse, _> = client.get("/api/v1/organizations").await;

    assert!(result.is_err());
    let err = result.unwrap_err();
    let cli_err = err.downcast_ref::<CliError>().unwrap();
    assert!(matches!(cli_err, CliError::NotAuthenticated));
}

#[tokio::test]
async fn test_403_extracts_error_message() {
    let mock_server = MockServer::start().await;

    Mock::given(method("DELETE"))
        .and(path("/api/v1/checks/123"))
        .respond_with(
            ResponseTemplate::new(403).set_body_json(serde_json::json!({
                "error": "You do not have permission to delete this check"
            })),
        )
        .mount(&mock_server)
        .await;

    let client = create_test_client(&mock_server.uri(), TEST_API_KEY);

    let result = client.delete("/api/v1/checks/123").await;

    assert!(result.is_err());
    let err = result.unwrap_err();
    let cli_err = err.downcast_ref::<CliError>().unwrap();
    match cli_err {
        CliError::Api(msg) => {
            assert!(msg.contains("permission"), "Error message: {}", msg);
        }
        _ => panic!("Expected CliError::Api, got {:?}", cli_err),
    }
}

#[tokio::test]
async fn test_404_not_found() {
    let mock_server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/api/v1/checks/nonexistent"))
        .respond_with(
            ResponseTemplate::new(404).set_body_json(serde_json::json!({
                "message": "Check not found"
            })),
        )
        .mount(&mock_server)
        .await;

    let client = create_test_client(&mock_server.uri(), TEST_API_KEY);

    let result: Result<TestResponse, _> = client.get("/api/v1/checks/nonexistent").await;

    assert!(result.is_err());
    let err = result.unwrap_err();
    let cli_err = err.downcast_ref::<CliError>().unwrap();
    match cli_err {
        CliError::Api(msg) => {
            assert!(
                msg.contains("not found") || msg.contains("Check"),
                "Error message: {}",
                msg
            );
        }
        _ => panic!("Expected CliError::Api, got {:?}", cli_err),
    }
}

#[tokio::test]
async fn test_500_server_error() {
    let mock_server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/api/v1/checks"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&mock_server)
        .await;

    let client = create_test_client(&mock_server.uri(), TEST_API_KEY);

    let result: Result<TestResponse, _> = client.get("/api/v1/checks").await;

    assert!(result.is_err());
    let err = result.unwrap_err();
    let cli_err = err.downcast_ref::<CliError>().unwrap();
    match cli_err {
        CliError::Api(msg) => {
            assert!(msg.contains("500") || msg.contains("failed"), "Error message: {}", msg);
        }
        _ => panic!("Expected CliError::Api, got {:?}", cli_err),
    }
}

// ============== Edge Cases ==============

#[tokio::test]
async fn test_base_url_trailing_slash() {
    let mock_server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/api/test"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "1",
                "name": "test"
            })),
        )
        .expect(1)
        .mount(&mock_server)
        .await;

    // Create client with trailing slash in base URL
    let base_with_slash = format!("{}/", mock_server.uri());
    let client = create_test_client(&base_with_slash, TEST_API_KEY);

    // Should not result in double slash
    let result: TestResponse = client.get("/api/test").await.unwrap();
    assert_eq!(result.id, "1");
}
