-- `CASE`'s type is `select_common_type` over its **results** — every `THEN` plus the
-- `ELSE` — and not over its operand or its `WHEN` conditions, so an enum reaching any
-- result arm types the whole expression. Both a `THEN` and the `ELSE` carry `m` here;
-- the simple form (`CASE m WHEN … THEN m ELSE m END`) is the same arm and diverged
-- identically before the fix.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE t (id INT, m mood NOT NULL);
INSERT INTO t VALUES (1, 'sad'), (2, 'ok'), (3, 'happy');

-- @test
SELECT id FROM t ORDER BY CASE WHEN id = 1 THEN m ELSE m END;
