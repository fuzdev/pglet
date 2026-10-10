-- A composite key names and values every part, `, `-joined in two parens.
-- @setup
CREATE TABLE t_u2 (a INT, b TEXT, UNIQUE (a, b));
INSERT INTO t_u2 VALUES (1, 'x');

-- @test
-- @expect-error: duplicate composite unique key
-- @no-profile-lock: same write-path enforcement as `unique_key_detail.sql` — the composite probe is `enforce_index_entries`, ungated, and the INSERT plans no predicate
-- @expect-message
INSERT INTO t_u2 VALUES (1, 'x');
