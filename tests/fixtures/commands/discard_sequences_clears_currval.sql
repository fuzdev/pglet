-- The one `DISCARD` target with an observable effect a fixture can reach:
-- `DISCARD SEQUENCES` drops the session's `currval` cache, so the `currval` after it
-- is `55000` on both engines — the same answer a session that never advanced the
-- sequence gets. Without the clearing this reads `1` and the fixture fails, so the
-- guard is not vacuous.
--
-- The setup cannot fail with `55000`, so this error fixture is not scoring off its
-- own setup.
-- @setup
CREATE SEQUENCE discard_seq;
-- @test
-- @expect-error: currval of sequence "discard_seq" is not yet defined in this session
SELECT nextval('discard_seq');
DISCARD SEQUENCES;
SELECT currval('discard_seq');
