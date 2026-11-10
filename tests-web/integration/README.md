# Behavioral Integration Tests

TypeScript tests for pglet behavior that SQL fixtures can't capture.

**Most tests should be SQL fixtures** (see `tests/fixtures/`). Use this file
only for:

- Connection/disconnection behavior
- Session state persistence and isolation
- Row count verification (INSERT/UPDATE/DELETE)
- Error recovery scenarios
- Concurrent connection tests

## Running

```bash
npm run test:integration            # self-contained — starts its own server on a port the kernel picks
npm run test:integration:server     # server-lifecycle tests (spawn/kill per test)
```

Broader query correctness lives in the SQL fixtures (`tests/fixtures/`); this
suite is only for behavior fixtures can't express.
