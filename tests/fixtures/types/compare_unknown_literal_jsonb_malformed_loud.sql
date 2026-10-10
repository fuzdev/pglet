-- The malformed half of `compare_unknown_literal_container_targets.sql`. Because the
-- unknown literal resolves to `jsonb`, a bad spelling is the **input function's**
-- error (`22P02`, invalid input syntax for type json) — not "operator does not
-- exist". Leaving the literal as `text` produced `42883`, the wrong SQLSTATE for the
-- wrong reason.
-- @setup
CREATE TABLE t (j JSONB);
INSERT INTO t VALUES ('{"k": 1}');
-- @test
-- @expect-error: an unknown literal resolved to jsonb fails on parse, not on operator lookup
SELECT * FROM t WHERE j = 'not json';
