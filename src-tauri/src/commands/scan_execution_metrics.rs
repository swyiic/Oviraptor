/// Directory discovery is a bounded exception for ordinary server-rendered
/// Web targets. Recognize both first-class tools and shell wrappers so the
/// runtime fuse can stop repeated wordlist scans.
#[cfg(test)]
fn is_directory_discovery_tool(name: &str, detail: &str) -> bool {
    let haystack = format!("{} {}", name, detail).to_ascii_lowercase();
    [
        "ffuf",
        "dirsearch",
        "gobuster",
        "feroxbuster",
        "ferox",
        "wfuzz",
    ]
    .iter()
    .any(|needle| haystack.contains(needle))
}

fn is_directory_block_signal(output: &str) -> bool {
    let value = output.to_ascii_lowercase();
    [
        "429 too many requests",
        "status: 429",
        "status_code\":429",
        "rate limit",
        "captcha",
        "cloudflare challenge",
        "cloudflare ray id",
        "cf-chl-",
        "aws waf",
        "akamai reference",
        "incapsula incident",
        "verify you are human",
        "waf blocked",
        "web application firewall",
        "js challenge",
        "验证码",
        "人机验证",
    ]
    .iter()
    .any(|needle| value.contains(needle))
}

fn hard_fuse_reason(reason: &str) -> bool {
    let value = reason.to_ascii_lowercase();
    let strong_signal = [
        "429 too many requests",
        "status: 429",
        "status_code\":429",
        "rate limit",
        "cloudflare challenge",
        "cloudflare ray id",
        "cf-chl-",
        "aws waf",
        "akamai reference",
        "incapsula incident",
        "waf blocked",
        "web application firewall",
        "js challenge",
    ]
    .iter()
    .any(|needle| value.contains(needle));
    if strong_signal {
        return true;
    }
    let challenge_signal = ["captcha", "verify you are human", "验证码", "人机验证"]
        .iter()
        .any(|needle| value.contains(needle));
    if !challenge_signal {
        return false;
    }
    // A URL, image tag, or a not-found response mentioning a captcha is only
    // passive discovery evidence. Fuse only an observed challenge response or
    // an explicit operator/runtime classification.
    if value.contains("http 404")
        || value.contains("status: 404")
        || value.contains("status_code\":404")
        || ((value.contains("http 200") || value.contains("status: 200"))
            && (value.contains("<img") || value.contains("src=") || value.contains("captchaimage")))
    {
        return false;
    }
    true
}
