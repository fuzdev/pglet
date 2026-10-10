-- `JOIN` binds tighter than comma, so a JOIN's `ON` can reference only its own
-- chain — not a *non-adjacent* sibling comma item. Here `a` is two comma
-- positions left of `c JOIN d`, so `a.x` in that ON is out of scope: Postgres
-- rejects at plan time (42P01, "invalid reference to FROM-clause entry for table
-- a"); pglet resolves the ON at eval time, so `c`/`d` are seeded to force the
-- inner join to evaluate the ON and surface the same 42P01. (The `c.z = d.w`
-- counterpart, where the qualifier IS the join's own operand, is valid.)
-- @expect-error: non-adjacent comma sibling out of a JOIN's ON scope (42P01)
-- @setup
CREATE TABLE a (x INT);
CREATE TABLE b (y INT);
CREATE TABLE c (z INT);
CREATE TABLE d (w INT);
INSERT INTO a VALUES (1);
INSERT INTO b VALUES (2);
INSERT INTO c VALUES (10), (20);
INSERT INTO d VALUES (10), (30);
-- @test
SELECT * FROM a, b, c JOIN d ON a.x = d.w;
