// Production entropy remains unchanged. Test fixtures select an id BEFORE INSERT.
fn fresh_web_draft_id() -> String {
    Uuid::new_v4().to_string()
}
