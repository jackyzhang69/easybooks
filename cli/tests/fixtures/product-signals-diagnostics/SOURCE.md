Copied from jackyzhang69/platform `contracts/fixtures/product-signals-diagnostics` @ 421e3112c; keep byte-identical.

These golden events are validated by both `jz-plugin-common` (`tests/signals_diagnostics.rs`) and accountd (`accountd/signals/client_diagnostics_test.go`). EasyBooks' local diagnostics mirror must reproduce them exactly: `cli/src/signals/diagnostics/conformance.rs` builds the same events from the same inputs and compares parsed JSON. When the platform fixtures change, re-copy them here rather than editing.
