-- `transformAExprIn` builds its boolean chain **pairwise**, not as one n-ary node:
-- `makeBoolExpr(…, list_make2(result, cmp))` runs inside the `foreach`, so an n-member
-- chain is n-1 nested binary `BoolExpr`s and `ruleutils.c` prints a paren per fold.
-- Every fixture pinning this renderer used a chain of **two** members, the one arity at
-- which a left fold and a flat join agree, so a flat join passed them all.
--
-- The neighbouring rule is the opposite one and both polarities are here: a *written*
-- `a = 1 OR a = 2 OR …` is flattened by `makeOrExpr` at **grammar** time, so it stays
-- n-ary (`m08`, `m09`). An `IN` is an `A_Expr` at that point and not a `BoolExpr`, so it
-- never joins its neighbour's flatten — `m09`'s chain is three args with the `IN`'s own
-- nested pair as the first, and `m10`'s two parenthesized `IN`s do not merge either.
-- @setup
CREATE TABLE chn (a INT, b INT, c INT, d INT, s TEXT);
-- Three and more members, both polarities, with the array member (`m01`/`m02`/`m05`)
-- and without it (`m03`/`m04`).
ALTER TABLE chn ADD CONSTRAINT m01 CHECK (a IN (1, 2, b, d));
ALTER TABLE chn ADD CONSTRAINT m02 CHECK (a NOT IN (1, 2, b, d));
ALTER TABLE chn ADD CONSTRAINT m03 CHECK (a IN (b, c, d));
ALTER TABLE chn ADD CONSTRAINT m04 CHECK (a NOT IN (b, c, d));
ALTER TABLE chn ADD CONSTRAINT m05 CHECK (a IN (1, 2, 3, b, c, d));
-- The two-member arity that agreed before the fix, and must not move.
ALTER TABLE chn ADD CONSTRAINT m06 CHECK (a IN (b, c));
-- The fold under an `AND` chain: the `IN`'s nested `OR` is one operand of a flat
-- three-conjunct `AND`.
ALTER TABLE chn ADD CONSTRAINT m07 CHECK (a IN (1, 2, b, c, d) AND b > 0 AND c > 0);
-- The written-`OR` neighbours, which stay flat.
ALTER TABLE chn ADD CONSTRAINT m08 CHECK (a = 1 OR a = 2 OR a IN (b, c));
ALTER TABLE chn ADD CONSTRAINT m09 CHECK (a IN (b, c) OR a = 1 OR a = 2);
ALTER TABLE chn ADD CONSTRAINT m10 CHECK ((a IN (b, c)) OR (a IN (1, 2, b, c)));
-- A text chain, so the fold is pinned over the resolved-unknown elements too.
ALTER TABLE chn ADD CONSTRAINT m11 CHECK (s IN ('x', 'y', s, 'z'));
-- Under a `NOT`.
ALTER TABLE chn ADD CONSTRAINT m12 CHECK (NOT (a IN (1, 2, b, c)));
-- @test
SELECT c.conname::text AS conname, pg_get_constraintdef(c.oid) AS definition
FROM pg_constraint c JOIN pg_class cls ON cls.oid = c.conrelid
WHERE cls.relname::text = 'chn' AND c.contype::text = 'c'
ORDER BY c.conname::text ASC
