//! The compaction tests (threshold/requested/manual compaction, the
//! durable outcome rows, the /compact command).
use super::*;

/// The faux model's per-request output budget (maxTokens `16_384` under the
/// `32_000` request cap): threshold fixtures subtract it from the window
/// alongside the headroom (the combined input+output ceiling).
const FAUX_REQUEST_BUDGET: u64 = 16_384;

/// The automatic threshold compaction at the turn boundary. The faux
/// provider estimates usage from the serialized context, so the headroom
/// sits halfway between a baseline turn and the baseline plus the big prompt.
#[test]
fn threshold_crossing_auto_compacts_with_the_event_pair() {
    let _faux = FAUX_TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let (probe, _probe_dir) = faux_engine_with_settings(
        &serde_json::json!({ "responses": [{"text": "seed reply"}] }),
        1,
    );
    let mut probe_events: Vec<EngineEvent> = Vec::new();
    admit(&probe, "seed turn".to_string(), &mut probe_events);
    let baseline = probe_events
        .iter()
        .find_map(|event| match event {
            EngineEvent::AssistantMessage(message) => message["usage"]["totalTokens"].as_u64(),
            _ => None,
        })
        .expect("probe turn produced usage");
    assert!(
        baseline < 100_000,
        "the probe baseline is implausibly large: {baseline}"
    );
    drop(probe);

    // ~12k tokens of deterministic extra context on the crossing turn.
    let big_prompt = format!("seed turn {} crossing", "x".repeat(48_000));
    let big_tokens = (48_000 + "seed turn  crossing".len() as u64).div_ceil(4);
    // The headroom sits between the two turns' usage (the f14 battery
    // shape: reserveTokens so exactly the seeded crossing fires).
    let headroom = baseline + big_tokens / 2;
    let (engine, _engine_dir) = faux_engine_with_settings(
        &serde_json::json!({
            "responses": [
                {"text": "seed reply"},
                {"text": "crossing reply"},
                {"text": "the summary"},
            ],
        }),
        128_000u64
            .saturating_sub(FAUX_REQUEST_BUDGET + headroom)
            .max(1),
    );

    let mut events: Vec<EngineEvent> = Vec::new();
    admit(&engine, "seed turn".to_string(), &mut events);
    assert_eq!(
        assistant_texts(&events),
        vec!["seed reply".to_string()],
        "the seed turn answered"
    );
    assert!(
        !events.iter().any(|event| matches!(
            event,
            EngineEvent::CompactionStart { .. } | EngineEvent::Compaction { .. }
        )),
        "no compaction below the headroom"
    );
    admit(&engine, big_prompt, &mut events);
    let assistant_index = events
        .iter()
        .rposition(|event| matches!(event, EngineEvent::AssistantMessage(_)))
        .expect("assistant message emitted");
    let start_index = events
        .iter()
        .position(|event| {
            matches!(event, EngineEvent::CompactionStart { event } if event["reason"] == "threshold")
        })
        .expect("threshold compaction_start emitted");
    assert!(
        start_index > assistant_index,
        "the check fires at the settled turn boundary"
    );
    let EngineEvent::CompactionStart { event } = &events[start_index] else {
        unreachable!();
    };
    assert_eq!(
        event,
        &serde_json::json!({ "type": "compaction_start", "reason": "threshold" })
    );
    let compaction_index = events
        .iter()
        .position(|event| matches!(event, EngineEvent::Compaction { .. }))
        .expect("compaction_end emitted");
    let EngineEvent::Compaction { entry, event } = &events[compaction_index] else {
        unreachable!();
    };
    assert!(compaction_index > start_index);
    assert_eq!(event["reason"], "threshold");
    assert_eq!(event["result"]["summary"], "the summary");
    assert_eq!(
        event["result"]["details"],
        serde_json::json!({ "readFiles": [], "modifiedFiles": [] })
    );
    assert!(entry["firstKeptEntryId"].is_string());
    // Exactly one pair: the pre-turn check sees no built session, the
    // post-turn check fires once.
    let start_count = events
        .iter()
        .filter(|event| matches!(event, EngineEvent::CompactionStart { .. }))
        .count();
    let end_count = events
        .iter()
        .filter(|event| matches!(event, EngineEvent::Compaction { .. }))
        .count();
    assert_eq!((start_count, end_count), (1, 1));
}

