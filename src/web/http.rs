//! The real transport: a reqwest-backed [`WebFetcher`] with a per-fetch timeout, a streaming byte
//! cap, and a server-side request forgery guard.
//!
//! The SSRF guard is not theatre: the instance's own control API listens on localhost, so an
//! agent-driven fetch that reached a private or loopback address could turn the agent into a confused
//! deputy against its own host. The guard refuses loopback, private (RFC 1918), link-local, and
//! unique-local ranges at two choke points — a custom DNS resolver validates every hostname's
//! resolved addresses (so a name that resolves to a private address is refused), and a redirect
//! policy re-validates every hop's address literal (so a redirect to `127.0.0.1`, the classic
//! bypass, is caught even though it never hits the resolver). Both are gated by
//! `allow_private_addresses`.

use std::{
    net::{IpAddr, Ipv6Addr, SocketAddr},
    sync::Arc,
    time::Duration,
};

use async_trait::async_trait;
use futures_util::StreamExt;
use reqwest::{
    Url,
    dns::{Addrs, Name, Resolve, Resolving},
    redirect,
};

use super::{FetchedPage, WebError, WebFetcher, is_html};

/// How the real fetcher is built: the per-fetch timeout, the response byte cap, the user agent, and
/// whether private addresses are permitted. Assembled by the serving host from [`WebSettings`]
/// (`crate::settings`), so a change to these takes effect on restart.
#[derive(Clone, Debug)]
pub struct HttpFetcherConfig {
    pub timeout: Duration,
    pub max_response_bytes: u64,
    pub user_agent: String,
    pub allow_private_addresses: bool,
}

/// A reqwest-backed [`WebFetcher`]: GET only, rustls TLS, gzip and brotli decoding, redirects
/// followed within the SSRF guard.
pub struct HttpFetcher {
    client: reqwest::Client,
    max_response_bytes: u64,
    allow_private_addresses: bool,
}

impl HttpFetcher {
    /// Build the fetcher from `config`. The reqwest client is configured once — its DNS resolver and
    /// redirect policy carry the SSRF guard — and reused across fetches for connection pooling.
    pub fn new(config: HttpFetcherConfig) -> Result<HttpFetcher, WebError> {
        let allow_private = config.allow_private_addresses;
        let resolver: Arc<dyn Resolve> = Arc::new(SsrfResolver { allow_private });
        let redirect_policy = redirect::Policy::custom(move |attempt| {
            if attempt.previous().len() >= MAX_REDIRECTS {
                return attempt.error(GuardError("too many redirects".to_owned()));
            }
            match check_redirect_target(attempt.url(), allow_private) {
                Ok(()) => attempt.follow(),
                Err(error) => attempt.error(error),
            }
        });
        let client = reqwest::Client::builder()
            .user_agent(config.user_agent)
            .timeout(config.timeout)
            .redirect(redirect_policy)
            .dns_resolver(resolver)
            .build()
            .map_err(|error| WebError::Transport {
                url: String::new(),
                reason: format!("could not build the HTTP client: {error}"),
            })?;
        Ok(HttpFetcher {
            client,
            max_response_bytes: config.max_response_bytes,
            allow_private_addresses: allow_private,
        })
    }
}

