-- The fold is resolved before any branch is coerced, so a branch set with no common type
-- is `42804` and the non-member literal beside it never reaches `enum_in` — in either
-- argument order, which is what says the rule is after the fold rather than interleaved
-- with it.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE t (id INT, m mood NOT NULL, s TEXT NOT NULL);
INSERT INTO t VALUES (1, 'sad', 'OK'), (2, 'ok', 'OK');

-- @test
-- @expect-error: GREATEST has no common type for mood and text, ahead of the literal
SELECT greatest(m, 'zzz', s) FROM t;