/// The compaction summarizer stays on the session's provider when a
/// fresh resolution drifts mid-session (R8). The settings default changes
/// under the built session, so `resolve_model` lands on a dead provider —
/// but the threshold arm follows `session_model`.
#[test]
fn threshold_compaction_stays_on_the_session_provider_after_a_resolution_drift() {
    let _faux = FAUX_TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let dir = tempfile::TempDir::new().unwrap();
    let agent_dir = dir.path().join("agent");
    std::fs::create_dir_all(&agent_dir).unwrap();
    let script = json!({
        "responses": [
            {"text": "seed reply"},
            {"text": "crossing reply"},
            {"text": "the drifted summary"},
        ],
    });
    let parsed = pa_ai::faux::script::parse_faux_script(&script).expect("faux script parses");
    let registration = pa_ai::faux::script::register_faux_provider_from_script(&parsed);
    // The registry catalog: the faux model, and the drift model — an
    // openai-completions endpoint nothing serves, so a request fails.
    std::fs::write(
        agent_dir.join("models.json"),
        json!({
            "providers": {
                "faux": {
                    "api": "faux",
                    "baseUrl": "http://localhost:0",
                    "apiKey": "sk-faux",
                    "models": [{
                        "id": "faux-1",
                        "name": "Faux Model",
                        "contextWindow": 128_000,
                        "maxTokens": 16384,
                    }],
                },
                "drift": {
                    "api": "openai-completions",
                    "baseUrl": "http://127.0.0.1:9",
                    "apiKey": "sk-drift",
                    "models": [{
                        "id": "drift-1",
                        "name": "Drift Model",
                        "contextWindow": 128_000,
                        "maxTokens": 16384,
                    }],
                },
            }
        })
        .to_string(),
    )
    .unwrap();
    let write_settings = |default_provider: &str, default_model: &str, reserve_tokens: u64| {
        std::fs::write(
            agent_dir.join("settings.json"),
            json!({
                "defaultProvider": default_provider,
                "defaultModel": default_model,
                "compaction": {
                    "enabled": true,
                    "reserveTokens": reserve_tokens,
                    "keepRecentTokens": 10,
                },
            })
            .to_string(),
        )
        .unwrap();
    };
    let new_engine = || {
        AgentSessionEngine::new(AgentEngineConfig {
            cwd: dir.path().to_path_buf(),
            agent_dir: agent_dir.clone(),
            provider: None,
            model: None,
            api_key: None,
            thinking: None,
            session_dir: None,
            session_file: None,
            faux_script: None,
            supervisor_link: None,
            cron_store: None,
            queued_steering_probe: None,
        })
        .unwrap()
    };
    write_settings("faux", "faux-1", 1);
    let probe = new_engine();
    let mut probe_events: Vec<EngineEvent> = Vec::new();
    admit(&probe, "seed turn".to_string(), &mut probe_events);
    let baseline = probe_events
        .iter()
        .find_map(|event| match event {
            EngineEvent::AssistantMessage(message) => message["usage"]["totalTokens"].as_u64(),
            _ => None,
        })
        .expect("probe turn produced usage");
    assert!(baseline < 100_000, "implausible baseline: {baseline}");
    drop(probe);

    // The combined input+output ceiling sits between the two turns'
    // usage (the 16_384 output budget is part of it).
    let big_prompt = format!("seed turn {} crossing", "x".repeat(48_000));
    let big_tokens = (48_000 + "seed turn  crossing".len() as u64).div_ceil(4);
    let headroom = baseline + big_tokens / 2;
    let reserve = 128_000u64
        .saturating_sub(FAUX_REQUEST_BUDGET + headroom)
        .max(1);
    write_settings("faux", "faux-1", reserve);
    registration.set_responses(parsed.responses);
    let engine = new_engine();
    let mut events: Vec<EngineEvent> = Vec::new();
    admit(&engine, "seed turn".to_string(), &mut events);
    assert_eq!(assistant_texts(&events), vec!["seed reply".to_string()]);
    assert!(
        !events.iter().any(|event| matches!(
            event,
            EngineEvent::CompactionStart { .. } | EngineEvent::Compaction { .. }
        )),
        "no compaction below the threshold"
    );

    // The mid-session resolution drift: the settings default changes
    // under the built session; the live model stays.
    write_settings("drift", "drift-1", reserve);
    let drifted = engine.resolve_model().expect("the drift model resolves");
    assert_eq!(
        (drifted.provider.as_str(), drifted.id.as_str()),
        ("drift", "drift-1")
    );
    let session = engine.session_model().expect("the session model resolves");
    assert_eq!(
        (session.provider.as_str(), session.id.as_str()),
        ("faux", "faux-1")
    );

    // The threshold arm compacts on the session's provider: the
    // summarizer runs through the faux provider, never the drift model.
    let calls_before_crossing = registration.call_count();
    let mut crossing_events: Vec<EngineEvent> = Vec::new();
    admit(&engine, big_prompt, &mut crossing_events);
    let starts = crossing_events
        .iter()
        .filter(
            |event| matches!(event, EngineEvent::CompactionStart { event } if event["reason"] == "threshold"),
        )
        .count();
    let ends = crossing_events
        .iter()
        .filter(|event| matches!(event, EngineEvent::Compaction { .. }))
        .count();
    assert_eq!((starts, ends), (1, 1));
    let summary = crossing_events
        .iter()
        .find_map(|event| match event {
            EngineEvent::Compaction { event, .. } => {
                event["result"]["summary"].as_str().map(str::to_string)
            }
            _ => None,
        })
        .expect("the compaction end carries the summarizer's text");
    assert_eq!(summary, "the drifted summary");
    assert_eq!(
        registration.call_count(),
        calls_before_crossing + 2,
        "the crossing turn and the summarizer ran on the session provider"
    );
    assert_eq!(
        assistant_texts(&crossing_events),
        vec!["crossing reply".to_string()]
    );
    // The summarizer followed the live target's key too (the R8
    // seam's key arm): every request carried the faux key.
    let keys = registration.received_api_keys();
    assert_eq!(keys.len() as u64, registration.call_count());
    assert!(
        keys.iter().all(|key| key.as_deref() == Some("sk-faux")),
        "every call followed the live target's key"
    );
    assert!(matches!(
        crossing_events.last(),
        Some(EngineEvent::Done(Ok(())))
    ));
}

