-- An `AND` / `OR` whose left operand is a literal `true` / `false`, a comparison of two
-- number literals, or `NOT` over one, and decides the connective, drops its right
-- operand's aggregates: 18.6 folds the connective before its `Agg` node collects them.
-- A nested `false AND …` decides a `CASE` arm the same way.
-- @setup
CREATE TABLE u (a INT, b INT NOT NULL, y TEXT);
INSERT INTO u VALUES (1, 10, 'x'), (2, 0, 'z');
-- @test
SELECT true OR sum(1/b) > 0 AS o, false AND sum(1/b) > 0 AS a,
  (1 < 2) OR sum(1/b) > 0 AS c, NOT false OR sum(1/b) > 0 AS n,
  NOT true AND sum(1/b) > 0 AS not_true,
  CASE WHEN false AND count(*) > 0 THEN sum(1/b) ELSE 0 END AS nested
FROM u;
