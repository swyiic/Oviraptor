// Bounded stderr diagnostics with the same actual Native journal/commit protocol.
struct NativeHelperObserver {
    journal: crate::native_pipeline::process::log::Journal,
    channel: crate::native_pipeline::process::output::Channel,
    error: Option<String>,
    finished: bool,
}
impl NativeHelperObserver {
    fn begin(
        owner: &NativeHelperLogBinding,
        helper: &Path,
        input: &[u8],
        environment: &JsonValue,
    ) -> Result<Self, String> {
        let scope = owner.scope(helper, input, environment)?;
        let journal = crate::native_pipeline::process::log::Journal::begin(&owner.path, scope)?;
        Ok(Self {
            journal,
            channel: crate::native_pipeline::process::output::Channel::new(),
            error: None,
            finished: false,
        })
    }
    fn deliver(&mut self) {
        let journal = &mut self.journal;
        self.channel.deliver(
            &mut |event| {
                journal
                    .append(event, &mut notify_native_process_log)
                    .map(|_| ())
            },
            &mut self.error,
            false,
        );
    }
    fn drain(&mut self) {
        let journal = &mut self.journal;
        self.channel.finish(
            &mut |event| {
                journal
                    .append(event, &mut notify_native_process_log)
                    .map(|_| ())
            },
            &mut self.error,
        );
    }
    fn finish(&mut self, state: &str, incomplete: bool) -> Result<(), String> {
        if incomplete {
            self.error
                .get_or_insert_with(|| "native_helper_log_capture_incomplete".into());
        }
        let state = if self.error.is_some() { "gap" } else { state };
        let result = self.journal.finish(state, &mut notify_native_process_log);
        self.finished = true;
        result?;
        if let Some(error) = &self.error {
            return Err(error.clone());
        }
        Ok(())
    }
}
impl Drop for NativeHelperObserver {
    fn drop(&mut self) {
        if !self.finished {
            let _ = self
                .journal
                .finish("failed", &mut notify_native_process_log);
        }
    }
}
