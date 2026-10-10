-- 18.6's `AND` / `OR` node is **N-ary**: `makeAndExpr` appends to the arg list on sight
-- while parsing a left-deep chain, looking through parentheses as it goes, and
-- `get_rule_expr`'s `BoolExpr` arm walks that flat list. pglet rendered its parse tree's
-- left-leaning binary nodes instead, so every chain of **three or more** operands came
-- out re-nested — `CHECK ((((a > 0) AND (b > 0)) AND (c > 0)))` against 18.6's
-- `CHECK (((a > 0) AND (b > 0) AND (c > 0)))` — while the **two**-operand arity, which is
-- the arity every fixture pinning this renderer used, agreed.
--
-- The nesting shapes are the discriminating half and are ordered first in intent: a
-- flatten that ignored the operator would collapse `bx_and_then_or` and `bx_or_under_and`,
-- one that ignored the **side** would collapse `bx_parens_right` (18.6 keeps that one
-- nested — its left operand is not a `BoolExpr`, so nothing appends), and one that
-- flattened every associative operator would collapse `bx_arith`, which 18.6 renders
-- `(((a + b) + c) > 0)`.
-- @setup
CREATE TABLE bchain (
  a INT, b INT, c INT, d INT,
  f BOOL DEFAULT (1 > 0 OR 2 > 0 OR 3 > 0),
  CONSTRAINT bx_and3 CHECK (a > 0 AND b > 0 AND c > 0),
  CONSTRAINT bx_and4 CHECK (a > 1 AND b > 1 AND c > 1 AND d > 1),
  CONSTRAINT bx_or3 CHECK (a > 2 OR b > 2 OR c > 2),
  CONSTRAINT bx_and2 CHECK (a > 3 AND b > 3),
  CONSTRAINT bx_parens_left CHECK ((a > 4 AND b > 4) AND c > 4),
  CONSTRAINT bx_parens_right CHECK (a > 5 AND (b > 5 AND c > 5)),
  CONSTRAINT bx_or_parens_right CHECK (a > 6 OR (b > 6 OR c > 6)),
  CONSTRAINT bx_and_then_or CHECK ((a > 7 AND b > 7) OR c > 7),
  CONSTRAINT bx_or_under_and CHECK (a > 8 AND (b > 8 OR c > 8)),
  CONSTRAINT bx_and3_with_or CHECK (a > 9 AND b > 9 AND (c > 9 OR d > 9)),
  CONSTRAINT bx_not_and CHECK (NOT (a > 10 AND b > 10)),
  CONSTRAINT bx_and_not_or CHECK (a > 11 AND NOT (b > 11 OR c > 11)),
  CONSTRAINT bx_arith CHECK (a + b + c > 12),
  CONSTRAINT bx_arith_right CHECK (a - (b - c) > 13)
);
CREATE INDEX bchain_and3_idx ON bchain (a) WHERE (a > 0 AND b > 0 AND c > 0);
CREATE INDEX bchain_or3_idx ON bchain (b) WHERE (a > 0 OR b > 0 OR c > 0);

-- @test
SELECT conname, pg_get_constraintdef(oid) AS def
FROM pg_constraint WHERE conname LIKE 'bx\_%' ORDER BY conname
