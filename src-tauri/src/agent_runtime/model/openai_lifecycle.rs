//! Safe constants only. No model bytes, credentials or request objects reach observers.
use super::*;
impl OpenAiCompatibleGateway {
    pub(crate) fn complete_once_observed_with_lifecycle(
        &self,
        request: &ModelRequest,
        cancel: &CancelToken,
        observer: StageObserver,
    ) -> Result<ModelResponse, OneShotFailure> {
        self.complete_once_lifecycle(request, cancel, Some(observer))
    }
    pub(super) fn complete_once_lifecycle(
        &self,
        request: &ModelRequest,
        cancel: &CancelToken,
        observer: Option<StageObserver>,
    ) -> Result<ModelResponse, OneShotFailure> {
        if cancel.is_cancelled() {
            return Err(OneShotFailure::BeforeTransport(ModelError::Cancelled));
        }
        let body = self.request_body(request);
        let _permit = admission::gate(self.profile.local)
            .acquire(cancel)
            .map_err(OneShotFailure::BeforeTransport)?;
        if cancel.is_cancelled() {
            return Err(OneShotFailure::BeforeTransport(ModelError::Cancelled));
        }
        self.round(&body, request, cancel, observer)
            .map_err(OneShotFailure::TransportOutcomeUnknown)
    }
}
