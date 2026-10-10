-- `JOIN … USING (k1, k2)` is Postgres's `left.k1 = right.k1 AND left.k2 = right.k2`,
-- resolved at parse time — so a mistyped *second* column errors even when no row pair
-- ever gets that far. pglet gated on the runtime values pair-by-pair and returned on
-- the first non-match, so with `k1` never matching, the `text = integer` on `k2` was
-- never reached and the query quietly returned zero rows. (`k1` deliberately does not
-- match: when it does, the loop reaches `k2` and the per-value gate already fires —
-- that spelling can't see this bug.) The gate now runs once on the declared types.
-- @setup
CREATE TABLE a (k1 INT, k2 TEXT, av TEXT);
CREATE TABLE b (k1 INT, k2 INT, bv TEXT);
INSERT INTO a VALUES (1, 'x', 'av');
INSERT INTO b VALUES (2, 9, 'bv');
-- @test
-- @expect-error: the second USING column has no operator, even though no pair reaches it
SELECT av, bv FROM a JOIN b USING (k1, k2);
