use base64::{Engine as _, engine::general_purpose};
use openssl::ssl::{SslConnector, SslMethod, SslVerifyMode};
use std::collections::HashMap;
use std::error::Error;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::path::PathBuf;
use std::str;
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct ProxyConfig {
    host: String,
    port: u16,
    auth: Option<(String, String)>,
    use_https: bool,
}

impl ProxyConfig {
    pub fn new(host: &str, port: u16) -> Self {
        ProxyConfig {
            host: host.to_string(),
            port,
            auth: None,
            use_https: false,
        }
    }

    pub fn with_auth(mut self, username: &str, password: &str) -> Self {
        self.auth = Some((username.to_string(), password.to_string()));
        self
    }

    pub fn with_https(mut self, use_https: bool) -> Self {
        self.use_https = use_https;
        self
    }
}

#[derive(Debug)]
pub enum HttpMethod {
    GET,
    POST,
    PUT,
    DELETE,
    PATCH,
    HEAD,
    CONNECT,
}

impl ToString for HttpMethod {
    fn to_string(&self) -> String {
        match self {
            HttpMethod::GET => "GET",
            HttpMethod::POST => "POST",
            HttpMethod::PUT => "PUT",
            HttpMethod::DELETE => "DELETE",
            HttpMethod::PATCH => "PATCH",
            HttpMethod::HEAD => "HEAD",
            HttpMethod::CONNECT => "CONNECT",
        }
        .to_string()
    }
}

#[derive(Debug)]
pub enum HttpError {
    ConnectionError(String),
    RequestError(String),
    ResponseError(String),
    TimeoutError(String),
    ParseError(String),
    ProxyError(String),
}

pub struct HttpClient {
    host: String,
    auth: Option<(String, String)>,
    headers: HashMap<String, String>,
    timeout: Option<Duration>,
    max_redirects: u32,
    proxy: Option<ProxyConfig>,
    verify_ssl: bool,
    enable_ipv6: bool,
}

#[derive(Debug)]
pub struct HttpResponse {
    pub status_code: u16,
    pub headers: HashMap<String, String>,
    pub body: Vec<u8>,
    pub content_type: Option<String>,
}

impl HttpClient {
    pub fn new(host: &str) -> Self {
        let clean_host = host.trim_end_matches('/').to_string();

        HttpClient {
            host: clean_host,
            auth: None,
            headers: HashMap::new(),
            timeout: None,
            max_redirects: 5,
            proxy: None,
            verify_ssl: true,
            enable_ipv6: true,
        }
    }

    pub fn set_auth(&mut self, username: &str, password: &str) {
        self.auth = Some((username.to_string(), password.to_string()));
    }

    pub fn set_header(&mut self, key: &str, value: &str) {
        self.headers.insert(key.to_string(), value.to_string());
    }

    pub fn set_timeout(&mut self, timeout: Duration) {
        self.timeout = Some(timeout);
    }

    pub fn set_max_redirects(&mut self, max_redirects: u32) {
        self.max_redirects = max_redirects;
    }

    pub fn set_proxy(&mut self, proxy: ProxyConfig) {
        self.proxy = Some(proxy);
    }

    pub fn set_verify_ssl(&mut self, verify: bool) {
        self.verify_ssl = verify;
    }

    pub fn set_enable_ipv6(&mut self, enable: bool) {
        self.enable_ipv6 = enable;
    }