#[async_trait]
impl WebFetcher for HttpFetcher {
    async fn fetch(&self, url: &str) -> Result<FetchedPage, WebError> {
        let parsed = Url::parse(url).map_err(|error| WebError::InvalidUrl {
            url: url.to_owned(),
            reason: error.to_string(),
        })?;
        match parsed.scheme() {
            "http" | "https" => {}
            scheme => {
                return Err(WebError::UnsupportedScheme {
                    url: url.to_owned(),
                    scheme: scheme.to_owned(),
                });
            }
        }
        // Guard the initial URL's host literal here: an address-literal host never reaches the DNS
        // resolver, so `http://127.0.0.1` would otherwise slip straight through.
        if !self.allow_private_addresses
            && let Some(ip) = literal_ip(&parsed)
            && is_disallowed_ip(ip)
        {
            return Err(WebError::BlockedAddress {
                url: url.to_owned(),
            });
        }

        let response = self
            .client
            .get(parsed)
            .send()
            .await
            .map_err(|error| map_reqwest_error(url, error))?;

        let status = response.status();
        if !status.is_success() {
            return Err(WebError::Status {
                url: url.to_owned(),
                status: status.as_u16(),
            });
        }

        // Check the content type from the header before draining the body, so a non-HTML resource
        // (a PDF, an image, a tarball) fails fast without downloading it.
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("")
            .to_owned();
        if !is_html(&content_type) {
            return Err(WebError::NotHtml {
                url: url.to_owned(),
                content_type: if content_type.is_empty() {
                    "an unknown content type".to_owned()
                } else {
                    content_type
                },
            });
        }
        let final_url = response.url().to_string();

        // Stream the body, enforcing the byte cap as it arrives so an oversized page is abandoned
        // mid-download rather than buffered whole.
        let mut body: Vec<u8> = Vec::new();
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|error| map_reqwest_error(url, error))?;
            if body.len() as u64 + chunk.len() as u64 > self.max_response_bytes {
                return Err(WebError::TooLarge {
                    url: url.to_owned(),
                    limit: self.max_response_bytes,
                });
            }
            body.extend_from_slice(&chunk);
        }
        // Decode as UTF-8, replacing invalid sequences. Sensible for the web's overwhelmingly-UTF-8
        // HTML; a legacy-encoded page degrades to replacement characters rather than failing outright.
        let body = String::from_utf8_lossy(&body).into_owned();

        Ok(FetchedPage {
            final_url,
            content_type,
            body,
        })
    }
}

/// The most redirect hops a fetch follows before giving up.
const MAX_REDIRECTS: usize = 10;

/// Map a reqwest failure to the catchable [`WebError`] the agent sees, distinguishing a timeout (its
/// own variant, since a slow page is a common, adaptable case) from every other transport failure —
/// including the SSRF guard's own [`GuardError`], which surfaces as a blocked-address refusal.
fn map_reqwest_error(url: &str, error: reqwest::Error) -> WebError {
    if error.is_timeout() {
        return WebError::Timeout {
            url: url.to_owned(),
        };
    }
    // A redirect refused by the guard arrives wrapped as a redirect error carrying our `GuardError`.
    let mut source: Option<&(dyn std::error::Error + 'static)> = std::error::Error::source(&error);
    while let Some(cause) = source {
        if cause.is::<GuardError>() {
            return WebError::BlockedAddress {
                url: url.to_owned(),
            };
        }
        source = cause.source();
    }
    WebError::Transport {
        url: url.to_owned(),
        reason: error.to_string(),
    }
}

/// The SSRF-guarding DNS resolver: it resolves a hostname the ordinary way, then drops every address
/// in a disallowed range, so a name that resolves only to private addresses fails to connect. Applied
/// to every hostname reqwest connects to, initial request and redirect alike.
struct SsrfResolver {
    allow_private: bool,
}

impl Resolve for SsrfResolver {
    fn resolve(&self, name: Name) -> Resolving {
        let allow_private = self.allow_private;
        Box::pin(async move {
            let host = name.as_str().to_owned();
            // Resolve with an arbitrary port (reqwest overrides it with the real one); `lookup_host`
            // runs the system resolver on a blocking thread.
            let resolved: Vec<SocketAddr> = tokio::net::lookup_host((host.as_str(), 0))
                .await
                .map_err(|error| Box::new(error) as Box<dyn std::error::Error + Send + Sync>)?
                .collect();
            let allowed: Vec<SocketAddr> = resolved
                .into_iter()
                .filter(|addr| allow_private || !is_disallowed_ip(addr.ip()))
                .collect();
            if allowed.is_empty() {
                return Err(Box::new(GuardError(format!(
                    "{host} resolves only to private addresses"
                )))
                    as Box<dyn std::error::Error + Send + Sync>);
            }
            Ok(Box::new(allowed.into_iter()) as Addrs)
        })
    }
}

/// A marker error carried through reqwest's redirect and connect machinery so [`map_reqwest_error`]
/// can recognise an SSRF refusal amongst ordinary transport failures.
#[derive(Debug)]
struct GuardError(String);

impl std::fmt::Display for GuardError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "web: {}", self.0)
    }
}

impl std::error::Error for GuardError {}

/// Re-validate a redirect hop's target. A hostname hop is validated by the DNS resolver when the
/// connection is made; only an address-literal target bypasses the resolver, so that is the case this
/// re-checks — a redirect to `http://127.0.0.1`, the classic guard bypass, is refused here.
fn check_redirect_target(url: &Url, allow_private: bool) -> Result<(), GuardError> {
    if !allow_private
        && let Some(ip) = literal_ip(url)
        && is_disallowed_ip(ip)
    {
        return Err(GuardError(format!("redirect to a private address {ip}")));
    }
    Ok(())
}

