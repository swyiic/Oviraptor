//! Public, bounded website identity research. A footer is a declaration, never
//! an official registry result. Its source is kept separate from CA leaf facts.
use super::*;
use std::{
    io::Read as _,
    net::{IpAddr, SocketAddr},
    time::{Duration, Instant},
};

const MAX_BODY: usize = 1024 * 1024;
const FRESH: i64 = 7 * 86400;
const MAX_SECONDS: u64 = 28;

fn root(name: &str) -> Option<String> {
    crate::commands::ownership::ownership_domain_parts(name)?.registrable
}
fn checked_url(url: &reqwest::Url, family: &str) -> Result<(), String> {
    let name = url.host_str().ok_or("官网来源没有域名")?;
    if !matches!(url.scheme(), "https" | "http")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port_or_known_default() != Some(if url.scheme() == "https" { 443 } else { 80 })
        || name.parse::<IpAddr>().is_ok()
        || root(name).as_deref() != Some(family)
    {
        return Err("官网来源不是HTTP(S)、带凭据、非标准端口或其他域；未继续请求".into());
    }
    Ok(())
}
fn checked_redirect(
    current: &reqwest::Url,
    next: &reqwest::Url,
    family: &str,
) -> Result<(), String> {
    checked_url(next, family)?;
    if (current.scheme() == "http" || next.scheme() == "http")
        && current.host_str() != next.host_str()
    {
        return Err("HTTP补充查证只允许同一主机跳转，未连接其他主机".into());
    }
    Ok(())
}
fn may_try_http(error: &str) -> bool {
    error.contains("官网公开页面请求失败")
}
fn decode(bytes: &[u8], content_type: &str) -> Result<String, String> {
    let meta = String::from_utf8_lossy(&bytes[..bytes.len().min(8192)]).to_ascii_lowercase();
    let label = regex::Regex::new(r#"(?i)charset\s*=\s*["']?\s*([a-zA-Z0-9_-]+)"#).unwrap();
    let name = label
        .captures(content_type)
        .or_else(|| label.captures(&meta))
        .and_then(|c| c.get(1))
        .map(|m| m.as_str());
    let encoding = name
        .and_then(|n| encoding_rs::Encoding::for_label(n.as_bytes()))
        .unwrap_or(encoding_rs::UTF_8);
    let (body, _, errors) = encoding.decode(bytes);
    if errors {
        return Err("官网正文编码无效或含损坏字符，未推断单位名称".into());
    }
    Ok(body.into_owned())
}
fn entities(s: &str) -> String {
    let mut out = s
        .replace('\u{fe0f}', "")
        .replace("&nbsp;", " ")
        .replace("&#160;", " ")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&copy;", "©");
    let numbers = regex::Regex::new(r"&#(?:x([0-9a-fA-F]{1,6})|([0-9]{1,7}));").unwrap();
    out = numbers
        .replace_all(&out, |c: &regex::Captures<'_>| {
            let n = c
                .get(1)
                .and_then(|m| u32::from_str_radix(m.as_str(), 16).ok())
                .or_else(|| c.get(2).and_then(|m| m.as_str().parse().ok()));
            n.and_then(char::from_u32)
                .map(String::from)
                .unwrap_or_default()
        })
        .into_owned();
    out
}
fn visible(html: &str) -> String {
    let hidden = regex::Regex::new(r"(?is)<!--.*?-->|<(?:script|style|template|noscript)\b[^>]*>.*?</(?:script|style|template|noscript)\s*>").unwrap();
    let tags = regex::Regex::new(r"(?s)<[^>]*>").unwrap();
    entities(&tags.replace_all(&hidden.replace_all(html, " "), "\n"))
        .lines()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}