    pub fn request(
        &self,
        method: HttpMethod,
        path: &str,
        body: Option<&[u8]>,
    ) -> Result<HttpResponse, HttpError> {
        let use_https = self.host.starts_with("https://");
        let host = self
            .host
            .trim_start_matches("http://")
            .trim_start_matches("https://");

        let mut headers = format!(
            "{} {} HTTP/1.1\r\nHost: {}\r\n",
            method.to_string(),
            path,
            host
        );

        if let Some(body_data) = body {
            headers.push_str(&format!("Content-Length: {}\r\n", body_data.len()));
        }

        if let Some((ref username, ref password)) = self.auth {
            let auth = format!("{}:{}", username, password);
            let encoded_auth = general_purpose::STANDARD.encode(auth);
            headers.push_str(&format!("Authorization: Basic {}\r\n", encoded_auth));
        }

        for (key, value) in &self.headers {
            headers.push_str(&format!("{}: {}\r\n", key, value));
        }

        headers.push_str("Connection: close\r\n\r\n");

        let raw_response = if use_https {
            self.send_https(&headers, body, host)?
        } else {
            self.send_http(&headers, body, host)?
        };

        let mut response = self.parse_response(&raw_response)?;

        let mut redirects = 0;
        while let Some(location) = response.headers.get("Location") {
            if redirects >= self.max_redirects {
                return Err(HttpError::RequestError(
                    "Max redirects exceeded".to_string(),
                ));
            }

            if location.starts_with("http") {
                let client = HttpClient::new(location);
                response = client.request(HttpMethod::GET, "/", None)?;
            } else {
                response = self.request(HttpMethod::GET, location, None)?;
            }

            redirects += 1;
        }

        Ok(response)
    }

    fn get_system_cert_path() -> Option<PathBuf> {
        if let Ok(path) = std::env::var("SSL_CERT_FILE") {
            let path_buf = PathBuf::from(path);
            if path_buf.exists() {
                return Some(path_buf);
            }
        }

        if let Ok(dir) = std::env::var("SSL_CERT_DIR") {
            let dir_path = PathBuf::from(dir);
            if dir_path.exists() {
                // Look for common certificate file names in the directory
                for name in ["ca-certificates.crt", "cert.pem", "ca-bundle.crt"] {
                    let cert_path = dir_path.join(name);
                    if cert_path.exists() {
                        return Some(cert_path);
                    }
                }
            }
        }

        // Common Unix-like system locations
        let cert_locations = vec![
            // Debian/Ubuntu/Mint
            PathBuf::from("/etc/ssl/certs/ca-certificates.crt"),
            // RHEL/Fedora/CentOS
            PathBuf::from("/etc/pki/tls/certs/ca-bundle.crt"),
            PathBuf::from("/etc/pki/tls/cacert.pem"),
            // OpenSUSE
            PathBuf::from("/etc/ssl/ca-bundle.pem"),
            // OpenBSD
            PathBuf::from("/etc/ssl/cert.pem"),
            // FreeBSD/BSD
            PathBuf::from("/usr/local/share/certs/ca-root-nss.crt"),
            // MacOS (Homebrew)
            PathBuf::from("/usr/local/etc/openssl/cert.pem"),
            PathBuf::from("/opt/homebrew/etc/openssl@3/cert.pem"),
        ];

        cert_locations.into_iter().find(|path| path.exists())
    }

    pub fn get(&self, path: &str) -> Result<HttpResponse, HttpError> {
        self.request(HttpMethod::GET, path, None)
    }

    pub fn post(&self, path: &str, body: &[u8]) -> Result<HttpResponse, HttpError> {
        self.request(HttpMethod::POST, path, Some(body))
    }

    pub fn put(&self, path: &str, body: &[u8]) -> Result<HttpResponse, HttpError> {
        self.request(HttpMethod::PUT, path, Some(body))
    }

    pub fn delete(&self, path: &str) -> Result<HttpResponse, HttpError> {
        self.request(HttpMethod::DELETE, path, None)
    }

