// Production entropy remains unchanged. Test fixtures select an id BEFORE INSERT.
fn fresh_web_draft_id() -> String {
    #[cfg(test)]
    if let Some(id) = WEB_CREATOR_TEST_ID.with(|slot| slot.borrow().clone()) {
        return id;
    }
    Uuid::new_v4().to_string()
}
#[cfg(test)]
thread_local! {static WEB_CREATOR_TEST_ID:std::cell::RefCell<Option<String>>=const {std::cell::RefCell::new(None)};}
#[cfg(test)]
fn with_web_creator_test_id<T>(id: &str, f: impl FnOnce() -> T) -> T {
    struct Reset(Option<String>);
    impl Drop for Reset {
        fn drop(&mut self) {
            WEB_CREATOR_TEST_ID.with(|slot| *slot.borrow_mut() = self.0.take());
        }
    }
    let _reset = Reset(WEB_CREATOR_TEST_ID.with(|slot| slot.replace(Some(id.into()))));
    f()
}
