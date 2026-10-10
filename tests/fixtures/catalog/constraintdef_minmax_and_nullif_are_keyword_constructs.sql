-- `greatest` / `least` / `nullif` are **not `pg_proc` calls** on 18.6: parse analysis
-- builds a `MinMaxExpr` and a `NullIfExpr`, and `get_rule_expr` prints each as the
-- uppercase keyword — exactly as it prints `COALESCE(`, which is the control here
-- (`n01`, already agreeing before this rule landed). `concat` / `concat_ws` *are*
-- `pg_proc` entries and stay lowercase, so they are the other half of the control.
--
-- The unknown-resolving rule splits the three the same way parse analysis does:
-- `GREATEST`/`LEAST` are `select_common_type` positions over all their arguments, so a
-- `varchar` sibling makes the literal `character varying` (`n08`), while `NULLIF`
-- resolves the `=` operator over its pair and that rule folds `varchar` to `text`.
--
-- **`NULLIF`'s varchar half cannot be a row here**, and the reason is not this rule:
-- 18.6 renders `nullif(v, 'x')` as `NULLIF((v)::text, 'x'::text)`, relabelling the
-- *column*, and pglet stores no such coercion (`EDGES.md` row 90) — so the literal
-- agrees and the operand does not. `n08`'s `IS NOT NULL` shape is chosen for the same
-- reason: `least(v, 'x') <> ''` puts the whole `LEAST` under a `text` comparison there
-- and picks up the same outer relabel. A pair whose known members disagree still
-- declines (`n10`, row 90 again).
-- @setup
CREATE TABLE mmn (a INT, b INT, s TEXT, v VARCHAR(10), n NUMERIC);
ALTER TABLE mmn ADD CONSTRAINT n01 CHECK (coalesce(a, b) > 0);
ALTER TABLE mmn ADD CONSTRAINT n02 CHECK (greatest(a, b) > 0);
ALTER TABLE mmn ADD CONSTRAINT n03 CHECK (least(a, b, 3) > 0);
ALTER TABLE mmn ADD CONSTRAINT n04 CHECK (nullif(a, b) IS NOT NULL);
ALTER TABLE mmn ADD CONSTRAINT n05 CHECK (nullif(a, 3) IS NOT NULL);
ALTER TABLE mmn ADD CONSTRAINT n06 CHECK (least('a', s) <> '');
ALTER TABLE mmn ADD CONSTRAINT n07 CHECK (greatest('a', s) <> '');
ALTER TABLE mmn ADD CONSTRAINT n08 CHECK (least(v, 'x') IS NOT NULL);
ALTER TABLE mmn ADD CONSTRAINT n09 CHECK (greatest(v, 'x') IS NOT NULL);
ALTER TABLE mmn ADD CONSTRAINT n10 CHECK (greatest(a) > 0);
ALTER TABLE mmn ADD CONSTRAINT n11 CHECK (coalesce(s, 'd') <> '');
ALTER TABLE mmn ADD CONSTRAINT n12 CHECK (nullif(s, 'x') IS NOT NULL);
ALTER TABLE mmn ADD CONSTRAINT n13 CHECK (concat_ws('-', s, 'x') <> '');
ALTER TABLE mmn ADD CONSTRAINT n14 CHECK (concat('a', s) <> '');
ALTER TABLE mmn ADD CONSTRAINT n15 CHECK (greatest('a', 'b') <> '');
ALTER TABLE mmn ADD CONSTRAINT n16 CHECK (nullif('a', 'b') IS NOT NULL);
-- @test
SELECT c.conname::text AS conname, pg_get_constraintdef(c.oid) AS definition
FROM pg_constraint c JOIN pg_class cls ON cls.oid = c.conrelid
WHERE cls.relname::text = 'mmn' AND c.contype::text = 'c'
ORDER BY c.conname::text ASC