fn legal_name(raw: &str) -> Option<String> {
    let name = raw.trim().trim_matches([
        ':', '：', '|', '丨', '·', '-', '—', '©', ' ', '。', '，', ',', ';', '；',
    ]);
    let name = regex::Regex::new(
        r"^(?i)(?:copyright\s*)?(?:©|\(c\))?\s*(?:\d{4}(?:\s*[-–—]\s*\d{4})?)?\s*",
    )
    .unwrap()
    .replace(name, "");
    let name = name.trim();
    let name = name.strip_prefix("版权所有").map(str::trim).unwrap_or(name);
    if name.chars().count() < 4 || name.chars().count() > 90 {
        return None;
    }
    // A complete legal name, not a broad brand, web title, technical provider,
    // trademark or arbitrary company mentioned in an article.
    let suffix = [
        "股份有限公司",
        "有限责任公司",
        "有限公司",
        "高等专科学校",
        "职业技术大学",
        "职业技术学院",
        "职业大学",
        "大学",
        "学院",
        "人民政府",
        "教育局",
        "事业单位",
        "行业协会",
        "协会",
        "委员会",
        "研究院",
        "中心",
        "学校",
    ];
    let lower = name.to_ascii_lowercase();
    let english_legal = lower.contains("university of ")
        || [
            "limited",
            "ltd.",
            "ltd",
            "inc.",
            "inc",
            "corporation",
            "university",
            "college",
        ]
        .iter()
        .any(|suffix| lower.ends_with(suffix));
    if !(suffix.iter().any(|end| name.ends_with(end)) || english_legal)
        || name.contains(['<', '>', '\n', '：', ':', '|', '丨'])
        || [
            "技术支持",
            "建设维护",
            "网站建设",
            "提供服务",
            "合作伙伴",
            "供应商",
            "客户",
            "example",
            "示例",
        ]
        .iter()
        .any(|s| name.contains(s))
    {
        return None;
    }
    Some(name.to_owned())
}
/// Only explicit operator/copyright declarations are promoted to facts.
/// A page title, news article mention, cloud logo and JSON-LD cannot establish it.
pub(super) fn declarations(html: &str) -> Vec<(String, String, String)> {
    let excluded =
        regex::Regex::new(r"(?is)<(?:article|blockquote)\b[^>]*>.*?</(?:article|blockquote)\s*>")
            .unwrap();
    let blocks =
        regex::Regex::new(r"(?is)</?(?:p|div|footer|section|li|tr|td|br|h[1-6])\b[^>]*>").unwrap();
    let clean = excluded.replace_all(html, " ");
    let text = visible(&blocks.replace_all(&clean, "\u{1f}"));
    let re = regex::Regex::new(r"(?i)(?:版权所有\s*(?:[:：©]|\(c\))*\s*|(?:网站)?主办(?:单位)?\s*[:：]|(?:网站)?运营(?:单位|主体)\s*[:：]|copyright\s*(?:©|\(c\))?\s*(?:\d{4}(?:\s*[-–—]\s*\d{4})?)?\s*)([\p{Han}A-Za-z0-9（）()·&\s]{4,140}?(?:股份有限公司|有限责任公司|有限公司|高等专科学校|职业技术大学|职业技术学院|职业大学|大学|学院|人民政府|教育局|行业协会|协会|委员会|研究院|中心|学校))").unwrap();
    let reverse = regex::Regex::new(r"([\p{Han}A-Za-z0-9（）()·&]{4,90}(?:股份有限公司|有限责任公司|有限公司|高等专科学校|职业技术大学|职业技术学院|职业大学|大学|学院|人民政府|教育局|行业协会|协会|委员会|研究院|中心|学校))\s*(?:版权所有|All\s+Rights\s+Reserved)").unwrap();
    let english = regex::Regex::new(r"(?i)(?:copyright\s*(?:©|\(c\))?\s*(?:\d{4}(?:\s*[-–—]\s*\d{4})?)?\s*)([A-Za-z][A-Za-z0-9&'().,\- ]{5,120}?(?:Limited|Ltd\.?|Inc\.?|Corporation|University(?:\s+of\s+[A-Za-z][A-Za-z &\-]{1,60})?|College))(?:\s+All\s+Rights\s+Reserved|\s*[|©\x1f]|$)").unwrap();
    let mut result = BTreeMap::new();
    for captures in re
        .captures_iter(&text)
        .chain(reverse.captures_iter(&text))
        .chain(english.captures_iter(&text))
    {
        let Some(name) = captures.get(1).and_then(|m| legal_name(m.as_str())) else {
            continue;
        };
        let statement = captures
            .get(0)
            .unwrap()
            .as_str()
            .chars()
            .take(180)
            .collect::<String>();
        let kind = if statement.to_ascii_lowercase().contains("copyright")
            || statement.contains("版权所有")
            || statement.contains("Reserved")
        {
            "copyright"
        } else {
            "site_operator"
        };
        result
            .entry(name.clone())
            .or_insert((name, kind.to_string(), statement));
    }
    result.into_values().take(8).collect()
}
fn about_identities(html: &str) -> Vec<(String, String, String)> {
    let text = visible(html);
    let re=regex::Regex::new(r"([\p{Han}A-Za-z0-9（）()·&]{4,90}(?:股份有限公司|有限责任公司|有限公司|高等专科学校|职业技术大学|职业技术学院|职业大学|大学|学院|研究院|行业协会))\s*(?:（[^）]{0,50}）|\([^)]{0,50}\))?\s*(?:成立于|始建于|始创于|创建于|创办于|是一家|是一所|是由)").unwrap();
    re.captures_iter(&text)
        .filter_map(|c| {
            Some((
                legal_name(c.get(1)?.as_str())?,
                "about_identity".into(),
                c.get(0)?.as_str().chars().take(180).collect(),
            ))
        })
        .take(4)
        .collect()
}
fn registration(html: &str, url: &reqwest::Url) -> Option<String> {
    let hidden=regex::Regex::new(r"(?is)<!--.*?-->|<(?:script|style|template|noscript|article|blockquote)\b[^>]*>.*?</(?:script|style|template|noscript|article|blockquote)\s*>").unwrap();
    let html = hidden.replace_all(html, " ");
    let anchors =
        regex::Regex::new(r#"(?is)<a\b[^>]*href\s*=\s*["']([^"']+)["'][^>]*>(.*?)</a\s*>"#)
            .unwrap();
    let number = regex::Regex::new(r"[京津冀晋蒙辽吉黑沪苏浙皖闽赣鲁豫鄂湘粤桂琼渝川贵云藏陕甘青宁新蜀黔滇]\s*[Ii][Cc][Pp]\s*备\s*[:：]?\s*[0-9]{5,20}\s*号(?:\s*[-－]\s*[0-9]{1,8})?").unwrap();
    for c in anchors.captures_iter(&html) {
        let span = c.get(0)?;
        let before = html[..span.start()]
            .chars()
            .rev()
            .take(3000)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect::<String>();
        let after = html[span.end()..].chars().take(3000).collect::<String>();
        let left = visible(&before)
            .chars()
            .rev()
            .take(250)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect::<String>();
        let right = visible(&after).chars().take(250).collect::<String>();
        let context = format!("{left} {right}").to_ascii_lowercase();
        if !["版权所有", "copyright", "主办单位", "运营主体", "运营单位"]
            .iter()
            .any(|marker| context.contains(marker))
        {
            continue;
        }
        let Ok(link) = url.join(&entities(c.get(1)?.as_str())) else {
            continue;
        };
        if !matches!(link.scheme(), "http" | "https")
            || link.host_str() != Some("beian.miit.gov.cn")
            || !link.username().is_empty()
            || link.password().is_some()
        {
            continue;
        }
        let content = visible(c.get(2)?.as_str());
        if let Some(m) = number.find(&content) {
            return Some(m.as_str().chars().filter(|c| !c.is_whitespace()).collect());
        }
    }
    None
}
fn about_link(html: &str, page: &reqwest::Url, family: &str) -> Option<reqwest::Url> {
    let anchors =
        regex::Regex::new(r#"(?is)<a\b[^>]*href\s*=\s*["']([^"']+)["'][^>]*>(.*?)</a\s*>"#)
            .unwrap();
    for c in anchors.captures_iter(html) {
        let label = visible(c.get(2)?.as_str()).replace(' ', "");
        if ![
            "公司简介",
            "企业简介",
            "学校简介",
            "本行简介",
            "集团简介",
            "关于我们",
            "AboutUs",
            "CompanyProfile",
        ]
        .iter()
        .any(|s| label.eq_ignore_ascii_case(s))
        {
            continue;
        }
        let Ok(url) = page.join(&entities(c.get(1)?.as_str())) else {
            continue;
        };
        if checked_url(&url, family).is_ok()
            && url.host_str() == page.host_str()
            && url.query().is_none()
            && url.fragment().is_none()
            && !url.path().to_ascii_lowercase().ends_with(".pdf")
        {
            return Some(url);
        }
    }
    None
}
fn relationship(html: &str, organization: &str) -> Vec<(String, String, &'static str)> {
    // Relation claims must be the declared operator's own statement, rather
    // than a quoted article, client case, script, or another page's title.
    let excluded = regex::Regex::new(
        r"(?is)<(?:article|blockquote|title)\b[^>]*>.*?</(?:article|blockquote|title)\s*>",
    )
    .unwrap();
    let blocks =
        regex::Regex::new(r"(?is)</?(?:p|div|footer|section|li|tr|td|br|h[1-6])\b[^>]*>").unwrap();
    let clean = excluded.replace_all(html, " ");
    let text = visible(&blocks.replace_all(&clean, "\u{1f}"));
    let entity = r"[\p{Han}A-Za-z0-9（）()·&]{4,90}(?:股份有限公司|有限责任公司|有限公司)";
    let subsidiary = regex::Regex::new(&format!(
        r"{}\s*(?:是|系|为)?\s*({entity})\s*(?:的)?\s*(?:全资子公司|控股子公司|直属单位|下属企业)",
        regex::escape(organization)
    ))
    .unwrap();
    let identities = declarations(&clean)
        .into_iter()
        .chain(about_identities(&clean))
        .map(|v| v.0)
        .collect::<BTreeSet<_>>();
    // A pronoun can only bind to one complete legal operator on this page.
    // With mixed operators we still accept a fully named subject, never guess.
    let pronoun = if identities.len() == 1 && identities.contains(organization) {
        "|(?:本公司|本企业)(?:的)?"
    } else {
        ""
    };
    let controller = regex::Regex::new(&format!(
        r"^\s*(?:{}(?:的)?{pronoun})\s*(?:最终控制方|最终控股股东|实际控制人)\s*(?:是|为)\s*[:：]?\s*({entity})\s*(?:[,，（(]|$)",
        regex::escape(organization)
    ))
    .unwrap();
    let mut result = Vec::new();
    let mut seen = BTreeSet::new();
    for sentence in text.split(['\u{1f}', '。', '；', ';', '！', '？']) {
        for (re, kind) in [
            (&subsidiary, "subsidiary_declared"),
            (&controller, "ultimate_controller_declared"),
        ] {
            for c in re.captures_iter(sentence) {
                let Some(parent) = c.get(1).and_then(|m| legal_name(m.as_str())) else {
                    continue;
                };
                if parent == organization || !seen.insert((parent.clone(), kind)) {
                    continue;
                }
                result.push((
                    parent,
                    c.get(0).unwrap().as_str().chars().take(220).collect(),
                    kind,
                ));
                if result.len() == 4 {
                    return result;
                }
            }
        }
    }
    result
}
fn public_address(ip: IpAddr) -> bool {
    if !crate::asset_discovery::certificate_tasks::tls::public(ip) {
        return false;
    }
    match ip {
        IpAddr::V4(v) => !(v.octets()[0] == 192 && v.octets()[1] == 0 && v.octets()[2] == 0),
        IpAddr::V6(v) => !(v.segments()[0] == 0x2001 && v.segments()[1] == 0xdb8),
    }
}
fn remaining(start: Instant) -> Result<Duration, String> {
    Duration::from_secs(MAX_SECONDS)
        .checked_sub(start.elapsed())
        .filter(|d| *d > Duration::from_millis(200))
        .ok_or_else(|| "官网交叉查证达到单次时限，保留已取得证据并等待继续".into())
}
fn resolve_public(
    client: &reqwest::blocking::Client,
    name: &str,
    start: Instant,
) -> Result<Vec<SocketAddr>, String> {
    let mut errors = Vec::new();
    for endpoint in [
        "https://dns.google/resolve",
        "https://cloudflare-dns.com/dns-query",
    ] {
        let mut url = reqwest::Url::parse(endpoint).unwrap();
        url.query_pairs_mut()
            .append_pair("name", name)
            .append_pair("type", "A");
        let outcome = (|| -> Result<Vec<SocketAddr>, String> {
            let response = client
                .get(url)
                .timeout(remaining(start)?.min(Duration::from_secs(4)))
                .header(reqwest::header::ACCEPT, "application/dns-json")
                .send()
                .map_err(|e| format!("公开地址核验请求失败：{}", e.without_url()))?;
            if !response.status().is_success() {
                return Err(format!("公开地址核验 HTTP {}", response.status().as_u16()));
            }
            let mut bytes = Vec::new();
            response
                .take(128 * 1024 + 1)
                .read_to_end(&mut bytes)
                .map_err(err)?;
            if bytes.len() > 128 * 1024 {
                return Err("公开地址核验响应过大".into());
            }
            let v: Value = serde_json::from_slice(&bytes).map_err(|_| "公开地址核验非有效JSON")?;
            if v["Status"] != 0 || v["TC"] == true {
                return Err(format!(
                    "公开地址核验DNS Status={} TC={}",
                    v["Status"], v["TC"]
                ));
            }
            if v["Question"].as_array().is_none_or(|q| {
                q.len() != 1
                    || q[0]["type"] != 1
                    || q[0]["name"].as_str().and_then(host).as_deref() != Some(name)
            }) {
                return Err("公开地址核验请求绑定不符".into());
            }
            // Follow only the answer's actual CNAME chain; unrelated extra A
            // records cannot supply a safe-looking pin for a private candidate.
            let rows = v["Answer"].as_array().cloned().unwrap_or_default();
            let mut current = name.to_string();
            let mut visited = BTreeSet::new();
            for _ in 0..16 {
                let next = rows
                    .iter()
                    .filter(|r| {
                        r["type"] == 5
                            && r["name"].as_str().and_then(host).as_deref()
                                == Some(current.as_str())
                    })
                    .filter_map(|r| r["data"].as_str().and_then(host))
                    .collect::<BTreeSet<_>>();
                if next.is_empty() {
                    break;
                }
                if next.len() != 1 || !visited.insert(current.clone()) {
                    return Err("公开地址核验CNAME冲突或循环".into());
                }
                current = next.into_iter().next().unwrap();
            }
            let addresses = rows
                .iter()
                .filter(|r| {
                    r["type"] == 1
                        && r["name"].as_str().and_then(host).as_deref() == Some(current.as_str())
                })
                .filter_map(|r| r["data"].as_str().and_then(|s| s.parse::<IpAddr>().ok()))
                .collect::<BTreeSet<_>>();
            if addresses.iter().any(|ip| !public_address(*ip)) {
                return Err("官网域名解析到非公开地址，未连接".into());
            }
            if addresses.is_empty() {
                return Err("官网域名没有已核验的公开IPv4地址".into());
            }
            Ok(addresses
                .into_iter()
                .take(8)
                .map(|ip| SocketAddr::new(ip, 443))
                .collect())
        })();
        match outcome {
            Ok(v) => return Ok(v),
            Err(e) => errors.push(e),
        }
    }
    Err(errors.join("；"))
}
fn certificate_facts(der: &[u8], url: &reqwest::Url, checked: i64) -> Vec<Value> {
    if url.scheme() != "https" {
        return vec![];
    }
    let Some(name) = url.host_str() else {
        return vec![];
    };
    let (Some(cert), Some((before, after))) = (
        crate::certificates::from_der(der),
        passive_evidence::validity(der),
    ) else {
        return vec![];
    };
    if before > checked
        || after < checked
        || !cert
            .server_names()
            .iter()
            .any(|n| enrichment::certificate_covers(name, n))
    {
        return vec![];
    }
    cert.organizations.iter().filter(|org|!org.trim().is_empty()).map(|org|json!({"host":name,"organization":org,"source":"public_site_certificate","wildcard":false,"certificateSha256":cert.sha256,"sourceUrl":url.as_str(),"checkedAt":checked,"notAfter":after,"notBefore":before,"evidenceTier":"ca_attested","certificateNames":cert.server_names()})).collect()
}
struct Page {
    url: reqwest::Url,
    html: String,
    sha: String,
    leaf: Vec<Value>,
    about: bool,
}
fn page(
    client: &reqwest::blocking::Client,
    initial: reqwest::Url,
    family: &str,
    proxy: &str,
    start: Instant,
) -> Result<Page, String> {
    let mut url = initial;
    let mut seen = BTreeSet::new();
    for _ in 0..4 {
        checked_url(&url, family)?;
        if !seen.insert(url.to_string()) {
            return Err("官网跳转循环，未采纳".into());
        }
        let name = url.host_str().unwrap().to_string();
        let pins = resolve_public(client, &name, start)?
            .into_iter()
            .map(|a| SocketAddr::new(a.ip(), url.port_or_known_default().unwrap()))
            .collect::<Vec<_>>();
        let mut builder = reqwest::blocking::Client::builder()
            .connect_timeout(Duration::from_secs(4))
            .timeout(remaining(start)?.min(Duration::from_secs(8)))
            .redirect(reqwest::redirect::Policy::none())
            .user_agent("Oviraptor/PublicOwnershipEvidence")
            .tls_info(true)
            .resolve_to_addrs(&name, &pins);
        if !proxy.is_empty() {
            builder = builder.proxy(reqwest::Proxy::all(proxy).map_err(|_| "业务代理格式无效")?)
        }
        let specific = builder
            .build()
            .map_err(|e| format!("官网客户端初始化失败：{}", e.without_url()))?;
        let response = specific
            .get(url.clone())
            .header(reqwest::header::ACCEPT, "text/html,application/xhtml+xml")
            .send()
            .map_err(|e| format!("官网公开页面请求失败：{}；VPN保持原状", e.without_url()))?;
        if response.status().is_redirection() {
            let location = response
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|h| h.to_str().ok())
                .ok_or("官网跳转缺少有效Location")?;
            let next = url.join(location).map_err(|_| "官网跳转地址无效")?;
            checked_redirect(&url, &next, family)?;
            url = next;
            continue;
        }
        if !response.status().is_success() {
            return Err(format!(
                "官网公开页面 HTTP {}；未推断无关",
                response.status().as_u16()
            ));
        }
        let content = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|h| h.to_str().ok())
            .unwrap_or("")
            .to_string();
        if !content.is_empty()
            && !content.to_ascii_lowercase().contains("text/html")
            && !content
                .to_ascii_lowercase()
                .contains("application/xhtml+xml")
        {
            return Err("官网返回非HTML正文，未采纳为单位声明".into());
        }
        let leaf = response
            .extensions()
            .get::<reqwest::tls::TlsInfo>()
            .and_then(|v| v.peer_certificate())
            .map(|der| certificate_facts(der, &url, now()))
            .unwrap_or_default();
        if response
            .content_length()
            .is_some_and(|n| n > MAX_BODY as u64)
        {
            return Err("官网正文超过1MiB限制，未解析截断结果".into());
        }
        let mut bytes = Vec::new();
        response
            .take(MAX_BODY as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(err)?;
        if bytes.len() > MAX_BODY {
            return Err("官网正文超过1MiB限制，未解析截断结果".into());
        }
        let html = decode(&bytes, &content)?;
        return Ok(Page {
            url,
            html,
            sha: format!("{:x}", Sha256::digest(&bytes)),
            leaf,
            about: false,
        });
    }
    Err("官网跳转次数超出限制，未扩大请求范围".into())
}
fn facts(page: &Page, target: &str, checked: i64) -> Vec<Value> {
    let mut result = page.leaf.clone();
    let registration = registration(&page.html, &page.url);
    let origin = page.url.origin().ascii_serialization();
    let mut declarations = declarations(&page.html);
    if page.about {
        declarations.extend(about_identities(&page.html));
    }
    for (organization, kind, snippet) in declarations {
        let base = json!({"host":page.url.host_str(),"requestedHost":target,"root":root(target),"organization":organization,"source":"official_site_operator","sourceUrl":page.url.as_str(),"sourceOrigin":origin,"bodySha256":page.sha,"checkedAt":checked,"notAfter":checked+FRESH,"evidenceTier":"declared","declarationKind":kind,"evidenceSnippet":snippet,"registryVerified":false,"transport":page.url.scheme(),"tlsVerified":page.url.scheme()=="https"});
        result.push(base.clone());
        if let Some(number) = registration.as_ref() {
            let mut fact = base.clone();
            fact["source"] = json!("website_icp_declared");
            fact["registrationNumber"] = json!(number);
            fact["registrySourceUrl"] = json!("https://beian.miit.gov.cn/");
            fact["samePageDeclaration"] = json!(true);
            result.push(fact)
        }
        for (parent, snippet, relationship) in relationship(&page.html, &organization) {
            let mut fact = base.clone();
            fact["source"] = json!("official_unit_relation");
            fact["parentOrganization"] = json!(parent);
            fact["evidenceSnippet"] = json!(snippet);
            fact["relationship"] = json!(relationship);
            result.push(fact)
        }
    }
    result
}
/// At most root/www homepage attempts and one same-origin About page within a
/// single 28s operation. No login, cookies, script, page traversal or paid API.
pub(super) fn execute(
    client: &reqwest::blocking::Client,
    target: &str,
    proxy: &str,
) -> Result<Value, String> {
    if host(target).as_deref() != Some(target) {
        return Err("官网候选不是规范域名，未请求".into());
    }
    let family = root(target).ok_or("官网候选缺少有效可注册根域")?;
    let start = Instant::now();
    let mut attempts = Vec::new();
    let mut pages = Vec::new();
    let candidates = homepage_candidates(target, &family);
    let mut planned = candidates.clone();
    for candidate in candidates {
        if remaining(start).is_err() {
            attempts.push(json!({"requestedUrl":candidate,"sourceUrl":candidate,"state":"not_attempted","error":"官网交叉查证达到单次时限，既定入口尚未读取；保留本轮未完成状态","checkedAt":now()}));
            continue;
        }
        let url = reqwest::Url::parse(&candidate).map_err(err)?;
        let mut result = page(client, url.clone(), &family, proxy, start);
        if let Err(error) = &result {
            attempts.push(json!({"sourceUrl":candidate,"state":"unavailable","error":error,"checkedAt":now()}));
            if may_try_http(error) && remaining(start).is_ok() {
                let mut fallback = url.clone();
                fallback
                    .set_scheme("http")
                    .map_err(|_| "同主机HTTP补查地址无效")?;
                result = page(client, fallback.clone(), &family, proxy, start);
                if let Err(error) = &result {
                    attempts.push(json!({"sourceUrl":fallback.as_str(),"state":"unavailable","error":error,"checkedAt":now(),"tlsVerified":false}))
                }
            }
        }
        if let Ok(v) = result {
            attempts.push(json!({"requestedUrl":candidate,"sourceUrl":v.url.as_str(),"state":"done","bodySha256":v.sha,"checkedAt":now(),"transport":v.url.scheme(),"tlsVerified":v.url.scheme()=="https"}));
            let identity = !declarations(&v.html).is_empty();
            pages.push(v);
            if identity {
                break;
            }
        }
    }
    if pages.is_empty() {
        return Err(format!(
            "官网交叉查证未完成：{}",
            attempts
                .iter()
                .map(|v| format!(
                    "{}：{}",
                    v["sourceUrl"].as_str().unwrap_or(""),
                    v["error"].as_str().unwrap_or("未返回主体证据")
                ))
                .collect::<Vec<_>>()
                .join("；")
        ));
    }
    if let Some(link) = pages
        .iter()
        .find_map(|p| about_link(&p.html, &p.url, &family))
    {
        // Discovering an About URL is a planned dependency even if the shared
        // time budget expires before the request can start. Never silently
        // call that bounded pass complete just because a homepage succeeded.
        planned.push(link.as_str().to_owned());
        if remaining(start).is_ok() {
            match page(client,link.clone(),&family,proxy,start){
                Ok(mut v)=>{v.about=true;attempts.push(json!({"requestedUrl":link.as_str(),"sourceUrl":v.url.as_str(),"state":"done","bodySha256":v.sha,"checkedAt":now()}));pages.push(v)},
                Err(e)=>attempts.push(json!({"requestedUrl":link.as_str(),"sourceUrl":link.as_str(),"state":"unavailable","error":e,"checkedAt":now()})),
            }
        } else {
            attempts.push(json!({"requestedUrl":link.as_str(),"sourceUrl":link.as_str(),"state":"not_attempted","error":"官网交叉查证达到单次时限，既定 About 页面尚未读取；保留本轮未完成状态","checkedAt":now()}));
        }
    }
    finalize(pages, attempts, planned, target, &family, now())
}
fn homepage_candidates(target: &str, family: &str) -> Vec<String> {
    if target == family {
        vec![
            format!("https://www.{target}/"),
            format!("https://{target}/"),
        ]
    } else {
        vec![format!("https://{target}/")]
    }
}
fn finalize(
    pages: Vec<Page>,
    attempts: Vec<Value>,
    planned: Vec<String>,
    target: &str,
    family: &str,
    checked: i64,
) -> Result<Value, String> {
    // A completed HTTP read with no identity is a bounded missing result, not
    // a transport failure. Require every planned logical request and its final
    // response/body receipt; redirected www/apex URLs must remain distinct
    // requestedUrl bindings. Any failed/unfinished alternate keeps retrying.
    let mut required = homepage_candidates(target, family);
    if let Some(about) = pages
        .iter()
        .filter(|p| !p.about)
        .find_map(|p| about_link(&p.html, &p.url, family))
    {
        required.push(about.as_str().to_owned());
    }
    let bound = |attempt: &Value| {
        let Some(source) = attempt["sourceUrl"].as_str() else {
            return false;
        };
        let Some(requested) = attempt["requestedUrl"].as_str() else {
            return false;
        };
        let sha = attempt["bodySha256"].as_str().unwrap_or("");
        attempt["state"] == "done"
            && attempt["error"].as_str().is_none_or(str::is_empty)
            && planned.iter().any(|url| url == requested)
            && reqwest::Url::parse(requested).is_ok_and(|u| checked_url(&u, family).is_ok())
            && reqwest::Url::parse(source).is_ok_and(|u| checked_url(&u, family).is_ok())
            && sha.len() == 64
            && sha.bytes().all(|b| b.is_ascii_hexdigit())
            && pages
                .iter()
                .any(|page| page.url.as_str() == source && page.sha == sha)
    };
    let bounded_pass_complete = !pages.is_empty()
        && !planned.is_empty()
        && pages.len() == attempts.len()
        && required.iter().all(|url| planned.contains(url))
        && attempts.iter().all(bound)
        && planned.iter().all(|url| {
            attempts
                .iter()
                .any(|a| a["requestedUrl"] == *url && bound(a))
        });
    let results = pages
        .iter()
        .flat_map(|p| facts(p, target, checked))
        .collect::<Vec<_>>();
    if results.is_empty() && !bounded_pass_complete {
        let receipts = attempts
            .iter()
            .map(|v| {
                format!(
                    "{}：{}；bodySha256={}",
                    v["sourceUrl"].as_str().unwrap_or(""),
                    v["error"]
                        .as_str()
                        .unwrap_or("HTTP成功但未找到完整法律主体声明或可用证书主体"),
                    v["bodySha256"].as_str().unwrap_or("未取得正文")
                )
            })
            .collect::<Vec<_>>()
            .join("；");
        return Err(format!(
            "官网交叉查证缺少主体证据，保留候选并按本域退避复查；不是无关结论：{receipts}"
        ));
    }
    let identity = results
        .iter()
        .any(|f| f["source"] == "official_site_operator");
    Ok(
        json!({"kind":"public_ownership","target":target,"root":family,"sourceUrl":pages[0].url.as_str(),"bodySha256":pages[0].sha,"checkedAt":checked,"facts":results,"sourceAttempts":attempts,"registryVerified":false,"registryStatus":"website_declaration_only","registryQueryUrl":"https://beian.miit.gov.cn/#/Integrated/recordQuery","complete":identity,"identityFound":identity,"outcome":if results.is_empty(){"missing_identity"}else{"evidence_collected"},"boundedPassComplete":bounded_pass_complete,"plannedSources":planned,"coverage":"bounded_public_website_declarations","ownershipConclusion":false,"hunterRequests":0,"networkPolicy":"configured_proxy_preserved_public_dns_preflight","proxyResolutionBoundary":"explicit_or_environment_proxy_resolves_CONNECT_host_remotely"}),
    )
}

#[cfg(test)]
#[path = "discovery_cleanup_ownership_tests.rs"]
mod tests;
