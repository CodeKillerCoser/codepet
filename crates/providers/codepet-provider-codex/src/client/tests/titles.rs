use super::*;

#[test]
fn title_generation_skips_existing_names() {
    let (session, requests, replies) = mock_session();
    let worker = session.clone();
    let task = thread::spawn(move || worker.generate_thread_title("target", "hello", None));
    let read = requests.recv_timeout(Duration::from_secs(1)).unwrap();
    assert_eq!(read["method"], "thread/read");
    assert_eq!(read["params"]["includeTurns"], false);
    let mut target = thread_fixture("target", "/tmp", "idle", vec![]);
    target["name"] = json!("User title");
    replies
        .send(json!({"id": read["id"], "result": {"thread": target}}))
        .unwrap();
    assert!(!task.join().unwrap().unwrap());
    assert!(requests.try_recv().is_err());
    session.shutdown().unwrap();
}

#[test]
fn title_generation_is_ephemeral_bounded_and_preserves_concurrent_names() {
    for scenario in ["success", "renamed", "invalid", "timeout"] {
        let (session, requests, replies) = mock_session();
        let worker = session.clone();
        let task = thread::spawn(move || {
            worker.generate_thread_title_with_timeout(
                "target",
                "fallback",
                Some("selected-model"),
                Duration::from_millis(100),
            )
        });
        let read = requests.recv_timeout(Duration::from_secs(1)).unwrap();
        let mut target = thread_fixture("target", "/tmp", "idle", vec![]);
        target["preview"] = json!("首条用户问题".repeat(2000));
        replies
            .send(json!({"id": read["id"], "result": {"thread": target}}))
            .unwrap();
        let start = requests.recv_timeout(Duration::from_secs(1)).unwrap();
        assert_eq!(start["method"], "thread/start");
        assert_eq!(start["params"]["ephemeral"], true);
        assert_eq!(start["params"]["model"], "selected-model");
        assert_eq!(start["params"]["sandbox"], "read-only");
        assert_eq!(start["params"]["approvalPolicy"], "never");
        assert_eq!(start["params"]["config"]["features.hooks"], false);
        let mut temporary = thread_fixture("title-job", "/tmp", "idle", vec![]);
        temporary["ephemeral"] = json!(true);
        // Native notifications may precede their RPC response.
        replies
            .send(json!({"method": "thread/started", "params": {"thread": temporary}}))
            .unwrap();
        replies.send(json!({"id": start["id"], "result": {"thread": temporary,
                "model": "selected-model", "modelProvider": "openai", "cwd": "/tmp",
                "approvalPolicy": "never", "approvalsReviewer": "user", "sandbox": {"type": "readOnly"}}})).unwrap();
        let turn = requests.recv_timeout(Duration::from_secs(1)).unwrap();
        assert_eq!(turn["method"], "turn/start");
        assert_eq!(
            turn["params"]["input"][0]["text"]
                .as_str()
                .unwrap()
                .chars()
                .count(),
            6000
        );
        assert!(turn["params"]["outputSchema"].is_object());
        assert!(session.is_ephemeral_thread("title-job"));
        if scenario != "timeout" {
            replies.send(json!({"method": "item/completed", "params": {
                    "threadId": "title-job", "turnId": "title-turn", "item": {
                        "id": "title-output", "type": "agentMessage",
                        "text": if scenario == "invalid" {"not JSON"} else {"{\"title\":\"生成的简短标题\"}"}
                    }
                }})).unwrap();
            replies
                .send(json!({"method": "turn/completed", "params": {
                    "threadId": "title-job", "turn": turn_fixture("title-turn", "completed")
                }}))
                .unwrap();
        }
        replies.send(json!({"id": turn["id"], "result": {"turn": turn_fixture("title-turn", "inProgress")}})).unwrap();
        if scenario == "timeout" || scenario == "invalid" {
            let cancel = requests.recv_timeout(Duration::from_secs(1)).unwrap();
            assert_eq!(cancel["method"], "turn/interrupt");
            replies
                .send(json!({"id": cancel["id"], "result": {}}))
                .unwrap();
        }
        let cleanup = requests.recv_timeout(Duration::from_secs(1)).unwrap();
        assert_eq!(cleanup["method"], "thread/unsubscribe");
        replies
            .send(json!({"id": cleanup["id"], "result": {}}))
            .unwrap();
        if scenario == "success" || scenario == "renamed" {
            let read = requests.recv_timeout(Duration::from_secs(1)).unwrap();
            assert_eq!(read["method"], "thread/read");
            if scenario == "renamed" {
                target["name"] = json!("User renamed meanwhile");
            }
            replies
                .send(json!({"id": read["id"], "result": {"thread": target}}))
                .unwrap();
            if scenario == "success" {
                let save = requests.recv_timeout(Duration::from_secs(1)).unwrap();
                assert_eq!(save["method"], "thread/name/set");
                assert_eq!(
                    save["params"],
                    json!({"threadId": "target", "name": "生成的简短标题"})
                );
                replies
                    .send(json!({"id": save["id"], "result": {}}))
                    .unwrap();
            }
            assert_eq!(task.join().unwrap().unwrap(), scenario == "success");
        } else {
            assert!(task.join().unwrap().is_err());
        }
        assert!(requests.try_recv().is_err());
        session.shutdown().unwrap();
    }
}
