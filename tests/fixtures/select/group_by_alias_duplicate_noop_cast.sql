-- Two target entries sharing an output alias are **not** ambiguous when they denote
-- the same expression, and a *no-op* cast does not make them differ.
--
-- Postgres compares parse nodes, and `coerce_type` has already elided any cast whose
-- target equals the operand's existing type — so over an `integer` x, `x` and
-- `x::integer` present as two identical `Var`s and `findTargetlistEntrySQL92` accepts
-- the key. pglet compares **post-coercion planner `Expr`s**, where the redundant
-- `Expr::Cast` survives, so it read as a genuine difference and raised `42702` on a
-- query Postgres answers.
--
-- The strip is type-directed and can only remove a cast that changes nothing, so a
-- widening cast still differs — see `group_by_alias_duplicate_widening_cast.sql`.
--
-- Wrapped in a `count(*)` so the two same-named output columns stay inside the
-- subquery: the harness keys its recorded rows by column name, and duplicate names
-- collapse.
-- @setup
CREATE TABLE gbdnc (x INT);
INSERT INTO gbdnc VALUES (1), (2), (2);
-- @test
SELECT count(*) AS n FROM (SELECT x AS a, x::integer AS a FROM gbdnc GROUP BY a) q;