    fn connect(&self, host: &str, port: u16) -> Result<TcpStream, HttpError> {
        let clean_host = host.trim_end_matches('/');
        let addr_str = format!("{}:{}", clean_host, port);

        let addrs: Vec<SocketAddr> = match (clean_host, port).to_socket_addrs() {
            Ok(iter) => iter
                .filter(|addr| {
                    if self.enable_ipv6 {
                        true
                    } else {
                        addr.is_ipv4()
                    }
                })
                .collect(),
            Err(e) => {
                return Err(HttpError::ConnectionError(format!(
                    "DNS resolution failed for {}: {}",
                    addr_str, e
                )));
            }
        };

        if addrs.is_empty() {
            return Err(HttpError::ConnectionError(format!(
                "No {} addresses found for {}",
                if self.enable_ipv6 {
                    "IPv4/IPv6"
                } else {
                    "IPv4"
                },
                addr_str
            )));
        }

        let mut sorted_addrs = addrs;
        if self.enable_ipv6 {
            sorted_addrs.sort_by(|a, b| match (a.is_ipv6(), b.is_ipv6()) {
                (true, false) => std::cmp::Ordering::Less,
                (false, true) => std::cmp::Ordering::Greater,
                _ => std::cmp::Ordering::Equal,
            });
        }

        let timeout = self.timeout.unwrap_or(Duration::from_secs(30));
        let mut errors = Vec::new();

        for addr in sorted_addrs {
            match TcpStream::connect_timeout(&addr, timeout) {
                Ok(stream) => {
                    if let Some(timeout) = self.timeout {
                        let _ = stream.set_read_timeout(Some(timeout));
                        let _ = stream.set_write_timeout(Some(timeout));
                    }

                    if let Err(e) = stream.set_nodelay(true) {
                        eprintln!("Warning: Failed to set TCP_NODELAY: {}", e);
                    }

                    return Ok(stream);
                }
                Err(e) => {
                    errors.push(format!("{}: {}", addr, e));
                }
            }
        }

        Err(HttpError::ConnectionError(format!(
            "Failed to connect to {} (IPv6 {}). Attempted addresses:\n{}",
            addr_str,
            if self.enable_ipv6 {
                "enabled"
            } else {
                "disabled"
            },
            errors.join("\n")
        )))
    }

    fn send_http(
        &self,
        headers: &str,
        body: Option<&[u8]>,
        host: &str,
    ) -> Result<String, HttpError> {
        let (host, port) = if let Some(i) = host.find(':') {
            let (h, p) = host.split_at(i);
            let port = p
                .trim_start_matches(':')
                .parse::<u16>()
                .map_err(|_| HttpError::ParseError("Invalid port number".to_string()))?;
            (h, port)
        } else {
            (host, 80)
        };

        let mut stream = match &self.proxy {
            Some(proxy) if !proxy.use_https => self.connect_proxy(proxy)?,
            _ => self.connect(host, port)?,
        };

        let request = if self.proxy.is_some() {
            let scheme = if self.host.starts_with("https://") {
                "https"
            } else {
                "http"
            };
            headers.replace(" / ", &format!(" {scheme}://{host}/ "))
        } else {
            headers.to_string()
        };

        stream
            .write_all(request.as_bytes())
            .map_err(|e| HttpError::RequestError(e.to_string()))?;

        if let Some(body_data) = body {
            stream
                .write_all(body_data)
                .map_err(|e| HttpError::RequestError(e.to_string()))?;
        }

        let mut response = Vec::new();
        stream
            .read_to_end(&mut response)
            .map_err(|e| HttpError::ResponseError(e.to_string()))?;

        String::from_utf8(response).map_err(|e| HttpError::ParseError(e.to_string()))
    }

