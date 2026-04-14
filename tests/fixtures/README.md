Real-sample fixtures for scan pipeline tests should stay small and checked in only when
they add coverage that tempdir-built fixtures cannot provide.

Requirements from the execution plan:
- Prefer synthetic temp projects in tests for the default path.
- If we add real samples later, keep them minimal and document which rule they cover.
- Do not commit large generated directories such as full `target/`, `node_modules/`, or `.venv/`.
- Any checked-in sample should be safe to inspect, deterministic, and easy to recreate.
