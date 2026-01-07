//! Shared test utilities for pakyas-cli integration tests

use pakyas_cli::client::ApiClient;

/// Standard test API key for consistency across tests
pub const TEST_API_KEY: &str = "pk_test_key_12345678901234567890";

/// Create a test client pointing to a mock server with authentication
pub fn create_test_client(base_url: &str, api_key: &str) -> ApiClient {
    ApiClient::with_base_url(base_url.to_string(), Some(api_key.to_string()))
        .expect("Failed to create test client")
}

/// Create a test client without authentication (for testing unauthenticated endpoints)
pub fn create_test_client_no_auth(base_url: &str) -> ApiClient {
    ApiClient::with_base_url(base_url.to_string(), None).expect("Failed to create test client")
}