    fn send_https(
        &self,
        headers: &str,
        body: Option<&[u8]>,
        host_with_port: &str,
    ) -> Result<String, HttpError> {
        let (host, port) = if let Some(i) = host_with_port.find(':') {
            let (h, p) = host_with_port.split_at(i);
            let port = p
                .trim_start_matches(':')
                .parse::<u16>()
                .map_err(|_| HttpError::ParseError("Invalid port number".to_string()))?;
            (h, port)
        } else {
            (host_with_port, 443)
        };

        let mut builder = SslConnector::builder(SslMethod::tls())
            .map_err(|e| HttpError::ConnectionError(e.to_string()))?;

        if self.verify_ssl {
            if let Some(cert_path) = Self::get_system_cert_path() {
                builder.set_ca_file(&cert_path).map_err(|e| {
                    HttpError::ConnectionError(format!(
                        "Failed to set CA file {}: {}",
                        cert_path.display(),
                        e
                    ))
                })?;
            } else {
                return Err(HttpError::ConnectionError(
                    "Could not find system root certificates. Consider disabling SSL verification for testing.".to_string()
                ));
            }
        } else {
            builder.set_verify(SslVerifyMode::NONE);
        }

        let connector = builder.build();

        let stream = match &self.proxy {
            Some(proxy) => {
                let mut proxy_stream = self.connect_proxy(proxy)?;

                let connect_req = format!(
                    "CONNECT {}:{} HTTP/1.1\r\nHost: {}:{}\r\n\r\n",
                    host, port, host, port
                );
                proxy_stream
                    .write_all(connect_req.as_bytes())
                    .map_err(|e| {
                        HttpError::ProxyError(format!("Failed to send CONNECT request: {}", e))
                    })?;

                let mut response = Vec::new();
                let mut buffer = [0; 1024];
                loop {
                    let n = proxy_stream.read(&mut buffer).map_err(|e| {
                        HttpError::ProxyError(format!("Failed to read proxy response: {}", e))
                    })?;
                    response.extend_from_slice(&buffer[..n]);
                    if response.windows(4).any(|w| w == b"\r\n\r\n") {
                        break;
                    }
                }

                let response_str = String::from_utf8_lossy(&response);
                if !response_str.starts_with("HTTP/1.1 200") {
                    return Err(HttpError::ProxyError(format!(
                        "Proxy CONNECT failed: {}",
                        response_str
                    )));
                }

                proxy_stream
            }
            None => self.connect(host, port)?,
        };

        let mut ssl_stream = connector
            .connect(host_with_port, stream)
            .map_err(|e| HttpError::ConnectionError(e.to_string()))?;

        ssl_stream
            .write_all(headers.as_bytes())
            .map_err(|e| HttpError::RequestError(e.to_string()))?;

        if let Some(body_data) = body {
            ssl_stream
                .write_all(body_data)
                .map_err(|e| HttpError::RequestError(e.to_string()))?;
        }

        let mut response = Vec::new();
        ssl_stream
            .read_to_end(&mut response)
            .map_err(|e| HttpError::ResponseError(e.to_string()))?;

        String::from_utf8(response).map_err(|e| HttpError::ParseError(e.to_string()))
    }

    fn connect_proxy(&self, proxy: &ProxyConfig) -> Result<TcpStream, HttpError> {
        let mut stream = self.connect(&proxy.host, proxy.port)?;

        if let Some((username, password)) = &proxy.auth {
            let auth = format!("{}:{}", username, password);
            let encoded_auth = general_purpose::STANDARD.encode(auth);
            let auth_header = format!("Proxy-Authorization: Basic {}\r\n", encoded_auth);
            stream
                .write_all(auth_header.as_bytes())
                .map_err(|e| HttpError::ProxyError(format!("Failed to send proxy auth: {}", e)))?;
        }

        Ok(stream)
    }

    fn parse_response(&self, raw_response: &str) -> Result<HttpResponse, HttpError> {
        let mut lines = raw_response.lines();
        let status_line = lines
            .next()
            .ok_or_else(|| HttpError::ParseError("Missing status line".to_string()))?;

        let status_code = status_line
            .split_whitespace()
            .nth(1)
            .ok_or_else(|| HttpError::ParseError("Malformed status line".to_string()))?
            .parse::<u16>()
            .map_err(|_| HttpError::ParseError("Invalid status code".to_string()))?;

        let mut headers = HashMap::new();
        let mut content_type = None;

        for line in lines.by_ref() {
            if line.is_empty() {
                break;
            }
            if let Some((key, value)) = line.split_once(": ") {
                if key.to_lowercase() == "content-type" {
                    content_type = Some(value.to_string());
                }
                headers.insert(key.to_string(), value.to_string());
            }
        }

        let body = lines.collect::<Vec<&str>>().join("\n");

        Ok(HttpResponse {
            status_code,
            headers,
            body: body.into_bytes(),
            content_type,
        })
    }
}

