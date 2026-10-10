-- `UPDATE … FROM src s` puts a **sibling** relation in scope whose columns are not
-- the target's. `s.n` and `s.m` are both `src2` integers; the target `tgt2` has its
-- own `n` (text). Resolving the static comparison gate on the bare name read `s.n` as
-- `tgt2.n` and rejected `text = integer` — a `42883` on a query Postgres runs.
-- Resolution is qualifier-aware now, and the `FROM` source's relations are registered
-- under their own aliases, so `s.n` reads `src2`'s `n`.
-- @setup
CREATE TABLE tgt2 (n TEXT, m INT, tag TEXT);
CREATE TABLE src2 (n INT, m INT);
INSERT INTO tgt2 VALUES ('a', 1, 'x'), ('b', 2, 'y');
INSERT INTO src2 VALUES (7, 7);
-- @test
UPDATE tgt2 SET tag = 'hit' FROM src2 s WHERE s.n = s.m AND tgt2.m = 1 RETURNING tgt2.n, tgt2.tag;
