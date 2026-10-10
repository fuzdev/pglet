-- **An error-order control — do not trim it.** `transformUpdateStmt` runs the `WHERE`
-- before the target list, so a relation the `WHERE`'s subquery names and the catalog
-- lacks is `42P01` on 18.6 and the `SET` list is never coerced. pglet raises that
-- `42P01` at **execution**, downstream of every plan-time verdict, so the assignment
-- gate has to decline for the whole statement — as it does for an unresolvable `FROM`.
--
-- Its opposite number is the `INSERT` side, which is **not** gated and must not be:
-- `transformInsertStmt` transforms the row source **first**, so `INSERT INTO t (id, b)
-- VALUES (1, 1) ON CONFLICT (id) DO UPDATE SET b = 1 WHERE EXISTS (SELECT 1 FROM
-- nosuchrel)` is 18.6's `42804` (measured, as is the `RETURNING`-subquery spelling).
-- The clause list is per statement kind, not a general rule.
--
-- The row is load-bearing: pglet resolves the subquery's relation at **eval**, so with
-- no row to evaluate it against the statement reports success on both this engine and
-- the pre-gate one — a pre-existing zero-row hole recorded in `EDGES.md`, and not one this gate
-- can close by refusing with the wrong code.
-- @setup
CREATE TABLE acmw (id INT, b BOOL);
INSERT INTO acmw VALUES (1, FALSE);

-- @expect-error: relation "acmw_nosuch" does not exist (42P01)
-- @test
UPDATE acmw SET b = 1 WHERE EXISTS (SELECT 1 FROM acmw_nosuch);