impl HttpResponse {
    pub fn is_success(&self) -> bool {
        (200..=299).contains(&self.status_code)
    }

    pub fn is_client_error(&self) -> bool {
        (400..=499).contains(&self.status_code)
    }

    pub fn is_server_error(&self) -> bool {
        (500..=599).contains(&self.status_code)
    }

    pub fn is_error(&self) -> bool {
        self.is_client_error() || self.is_server_error()
    }
}

#[derive(Debug)]
pub struct ParsedUrl {
    pub scheme: String,
    pub host: String,
    pub path: String,
    pub query: Option<String>,
}

pub fn parse_url(url: &str) -> Result<ParsedUrl, Box<dyn Error>> {
    let (scheme, rest) = url
        .split_once("://")
        .ok_or("URL must contain scheme (http:// or https://)")?;

    let (host, path_and_query) = rest
        .find('/')
        .map(|i| rest.split_at(i))
        .unwrap_or((rest, ""));

    let (path, query) = if path_and_query.is_empty() {
        ("/", None)
    } else {
        match path_and_query.split_once('?') {
            Some((p, q)) => (p, Some(q.to_string())),
            None => (path_and_query, None),
        }
    };

    Ok(ParsedUrl {
        scheme: scheme.to_lowercase(),
        host: host.trim_end_matches('/').to_string(),
        path: path.to_string(),
        query: query,
    })
}

pub fn create_client_from_url(url: &str) -> Result<(HttpClient, String), Box<dyn Error>> {
    let parsed = parse_url(url)?;
    let base_url = format!("{}://{}", parsed.scheme, parsed.host);

    let full_path = if let Some(query) = parsed.query {
        format!("{}?{}", parsed.path, query)
    } else {
        parsed.path
    };

    Ok((HttpClient::new(&base_url), full_path))
}

