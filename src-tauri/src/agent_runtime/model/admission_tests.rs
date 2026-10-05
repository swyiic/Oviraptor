use super::{completion, gateway, raw, MockProvider};
use crate::agent_runtime::model::{
    admission::{AdmissionGate, CLOUD_MAX_IN_FLIGHT},
    CancelToken, ModelError, ModelRequest, OpenAiCompatibleGateway,
};
use std::{
    sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc, Arc,
    },
    thread,
    time::{Duration, Instant},
};

fn wait_for_queue(gate: &AdmissionGate, count: usize) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while gate.queued() < count && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(
        gate.queued(),
        count,
        "request did not enter admission queue"
    );
}

#[test]
fn local_admission_is_fifo_and_cancelled_waiters_do_not_acquire() {
    let gate = Arc::new(AdmissionGate::new(1));
    let first = gate.acquire(&CancelToken::new()).unwrap();
    let (sent, received) = mpsc::channel();
    let cancel = CancelToken::new();
    for number in 0..3 {
        let queued_gate = gate.clone();
        let sender = sent.clone();
        let token = if number == 1 {
            cancel.clone()
        } else {
            CancelToken::new()
        };
        thread::spawn(move || {
            let acquired = queued_gate.acquire(&token);
            sender.send((number, acquired.is_ok())).unwrap();
        });
        wait_for_queue(&gate, number + 1);
    }
    cancel.cancel();
    assert_eq!(
        received.recv_timeout(Duration::from_secs(2)).unwrap(),
        (1, false)
    );
    assert_eq!(gate.queued(), 2);
    drop(first);
    assert_eq!(
        received.recv_timeout(Duration::from_secs(2)).unwrap(),
        (0, true)
    );
    assert_eq!(
        received.recv_timeout(Duration::from_secs(2)).unwrap(),
        (2, true)
    );
}

#[test]
fn cloud_admission_caps_independent_gateway_instances_globally() {
    let active = Arc::new(AtomicUsize::new(0));
    let peak = Arc::new(AtomicUsize::new(0));
    let active_handler = active.clone();
    let peak_handler = peak.clone();
    let provider = MockProvider::spawn(move |_, _| {
        let now = active_handler.fetch_add(1, Ordering::SeqCst) + 1;
        peak_handler.fetch_max(now, Ordering::SeqCst);
        thread::sleep(Duration::from_millis(120));
        active_handler.fetch_sub(1, Ordering::SeqCst);
        raw(completion("ok", &[], 10))
    });
    let clients: Vec<OpenAiCompatibleGateway> =
        (0..8).map(|_| gateway(&provider.base_url, false)).collect();
    let handles: Vec<_> = clients
        .into_iter()
        .map(|client| {
            thread::spawn(move || {
                client.complete_once(&ModelRequest::default(), &CancelToken::new())
            })
        })
        .collect();
    for handle in handles {
        handle.join().unwrap().unwrap();
    }
    assert_eq!(provider.hits(), 8);
    assert!(peak.load(Ordering::SeqCst) <= CLOUD_MAX_IN_FLIGHT);
}

#[test]
fn waiting_cloud_request_can_cancel_without_taking_a_slot() {
    let gate = Arc::new(AdmissionGate::new(CLOUD_MAX_IN_FLIGHT));
    let occupied: Vec<_> = (0..CLOUD_MAX_IN_FLIGHT)
        .map(|_| gate.acquire(&CancelToken::new()).unwrap())
        .collect();
    let token = CancelToken::new();
    let waiting = token.clone();
    let queued_gate = gate.clone();
    let handle = thread::spawn(move || queued_gate.acquire(&waiting).map(|_| ()));
    wait_for_queue(&gate, 1);
    token.cancel();
    assert_eq!(handle.join().unwrap(), Err(ModelError::Cancelled));
    assert_eq!(gate.queued(), 0);
    drop(occupied);
}
