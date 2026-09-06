use crate::generated::{dispatch, JsonRpcInboundRequest, JsonRpcResponse, JsonRpcResponsePayload,
    ProtocolDispatchLane, ProtocolError, ProtocolMethod, ProtocolServer,
    ProviderShutdownRequest, ProviderWireMessage, RpcError};
use crate::message::{ProviderMux, error};
use crate::transport::default_transport_limits;
use super::{ProviderEventSink, StdioServerError, StdioServerOptions};
use std::{io::{Read, Write}, sync::{Arc, atomic::{AtomicBool, Ordering}}};
use tokio::sync::Semaphore;

fn response(id: Option<String>, result: Result<serde_json::Value, ProtocolError>) -> JsonRpcResponse {
    JsonRpcResponse { jsonrpc: "2.0".into(), id, response: match result {
        Ok(result) => JsonRpcResponsePayload::Ok { result },
        Err(e) => JsonRpcResponsePayload::Error { error: RpcError { code: -32000, message: e.message.clone(), data: serde_json::to_value(e).ok().and_then(|v| v.as_object().map(|v| v.iter().map(|(k,v)| (k.clone(),v.clone())).collect())) } },
    } }
}

/// Explicit mux runtime for embedders and conformance tests. Blocking stdio pumps are detached:
/// arbitrary Read implementations cannot be interrupted, and must not occupy Tokio's blocking pool.
pub async fn serve_mux_with_io<R, W, P, F>(reader: R, writer: W, options: StdioServerOptions, factory: F) -> Result<(), StdioServerError>
where R: Read + Send + 'static, W: Write + Send + 'static, P: ProtocolServer + 'static,
    F: FnOnce(Arc<dyn ProviderEventSink>) -> P {
    super::options::validate_options(options)?;
    let (io, mut output_flushed) = super::io::bridge(reader, writer)?;
    let (read, write) = tokio::io::split(io);
    let mut limits = default_transport_limits();
    limits.max_encoded_message_bytes = options.max_frame_bytes as u64;
    let total = options.max_concurrent_requests.saturating_add(options.max_pending_requests) as u64;
    limits.small_streams = limits.small_streams.min((total / 2).max(1));
    limits.normal_streams = limits.normal_streams.min(total.saturating_sub(limits.small_streams).max(1));
    limits.control_streams = limits.control_streams.min(options.max_concurrent_control_requests.saturating_add(options.max_pending_control_requests) as u64);
    let (peer, mut incoming, mut driver) = ProviderMux::connect(read, write, false, limits).await?;
    let output_closed = Arc::new(AtomicBool::new(false));
    let (sink, mut event_rx) = super::events::queued_events(output_closed.clone());
    let activity = Arc::new(super::activity::Activity::default());
    let observed_activity = activity.clone();
    let observed_sink: Arc<dyn ProviderEventSink> = Arc::new(move |event: crate::generated::ProtocolEvent| {
        observed_activity.event(&event);
        sink.publish(event)
    });
    let provider = Arc::new(factory(observed_sink.clone()));
    let event_peer = peer.clone();
    let mut event_task = tokio::spawn(async move {
        while let Some((event, _permit)) = event_rx.recv().await {
            if let Err(e) = event_peer.send_event(event).await {
                if e.code == "provider_message_encode_failed" {
                    eprintln!("Provider event rejected before body transmission: {}", e.message);
                } else { return Err(e); }
            }
        }
        Ok::<_, ProtocolError>(())
    });
    let heartbeat = super::heartbeat::HostHeartbeat::new();
    let monitor = tokio::spawn(heartbeat.clone().run(provider.clone(), activity.clone()));
    let initialized = Arc::new(AtomicBool::new(false));
    let dispatch_closed = Arc::new(AtomicBool::new(false));
    let executions = Arc::new(std::sync::Mutex::new(Vec::<tokio::task::AbortHandle>::new()));
    let normal = Arc::new(Semaphore::new(options.max_concurrent_requests));
    let control = Arc::new(Semaphore::new(options.max_concurrent_control_requests));
    let mut tasks: tokio::task::JoinSet<Result<bool, ProtocolError>> = tokio::task::JoinSet::new();
    let mut shutdown_called = false;
    let mut driver_observed = false;
    let mut output_observed = false;
    let mut result = Ok(());
    loop {
        tokio::select! {
            output = &mut output_flushed => {
                output_observed = true;
                match output {
                    Ok(Err(error)) => result = Err(StdioServerError::new(error)),
                    Err(error) => result = Err(StdioServerError::new(error.to_string())),
                    Ok(Ok(())) => {},
                }
                break;
            }
            transport = &mut driver => {
                driver_observed = true;
                match transport { Ok(Err(e)) => result = Err(e.into()), Err(e) => result = Err(StdioServerError::new(e.to_string())), _ => {} }
                break;
            }
            events = &mut event_task => {
                match events { Ok(Err(e)) => result = Err(e.into()), Err(e) => result = Err(StdioServerError::new(e.to_string())), _ => {} }
                break;
            }
            done = tasks.join_next(), if !tasks.is_empty() => {
                match done {
                    Some(Ok(Ok(true))) => { shutdown_called = true; break; }
                    Some(Ok(Err(e))) => eprintln!("Provider stream ended: {}", e.message),
                    Some(Err(e)) => { result = Err(StdioServerError::new(e.to_string())); break; }
                    _ => {}
                }
            }
            next = incoming.recv() => {
                let Some(mut stream) = next else { break; };
                let provider = provider.clone(); let heartbeat = heartbeat.clone();
                let activity = activity.clone();
                let executions = executions.clone(); let dispatch_closed = dispatch_closed.clone();
                let initialized = initialized.clone(); let normal = normal.clone(); let control = control.clone();
                let lifetime = peer.stream_timeout();
                tasks.spawn(async move {
                    tokio::time::timeout(lifetime, async move {
                        let received = stream.receive().await?;
                        match received.message {
                            ProviderWireMessage::Request(JsonRpcInboundRequest::Typed(request)) => {
                                let method = request.method(); let id = request.id().clone();
                                let lane = if method.dispatch_lane() == ProtocolDispatchLane::Control { control } else { normal };
                                // Heartbeats retain their runtime fast path even if control handlers are busy.
                                let reply = if let crate::generated::ProtocolRequest::ProviderPing { params, .. } = request {
                                    response(Some(id), if initialized.load(Ordering::SeqCst) {
                                        heartbeat.ping(params).and_then(|v| serde_json::to_value(v).map_err(error))
                                    } else { Err(ProtocolError { code: "provider_not_initialized".into(), message: "Initialize before heartbeat".into(), retryable: true, details: None }) })
                                } else {
                                    let detached_start = matches!(method, ProtocolMethod::TurnStart | ProtocolMethod::TurnSteer);
                                    let pending_executions = executions.clone();
                                    let work = async move {
                                        let _execution = activity.gate.read().await;
                                        let _permit = lane.acquire_owned().await.map_err(error)?;
                                        if dispatch_closed.load(Ordering::SeqCst) { return Err(error("Provider is shutting down")); }
                                        if method == ProtocolMethod::ProviderShutdown {
                                            dispatch_closed.store(true, Ordering::SeqCst);
                                            for execution in pending_executions.lock().unwrap_or_else(|e| e.into_inner()).drain(..) { execution.abort(); }
                                        }
                                        let reply = dispatch(provider.as_ref(), request).await;
                                        activity.response(method, &reply);
                                        Ok::<_, ProtocolError>(reply)
                                    };
                                    if detached_start {
                                        // Once admitted, a turn mutation outlives its response stream.
                                        // The task retains the execution guard until its result is recorded;
                                        // Host EOF/shutdown still force-cleans the adapter generation.
                                        let mut execution = tokio::spawn(work);
                                        {
                                            let mut pending = executions.lock().unwrap_or_else(|e| e.into_inner());
                                            pending.retain(|task| !task.is_finished());
                                            pending.push(execution.abort_handle());
                                        }
                                        tokio::select! {
                                            result = &mut execution => result.map_err(error)??,
                                            _ = stream.cancelled() => return Ok(false),
                                        }
                                    } else {
                                        tokio::select! {
                                            result = work => result?,
                                            _ = stream.cancelled() => return Ok(false),
                                        }
                                    }
                                };
                                if method == ProtocolMethod::ProviderInitialize && matches!(reply.response, JsonRpcResponsePayload::Ok { .. }) { initialized.store(true, Ordering::SeqCst); }
                                let id = reply.id.clone();
                                // A failed encoding has not emitted an envelope, so a bounded error can still use this stream.
                                let written = stream.respond_with_fallback(ProviderWireMessage::Response(reply), id).await;
                                if method == ProtocolMethod::ProviderShutdown { return Ok(true); }
                                written?; Ok(false)
                            }
                            ProviderWireMessage::Request(JsonRpcInboundRequest::Rejected(rejected)) => {
                                stream.respond(ProviderWireMessage::Response(rejected.into_response())).await?; Ok(false)
                            }
                            _ => Err(error("Host must open request streams")),
                        }
                    }).await.map_err(|_| error("stream lifetime exceeded"))?
                });
            }
        }
    }
    dispatch_closed.store(true, Ordering::SeqCst);
    for execution in executions.lock().unwrap_or_else(|e| e.into_inner()).drain(..) { execution.abort(); }
    output_closed.store(true, Ordering::SeqCst);
    monitor.abort(); event_task.abort();
    // Once the connection is lost, queued handlers cannot deliver a response.
    if !shutdown_called { tasks.abort_all(); }
    let _ = tokio::time::timeout(options.dispatch_drain_timeout, async {
        while tasks.join_next().await.is_some() {}
    }).await;
    tasks.abort_all(); while tasks.join_next().await.is_some() {}
    if !shutdown_called {
        let cleanup = tokio::time::timeout(options.dispatch_drain_timeout, provider.provider_shutdown(ProviderShutdownRequest {})).await;
        match cleanup {
            Ok(Err(e)) => result = Err(e.into()),
            Err(_) => result = Err(StdioServerError::new("Provider shutdown cleanup timed out")),
            _ => {}
        }
    }
    peer.close();
    // The incoming channel may close before select observes the driver result.
    // Always collect that result once, including errors discovered during shutdown.
    if !driver_observed {
        match tokio::time::timeout(options.dispatch_drain_timeout, &mut driver).await {
            Ok(Ok(Err(error))) => result = Err(error.into()),
            Ok(Err(error)) => result = Err(StdioServerError::new(error.to_string())),
            Err(_) => result = Err(StdioServerError::new("Provider mux driver drain timed out")),
            _ => {},
        }
    }
    driver.abort();
    // Duplex flush only hands bytes to the pump. A successful runtime return must also
    // mean the actual blocking stdout owner has drained, or process exit can truncate the reply.
    if !output_observed {
        match tokio::time::timeout(options.dispatch_drain_timeout, output_flushed).await {
            Ok(Ok(Ok(()))) => {},
            Ok(Ok(Err(e))) => result = Err(StdioServerError::new(e)),
            Ok(Err(e)) => result = Err(StdioServerError::new(e.to_string())),
            Err(_) => result = Err(StdioServerError::new("Provider stdout drain timed out")),
        }
    }
    result
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    struct SlowWriter { io: std::os::unix::net::UnixStream, dropped: Arc<AtomicBool> }
    impl Write for SlowWriter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> { self.io.write(bytes) }
        fn flush(&mut self) -> std::io::Result<()> {
            std::thread::sleep(std::time::Duration::from_millis(40)); self.io.flush()
        }
    }
    impl Drop for SlowWriter { fn drop(&mut self) { self.dropped.store(true, Ordering::SeqCst); } }
    struct ShutdownOnly;
    impl ProtocolServer for ShutdownOnly {
        fn provider_shutdown<'a>(&'a self, _: ProviderShutdownRequest) -> crate::ProtocolFuture<'a, crate::ProviderShutdownResponse> {
            Box::pin(async { Ok(crate::ProviderShutdownResponse { accepted: true }) })
        }
    }
    #[tokio::test]
    async fn shutdown_waits_for_the_blocking_output_pump_to_flush_and_finish() {
        let (host, provider) = std::os::unix::net::UnixStream::pair().unwrap();
        host.set_nonblocking(true).unwrap();
        let host = tokio::net::UnixStream::from_std(host).unwrap();
        let (read, write) = tokio::io::split(host);
        let dropped = Arc::new(AtomicBool::new(false)); let check = dropped.clone();
        let server = tokio::spawn(async move {
            let result = serve_mux_with_io(provider.try_clone().unwrap(), SlowWriter { io: provider, dropped }, StdioServerOptions::default(), |_| ShutdownOnly).await;
            (result, check.load(Ordering::SeqCst))
        });
        let (peer, _, _driver) = ProviderMux::connect(read, write, true, default_transport_limits()).await.unwrap();
        let request = crate::generated::ProtocolRequest::from_method_params(ProtocolMethod::ProviderShutdown, "shutdown".into(), serde_json::json!({})).unwrap();
        let response = peer.exchange(ProviderWireMessage::Request(JsonRpcInboundRequest::Typed(request))).await;
        let (result, flushed) = server.await.unwrap();
        assert!(result.is_ok()); assert!(response.is_ok());
        assert!(flushed, "serve_mux_with_io returned before its stdout owner finished flushing");
    }
    struct StartProbe {
        entered: Arc<tokio::sync::Notify>,
        release: Arc<tokio::sync::Semaphore>,
        finished: Arc<AtomicBool>,
    }
    impl ProtocolServer for StartProbe {
        fn turn_start<'a>(&'a self, _: crate::TurnStartRequest) -> crate::ProtocolFuture<'a, crate::TurnStartResponse> {
            Box::pin(async move {
                self.entered.notify_one();
                let _permit = self.release.acquire().await.unwrap();
                self.finished.store(true, Ordering::SeqCst);
                Err(error("fixture completed"))
            })
        }
        fn provider_shutdown<'a>(&'a self, _: ProviderShutdownRequest) -> crate::ProtocolFuture<'a, crate::ProviderShutdownResponse> {
            Box::pin(async { Ok(crate::ProviderShutdownResponse { accepted: true }) })
        }
    }

    #[tokio::test]
    async fn admitted_start_outlives_response_cancellation_but_not_provider_shutdown() {
        for shutdown_during_start in [false, true] {
            let (host, provider) = std::os::unix::net::UnixStream::pair().unwrap();
            host.set_nonblocking(true).unwrap();
            let host = tokio::net::UnixStream::from_std(host).unwrap();
            let (read, write) = tokio::io::split(host);
            let entered = Arc::new(tokio::sync::Notify::new());
            let release = Arc::new(tokio::sync::Semaphore::new(0));
            let finished = Arc::new(AtomicBool::new(false));
            let probe = StartProbe { entered: entered.clone(), release: release.clone(), finished: finished.clone() };
            let server = tokio::spawn(async move {
                serve_mux_with_io(provider.try_clone().unwrap(), provider, StdioServerOptions::default(), |_| probe).await
            });
            let (peer, _, _driver) = ProviderMux::connect(read, write, true, default_transport_limits()).await.unwrap();
            let request = crate::ProtocolRequest::from_method_params(ProtocolMethod::TurnStart, "start".into(), serde_json::json!({
                "conversation":{"deviceId":"d","providerPluginId":"p","providerInstanceId":"i","nativeResourceId":"c"},
                "clientRequestId":"message", "capabilityRevision":"1", "input":{"kind":"text","text":"hello"},
                "selection":{"accessModeId":"workspace-write","model":{"kind":"flat","modelId":"fixture"}}
            })).unwrap();
            let start_peer = peer.clone();
            let start = tokio::spawn(async move { start_peer.exchange(ProviderWireMessage::Request(JsonRpcInboundRequest::Typed(request))).await });
            tokio::time::timeout(std::time::Duration::from_secs(2), entered.notified()).await.unwrap();
            start.abort(); let _ = start.await;
            if !shutdown_during_start {
                release.add_permits(1);
                tokio::time::timeout(std::time::Duration::from_secs(2), async {
                    while !finished.load(Ordering::SeqCst) { tokio::task::yield_now().await; }
                }).await.expect("cancelled response must not cancel admitted execution");
            }
            let shutdown = crate::ProtocolRequest::from_method_params(ProtocolMethod::ProviderShutdown, "shutdown".into(), serde_json::json!({})).unwrap();
            peer.exchange(ProviderWireMessage::Request(JsonRpcInboundRequest::Typed(shutdown))).await.unwrap();
            server.await.unwrap().unwrap();
            if shutdown_during_start {
                release.add_permits(1);
                tokio::task::yield_now().await;
                assert!(!finished.load(Ordering::SeqCst), "shutdown must abort outstanding detached starts");
            }
        }
    }

    struct CleanupProbe(Arc<AtomicBool>);
    impl ProtocolServer for CleanupProbe {
        fn provider_shutdown<'a>(&'a self, _: ProviderShutdownRequest) -> crate::ProtocolFuture<'a, crate::ProviderShutdownResponse> {
            Box::pin(async { self.0.store(true, Ordering::SeqCst); Ok(crate::ProviderShutdownResponse { accepted: true }) })
        }
    }
    struct FailingWriter { io: std::os::unix::net::UnixStream, fail: Arc<AtomicBool> }
    impl Write for FailingWriter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if self.fail.load(Ordering::SeqCst) { return Err(std::io::Error::new(std::io::ErrorKind::BrokenPipe, "injected output failure")); }
            self.io.write(bytes)
        }
        fn flush(&mut self) -> std::io::Result<()> { self.io.flush() }
    }
    #[tokio::test]
    async fn input_eof_and_output_failure_trigger_cleanup_without_waiting_for_other_pipe() {
        for output_failure in [false, true] {
            let (host, provider) = std::os::unix::net::UnixStream::pair().unwrap();
            let input_control = host.try_clone().unwrap();
            host.set_nonblocking(true).unwrap();
            let host = tokio::net::UnixStream::from_std(host).unwrap();
            let (read, write) = tokio::io::split(host);
            let fail = Arc::new(AtomicBool::new(false));
            let cleaned = Arc::new(AtomicBool::new(false));
            let probe = cleaned.clone(); let fail_writer = fail.clone();
            let server = tokio::spawn(async move {
                super::super::serve_stdio_with_io(provider.try_clone().unwrap(), FailingWriter { io: provider, fail: fail_writer },
                    StdioServerOptions::default(), |_| CleanupProbe(probe)).await
            });
            let (peer, _, _driver) = ProviderMux::connect(read, write, true, default_transport_limits()).await.unwrap();
            let request_task = if output_failure {
                fail.store(true, Ordering::SeqCst);
                Some(tokio::spawn(async move {
                    let request = crate::ProtocolRequest::from_method_params(ProtocolMethod::ProviderDescribe, "broken-output".into(), serde_json::json!({})).unwrap();
                    peer.exchange(ProviderWireMessage::Request(JsonRpcInboundRequest::Typed(request))).await
                }))
            } else {
                input_control.shutdown(std::net::Shutdown::Write).unwrap();
                None
            };
            let result = tokio::time::timeout(std::time::Duration::from_secs(3), server).await.unwrap().unwrap();
            assert_eq!(result.is_err(), output_failure, "{result:?}");
            assert!(cleaned.load(Ordering::SeqCst));
            if let Some(request) = request_task { request.abort(); }
            // Release the blocking input pump only after proving output failure triggers cleanup.
            let _ = input_control.shutdown(std::net::Shutdown::Write);
        }
    }

}
