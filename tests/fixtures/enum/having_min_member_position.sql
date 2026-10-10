-- `HAVING min(m) < 'ok'` runs on the **aggregate** walk, which is separate code from the
-- scalar one — the scalar walk refuses an `Expr::Aggregate` outright, so no other shape
-- reaches this comparison. `min(m)` carries `m`'s enum because `min`/`max` are the two
-- aggregates whose result type is their argument's.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE t (id INT, m mood NOT NULL, g INT);
INSERT INTO t VALUES (1, 'sad', 1), (2, 'ok', 1), (3, 'happy', 2), (4, 'ok', 2);

-- @test
SELECT g FROM t GROUP BY g HAVING min(m) < 'ok' ORDER BY g;