#[test]
fn retire_clears_the_provider_target_for_the_replacement_build() {
    let _faux = FAUX_TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let dir = tempfile::TempDir::new().unwrap();
    let agent_dir = dir.path().join("agent");
    std::fs::create_dir_all(&agent_dir).unwrap();
    let script = json!({ "responses": [{"text": "seed reply"}] });
    let parsed = pa_ai::faux::script::parse_faux_script(&script).expect("faux script parses");
    let _registration = pa_ai::faux::script::register_faux_provider_from_script(&parsed);
    std::fs::write(
        agent_dir.join("models.json"),
        json!({
            "providers": {
                "faux": {
                    "api": "faux", "baseUrl": "http://localhost:0", "apiKey": "sk-faux",
                    "models": [{
                        "id": "faux-1", "name": "Faux Model",
                        "contextWindow": 128_000, "maxTokens": 16384,
                    }],
                },
                "drift": {
                    "api": "faux", "baseUrl": "http://localhost:0", "apiKey": "sk-drift",
                    "models": [{
                        "id": "drift-1", "name": "Drift Model",
                        "contextWindow": 128_000, "maxTokens": 16384,
                    }],
                },
            }
        })
        .to_string(),
    )
    .unwrap();
    let write_settings = |default_provider: &str, default_model: &str| {
        std::fs::write(
            agent_dir.join("settings.json"),
            json!({
                "defaultProvider": default_provider,
                "defaultModel": default_model,
            })
            .to_string(),
        )
        .unwrap();
    };
    let new_engine = || {
        AgentSessionEngine::new(AgentEngineConfig {
            cwd: dir.path().to_path_buf(),
            agent_dir: agent_dir.clone(),
            provider: None,
            model: None,
            api_key: None,
            thinking: None,
            session_dir: None,
            session_file: None,
            faux_script: None,
            supervisor_link: None,
            cron_store: None,
            queued_steering_probe: None,
        })
        .unwrap()
    };
    write_settings("faux", "faux-1");
    let engine = new_engine();
    let mut events: Vec<EngineEvent> = Vec::new();
    admit(&engine, "seed turn".to_string(), &mut events);
    let model = engine.session_model().expect("the session model resolves");
    assert_eq!(
        (model.provider.as_str(), model.id.as_str()),
        ("faux", "faux-1")
    );

    write_settings("drift", "drift-1");
    engine
        .runtime
        .block_on(async { engine.retire_session_runtime().await });
    assert!(engine
        .runtime
        .block_on(async { engine.session.lock().await.is_none() }));

    let model = engine
        .session_model()
        .expect("the replacement model resolves");
    assert_eq!(
        (model.provider.as_str(), model.id.as_str()),
        ("drift", "drift-1"),
        "the retired session's provider target must not outlive it"
    );
}

