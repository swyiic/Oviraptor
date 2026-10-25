use super::*;

#[test]
fn cleanup_ownership_declaration_requires_operator_not_arbitrary_news_or_script() {
    let html = r#"<title>广州农村商业银行股份有限公司</title><p>合作客户：中国移动通信集团有限公司</p><script>版权所有：错误技术有限公司</script><footer>版权所有：广州农村商业银行股份有限公司 <a href="https://beian.miit.gov.cn/">粤ICP备09040001号-1</a></footer>"#;
    let found = declarations(html);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].0, "广州农村商业银行股份有限公司");
    assert!(declarations(
        "<title>中国移动通信集团有限公司</title><p>客户是广州农村商业银行股份有限公司</p>"
    )
    .is_empty());
    assert!(declarations("<article>他在文章中写：版权所有 广州错误有限公司</article>").is_empty());
    assert!(declarations("<footer>技术支持：广州某某技术有限公司</footer>").is_empty());
    assert!(
        declarations("<p>Copyright 2026 Guangzhou</p><p>中国移动通信集团有限公司</p>").is_empty()
    );
    assert!(declarations(
        "<script>版权所有 广州错误有限公司</script><!--版权所有：广州错误有限公司-->"
    )
    .is_empty());
}
#[test]
fn cleanup_ownership_registration_requires_actual_number_and_official_link() {
    let url = reqwest::Url::parse("https://www.guub.cn/").unwrap();
    assert_eq!(
        registration(
            r#"<footer>版权所有 广州国保科技有限公司 <a href="https://beian.miit.gov.cn/">粤ICP备16003366号-2</a></footer>"#,
            &url
        )
        .as_deref(),
        Some("粤ICP备16003366号-2")
    );
    for html in [
        r#"<a href="https://beian.miit.gov.cn.evil.example/">粤ICP备16003366号-2</a>"#,
        r#"<a href="https://beian.miit.gov.cn/">ICP备案</a>"#,
        r#"<p>粤ICP备16003366号-2</p>"#,
        r#"<a href="https://user@beian.miit.gov.cn/">粤ICP备16003366号-2</a>"#,
    ] {
        assert!(registration(html, &url).is_none(), "{html}")
    }
}
#[test]
fn cleanup_ownership_source_is_https_same_family_no_credentials_private_or_port() {
    for raw in [
        "https://guub.cn@127.0.0.1/",
        "https://www.guub.cn:444/",
        "https://www.guub.cn.evil.example/",
        "https://user:pass@www.guub.cn/",
        "https://127.0.0.1/",
        "file:///etc/passwd",
    ] {
        assert!(
            checked_url(&reqwest::Url::parse(raw).unwrap(), "guub.cn").is_err(),
            "{raw}"
        );
    }
    assert!(checked_url(
        &reqwest::Url::parse("https://www.guub.cn/about").unwrap(),
        "guub.cn"
    )
    .is_ok());
    for ip in [
        "127.0.0.1",
        "10.1.2.3",
        "169.254.169.254",
        "100.64.0.1",
        "198.18.0.1",
        "192.0.0.1",
        "192.0.2.1",
        "::1",
        "fc00::1",
        "2001:db8::1",
    ] {
        assert!(!public_address(ip.parse().unwrap()), "{ip}")
    }
    assert!(public_address("8.8.8.8".parse().unwrap()));
}
#[test]
fn cleanup_ownership_about_link_is_one_explicit_same_origin_page() {
    let page = reqwest::Url::parse("https://www.guub.cn/").unwrap();
    let html = r#"<a href="https://other.guub.cn/about">公司简介</a><a href="https://evil.example/a">关于我们</a><a href="/uploads/file.pdf">公司简介</a><a href="/about/company.html">公司简介</a>"#;
    assert_eq!(
        about_link(html, &page, "guub.cn").unwrap().as_str(),
        "https://www.guub.cn/about/company.html"
    );
}
#[test]
fn cleanup_ownership_bound_receipt_distinguishes_declared_icp_from_registry_query() {
    let page=Page{url:reqwest::Url::parse("https://www.guub.cn/").unwrap(),html:r#"<footer>版权所有 广州国保科技有限公司 <a href="https://beian.miit.gov.cn/">粤ICP备16003366号-2</a></footer>"#.into(),sha:"a".repeat(64),leaf:vec![],about:false};
    let facts = facts(&page, "guub.cn", 100);
    assert_eq!(facts.len(), 2);
    assert!(facts.iter().all(|f| f["host"] == "www.guub.cn"
        && f["bodySha256"] == "a".repeat(64)
        && f["notAfter"] == 100 + FRESH
        && f["registryVerified"] == false));
    assert_eq!(facts[1]["registrationNumber"], "粤ICP备16003366号-2");
    assert_eq!(facts[1]["evidenceTier"], "declared");
    assert!(facts
        .iter()
        .all(|f| f["organization"] == "广州国保科技有限公司"));
}
#[test]
fn cleanup_ownership_relation_requires_legal_child_subject_and_explicit_control() {
    let child = "中移示例技术有限公司";
    assert_eq!(
        relationship(
            "中移示例技术有限公司是中国移动通信集团有限公司的全资子公司",
            child
        )
        .len(),
        1
    );
    assert!(relationship(
        "客户：中国移动通信集团有限公司；伙伴：中移示例技术有限公司",
        child
    )
    .is_empty());
    assert!(relationship(
        "中移示例技术有限公司是中国移动通信集团有限公司的供应商",
        child
    )
    .is_empty());
}
#[test]
fn cleanup_ownership_ultimate_controller_is_explicit_full_entity_and_source_bound() {
    let child = "启明星辰信息技术集团股份有限公司";
    let parent = "中国移动通信集团有限公司";
    for statement in [
        format!("{child}的最终控制方是{parent}。"),
        format!("{child}的最终控股股东是{parent}（简称中国移动集团）。"),
        format!("{child}实际控制人为{parent}。"),
        format!("{child}成立于1996年。 本公司最终控制方是{parent}。"),
        format!("{child}成立于1996年。 本企业的实际控制人为{parent}。"),
    ] {
        let html = format!("<p>{statement}</p><footer>版权所有：{child}</footer>");
        let found = relationship(&html, child);
        assert_eq!(found.len(), 1, "{found:?} for {statement}");
        assert_eq!(found[0].0, parent);
        assert_eq!(found[0].2, "ultimate_controller_declared");
    }
    let page = Page {
        url: reqwest::Url::parse("https://www.venusgroup.com.cn/about/").unwrap(),
        html: format!("<p>{child}成立于1996年。</p><p>本公司的最终控制方是{parent}。</p>"),
        sha: "b".repeat(64),
        leaf: vec![],
        about: true,
    };
    let found = facts(&page, "venusgroup.com.cn", 100);
    let relation = found
        .iter()
        .find(|f| f["source"] == "official_unit_relation")
        .unwrap();
    assert_eq!(relation["organization"], child);
    assert_eq!(relation["parentOrganization"], parent);
    assert_eq!(relation["relationship"], "ultimate_controller_declared");
    assert_eq!(relation["bodySha256"], page.sha);
    assert_eq!(relation["sourceUrl"], page.url.as_str());
    assert_eq!(relation["checkedAt"], 100);
    assert_eq!(relation["notAfter"], 100 + FRESH);
    assert_eq!(relation["registryVerified"], false);
}
#[test]
fn cleanup_ownership_ultimate_control_does_not_infer_from_customer_founder_or_shareholding() {
    let child = "启明星辰信息技术集团股份有限公司";
    let parent = "中国移动通信集团有限公司";
    for statement in [
        format!("{child}是独立企业，与{parent}没有明确关系资料。"),
        format!("{parent}是{child}的发起人。"),
        format!("{parent}持有{child}27.93%的股份。"),
        format!("{child}的客户是{parent}。"),
        format!("{child}的控股股东是{parent}。"),
        format!("{child}的实际控制人不是{parent}。"),
        format!("{child}的实际控制人是中国移动。"),
        format!("传闻{child}的实际控制人是{parent}。"),
        format!("<article>{child}的实际控制人是{parent}。</article>"),
        format!("<blockquote>{child}的实际控制人是{parent}。</blockquote>"),
        format!("<script>{child}的实际控制人是{parent}。</script>"),
        format!("<title>{child}的实际控制人是{parent}。</title>"),
        format!("其他银行股份有限公司的实际控制人是{parent}。"),
    ] {
        let html = format!("<div>{statement}</div><footer>版权所有：{child}</footer>");
        assert!(relationship(&html, child).is_empty(), "{statement}");
    }
    // Ambiguous page operators cannot bind a pronoun to either company.
    let mixed = format!("<footer>版权所有：{child}</footer><p>运营单位：其他银行股份有限公司</p><p>本公司的实际控制人是{parent}。</p>");
    assert!(relationship(&mixed, child).is_empty());
    assert!(relationship(&format!("本公司最终控制方是{parent}"), child).is_empty());
}
#[test]
fn cleanup_ownership_gbk_decode_is_bounded_and_invalid_utf8_is_not_replaced() {
    let (raw, _, errors) = encoding_rs::GBK.encode("版权所有：广州国保科技有限公司");
    assert!(!errors);
    assert_eq!(
        decode(&raw, "text/html;charset=gb2312").unwrap(),
        "版权所有：广州国保科技有限公司"
    );
    assert!(decode(&[0xff, 0xfe, 0x7f], "text/html;charset=utf-8").is_err());
}
#[test]
#[ignore = "explicit public website validation; no credentials, paid APIs, or project data"]
fn cleanup_ownership_live_public_examples() {
    let client = passive_evidence::client("").unwrap();
    let dir =
        std::env::var("OVI_OWNERSHIP_LIVE_OUTPUT").expect("private output directory required");
    std::fs::create_dir_all(&dir).unwrap();
    for name in ["cdtc.edu.cn", "guub.cn", "bjut.edu.cn", "grcbank.com"] {
        let value = match execute(&client, name, "") {
            Ok(v) => v,
            Err(e) => json!({"target":name,"error":e,"hunterRequests":0}),
        };
        std::fs::write(
            std::path::Path::new(&dir).join(format!("{name}.json")),
            serde_json::to_vec_pretty(&value).unwrap(),
        )
        .unwrap();
    }
}

#[test]
fn cleanup_ownership_about_identity_is_explicit_subject_and_not_homepage_news() {
    let body="<h1>公司简介</h1><p>广州国保科技有限公司成立于1992年。</p><p>合作客户：中国移动通信集团有限公司</p>";
    let found = about_identities(body);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].0, "广州国保科技有限公司");
    let mut page = Page {
        url: reqwest::Url::parse("https://www.guub.cn/about.html").unwrap(),
        html: body.into(),
        sha: "a".repeat(64),
        leaf: vec![],
        about: false,
    };
    assert!(facts(&page, "guub.cn", 100).is_empty());
    page.about = true;
    assert_eq!(
        facts(&page, "guub.cn", 100)[0]["declarationKind"],
        "about_identity"
    );
}

