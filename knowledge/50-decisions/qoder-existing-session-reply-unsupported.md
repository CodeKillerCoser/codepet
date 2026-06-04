# Qoder Existing Session Reply Unsupported

## Background

Qoder remote-control supports registering the machine as a remote environment, but Code Pet needs to send messages to an existing local Qoder session.

## Decision

Do not expose Qoder existing-session reply in the pet UI until a stable, verified API exists.

## Alternatives Considered

- Use Qoder remote-control daemon as if it were a local chat API: rejected because the observed shape registers an environment through a broker.
- Use local MCP-like port `127.0.0.1:52345`: rejected because probing found VM info tooling, not chat/session send APIs.
- Use focus and paste: rejected as unreliable for a provider-specific remote-control feature.

## Impact

Backend `QoderDriver` returns unsupported for reply. Frontend sets `canReply` to false for Qoder.

## Follow-Up

Investigate official qoder.com broker or remote-control APIs before enabling this capability.
