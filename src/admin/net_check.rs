//! 代理网络诊断
//!
//! 通过指定代理（或直连）完成两类检查：
//! - 出口 IP 的归属地、运营商：依次尝试多个 IP 服务，单个被限流或不通时换下一个；
//! - 站点连通性与延迟：Kiro 上游接口、Kiro 登录刷新接口，以及几个常用站点。
//!
//! 测试目标固定写在服务端，调用方只能选择「走哪个代理」，不能让服务器去访问任意地址。
//! 只等到响应头就丢弃响应：首页体积各不相同，下载完整页面测出来的就不是网络延迟了。

use std::time::Instant;

use anyhow::Context;
use reqwest::Client;
use serde::Serialize;

use crate::http_client::{ProxyConfig, build_client};
use crate::model::config::TlsBackend;

const IP_TIMEOUT_SECS: u64 = 8;
const SITE_TIMEOUT_SECS: u64 = 10;

/// (标识, 地址)。任何 HTTP 响应（含 403/404）都说明链路是通的。
const SITES: [(&str, &str); 6] = [
    ("kiroApi", "https://q.us-east-1.amazonaws.com/"),
    ("kiroAuth", "https://prod.us-east-1.auth.desktop.kiro.dev/"),
    ("kiroSite", "https://kiro.dev"),
    ("github", "https://github.com"),
    ("google", "https://www.google.com"),
    ("amazon", "https://www.amazon.com"),
];

/// 部分站点会拒绝没有 UA 的请求，那样会被误判成不通
const BROWSER_UA: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0 Safari/537.36";

/// 出口 IP 信息
#[derive(Debug, Clone, Default, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct IpInfo {
    pub ip: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub country: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub country_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub region: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub city: Option<String>,
    /// 运营商 / ASN
    #[serde(skip_serializing_if = "Option::is_none")]
    pub org: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timezone: Option<String>,
    /// 数据来源（哪个 IP 服务返回的）
    pub source: String,
}

