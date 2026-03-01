# Library-First Migration Checklist

Status for `todos/pikahut-library-first-integration-tests.md`.

- [x] 1. Integration matrix document created and scoped.
- [x] 2. `pikahut::testing` public API contract introduced.
- [x] 3. Deterministic lifecycle semantics implemented.
- [x] 4. Typed command orchestration helpers implemented.
- [x] 5. Capability gating and explicit skip primitives implemented.
- [x] 6. `test_harness` reduced to thin dispatch onto library scenarios.
- [x] 7. Deterministic local integration scenarios have Rust selectors.
- [x] 8. Full OpenClaw E2E has Rust selector with artifact-first handling.
- [x] 9. Public/deployed UI+call flows have Rust selectors.
- [x] 10. Primal nightly path simplified to single smoke selector with evidence capture.
- [x] 11. CI/just lane execution switched to Rust selector contracts.
- [x] 12. Integration shell scripts reduced to selector wrappers.
- [x] 13. Guardrails + docs for API stability and migration completeness added.
- [ ] 14. Manual QA gate completed by user sign-off.

## Notes

- Manual QA sign-off remains the final gate and cannot be auto-completed in unattended mode.
