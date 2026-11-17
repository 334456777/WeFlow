//! Where every HTTP client of weflow is made (#371). reqwest 0.13's own TLS provider for rustls is aws-lc-rs, which
//! needs a C toolchain (and NASM on x86-64 Windows) on every target weflow ships for; weflow uses ring
//! instead and installs it as rustls' process-wide provider before the first client. A client made without
//! this panics on its first build. Certificates are checked against the system's store (rustls-platform-verifier).
use std::sync::Once;

/// Hosts of WeChat's sticker CDN whose `https:` certificate is not made out to their name: a request to them over
/// `https:` fails the check every time (`NotValidForName`), while their `http:` addresses, which nearly every sticker
/// has, and an unchecked `https:` request work (#814, measured on a real account). On a real account these two hosts
/// served six in ten stickers; the others (`wxapp.tc.qq.com`, `mmbiz.qpic.cn`, `snsvideo.hk.wechat.com`) pass.
const STICKER_HOSTS_WITH_A_WRONG_NAME: &[&str] = &["vweixinf.tc.qq.com", "snsvideo.c2c.wechat.com"];

/// Whether a sticker at `url` is fetched without checking the certificate: only from the hosts whose certificate
/// names another host (`STICKER_HOSTS_WITH_A_WRONG_NAME`). Every other address is checked.
pub fn sticker_certificate_unchecked(url: &str) -> bool {
    reqwest::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(str::to_ascii_lowercase))
        .is_some_and(|host| STICKER_HOSTS_WITH_A_WRONG_NAME.contains(&host.as_str()))
}

/// `reqwest::Client::builder()`, with ring installed as the TLS provider.
pub fn builder() -> reqwest::ClientBuilder {
    static PROVIDER: Once = Once::new();
    PROVIDER.call_once(|| {
        // Err: a provider is installed already (by a host of this library), which reqwest takes as it is
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
    reqwest::Client::builder()
}

/// What went wrong with a request, in words that leave out its address (#811): reqwest names the whole address in
/// its error, and an address can carry a secret, such as the bot token in Telegram's or the signature of a Moments
/// image. What is left is the kind of failure and its causes, as `error sending request: client error (Connect): tcp
/// connect error: …`.
pub fn failure(err: reqwest::Error) -> String {
    format!("{:#}", anyhow::Error::from(err.without_url()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A request that cannot be sent is reported without the token in its address, and with why it failed.
    #[tokio::test]
    async fn a_failure_leaves_the_address_out() {
        let closed = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = closed.local_addr().unwrap().port();
        drop(closed);
        let err = builder()
            .build()
            .unwrap()
            .post(format!(
                "http://127.0.0.1:{port}/bot123456:SECRET-TOKEN/sendMessage?sign=SECRET-SIGN"
            ))
            .send()
            .await
            .unwrap_err();
        assert!(err.to_string().contains("SECRET-TOKEN"), "{err}");
        let text = failure(err);
        assert!(!text.contains("SECRET"), "{text}");
        assert!(text.starts_with("error sending request: "), "{text}");
    }

    #[test]
    fn only_the_hosts_with_a_wrong_name_go_unchecked() {
        for url in [
            "https://vweixinf.tc.qq.com/110/20402/stodownload?m=1",
            "http://snsvideo.c2c.wechat.com/x",
            "https://VWEIXINF.TC.QQ.COM/x",
        ] {
            assert!(sticker_certificate_unchecked(url), "{url}");
        }
        for url in [
            "https://wxapp.tc.qq.com/262/20304/stodownload?m=1",
            "https://mmbiz.qpic.cn/x",
            "https://vweixinf.tc.qq.com.example.com/x",
            "https://example.com/vweixinf.tc.qq.com",
            "https://user@example.com/?h=snsvideo.c2c.wechat.com",
            "not an address",
        ] {
            assert!(!sticker_certificate_unchecked(url), "{url}");
        }
    }
}
