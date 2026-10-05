// Native race size policy only. This supplies no target permission or budget.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct NativeRaceLimits {
    concurrency: usize,
    attempts: usize,
}

fn native_race_limits(contract: &JsonValue) -> Result<NativeRaceLimits, &'static str> {
    let concurrency = match contract.get("concurrency") {
        None => 2,
        Some(value) => value
            .as_u64()
            .filter(|count| (2..=3).contains(count))
            .ok_or("race_concurrency_invalid")? as usize,
    };
    // An explicit bounded contract may have several waves. The exact number
    // still needs the frozen worker/Broker request budget before execution.
    let attempts = match contract.get("attempts") {
        None => concurrency,
        Some(value) => value
            .as_u64()
            .filter(|count| (concurrency as u64..=128).contains(count))
            .ok_or("race_attempts_invalid")? as usize,
    };
    Ok(NativeRaceLimits {
        concurrency,
        attempts,
    })
}
