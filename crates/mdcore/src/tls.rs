//! The TLS settings for talking to the server (HTTPS registry, `wss://`
//! relay): trust the bundled Mozilla roots **and** the OS certificate store.
//!
//! Why both: company networks often inspect HTTPS with a proxy (Fortinet,
//! Zscaler, ...) that re-signs every site with the company's own CA, which IT
//! installs into the OS store. Browsers trust it; with only the bundled roots
//! we refused the server ("invalid peer certificate: UnknownIssuer"). Messages
//! between devices are end-to-end encrypted either way, so such a proxy sees
//! only the server calls.

use std::sync::{Arc, OnceLock};

use rustls::{ClientConfig, RootCertStore};

/// The bundled roots plus whatever the OS store trusts. (Android has no
/// readable system store here; the bundled roots cover it. Unparsable OS
/// certificates are skipped.)
fn root_store() -> RootCertStore {
    let mut roots = RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let native = rustls_native_certs::load_native_certs();
    roots.add_parsable_certificates(native.certs);
    roots
}

/// One shared client config (loading the OS store takes a few milliseconds).
pub fn client_config() -> Arc<ClientConfig> {
    static CONFIG: OnceLock<Arc<ClientConfig>> = OnceLock::new();
    CONFIG
        .get_or_init(|| {
            let roots = root_store();
            Arc::new(
                ClientConfig::builder_with_provider(Arc::new(
                    rustls::crypto::ring::default_provider(),
                ))
                .with_safe_default_protocol_versions()
                // ring supports every default protocol version.
                .unwrap_or_else(|e| panic!("tls: {e}"))
                .with_root_certificates(roots)
                .with_no_client_auth(),
            )
        })
        .clone()
}

/// The HTTP client for server calls, using [`client_config`].
pub fn http() -> &'static ureq::Agent {
    static AGENT: OnceLock<ureq::Agent> = OnceLock::new();
    AGENT.get_or_init(|| {
        ureq::AgentBuilder::new()
            .tls_config(client_config())
            .build()
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn trusts_the_bundled_roots_and_the_os_store() {
        // Every bundled root, plus (on a desktop) the OS's own.
        assert!(super::root_store().len() >= webpki_roots::TLS_SERVER_ROOTS.len());
        assert!(std::ptr::eq(super::http(), super::http()));
    }
}