fn threshold_below_the_headroom_stays_silent() {
    let _faux = FAUX_TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let dir = tempfile::TempDir::new().unwrap();
    let engine = AgentSessionEngine::new(AgentEngineConfig {
        cwd: dir.path().to_path_buf(),
        agent_dir: dir.path().join("agent"),
        provider: None,
        model: None,
        api_key: None,
        thinking: None,
        session_dir: None,
        session_file: None,
        faux_script: Some(
            serde_json::json!({ "responses": [{"text": "plain reply"}] }).to_string(),
        ),
        supervisor_link: None,
        cron_store: None,
        queued_steering_probe: None,
    })
    .unwrap();
    let mut events: Vec<EngineEvent> = Vec::new();
    engine.run_prompt(
        0,
        PromptRequest {
            batch: Vec::new(),
            images: Vec::new(),
            message: "a small turn".to_string(),
            source: "user".to_string(),
            agent_message_id: None,
            custom_message: None,
        },
        &|| false,
        &mut |event| {
            events.push(event);
            true
        },
    );
    assert_eq!(assistant_texts(&events), vec!["plain reply".to_string()]);
    assert!(
        !events.iter().any(|event| matches!(
            event,
            EngineEvent::CompactionStart { .. } | EngineEvent::Compaction { .. }
        )),
        "no compaction events below the headroom"
    );
}

/// The wire events one `/compact` produced, in order.
#[cfg(test)]
fn compaction_events(events: &[EngineEvent]) -> Vec<serde_json::Value> {
    events
        .iter()
        .filter_map(|event| match event {
            EngineEvent::CompactionStart { event } | EngineEvent::Compaction { event, .. } => {
                Some(event.clone())
            }
            _ => None,
        })
        .collect()
}

#[test]
fn compact_session_command_emits_the_ts_event_pair_on_a_skip() {
    let (_engine, events) = run_prompts(
        &serde_json::json!({ "responses": ["unused"] }),
        &["/compact"],
    );
    // The echo row precedes the events (TS `_executeSelectedSessionCommand`
    // records it before the queue runs the command); a skip records no
    // result row.
    let rows = custom_rows(&events);
    assert_eq!(rows.len(), 1, "echo only, no result row: {rows:?}");
    assert_eq!(rows[0]["customType"], "session_slash_command");
    assert_eq!(rows[0]["content"], "/compact");
    let compaction = compaction_events(&events);
    assert_eq!(compaction.len(), 2, "start + end: {compaction:?}");
    assert_eq!(
        compaction[0],
        serde_json::json!({ "type": "compaction_start", "reason": "manual" })
    );
    assert_eq!(
        compaction[1],
        serde_json::json!({
            "type": "compaction_end",
            "reason": "manual",
            "aborted": false,
            "willRetry": false,
            "errorMessage": "Session is too short to compact \u{2014} try again once it grows",
            "errorSeverity": "warning",
        })
    );
    assert_eq!(events.last(), Some(&EngineEvent::Done(Ok(()))));
}