/// The host of `url` as an IP address, when it is an address literal (`http://127.0.0.1`,
/// `http://[::1]`) rather than a name. A literal never passes through the DNS resolver, so it is the
/// case the resolver-based guard cannot see.
fn literal_ip(url: &Url) -> Option<IpAddr> {
    match url.host() {
        Some(url::Host::Ipv4(ip)) => Some(IpAddr::V4(ip)),
        Some(url::Host::Ipv6(ip)) => Some(IpAddr::V6(ip)),
        _ => None,
    }
}

/// Whether `ip` is one the SSRF guard refuses: loopback, unspecified, private (RFC 1918 and the
/// 100.64/10 shared/CGNAT range), link-local, or IPv6 unique-local. An IPv4-mapped IPv6 address is
/// unwrapped and judged as its embedded IPv4, so `::ffff:127.0.0.1` cannot smuggle a loopback past
/// the check. Written against stable `std` primitives, since the range predicates that would cover
/// these directly (`is_global`, `is_unique_local`) are unstable.
fn is_disallowed_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_unspecified()
                // 100.64.0.0/10 — the shared/carrier-grade-NAT range, neither public nor RFC 1918.
                || (v4.octets()[0] == 100 && (v4.octets()[1] & 0xc0) == 0x40)
        }
        IpAddr::V6(v6) => {
            if let Some(mapped) = v6.to_ipv4_mapped() {
                return is_disallowed_ip(IpAddr::V4(mapped));
            }
            v6.is_loopback()
                || v6.is_unspecified()
                || is_unique_local_v6(v6)
                || is_link_local_v6(v6)
        }
    }
}

/// Whether `v6` is in the IPv6 unique-local range `fc00::/7`.
fn is_unique_local_v6(v6: Ipv6Addr) -> bool {
    (v6.segments()[0] & 0xfe00) == 0xfc00
}

/// Whether `v6` is in the IPv6 link-local range `fe80::/10`.
fn is_link_local_v6(v6: Ipv6Addr) -> bool {
    (v6.segments()[0] & 0xffc0) == 0xfe80
}

#[cfg(test)]
mod tests {
    use std::{
        net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
        time::Duration,
    };

