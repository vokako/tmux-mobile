//! Integration tests for the bus: @-mention addressing, the requires_reply discipline,
//! cursor delivery, broadcast wakeup, roster-in-wait, hire/fire, and quiescence.

use agora::bus::{Bus, Quiescence, WaitOutcome};
use agora::store;
use std::time::Duration;

fn new_bus() -> Bus {
    Bus::new(store::open_in_memory().expect("db"), "main")
}

async fn drain(bus: &Bus, agent: &str) {
    loop {
        match bus.wait(agent, Some(Duration::from_millis(50))).await.unwrap() {
            WaitOutcome::Idle { .. } => break,
            _ => continue,
        }
    }
}

#[tokio::test]
async fn wait_delivers_addressed_and_human_messages_with_roster() {
    let bus = new_bus();
    bus.join("alice", None).unwrap();
    bus.join("bob", None).unwrap();
    drain(&bus, "alice").await;
    drain(&bus, "bob").await;

    // An agent message addressed to bob (@bob) is a trigger → delivered.
    bus.post("alice", "@bob hello room", false).unwrap();

    match bus.wait("bob", Some(Duration::from_millis(500))).await.unwrap() {
        WaitOutcome::Delivered { messages, roster, .. } => {
            assert!(messages.iter().any(|m| m.body.contains("hello room")));
            assert!(messages.iter().all(|m| m.from != "bob"));
            assert!(roster.iter().any(|a| a.name == "alice"));
        }
        other => panic!("expected delivery, got {other:?}"),
    }
}

#[tokio::test]
async fn agent_chatter_is_held_then_flushed_on_trigger() {
    // Push throttling: another agent's un-addressed broadcast does NOT wake me;
    // it's held until a trigger (here a human message) flushes the whole batch.
    let bus = new_bus();
    bus.join("lead", None).unwrap();
    bus.join("worker", None).unwrap();
    drain(&bus, "lead").await;
    drain(&bus, "worker").await;

    // worker thinks out loud (broadcast, not @lead) → lead must NOT be woken.
    bus.post("worker", "thinking out loud", false).unwrap();
    let held = bus.wait("lead", Some(Duration::from_millis(200))).await.unwrap();
    assert!(matches!(held, WaitOutcome::Idle { .. }),
        "un-addressed agent chatter must be held, got {held:?}");

    // A human message is always a trigger → flush EVERYTHING new in one batch.
    bus.post("human", "@lead what's the status?", false).unwrap();
    match bus.wait("lead", Some(Duration::from_millis(500))).await.unwrap() {
        WaitOutcome::Delivered { messages, .. } => {
            assert!(messages.iter().any(|m| m.body.contains("thinking out loud")),
                "the held chatter must ride along in the flushed batch");
            assert!(messages.iter().any(|m| m.body.contains("status")),
                "the triggering human message must be delivered");
        }
        other => panic!("expected a flushed batch, got {other:?}"),
    }
}

#[tokio::test]
async fn broadcast_creates_no_obligation() {
    let bus = new_bus();
    bus.join("alice", None).unwrap();
    bus.join("bob", None).unwrap();
    drain(&bus, "bob").await;

    bus.post("alice", "fyi everyone", false).unwrap();
    let _ = bus.wait("bob", Some(Duration::from_millis(200))).await.unwrap();
    let again = bus.wait("bob", Some(Duration::from_millis(200))).await.unwrap();
    assert!(matches!(again, WaitOutcome::Idle { .. }), "broadcast must not block, got {again:?}");
}

#[tokio::test]
async fn mention_without_requires_reply_does_not_block() {
    let bus = new_bus();
    bus.join("lead", None).unwrap();
    bus.join("worker", None).unwrap();
    drain(&bus, "worker").await;

    // @mention but requires_reply=false -> informational; worker is NOT obligated.
    bus.post("lead", "@worker fyi, the spec changed", false).unwrap();
    let _ = bus.wait("worker", Some(Duration::from_millis(200))).await.unwrap();
    let again = bus.wait("worker", Some(Duration::from_millis(200))).await.unwrap();
    assert!(matches!(again, WaitOutcome::Idle { .. }),
        "an @mention without requires_reply must not block, got {again:?}");
}

