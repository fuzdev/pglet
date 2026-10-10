-- The **zero-row** twin of `update/returning_from_source_column_ambiguous.sql`: the same
-- ambiguous bare `RETURNING v`, over a statement whose `WHERE` matches nothing. `42702`
-- on 18.6 either way, because the verdict is parse analysis's and has nothing to do with
-- the data.
--
-- It exists because its sibling is **not** discriminating on its own. Two independent
-- things answer `42702` here: the plan-time binding gate (`ColumnScope::check_binding`,
-- counting the claimants the statement's namespace holds) and the executor's own
-- resolver, whose dotted multi-relation names make a bare `v` match two columns by
-- suffix. With a matching row the executor's fires whether or not the plan-time one
-- exists, so removing the plan-time count leaves the sibling **green** — measured.
--
-- With no matching row the executor never evaluates a projection item at all, so only
-- the plan-time verdict is left. Measured under exactly that removal: this statement
-- answers `(0 rows)` with a `text` column and no error, where 18.6 says `42702` — the
-- row-count-dependent shape the plan-time gate exists to remove, invisible to every
-- other fixture in the family.
--
-- Do not "simplify" this file by making the `WHERE` match. That is the sibling.
-- @setup
CREATE TABLE rbn_t (id INT, v TEXT);
CREATE TABLE rbn_s (id INT, v TEXT, k INT);
INSERT INTO rbn_t VALUES (1, 'TARGET');
INSERT INTO rbn_s VALUES (1, 'SOURCE', 7);

-- @test
-- @expect-error: 42702 — the ambiguity is parse analysis's verdict, so no row need match
UPDATE rbn_t SET id = 1 FROM rbn_s s WHERE 1 = 0 RETURNING v;