// Tests
#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn test_proxy_configuration() {
        let mut client = HttpClient::new("http://example.com");
        let proxy = ProxyConfig::new("proxy.example.com", 8080)
            .with_auth("username", "password")
            .with_https(false);

        client.set_proxy(proxy.clone());

        match &client.proxy {
            Some(configured_proxy) => {
                assert_eq!(configured_proxy.host, "proxy.example.com");
                assert_eq!(configured_proxy.port, 8080);
                assert_eq!(
                    configured_proxy.auth,
                    Some(("username".to_string(), "password".to_string()))
                );
                assert_eq!(configured_proxy.use_https, false);
            }
            None => panic!("Proxy should be configured"),
        }
    }

    #[test]
    fn test_http_method_to_string() {
        assert_eq!(HttpMethod::GET.to_string(), "GET");
        assert_eq!(HttpMethod::POST.to_string(), "POST");
        assert_eq!(HttpMethod::PUT.to_string(), "PUT");
        assert_eq!(HttpMethod::DELETE.to_string(), "DELETE");
        assert_eq!(HttpMethod::PATCH.to_string(), "PATCH");
        assert_eq!(HttpMethod::HEAD.to_string(), "HEAD");
        assert_eq!(HttpMethod::CONNECT.to_string(), "CONNECT");
    }

    #[test]
    fn test_http_client_initialization() {
        let client = HttpClient::new("https://api.example.com");
        assert_eq!(client.host, "https://api.example.com");
        assert!(client.auth.is_none());
        assert!(client.headers.is_empty());
        assert!(client.timeout.is_none());
        assert_eq!(client.max_redirects, 5);
        assert!(client.proxy.is_none());
        assert!(client.verify_ssl);
        assert!(client.enable_ipv6);
    }

    #[test]
    fn test_http_client_configuration() {
        let mut client = HttpClient::new("https://api.example.com");

        client.set_auth("username", "password");
        assert_eq!(
            client.auth,
            Some(("username".to_string(), "password".to_string()))
        );

        client.set_header("User-Agent", "Test Client");
        assert_eq!(
            client.headers.get("User-Agent"),
            Some(&"Test Client".to_string())
        );

        let timeout = Duration::from_secs(30);
        client.set_timeout(timeout);
        assert_eq!(client.timeout, Some(timeout));

        client.set_max_redirects(3);
        assert_eq!(client.max_redirects, 3);

        client.set_verify_ssl(false);
        assert!(!client.verify_ssl);

        client.set_enable_ipv6(false);
        assert!(!client.enable_ipv6);
    }

    #[test]
    fn test_http_response_status_checks() {
        let success_response = HttpResponse {
            status_code: 200,
            headers: HashMap::new(),
            body: Vec::new(),
            content_type: None,
        };
        assert!(success_response.is_success());
        assert!(!success_response.is_error());

        let client_error_response = HttpResponse {
            status_code: 404,
            headers: HashMap::new(),
            body: Vec::new(),
            content_type: None,
        };
        assert!(client_error_response.is_client_error());
        assert!(client_error_response.is_error());

        let server_error_response = HttpResponse {
            status_code: 500,
            headers: HashMap::new(),
            body: Vec::new(),
            content_type: None,
        };
        assert!(server_error_response.is_server_error());
        assert!(server_error_response.is_error());
    }

    #[test]
    fn test_parse_url() -> Result<(), Box<dyn Error>> {
        let test_cases = vec![
            ("https://example.com/path?query=value", ParsedUrl {
                scheme: "https".to_string(),
                host: "example.com".to_string(),
                path: "/path".to_string(),
                query: Some("query=value".to_string()),
            }),
            ("http://example.com", ParsedUrl {
                scheme: "http".to_string(),
                host: "example.com".to_string(),
                path: "/".to_string(),
                query: None,
            }),
            ("https://api.example.com/v1/users/", ParsedUrl {
                scheme: "https".to_string(),
                host: "api.example.com".to_string(),
                path: "/v1/users/".to_string(),
                query: None,
            }),
        ];

        for (input, expected) in test_cases {
            let parsed = parse_url(input)?;
            assert_eq!(parsed.scheme, expected.scheme);
            assert_eq!(parsed.host, expected.host);
            assert_eq!(parsed.path, expected.path);
            assert_eq!(parsed.query, expected.query);
        }

        Ok(())
    }

    #[test]
    fn test_create_client_from_url() -> Result<(), Box<dyn Error>> {
        let (client, path) =
            create_client_from_url("https://api.example.com/v1/users?active=true")?;

        assert_eq!(client.host, "https://api.example.com");
        assert_eq!(path, "/v1/users?active=true");

        let (client2, path2) = create_client_from_url("http://example.com")?;
        assert_eq!(client2.host, "http://example.com");
        assert_eq!(path2, "/");

        Ok(())
    }

    #[test]
    fn test_invalid_url() {
        let result = parse_url("invalid-url");
        assert!(result.is_err());

        let result = create_client_from_url("invalid-url");
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_response() {
        let client = HttpClient::new("https://example.com");
        let raw_response = "HTTP/1.1 200 OK\r\n\
                           Content-Type: application/json\r\n\
                           Content-Length: 2\r\n\
                           \r\n\
                           {}";

        let response = client.parse_response(raw_response).unwrap();
        assert_eq!(response.status_code, 200);
        assert_eq!(response.content_type, Some("application/json".to_string()));
        assert_eq!(response.body, "{}".as_bytes());
    }
}