#[test]
fn cleanup_ownership_actual_leaf_requires_exact_name_validity_and_der_hash() {
    let der = include_bytes!("../../test-fixtures/tls/cleanup-organization.der");
    let (before, after) = passive_evidence::validity(der).unwrap();
    let url = reqwest::Url::parse("https://owned.cleanup-fixture.test/").unwrap();
    let facts = certificate_facts(der, &url, before + 1);
    assert_eq!(facts.len(), 1);
    assert_eq!(facts[0]["organization"], "Other Bank Limited");
    assert_eq!(
        facts[0]["certificateSha256"],
        format!("{:x}", Sha256::digest(der))
    );
    assert!(certificate_facts(
        der,
        &reqwest::Url::parse("https://other.cleanup-fixture.test/").unwrap(),
        before + 1
    )
    .is_empty());
    assert!(certificate_facts(der, &url, after + 1).is_empty());
    assert!(certificate_facts(b"not a certificate", &url, before + 1).is_empty());
}

#[test]
fn cleanup_ownership_english_legal_copyright_keeps_exact_entity_not_brand() {
    let facts = declarations(
        "<footer>Copyright © 2026 Beijing Normal University All Rights Reserved</footer>",
    );
    assert_eq!(facts.len(), 1);
    assert_eq!(facts[0].0, "Beijing Normal University");
    assert!(declarations("<footer>Copyright © 2026 Guub www.guub.cn</footer>").is_empty());
}
#[test]
fn cleanup_ownership_canonical_homepage_preferred_and_partial_empty_shell_is_retryable() {
    assert_eq!(
        homepage_candidates("college.test", "college.test"),
        vec!["https://www.college.test/", "https://college.test/"]
    );
    assert_eq!(
        homepage_candidates("console.college.test", "college.test"),
        vec!["https://console.college.test/"]
    );
    let page = Page {
        url: reqwest::Url::parse("https://www.college.test/").unwrap(),
        html: "<html><script>renderSite()</script></html>".into(),
        sha: "a".repeat(64),
        leaf: vec![],
        about: false,
    };
    let error = finalize(
        vec![page],
        vec![json!({"requestedUrl":"https://www.college.test/","sourceUrl":"https://www.college.test/","state":"done","bodySha256":"a".repeat(64)})],
        homepage_candidates("college.test","college.test"),
        "college.test",
        "college.test",
        100,
    )
    .unwrap_err();
    assert!(error.contains("不是无关结论"));
    assert!(error.contains("https://www.college.test/"));
    assert!(error.contains(&"a".repeat(64)));
}
#[test]
fn cleanup_ownership_http_is_same_host_declared_and_never_ca_verified() {
    let https = reqwest::Url::parse("https://www.guub.cn/").unwrap();
    let http = reqwest::Url::parse("http://www.guub.cn/").unwrap();
    assert!(checked_redirect(&https, &http, "guub.cn").is_ok());
    assert!(checked_redirect(&http, &https, "guub.cn").is_ok());
    assert!(checked_redirect(
        &https,
        &reqwest::Url::parse("http://other.guub.cn/").unwrap(),
        "guub.cn"
    )
    .is_err());
    assert!(checked_redirect(
        &http,
        &reqwest::Url::parse("https://other.guub.cn/").unwrap(),
        "guub.cn"
    )
    .is_err());
    assert!(checked_url(
        &reqwest::Url::parse("http://www.guub.cn:8080/").unwrap(),
        "guub.cn"
    )
    .is_err());
    assert!(may_try_http(
        "官网公开页面请求失败：TLS certificate expired"
    ));
    assert!(!may_try_http("官网公开页面 HTTP 403"));
    assert!(!may_try_http("官网公开页面 HTTP 429"));
    let page = Page {
        url: http.clone(),
        html: "<footer>版权所有 广州国保科技有限公司</footer>".into(),
        sha: "a".repeat(64),
        leaf: vec![],
        about: false,
    };
    let fact = facts(&page, "guub.cn", 100).remove(0);
    assert_eq!(fact["transport"], "http");
    assert_eq!(fact["tlsVerified"], false);
    assert!(certificate_facts(
        include_bytes!("../../test-fixtures/tls/cleanup-organization.der"),
        &http,
        100
    )
    .is_empty());
}

