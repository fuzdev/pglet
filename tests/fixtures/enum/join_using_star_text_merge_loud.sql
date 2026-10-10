-- The **other** reader of the `USING` merge's equality. `join_using_text_merge_loud`
-- spells its projection, which reaches the executor's merge builder; a `SELECT *` over
-- the same join is expanded by the planner, which resolves the same `left.m = right.m`
-- through its own copy of the rulebook. Reverting either one alone leaves exactly one of
-- these two fixtures red, which is what says they are two readers and not one.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE t (id INT, m mood NOT NULL, s TEXT NOT NULL);
INSERT INTO t VALUES (1, 'sad', 'ok'), (2, 'ok', 'ok'), (3, 'happy', 'ok');

-- @test
-- @expect-error: no operator for mood = text
-- @expect-message
SELECT * FROM t a JOIN (SELECT id, s AS m FROM t) b USING (m);
