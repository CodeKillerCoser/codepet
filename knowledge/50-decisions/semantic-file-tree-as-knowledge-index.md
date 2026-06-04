# Semantic File Tree As Knowledge Index

## Background

The project needs living documentation that agents can navigate without a fragile central mapping file.

## Decision

Use the `knowledge/` directory tree and document titles as the knowledge index. Do not create `map.yaml` or a similar central index.

## Alternatives Considered

- Central YAML map: easy to query, but becomes another stale artifact to maintain.
- One large document: simple, but hard for agents to scope and easy to bloat.

## Rationale

Semantic directory names let agents select relevant facts by reading the file tree. This matches the project goal of living documents that evolve with code changes and bug fixes.

## Impact

Every directory needs a `README.md`, and new docs should be placed where their path expresses their role.

## Follow-Up

If navigation becomes hard, improve names and local README entries before adding a separate map.