#[test]
fn compact_session_command_emits_the_result_on_success() {
    // Two big turns (~12k tokens each) push the history past the
    // keep-recent budget. The crossing rides the second turn's USER
    // message — a cut inside a turn would make TWO summarizer wire
    // calls (TS parity), which this single-summary script does not serve.
    let filler = "history ".repeat(6_000); // ~48k chars = ~12k tokens each
    let big_second = format!("second {}", "padded ".repeat(6_000)); // ~10.5k tokens
    let (_engine, events) = run_prompts(
        &serde_json::json!({
            "responses": [
                { "text": filler },
                { "text": filler },
                { "text": "## Summary\nthe session story" },
            ]
        }),
        &["first", &big_second, "/compact focus on the goal"],
    );
    let compaction = compaction_events(&events);
    assert_eq!(compaction.len(), 2, "{compaction:?}");
    assert_eq!(
        compaction[0],
        serde_json::json!({
            "type": "compaction_start",
            "reason": "manual",
            "customInstructions": "focus on the goal",
        })
    );
    let end = &compaction[1];
    assert_eq!(end["type"], "compaction_end");
    assert_eq!(end["reason"], "manual");
    assert_eq!(end["aborted"], false);
    assert_eq!(end["customInstructions"], "focus on the goal");
    let result = end["result"].as_object().expect("the result payload");
    assert_eq!(result["summary"], "## Summary\nthe session story");
    assert!(result["tokensBefore"].as_u64().unwrap_or_default() > 0);
    // The TS dataKeys on the wire result (the live golden,
    // `tests/goldens/compaction-live-ts.json`): summary, firstKeptEntryId,
    // tokensBefore, details — the file-op lists verbatim from the durable
    // entry, and the summarizer usage never rides the wire.
    let mut result_keys: Vec<&str> = result.keys().map(String::as_str).collect();
    result_keys.sort_unstable();
    assert_eq!(
        result_keys,
        ["details", "firstKeptEntryId", "summary", "tokensBefore"],
        "CompactionResult key set"
    );
    assert_eq!(
        result["details"],
        serde_json::json!({ "readFiles": [], "modifiedFiles": [] })
    );
    assert!(result.get("usage").is_none());
    // The durable rows stay minimal (TS's queued `/compact` catch arm
    // records no result row): the echo row is the only custom row —
    // except the kernel-dependent `ipython_state` notice, scoped out of
    // this assertion.
    let rows: Vec<_> = custom_rows(&events)
        .into_iter()
        .filter(|row| row["customType"] != "ipython_state")
        .collect();
    assert_eq!(rows.len(), 1, "the /compact echo only: {rows:?}");
    assert_eq!(rows[0]["customType"], "session_slash_command");
}

#[test]
fn threshold_crossing_inside_one_run_compacts_before_the_next_model_turn() {
    let _faux = FAUX_TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let (engine, _dir) = faux_engine_with_settings(
        &json!({
            "responses": [
                {"text":"seed reply"},
                {"content":[{"type":"text","text":"x".repeat(80_000)}, {"type":"toolCall","id":"synthetic-call","name":"synthetic_missing_tool","arguments":{}}]},
                {"text":"history summary"},
                {"text":"prefix summary"},
                {"text":"task finished"}
            ]
        }),
        128_000 - FAUX_REQUEST_BUDGET - 20_000,
    );
    let mut events = Vec::new();
    admit(&engine, "seed".into(), &mut events);
    events.clear();
    admit(&engine, "continue task".into(), &mut events);
    let compaction = events
        .iter()
        .position(|event| matches!(event, EngineEvent::CompactionStart { .. }))
        .expect("threshold crossed inside run");
    let assistants: Vec<_> = events
        .iter()
        .enumerate()
        .filter_map(|(index, event)| {
            matches!(event, EngineEvent::AssistantMessage(_)).then_some(index)
        })
        .collect();
    assert_eq!(assistants.len(), 2, "one tool turn and one final turn");
    assert!(
        assistants[0] < compaction && compaction < assistants[1],
        "compaction must precede the next ordinary model call"
    );
    assert_eq!(
        assistant_texts(&events).last().map(String::as_str),
        Some("task finished")
    );
}

#[test]
fn failed_mid_run_compaction_resumes_without_retrying_each_tool_round() {
    let _faux = FAUX_TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let tool_turn = json!({"content":[{"type":"text","text":"x".repeat(80_000)}, {"type":"toolCall","id":"synthetic-call","name":"synthetic_missing_tool","arguments":{}}]});
    let (engine, _dir) = faux_engine_with_settings(
        &json!({
            "responses": [
                {"text":"seed reply"}, tool_turn,
                {"text":"", "stopReason":"error", "errorMessage":"synthetic summary failure"},
                {"text":"prefix summary"}, tool_turn,
                {"text":"task finished"}
            ]
        }),
        128_000 - FAUX_REQUEST_BUDGET - 20_000,
    );
    let mut events = Vec::new();
    admit(&engine, "seed".into(), &mut events);
    events.clear();
    admit(&engine, "continue task".into(), &mut events);
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, EngineEvent::CompactionStart { .. }))
            .count(),
        1
    );
    assert_eq!(
        assistant_texts(&events).last().map(String::as_str),
        Some("task finished")
    );
    assert!(events
        .iter()
        .any(|event| matches!(event, EngineEvent::Done(Ok(())))));
    let core = engine.session.blocking_lock().clone().unwrap();
    let entries = engine
        .runtime
        .block_on(async { core.session.shared_persistence().lock().await.get_entries() });
    let user_prompts: Vec<_> = entries
        .iter()
        .filter_map(|entry| match entry {
            pa_types::session::FileEntry::Message {
                message: pa_types::session::AgentMessage::User(user),
                ..
            } => Some(user.content.text()),
            _ => None,
        })
        .collect();
    assert_eq!(user_prompts, ["seed", "continue task"]);
}

