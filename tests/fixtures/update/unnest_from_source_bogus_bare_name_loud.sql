-- A third registration site, and the second control on the exhaustiveness split.
--
-- An `UPDATE … FROM` holds a FROM sub-plan and no parsed `(name, alias)` list, so its
-- scope is built by `push_plan_relations` over `plan_relations` rather than by
-- `build_column_scope` — and that builder has only `push_relation`, so its `unnest`
-- source has always been columns-exhaustive. This statement is therefore `42703` on both
-- engines before the `build_column_scope` flip and after it, and reverting the flip
-- leaves it green.
--
-- Keeping it beside `select/unnest_bogus_bare_name_own_level_loud.sql` is the point: the
-- two statements name the same defect in the same clause position over the same relation
-- kind, and only one of them was open, because the two lowerings ask different builders.
-- A repair written against "the unnest arm" without asking which builder each route
-- reaches would have looked complete from either fixture alone.
--
-- The array is **empty** so no row reaches the eval-time resolver: an `UPDATE` matching
-- nothing reports `UPDATE 0` here when the plan-time gate declines, which is the silent
-- half this whole family is about.
--
-- The setup cannot raise the asserted `42703` — two statements, neither carrying a
-- reference to anything but its own columns.
-- @setup
CREATE TABLE pt (id INT, v TEXT);
INSERT INTO pt VALUES (1, 'a'), (2, 'b');

-- @test
-- @expect-error: bare `bogus` names neither the target nor the unnest source (42703)
UPDATE pt SET v = 'x' FROM unnest(ARRAY[]::int[]) AS u(k) WHERE bogus = 1;