    use reqwest::Url;
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };

    use super::{HttpFetcher, HttpFetcherConfig, check_redirect_target, is_disallowed_ip};
    use crate::web::{WebError, WebFetcher};

    #[test]
    fn the_guard_refuses_loopback_private_and_link_local_literals() {
        for disallowed in [
            "127.0.0.1",
            "10.0.0.1",
            "172.16.5.4",
            "192.168.1.1",
            "169.254.1.1",
            "100.64.0.1",
            "0.0.0.0",
        ] {
            let ip: IpAddr = disallowed.parse().unwrap();
            assert!(is_disallowed_ip(ip), "{disallowed} should be refused");
        }
        // IPv6 loopback, unique-local, link-local, and an IPv4-mapped loopback.
        for disallowed in [
            "::1",
            "fc00::1",
            "fd12:3456::1",
            "fe80::1",
            "::ffff:127.0.0.1",
        ] {
            let ip: IpAddr = disallowed.parse().unwrap();
            assert!(is_disallowed_ip(ip), "{disallowed} should be refused");
        }
    }

    #[test]
    fn the_guard_admits_public_addresses() {
        for allowed in ["8.8.8.8", "1.1.1.1", "93.184.216.34"] {
            let ip = IpAddr::V4(allowed.parse::<Ipv4Addr>().unwrap());
            assert!(!is_disallowed_ip(ip), "{allowed} should be admitted");
        }
        let public_v6 = IpAddr::V6("2606:4700:4700::1111".parse::<Ipv6Addr>().unwrap());
        assert!(
            !is_disallowed_ip(public_v6),
            "a public v6 should be admitted"
        );
    }

    #[test]
    fn a_redirect_target_on_localhost_is_refused_and_admitted_when_private_is_allowed() {
        // The exact decision the redirect policy makes on each hop: a redirect that lands on a
        // loopback literal is refused when private addresses are disallowed, and admitted when they
        // are allowed.
        let loopback = Url::parse("http://127.0.0.1:8080/admin").unwrap();
        assert!(check_redirect_target(&loopback, false).is_err());
        assert!(check_redirect_target(&loopback, true).is_ok());
        // A public target is admitted regardless.
        let public = Url::parse("http://93.184.216.34/").unwrap();
        assert!(check_redirect_target(&public, false).is_ok());
    }

    fn fetcher(allow_private: bool) -> HttpFetcher {
        HttpFetcher::new(HttpFetcherConfig {
            timeout: Duration::from_secs(5),
            max_response_bytes: 1_000_000,
            user_agent: "zuihitsu-test".to_owned(),
            allow_private_addresses: allow_private,
        })
        .unwrap()
    }

    #[tokio::test]
    async fn a_direct_fetch_of_a_loopback_literal_is_refused() {
        // The initial-URL guard: a loopback literal never reaches the network, refused before connect.
        let error = fetcher(false)
            .fetch("http://127.0.0.1:1/")
            .await
            .unwrap_err();
        assert!(
            matches!(error, WebError::BlockedAddress { .. }),
            "expected a blocked-address refusal, got {error:?}"
        );
    }

    /// Serve one HTTP response on `listener` and then stop. `response` is the raw response bytes.
    async fn serve_one(listener: TcpListener, response: String) {
        if let Ok((mut socket, _)) = listener.accept().await {
            let mut buf = [0u8; 1024];
            let _ = socket.read(&mut buf).await;
            let _ = socket.write_all(response.as_bytes()).await;
            let _ = socket.flush().await;
        }
    }

    fn html_response(html: &str) -> String {
        format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\n\r\n{}",
            html.len(),
            html
        )
    }

    const LOCAL_ARTICLE: &str = include_str!("fixtures/local_article.html");

    #[tokio::test]
    async fn allow_private_admits_a_loopback_page() {
        // With the gate open, a loopback page is fetched, extracted, and returned as Markdown.
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let addr: SocketAddr = listener.local_addr().unwrap();
        let server = tokio::spawn(serve_one(listener, html_response(LOCAL_ARTICLE)));
        let page = fetcher(true)
            .fetch(&format!("http://127.0.0.1:{}/", addr.port()))
            .await
            .expect("the loopback fetch succeeds with private allowed");
        server.abort();
        assert_eq!(page.content_type, "text/html; charset=utf-8");
        assert!(
            page.body.contains("real article content"),
            "expected the article body, got {:?}",
            page.body
        );
    }

    #[tokio::test]
    async fn a_redirect_is_followed_through_a_real_listener() {
        // Two loopback listeners: the first 302s to the second, which serves the page. With private
        // allowed, the redirect policy follows the hop through live listeners and the final page's
        // body comes back — the live counterpart to the redirect-guard unit test above.
        let first = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let second = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let first_addr: SocketAddr = first.local_addr().unwrap();
        let second_addr: SocketAddr = second.local_addr().unwrap();
        let redirect = format!(
            "HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:{}/final\r\nContent-Length: 0\r\n\r\n",
            second_addr.port()
        );
        let first_server = tokio::spawn(serve_one(first, redirect));
        let second_server = tokio::spawn(serve_one(second, html_response(LOCAL_ARTICLE)));
        let page = fetcher(true)
            .fetch(&format!("http://127.0.0.1:{}/", first_addr.port()))
            .await
            .expect("the redirect is followed to the final page");
        first_server.abort();
        second_server.abort();
        assert!(
            page.final_url.contains(&second_addr.port().to_string()),
            "the final URL should be the redirect target, got {:?}",
            page.final_url
        );
        assert!(page.body.contains("real article content"));
    }

    #[tokio::test]
    async fn a_non_html_response_is_rejected() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let addr: SocketAddr = listener.local_addr().unwrap();
        let body = "{\"not\":\"html\"}";
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
            body.len(),
            body
        );
        let server = tokio::spawn(serve_one(listener, response));
        let error = fetcher(true)
            .fetch(&format!("http://127.0.0.1:{}/", addr.port()))
            .await
            .unwrap_err();
        server.abort();
        assert!(
            matches!(error, WebError::NotHtml { .. }),
            "expected a non-HTML rejection, got {error:?}"
        );
    }

    #[tokio::test]
    async fn a_non_http_scheme_is_rejected() {
        let error = fetcher(true)
            .fetch("ftp://example.com/file")
            .await
            .unwrap_err();
        assert!(matches!(error, WebError::UnsupportedScheme { .. }));
    }
}
