//! Where every HTTP client of weflow is made (#371). reqwest 0.13's own TLS provider for rustls is aws-lc-rs, which
//! needs a C toolchain (and NASM on x86-64 Windows) on every target weflow ships for; weflow keeps ring, as with
//! reqwest 0.12, and installs it as rustls' process-wide provider before the first client. A client made without
//! this panics on its first build. Certificates are checked against the system's store (rustls-platform-verifier),
//! no longer against a list built into the binary.
use std::sync::Once;

/// `reqwest::Client::builder()`, with ring installed as the TLS provider.
pub fn builder() -> reqwest::ClientBuilder {
    static PROVIDER: Once = Once::new();
    PROVIDER.call_once(|| {
        // Err: a provider is installed already (by a host of this library), which reqwest takes as it is
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
    reqwest::Client::builder()
}
