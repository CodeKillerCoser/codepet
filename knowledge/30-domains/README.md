# Domain Knowledge

Domain documents connect product behavior to source modules and tests.

## Domains

- `agent-events/`: hook ingestion, normalization, activity merge, and filtering risks.
- `agent-control/`: provider capabilities, Codex app-server, Qoder remote-control boundary, reply and approval.
- `pet-window/`: overlay layout, sizing, monitor bounds, and reply editor.
- `settings-and-personalization/`: settings model, pet personalization, and notifications.
- `theme-system/`: theme token library and migration rules.

## Maintenance

When a bug touches one domain repeatedly, update the domain `known-risks.md` and consider adding a reusable rule under `../60-rules/`.
