-- To 18.6 a parenthesized query and the clauses after it are one query (`gram.y`'s
-- `insertSelectOptions`), so a `LIMIT` both carry is refused as the grammar builds the
-- statement. pglet merges its record of which clauses each half carries the same way, and
-- refuses the same repeat, but plans the parenthesized query's own clauses before the ones
-- written after it (`EDGES.md` row 167).
-- @setup
CREATE TABLE pqr_s (x INT);
INSERT INTO pqr_s VALUES (2), (1);

-- @test
-- @expect-error: multiple LIMIT clauses not allowed
-- @expect-message
(SELECT x FROM pqr_s ORDER BY x LIMIT 1) LIMIT 2;
