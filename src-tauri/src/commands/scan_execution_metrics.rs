
fn is_directory_block_signal(output: &str) -> bool {
    let value = output.to_ascii_lowercase();
    // Strong, unambiguous block markers (a real WAF, rate limiter or JS
    // challenge). These are a block wherever they appear.
    let strong = [
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
    if strong {
        return true;
    }
    // Weak challenge tokens. A page that merely *renders* a captcha field or says
    // "verify you are human" is ordinary content — a normal login/registration
    // form with a 验证码 input is NOT a WAF. Treat these as a block only when the
    // response is actually a block, never on a 200/404 that just displays them.
    // (Same discipline as hard_fuse_reason; a bare captcha mention used to flag a
    // plain 200 login page as protected and abort the whole scan.)
    let weak = ["captcha", "verify you are human", "验证码", "人机验证"]
        .iter()
        .any(|needle| value.contains(needle));
    if !weak {
        return false;
    }
    if value.contains("http 404")
        || value.contains("status: 404")
        || value.contains("status_code\":404")
        || ((value.contains("http 200") || value.contains("status: 200"))
            && (value.contains("<img")
                || value.contains("src=")
                || value.contains("captchaimage")
                || value.contains("<form")
                || value.contains("<input")
                || value.contains("placeholder")))
    {
        return false;
    }
    true
}

#[cfg(test)]
mod directory_block_signal_tests {
    use super::is_directory_block_signal;

    #[test]
    fn ruoyi_login_form_with_captcha_field_is_not_a_block() {
        // A normal 200 HTML login page that merely offers a 验证码 input (RuoYi WMS
        // login). It used to be flagged protected and abort the whole scan.
        let page = "http 200 text/html;charset=utf-8 <form method=\"POST\"> \
            <input type=\"text\" name=\"validateCode\" placeholder=\"验证码\" maxlength=\"5\"> \
            <img src=\"captcha/captchaImage?type=char\"> var needCaptcha='0';";
        assert!(!is_directory_block_signal(page));
    }

    #[test]
    fn strong_waf_and_rate_limit_markers_remain_a_block() {
        assert!(is_directory_block_signal("http 403 cloudflare challenge cf-chl-"));
        assert!(is_directory_block_signal("http 429 too many requests"));
        assert!(is_directory_block_signal("status: 429 rate limit"));
        assert!(is_directory_block_signal("web application firewall blocked this request"));
        assert!(is_directory_block_signal("http 200 <div>js challenge</div>")); // strong wins over 200
    }

    #[test]
    fn a_bare_challenge_response_body_is_still_a_block() {
        // Callers that pass a raw challenge body (no status/form markers) keep the
        // prior behavior: a weak token alone still counts as a block.
        assert!(is_directory_block_signal("请完成人机验证 to continue"));
        assert!(is_directory_block_signal("验证码 required before access"));
    }

    #[test]
    fn captcha_mention_on_404_or_plain_pages_is_not_a_block() {
        assert!(!is_directory_block_signal("http 404 not found captcha"));
        assert!(!is_directory_block_signal("http 200 text/html <p>ordinary content</p>"));
        assert!(!is_directory_block_signal("http 200 <form><input placeholder=\"captcha\"></form>"));
    }
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