/// 单个站点的测试结果
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SiteResult {
    pub id: String,
    pub url: String,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<u16>,
    pub latency_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// 一次诊断的完整结果
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NetworkCheckResult {
    /// 本次走的出口：「直连」或去掉账号密码的代理地址
    pub via: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ip: Option<IpInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ip_error: Option<String>,
    pub sites: Vec<SiteResult>,
    pub total_ms: u64,
}

/// 把代理地址转成可展示的文字，去掉其中的 `user:pass@`
pub fn describe_proxy(proxy: Option<&ProxyConfig>) -> String {
    let Some(proxy) = proxy else {
        return "直连".to_string();
    };
    match proxy.url.split_once("://") {
        Some((scheme, rest)) => {
            let host = rest.rsplit_once('@').map(|(_, h)| h).unwrap_or(rest);
            format!("{}://{}", scheme, host)
        }
        None => proxy.url.clone(),
    }
}

/// 通过 `proxy`（None 为直连）运行完整诊断
pub async fn run(proxy: Option<ProxyConfig>, tls_backend: TlsBackend) -> anyhow::Result<NetworkCheckResult> {
    let started = Instant::now();
    let ip_client = build_client(proxy.as_ref(), IP_TIMEOUT_SECS, tls_backend)
        .context("代理配置无效，无法建立连接")?;
    let site_client = build_client(proxy.as_ref(), SITE_TIMEOUT_SECS, tls_backend)
        .context("代理配置无效，无法建立连接")?;

    let sites = SITES
        .iter()
        .map(|(id, url)| test_site(&site_client, id, url));
    let (ip_result, sites) = tokio::join!(fetch_ip_info(&ip_client), futures::future::join_all(sites));

    let (ip, ip_error) = match ip_result {
        Ok(info) => (Some(info), None),
        Err(e) => (None, Some(e)),
    };
    Ok(NetworkCheckResult {
        via: describe_proxy(proxy.as_ref()),
        ip,
        ip_error,
        sites,
        total_ms: started.elapsed().as_millis() as u64,
    })
}

async fn test_site(client: &Client, id: &str, url: &str) -> SiteResult {
    let started = Instant::now();
    let result = client
        .get(url)
        .header("User-Agent", BROWSER_UA)
        .header("Accept", "text/html,*/*")
        .send()
        .await;
    let latency_ms = started.elapsed().as_millis() as u64;
    match result {
        // 响应对象在此处被丢弃，不读取正文
        Ok(resp) => SiteResult {
            id: id.to_string(),
            url: url.to_string(),
            ok: true,
            status: Some(resp.status().as_u16()),
            latency_ms,
            error: None,
        },
        Err(e) => SiteResult {
            id: id.to_string(),
            url: url.to_string(),
            ok: false,
            status: None,
            latency_ms,
            error: Some(if e.is_timeout() {
                format!("连接超时（{}s）", SITE_TIMEOUT_SECS)
            } else {
                short_error(&e)
            }),
        },
    }
}

/// reqwest 的错误链很长，取最内层原因便于展示
fn short_error(error: &reqwest::Error) -> String {
    let mut message = error.to_string();
    let mut source = std::error::Error::source(error);
    while let Some(inner) = source {
        message = inner.to_string();
        source = inner.source();
    }
    message
}

type IpParser = fn(&serde_json::Value) -> Option<IpInfo>;

/// 各家 IP 服务字段命名不同，逐个适配成统一结构
const IP_PROVIDERS: [(&str, &str, IpParser); 3] = [
    ("ipinfo.io", "https://ipinfo.io/json", parse_ipinfo),
    ("ipapi.co", "https://ipapi.co/json/", parse_ipapi),
    ("ip.sb", "https://api.ip.sb/geoip", parse_ipsb),
];

fn text(value: &serde_json::Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn join_asn(asn: Option<String>, org: Option<String>) -> Option<String> {
    match (asn, org) {
        (Some(a), Some(o)) => Some(format!("{} {}", a, o)),
        (None, org) => org,
        (asn, None) => asn,
    }
}

fn parse_ipinfo(v: &serde_json::Value) -> Option<IpInfo> {
    Some(IpInfo {
        ip: text(v, "ip")?,
        country_code: text(v, "country"),
        region: text(v, "region"),
        city: text(v, "city"),
        org: text(v, "org"),
        timezone: text(v, "timezone"),
        source: "ipinfo.io".to_string(),
        ..Default::default()
    })
}

fn parse_ipapi(v: &serde_json::Value) -> Option<IpInfo> {
    if v.get("error").is_some_and(|e| !e.is_null() && e != &serde_json::Value::Bool(false)) {
        return None;
    }
    Some(IpInfo {
        ip: text(v, "ip")?,
        country: text(v, "country_name"),
        country_code: text(v, "country_code"),
        region: text(v, "region"),
        city: text(v, "city"),
        org: join_asn(text(v, "asn"), text(v, "org")),
        timezone: text(v, "timezone"),
        source: "ipapi.co".to_string(),
    })
}

fn parse_ipsb(v: &serde_json::Value) -> Option<IpInfo> {
    let asn = v.get("asn").and_then(|a| a.as_u64()).map(|a| format!("AS{}", a));
    Some(IpInfo {
        ip: text(v, "ip")?,
        country: text(v, "country"),
        country_code: text(v, "country_code"),
        region: text(v, "region"),
        city: text(v, "city"),
        org: join_asn(asn, text(v, "organization").or_else(|| text(v, "isp"))),
        timezone: text(v, "timezone"),
        source: "ip.sb".to_string(),
    })
}

/// 依次尝试各 IP 服务，第一个成功的为准；全部失败时返回汇总原因
async fn fetch_ip_info(client: &Client) -> Result<IpInfo, String> {
    let mut errors: Vec<String> = Vec::new();
    for (name, url, parse) in IP_PROVIDERS {
        let resp = match client
            .get(url)
            .header("Accept", "application/json")
            .header("User-Agent", "curl/8.0")
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                errors.push(format!(
                    "{} {}",
                    name,
                    if e.is_timeout() { "超时".to_string() } else { short_error(&e) }
                ));
                continue;
            }
        };
        if !resp.status().is_success() {
            errors.push(format!("{} HTTP {}", name, resp.status().as_u16()));
            continue;
        }
        match resp.json::<serde_json::Value>().await {
            Ok(body) => match parse(&body) {
                Some(info) => return Ok(info),
                None => errors.push(format!("{} 返回内容无法识别", name)),
            },
            Err(e) => errors.push(format!("{} 返回内容不是 JSON: {}", name, e)),
        }
    }
    Err(format!("查询出口 IP 失败：{}", errors.join("；")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn describe_proxy_hides_credentials() {
        assert_eq!(describe_proxy(None), "直连");
        let with_auth = ProxyConfig::new("socks5://user:secret@10.0.0.1:1080");
        assert_eq!(describe_proxy(Some(&with_auth)), "socks5://10.0.0.1:1080");
        let plain = ProxyConfig::new("http://10.0.0.2:8080");
        assert_eq!(describe_proxy(Some(&plain)), "http://10.0.0.2:8080");
    }

    #[test]
    fn parses_ipinfo() {
        let info = parse_ipinfo(&json!({
            "ip": "1.2.3.4", "country": "US", "region": "Virginia",
            "city": "Ashburn", "org": "AS31898 Oracle Corporation", "timezone": "America/New_York"
        }))
        .unwrap();
        assert_eq!(info.ip, "1.2.3.4");
        assert_eq!(info.country_code.as_deref(), Some("US"));
        assert_eq!(info.org.as_deref(), Some("AS31898 Oracle Corporation"));
        assert_eq!(info.source, "ipinfo.io");
    }

    #[test]
    fn parses_ipapi_and_rejects_error_payload() {
        let ok = parse_ipapi(&json!({
            "ip": "5.6.7.8", "country_name": "Germany", "country_code": "DE",
            "city": "Frankfurt", "asn": "AS16509", "org": "AMAZON-02"
        }))
        .unwrap();
        assert_eq!(ok.country.as_deref(), Some("Germany"));
        assert_eq!(ok.org.as_deref(), Some("AS16509 AMAZON-02"));

        assert!(parse_ipapi(&json!({"error": true, "reason": "RateLimited"})).is_none());
    }

    #[test]
    fn parses_ipsb_with_numeric_asn() {
        let info = parse_ipsb(&json!({
            "ip": "9.9.9.9", "country": "United States", "country_code": "US",
            "asn": 19281, "organization": "Quad9"
        }))
        .unwrap();
        assert_eq!(info.org.as_deref(), Some("AS19281 Quad9"));
    }

    #[test]
    fn missing_ip_is_unrecognized() {
        assert!(parse_ipinfo(&json!({"country": "US"})).is_none());
        assert!(parse_ipsb(&json!({"ip": "  "})).is_none());
    }
}
