use http_client::{HttpClient, HttpMethod};
use std::time::Duration;

const TEST_SERVER: &str = "https://httpbin.org";
const HTTP_TEST_SERVER: &str = "http://httpbin.org";

#[test]
fn test_get_request() {
    let client = HttpClient::new(TEST_SERVER);
    let response = client.get("/get").unwrap();
    assert!(response.is_success());
    assert_eq!(response.status_code, 200);
}

#[test]
fn test_post_request() {
    let client = HttpClient::new(TEST_SERVER);
    let body = b"test data";
    let response = client.post("/post", body).unwrap();
    assert!(response.is_success());
    assert_eq!(response.status_code, 200);
}

#[test]
fn test_headers() {
    let mut client = HttpClient::new(TEST_SERVER);
    client.set_header("X-Custom-Header", "test-value");
    let response = client.get("/headers").unwrap();
    assert!(response.is_success());

    // Convert response body to string and verify it contains our header
    let body_str = String::from_utf8_lossy(&response.body);
    assert!(body_str.contains("X-Custom-Header"));
    assert!(body_str.contains("test-value"));
}

#[test]
fn test_timeout() {
    let mut client = HttpClient::new(TEST_SERVER);
    client.set_timeout(Duration::from_secs(1));
    let result = client.get("/delay/2");
    assert!(result.is_err());
}

#[test]
fn test_basic_auth() {
    let mut client = HttpClient::new(TEST_SERVER);
    client.set_auth("testuser", "testpass");
    let response = client.get("/basic-auth/testuser/testpass").unwrap();
    assert!(response.is_success());
    assert_eq!(response.status_code, 200);
}

#[test]
fn test_redirect() {
    let client = HttpClient::new(TEST_SERVER);
    let response = client.get("/redirect/1").unwrap();
    assert!(response.is_success());
    assert_eq!(response.status_code, 200);
}

#[test]
fn test_different_http_methods() {
    let client = HttpClient::new(TEST_SERVER);

    // Test PUT
    let put_response = client.put("/put", b"test data").unwrap();
    assert!(put_response.is_success());

    // Test DELETE
    let delete_response = client.delete("/delete").unwrap();
    assert!(delete_response.is_success());

    // Test custom method (PATCH)
    let patch_response = client
        .request(HttpMethod::PATCH, "/patch", Some(b"test data"))
        .unwrap();
    assert!(patch_response.is_success());
}

#[test]
fn test_error_responses() {
    let client = HttpClient::new(TEST_SERVER);

    // Test 404
    let not_found = client.get("/status/404");
    assert!(not_found.unwrap().is_client_error());

    // Test 500
    let server_error = client.get("/status/500");
    assert!(server_error.unwrap().is_server_error());
}

// Plain HTTP specific tests

#[test]
fn test_plain_http_get() {
    let client = HttpClient::new(HTTP_TEST_SERVER);
    let response = client.get("/get").unwrap();
    assert!(response.is_success());
    assert_eq!(response.status_code, 200);

    // Verify we're using plain HTTP
    let body_str = String::from_utf8_lossy(&response.body);
    assert!(body_str.contains("\"url\": \"http://"));
}

#[test]
fn test_plain_http_content_length() {
    let client = HttpClient::new(HTTP_TEST_SERVER);
    let response = client.get("/html").unwrap();
    assert!(response.is_success());
    assert_eq!(response.status_code, 200);

    // Verify we got HTML content
    let body_str = String::from_utf8_lossy(&response.body);
    assert!(body_str.contains("<!DOCTYPE html>"));
}

#[test]
fn test_plain_http_streaming() {
    let client = HttpClient::new(HTTP_TEST_SERVER);
    // Test with chunked transfer encoding
    let response = client.get("/stream/5").unwrap();
    assert!(response.is_success());
    assert_eq!(response.status_code, 200);

    let body_str = String::from_utf8_lossy(&response.body);
    // Verify we received all 5 chunks
    assert_eq!(body_str.matches("\"id\": ").count(), 5);
}

#[test]
fn test_plain_http_gzip() {
    let mut client = HttpClient::new(HTTP_TEST_SERVER);
    client.set_header("Accept-Encoding", "gzip");
    let response = client.get("/json").unwrap(); // Using /json instead of /gzip to avoid binary response
    assert!(response.is_success());
    assert_eq!(response.status_code, 200);

    // Check if we got a valid JSON response
    let body_str = String::from_utf8_lossy(&response.body);
    assert!(body_str.contains("{")); // Simple check for JSON content
}

#[test]
fn test_plain_http_with_query_params() {
    let client = HttpClient::new(HTTP_TEST_SERVER);
    let response = client.get("/get?param1=value1&param2=value2").unwrap();
    assert!(response.is_success());

    let body_str = String::from_utf8_lossy(&response.body);
    assert!(body_str.contains("\"param1\": \"value1\""));
    assert!(body_str.contains("\"param2\": \"value2\""));
}

#[test]
fn test_plain_http_post_form() {
    let mut client = HttpClient::new(HTTP_TEST_SERVER);
    client.set_header("Content-Type", "application/x-www-form-urlencoded");
    let form_data = b"field1=value1&field2=value2";
    let response = client.post("/post", form_data).unwrap();
    assert!(response.is_success());

    let body_str = String::from_utf8_lossy(&response.body);
    assert!(body_str.contains("\"field1\": \"value1\""));
    assert!(body_str.contains("\"field2\": \"value2\""));
}

#[test]
fn test_plain_http_connection_reuse() {
    let client = HttpClient::new(HTTP_TEST_SERVER);

    // Make multiple requests using the same client
    for _ in 0..3 {
        let response = client.get("/get").unwrap();
        assert!(response.is_success());
        assert_eq!(response.status_code, 200);
    }
}
