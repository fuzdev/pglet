-- 18.6 has no `IN` node: `transformAExprIn` (`parse_expr.c`) rewrites the list at parse
-- analysis, and `ruleutils.c` deparses whatever it built. The rewrite is not one shape
-- but five, and taking `= ANY (ARRAY[…])` for the rule is what a fix gets wrong — the
-- array is built **only** for more than one element that holds no column, `NOT IN`
-- flips to `<> ALL` and an `AND` chain, a lone element is a plain comparison, a column
-- element stays its own chain operand, and the array comes first with the column
-- elements after it in list order.
--
-- The polarity that must not move is `k09`: a user-written `= ANY (ARRAY[…])` already
-- agreed, and still has to.
-- @setup
CREATE TABLE inl (a INT, b INT, c INT, s TEXT);
ALTER TABLE inl ADD CONSTRAINT k01 CHECK (a IN (1,2,3));
ALTER TABLE inl ADD CONSTRAINT k02 CHECK (a NOT IN (1,2,3));
ALTER TABLE inl ADD CONSTRAINT k03 CHECK (a IN (1));
ALTER TABLE inl ADD CONSTRAINT k04 CHECK (a NOT IN (1));
ALTER TABLE inl ADD CONSTRAINT k05 CHECK (a IN (b, c));
ALTER TABLE inl ADD CONSTRAINT k06 CHECK (a NOT IN (b, c));
ALTER TABLE inl ADD CONSTRAINT k07 CHECK (s IN ('viewer', 'editor'));
ALTER TABLE inl ADD CONSTRAINT k09 CHECK (a = ANY (ARRAY[1,2,3]));
ALTER TABLE inl ADD CONSTRAINT k10 CHECK (a IN (1, b));
ALTER TABLE inl ADD CONSTRAINT k11 CHECK (a IN (1, 2, b));
ALTER TABLE inl ADD CONSTRAINT k12 CHECK (a NOT IN (1, 2, b));
ALTER TABLE inl ADD CONSTRAINT k13 CHECK (s IN ('x'));
ALTER TABLE inl ADD CONSTRAINT k14 CHECK (NOT (a IN (1, 2)));
-- The left operand is itself an *unknown* literal, so nothing in the statement carries
-- a type and `select_common_type` falls to its all-unknown answer, `text` — the rule
-- that lends the element type here is the same one that lends it in `k07`.
ALTER TABLE inl ADD CONSTRAINT k15 CHECK ('x' IN ('a', 'b'));
-- The written `= ANY (ARRAY[…])` and the written `IN` are **one node** on 18.6, so they
-- must read back byte-identical: `k16` against `k07`, `k17` against `k02`. pglet keeps
-- them as two persisted variants, so this is the pair that catches the two arms
-- answering the same 18.6 node two different ways.
ALTER TABLE inl ADD CONSTRAINT k16 CHECK (s = ANY (ARRAY['viewer', 'editor']));
ALTER TABLE inl ADD CONSTRAINT k17 CHECK (s <> ALL (ARRAY['viewer', 'editor']));
-- @test
SELECT c.conname::text AS conname, pg_get_constraintdef(c.oid) AS definition
FROM pg_constraint c JOIN pg_class cls ON cls.oid = c.conrelid
WHERE cls.relname::text = 'inl' AND c.contype::text = 'c'
ORDER BY c.conname::text ASC