#[test]
fn cleanup_ownership_registration_news_scripts_and_unrelated_body_link_cannot_bind_operator() {
    let url = reqwest::Url::parse("https://www.college.test/").unwrap();
    for html in [
        r#"<article>版权所有 新闻来源示例 <a href="https://beian.miit.gov.cn/">京ICP备12345678号</a></article>"#,
        r#"<script>var template='<footer>版权所有 <a href="https://beian.miit.gov.cn/">京ICP备12345678号</a></footer>';</script>"#,
        r#"<p>案例中的备案号 <a href="https://beian.miit.gov.cn/">京ICP备12345678号</a></p>"#,
    ] {
        assert!(registration(html, &url).is_none(), "{html}")
    }
    let positive = r#"<div class="copyright">版权所有 © 重庆邮电大学 <a href="https://beian.miit.gov.cn/">渝ICP备:17002788号-1</a></div>"#;
    assert_eq!(
        registration(positive, &url).as_deref(),
        Some("渝ICP备:17002788号-1")
    );
    let english = declarations("<p>COPYRIGHT © BEIJING UNIVERSITY OF TECHNOLOGY</p>");
    assert_eq!(english[0].0, "BEIJING UNIVERSITY OF TECHNOLOGY");
    assert!(!english.iter().any(|v| v.0 == "BEIJING UNIVERSITY"));
}

