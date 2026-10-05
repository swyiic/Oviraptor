//! Conservative draft-time classification, not an execution permission check.
//! A pause in one clause must not turn another clause's write into read-only work.

const WRITE_ACTIONS: &[&str] = &[
    "上传",
    "删除",
    "修改",
    "写入",
    "退款",
    "下单",
    "创建订单",
    "业务状态",
    "并发验证",
    "影响验证",
    "upload",
    "delete",
    "modify",
    "write",
    "refund",
    "create order",
    "state change",
    "impact validation",
];

// Only an immediately negated action is a prohibition. A pause elsewhere in
// the message cannot erase a later write. Ambiguous wording stays classified
// as a write proposal and still needs the real capability/cleanup gates.
const ACTION_NEGATIONS: &[&str] = &[
    "先不要再",
    "不要再",
    "先不要",
    "不要",
    "请勿",
    "禁止",
    "别再",
    "别",
    "暂停",
    "停止",
    "do not",
    "don't",
    "never",
    "stop",
    "pause",
    "disable",
];

// Explicitly declining cleanup is a request to bypass the write contract,
// not evidence that a cleanup contract exists. The caller checks this before
// considering any write proposal for confirmation.
const CLEANUP_BYPASS_PHRASES: &[&str] = &[
    "无需清理",
    "不要清理",
    "不做清理",
    "不用清理",
    "不清理",
    "无需回滚",
    "不要回滚",
    "不做回滚",
    "不用回滚",
    "不回滚",
    "无需补偿",
    "不要补偿",
    "不做补偿",
    "不用补偿",
    "不补偿",
    "skip cleanup",
    "skip rollback",
    "skip compensation",
    "no cleanup",
    "no rollback",
    "no compensation",
    "without cleanup",
    "without rollback",
    "without compensation",
    "do not clean up",
    "don't clean up",
    "do not rollback",
    "don't rollback",
];

pub(super) fn requests_cleanup_bypass(text_lowercase: &str) -> bool {
    CLEANUP_BYPASS_PHRASES
        .iter()
        .any(|phrase| text_lowercase.contains(phrase))
}

pub(super) fn contains_active_write(text_lowercase: &str) -> bool {
    WRITE_ACTIONS.iter().any(|action| {
        text_lowercase.match_indices(action).any(|(start, _)| {
            let prefix = text_lowercase[..start].trim_end();
            !ACTION_NEGATIONS
                .iter()
                .any(|negation| prefix.ends_with(negation))
        })
    })
}