#[tokio::test]
async fn requires_reply_blocks_until_replied_and_resurfaces_message() {
    let bus = new_bus();
    bus.join("lead", None).unwrap();
    bus.join("worker", None).unwrap();
    drain(&bus, "worker").await;

    // @worker + requires_reply -> worker owes lead.
    bus.post("lead", "@worker build the API", true).unwrap();
    assert!(matches!(bus.wait("worker", Some(Duration::from_millis(300))).await.unwrap(),
        WaitOutcome::Delivered { .. }));

    // Re-wait without replying -> blocked, and the exact message is re-surfaced.
    match bus.wait("worker", Some(Duration::from_millis(300))).await.unwrap() {
        WaitOutcome::Blocked { you_owe, pending } => {
            assert_eq!(you_owe, vec!["lead".to_string()]);
            assert!(pending.iter().any(|m| m.body.contains("build the API")));
        }
        other => panic!("expected Blocked, got {other:?}"),
    }

    // A plain broadcast ack does NOT discharge (no @lead).
    bus.post("worker", "收到，正在处理", false).unwrap();
    assert!(matches!(bus.wait("worker", Some(Duration::from_millis(200))).await.unwrap(),
        WaitOutcome::Blocked { .. }), "broadcast ack must not discharge");

    // Replying with @lead discharges.
    bus.post("worker", "@lead done, see out.md", false).unwrap();
    let after = bus.wait("worker", Some(Duration::from_millis(300))).await.unwrap();
    assert!(matches!(after, WaitOutcome::Idle { .. } | WaitOutcome::Delivered { .. }),
        "after @lead reply, wait should be allowed, got {after:?}");
}

#[tokio::test]
async fn reply_does_not_ping_pong() {
    let bus = new_bus();
    bus.join("lead", None).unwrap();
    bus.join("worker", None).unwrap();

    bus.post("lead", "@worker do X", true).unwrap();   // worker owes lead
    bus.post("worker", "@lead done X", false).unwrap(); // discharges; no new obligation

    let lead = bus.wait("lead", Some(Duration::from_millis(200))).await.unwrap();
    assert!(!matches!(lead, WaitOutcome::Blocked { .. }), "reply must not obligate lead, got {lead:?}");
}

#[tokio::test]
async fn can_discharge_debt_to_unregistered_human() {
    // Regression: the human operator is never a registered agent. When the
    // human's directed message obligates an agent, the agent MUST be able to
    // discharge by replying "@human …" even though "human" isn't in the roster.
    // Before the fix, mentioned_names dropped @human, the debt never cleared,
    // and the agent's wait stayed Blocked forever (the real "@human 在线" spam).
    let bus = new_bus();
    bus.join("worker", None).unwrap();
    drain(&bus, "worker").await;

    // Human (unregistered) directs worker with a required reply.
    bus.post("human", "@worker 你在线吗", true).unwrap();
    assert!(matches!(
        bus.wait("worker", Some(Duration::from_millis(300))).await.unwrap(),
        WaitOutcome::Delivered { .. }
    ));

    // Worker owes "human"; re-wait is blocked until it answers.
    assert!(matches!(
        bus.wait("worker", Some(Duration::from_millis(200))).await.unwrap(),
        WaitOutcome::Blocked { .. }
    ));

    // Replying "@human …" discharges the debt even though human is unregistered.
    bus.post("worker", "@human 在线", false).unwrap();
    let after = bus.wait("worker", Some(Duration::from_millis(300))).await.unwrap();
    assert!(
        matches!(after, WaitOutcome::Idle { .. } | WaitOutcome::Delivered { .. }),
        "debt to unregistered human must clear; got {after:?}"
    );
}

#[tokio::test]
async fn mention_of_unregistered_name_creates_no_obligation() {
    let bus = new_bus();
    bus.join("manager", None).unwrap();
    // @ghost is not a registered agent -> no recipient, no obligation.
    bus.post("manager", "@ghost please report", true).unwrap();
    let m = bus.wait("manager", Some(Duration::from_millis(200))).await.unwrap();
    assert!(!matches!(m, WaitOutcome::Blocked { .. }));
}

#[tokio::test]
async fn quiescence_done_vs_deadlock() {
    let bus = new_bus();
    bus.join("lead", None).unwrap();
    bus.join("worker", None).unwrap();
    drain(&bus, "lead").await;
    drain(&bus, "worker").await;

    assert!(matches!(bus.quiescence().unwrap(), Quiescence::Done));

    // lead requires a reply from worker; worker stays waiting but now owes -> deadlock.
    bus.post("lead", "@worker do Y", true).unwrap();
    drain(&bus, "lead").await;
    assert!(matches!(bus.quiescence().unwrap(), Quiescence::Deadlock { .. }));
}

#[tokio::test]
async fn hire_uniqueness_and_fire() {
    let bus = new_bus();
    bus.join("manager", None).unwrap();

    bus.hire("manager", "search-worker", "web search").unwrap();
    assert!(bus.employees().unwrap().iter().any(|e| e.name == "search-worker" && e.state == "requested"));

    assert!(bus.hire("manager", "search-worker", "x").is_err()); // duplicate employee
    assert!(bus.hire("manager", "manager", "x").is_err());       // name of an online agent

    bus.join("search-worker", None).unwrap();
    assert!(bus.employees().unwrap().iter().any(|e| e.name == "search-worker" && e.state == "active"));

    bus.fire("manager", "search-worker").unwrap();
    assert!(bus.employees().unwrap().iter().any(|e| e.name == "search-worker" && e.state == "disabled"));
    bus.hire("manager", "search-worker", "again").unwrap(); // re-hire after fire
}