/// A not-due recheck after a threshold yield keeps the in-run threshold
/// machinery armed: the suppression is reserved for a summary attempt
/// that failed to commit (cursor: "Recheck mismatch disables further
/// compaction").
#[test]
fn a_not_due_recheck_keeps_threshold_checks_armed() {
    use crate::auto_compaction::AutoCompactionRun;
    assert!(
        AutoCompactionRun::NotDue.continues_threshold_checks(),
        "a not-due recheck is not a failed summary"
    );
    assert!(
        AutoCompactionRun::Compacted.continues_threshold_checks(),
        "a committed summary keeps checking"
    );
    assert!(
        !AutoCompactionRun::Ran.continues_threshold_checks(),
        "a failed or skipped summary suppresses the rest of the run"
    );
}

#[test]
fn low_custom_percentage_compacts_real_history_once_and_then_makes_progress() {
    let _faux = FAUX_TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let (engine, dir) = faux_engine_with_settings(
        &serde_json::json!({
            "responses": vec![serde_json::json!({"text": "short response"}); 80],
        }),
        16_384,
    );
    // The faux provider's conversation-scoped prompt cache reports a rewritten
    // prompt as both input and cache-write tokens, so the first turn after a
    // compaction reads about twice the retained history. Keep that doubled
    // reading below the 42_769 retention floor (10_000 + 2 * 16_384 + 1).
    std::fs::write(
        dir.path().join("agent/settings.json"),
        serde_json::json!({"compaction": {
            "enabled": true, "thresholdPercent": 5,
            "reserveTokens": 16_384, "keepRecentTokens": 10_000
        }})
        .to_string(),
    )
    .unwrap();
    let engine = std::sync::Arc::new(engine);
    engine.register_arc();
    let mut events = Vec::new();
    let mut compacted = false;
    for turn in 0..30 {
        admit(
            &engine,
            format!("turn {turn} {}", "x".repeat(12_000)),
            &mut events,
        );
        if let Some(EngineEvent::Compaction { event, entry }) = events
            .iter()
            .find(|event| matches!(event, EngineEvent::Compaction { .. }))
        {
            assert!(event["result"]["summary"]
                .as_str()
                .unwrap()
                .contains("short response"));
            assert!(entry["firstKeptEntryId"].is_string());
            compacted = true;
            break;
        }
    }
    assert!(compacted, "the 5% setting ran an actual compaction");
    let usage = events
        .iter()
        .filter_map(|event| match event {
            EngineEvent::AssistantMessage(message) => message["usage"]["totalTokens"].as_u64(),
            _ => None,
        })
        .next_back()
        .unwrap();
    assert!(
        usage < 95_232,
        "compaction occurred before the unchanged default trigger"
    );
    for turn in 0..4 {
        admit(&engine, format!("short continuation {turn}"), &mut events);
    }
    let starts = events
        .iter()
        .filter(|event| matches!(event, EngineEvent::CompactionStart { .. }))
        .count();
    let ends = events
        .iter()
        .filter(|event| matches!(event, EngineEvent::Compaction { .. }))
        .count();
    assert_eq!(
        (starts, ends),
        (1, 1),
        "short turns do not recompact retained history"
    );
    assert_eq!(
        assistant_texts(&events).last().map(String::as_str),
        Some("short response")
    );

    // The autonomous mirror shares the same live getter rather than a copied refresh.
    let mut settings =
        pa_core::settings::SettingsManager::create(dir.path(), dir.path().join("agent"));
    settings.set_compaction_threshold_percent(75.0).unwrap();
    let mirror = engine.autonomous_boundary.lock().unwrap().clone().unwrap();
    let session = mirror.session.upgrade().unwrap();
    assert_eq!(
        session.session.compaction_settings(),
        pa_core::session_engine::compaction::CompactionSettings {
            threshold_percent: 75.0,
            reserve_tokens: 16_384,
            keep_recent_tokens: 10_000,
            ..Default::default()
        }
    );
}