fn blank_page(url: &str, html: &str, about: bool) -> Page {
    Page {
        url: reqwest::Url::parse(url).unwrap(),
        html: html.into(),
        sha: format!("{:x}", Sha256::digest(html.as_bytes())),
        leaf: vec![],
        about,
    }
}
fn successful_read(requested: &str, page: &Page) -> Value {
    json!({"requestedUrl":requested,"sourceUrl":page.url.as_str(),"state":"done","bodySha256":page.sha,"checkedAt":100})
}
#[test]
fn cleanup_ownership_fully_read_empty_homepages_are_missing_identity_not_a_transport_retry() {
    let www = blank_page(
        "https://www.college.test/",
        "<html><script>renderSite()</script></html>",
        false,
    );
    let apex = blank_page(
        "https://college.test/",
        "<html><p>Welcome</p></html>",
        false,
    );
    let attempts = vec![
        successful_read("https://www.college.test/", &www),
        successful_read("https://college.test/", &apex),
    ];
    let result = finalize(
        vec![www, apex],
        attempts.clone(),
        homepage_candidates("college.test", "college.test"),
        "college.test",
        "college.test",
        100,
    )
    .unwrap();
    assert_eq!(result["outcome"], "missing_identity");
    assert_eq!(result["boundedPassComplete"], true);
    assert_eq!(result["identityFound"], false);
    assert_eq!(
        result["complete"], false,
        "complete identity is never inferred from an empty body"
    );
    assert_eq!(result["facts"], json!([]));
    assert_eq!(result["ownershipConclusion"], false);
    assert_eq!(result["hunterRequests"], 0);
    assert_eq!(result["sourceAttempts"], json!(attempts));
    assert_eq!(
        result["plannedSources"],
        json!(["https://www.college.test/", "https://college.test/"])
    );
    assert_eq!(result["coverage"], "bounded_public_website_declarations");
    assert_eq!(result["checkedAt"], 100);
}
#[test]
fn cleanup_ownership_empty_homepages_and_discovered_about_must_all_be_read_before_settling() {
    let www = blank_page(
        "https://www.college.test/",
        r#"<a href="/about">AboutUs</a>"#,
        false,
    );
    let apex = blank_page(
        "https://college.test/",
        "<html><p>Welcome</p></html>",
        false,
    );
    let about = blank_page(
        "https://www.college.test/about",
        "<html><p>Information coming soon</p></html>",
        true,
    );
    let attempts = vec![
        successful_read("https://www.college.test/", &www),
        successful_read("https://college.test/", &apex),
        successful_read("https://www.college.test/about", &about),
    ];
    let mut planned = homepage_candidates("college.test", "college.test");
    planned.push("https://www.college.test/about".into());
    let result = finalize(
        vec![www, apex, about],
        attempts,
        planned,
        "college.test",
        "college.test",
        100,
    )
    .unwrap();
    assert_eq!(result["outcome"], "missing_identity");
    assert_eq!(result["boundedPassComplete"], true);
    assert_eq!(result["identityFound"], false);
    assert_eq!(result["sourceAttempts"].as_array().unwrap().len(), 3);
}
#[test]
fn cleanup_ownership_missing_identity_never_settles_partial_budget_failed_or_truncated_about() {
    for case in [
        "budget",
        "unavailable",
        "truncated",
        "unplanned_about",
        "missing_receipt",
    ] {
        let www = blank_page(
            "https://www.college.test/",
            r#"<a href="/about">AboutUs</a>"#,
            false,
        );
        let apex = blank_page(
            "https://college.test/",
            "<html><p>Welcome</p></html>",
            false,
        );
        let mut planned = homepage_candidates("college.test", "college.test");
        let mut attempts = vec![
            successful_read("https://www.college.test/", &www),
            successful_read("https://college.test/", &apex),
        ];
        if case != "unplanned_about" {
            planned.push("https://www.college.test/about".into());
        }
        if !matches!(case, "missing_receipt" | "unplanned_about") {
            attempts.push(json!({"requestedUrl":"https://www.college.test/about","sourceUrl":"https://www.college.test/about","state":if case=="budget"{"not_attempted"}else{"unavailable"},"error":match case{"budget"=>"官网交叉查证达到单次时限，About尚未读取","truncated"=>"官网正文超过1MiB限制，未解析截断结果",_=>"HTTP 502"},"checkedAt":100}));
        }
        let result = finalize(
            vec![www, apex],
            attempts,
            planned,
            "college.test",
            "college.test",
            100,
        );
        assert!(
            result.is_err(),
            "discovered About must not disappear from the plan or success receipt: {case}"
        );
        let error = result.unwrap_err();
        assert!(error.contains("不是无关结论"));
        assert!(error.contains("bodySha256="));
    }
}
#[test]
fn cleanup_ownership_missing_identity_requires_every_homepage_and_real_bound_body_receipt() {
    for case in [
        "unattempted_apex",
        "omitted_plan",
        "failed_apex",
        "forged_done",
        "wrong_hash",
        "wrong_final_url",
        "absent_requested_url",
    ] {
        let www = blank_page(
            "https://www.college.test/",
            "<html><p>Welcome</p></html>",
            false,
        );
        let apex = blank_page(
            "https://college.test/",
            "<html><p>Welcome</p></html>",
            false,
        );
        let mut attempts = vec![
            successful_read("https://www.college.test/", &www),
            successful_read("https://college.test/", &apex),
        ];
        let mut planned = homepage_candidates("college.test", "college.test");
        let mut pages = vec![www, apex];
        match case {
            "unattempted_apex" => {
                pages.pop();
                attempts.pop();
            }
            "omitted_plan" => {
                planned.pop();
            }
            "failed_apex" => {
                pages.pop();
                attempts[1] = json!({"requestedUrl":"https://college.test/","sourceUrl":"https://college.test/","state":"unavailable","error":"HTTP 503"});
            }
            "forged_done" => {
                pages.pop();
                attempts[1]["sourceUrl"] = attempts[0]["sourceUrl"].clone();
                attempts[1]["bodySha256"] = attempts[0]["bodySha256"].clone();
            }
            "wrong_hash" => attempts[1]["bodySha256"] = json!("b".repeat(64)),
            "wrong_final_url" => attempts[1]["sourceUrl"] = json!("https://other.college.test/"),
            _ => attempts[1]["requestedUrl"] = Value::Null,
        }
        assert!(
            finalize(
                pages,
                attempts,
                planned,
                "college.test",
                "college.test",
                100
            )
            .is_err(),
            "{case}"
        );
    }
}
#[test]
fn cleanup_ownership_complete_empty_redirects_bind_requested_urls_separately_from_final_url() {
    let www = blank_page(
        "https://college.test/",
        "<html><p>Welcome</p></html>",
        false,
    );
    let apex = blank_page(
        "https://college.test/",
        "<html><p>Welcome</p></html>",
        false,
    );
    let attempts = vec![
        successful_read("https://www.college.test/", &www),
        successful_read("https://college.test/", &apex),
    ];
    let result = finalize(
        vec![www, apex],
        attempts,
        homepage_candidates("college.test", "college.test"),
        "college.test",
        "college.test",
        100,
    )
    .unwrap();
    assert_eq!(result["outcome"], "missing_identity");
    assert_eq!(result["boundedPassComplete"], true);
    assert_eq!(
        result["sourceAttempts"][0]["requestedUrl"],
        "https://www.college.test/"
    );
    assert_eq!(
        result["sourceAttempts"][0]["sourceUrl"],
        "https://college.test/"
    );
}
#[test]
fn cleanup_ownership_successful_empty_http_fallback_does_not_erase_an_earlier_tls_failure() {
    let www = blank_page(
        "http://www.college.test/",
        "<html><p>Welcome</p></html>",
        false,
    );
    let apex = blank_page(
        "https://college.test/",
        "<html><p>Welcome</p></html>",
        false,
    );
    let attempts = vec![
        json!({"sourceUrl":"https://www.college.test/","state":"unavailable","error":"TLS handshake failure"}),
        successful_read("https://www.college.test/", &www),
        successful_read("https://college.test/", &apex),
    ];
    assert!(finalize(
        vec![www, apex],
        attempts,
        homepage_candidates("college.test", "college.test"),
        "college.test",
        "college.test",
        100
    )
    .is_err());
}
#[test]
fn cleanup_ownership_existing_positive_fact_path_preserves_partial_source_history() {
    let www = blank_page(
        "https://www.college.test/",
        "<footer>Copyright 2026 Example University</footer>",
        false,
    );
    let attempts = vec![successful_read("https://www.college.test/", &www)];
    let result = finalize(
        vec![www],
        attempts,
        homepage_candidates("college.test", "college.test"),
        "college.test",
        "college.test",
        100,
    )
    .unwrap();
    assert_eq!(result["outcome"], "evidence_collected");
    assert_eq!(result["identityFound"], true);
    assert_eq!(
        result["boundedPassComplete"], false,
        "a positive declaration does not pretend every alternate was fetched"
    );
    assert_eq!(result["ownershipConclusion"], false);
    assert!(result["facts"]
        .as_array()
        .unwrap()
        .iter()
        .any(|f| f["source"] == "official_site_operator"));
}
