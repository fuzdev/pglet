-- The positive counterpart of comma_join_on_scope_loud: `FROM a, b JOIN c ON …`
-- is two FROM items — `a` and `(b JOIN c)` — and an ON referencing only the
-- join's own operands (`b`/`c`) resolves cleanly. The comma item `a` cross-joins
-- the `b JOIN c` result.
-- @setup
CREATE TABLE a (x INT, av TEXT);
CREATE TABLE b (x INT, bv TEXT);
CREATE TABLE c (x INT, cv TEXT);
INSERT INTO a VALUES (1, 'a1'), (2, 'a2');
INSERT INTO b VALUES (1, 'b1'), (2, 'b2');
INSERT INTO c VALUES (1, 'c1'), (2, 'c2');

-- @test
SELECT a.av, b.bv, c.cv
FROM a, b JOIN c ON b.x = c.x
ORDER BY a.av, b.bv;
