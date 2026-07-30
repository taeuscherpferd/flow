# flowmation-domain

Pure and compatibility-sensitive domain behavior for the Rust Flowmation core.

The crate exposes:

- strong persisted IDs and Serde-compatible agent, workflow, schedule, chat,
  tool, and configuration records;
- model configuration merging, validation, and alias resolution;
- the legacy directory SHA-256 fingerprint and symbolic-link policy;
- five-field cron parsing with IANA timezone and daylight-saving behavior;
- workflow input-schema validation;
- bounded input-history navigation and atomic, locked history persistence.

Infrastructure and application orchestration belong in the sibling crates.
The modules in this crate do not perform terminal I/O or provider requests.
