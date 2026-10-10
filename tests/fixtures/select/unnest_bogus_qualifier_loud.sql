-- The unnest arm's non-correlated half: a qualifier naming no relation in the statement,
-- at the `unnest(…)` item's own level. Without a `Plan::Unnest` arm in
-- `single_table_name` this level's `SimpleColumns` had no relation identity, so its last
-- tier dropped the qualifier and answered `v`'s `b`; 18.6 raises `42P01`.
--
-- The pair with `select/qualified_outer_ref_unnest_from_item_loud.sql`, which reaches
-- the same missing identity through the correlated outer frame instead. This one is the
-- cheaper guard — no subquery and no quoted column — and reddens on the same one-arm
-- revert.
--
-- **The correlated spelling of this statement was a separate divergence and is closed by
-- a separate rule.** `SELECT (SELECT count(*) FROM u2 WHERE u2.yv = zzz.b) FROM unnest(…)
-- AS v(b)` answered here where 18.6 is `42P01`, because a qualified reference no frame
-- owned fell past the identity tier to a bare match. The outer chain declines that now,
-- so both spellings are `42P01` — reached by two different rules, which is why this arm's
-- own revert still reddens only this fixture.
-- @setup
CREATE TABLE anchor (id INT);
INSERT INTO anchor VALUES (1);

-- @expect-error: missing FROM-clause entry for table "zzz" (42P01)
-- @test
SELECT zzz.b FROM unnest(ARRAY['x', 'y']) AS v(b);
