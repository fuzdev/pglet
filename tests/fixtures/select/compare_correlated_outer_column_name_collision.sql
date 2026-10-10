-- A correlated reference belongs to the **outer** scope, so the inner query's
-- column-type map must not claim it. `o.x` is `oc.x` (integer); the subquery is
-- planned against `ic`, whose own `x` is text. Resolving the static comparison gate
-- on the bare name read `o.x` as `ic.x` and rejected `integer = text` — a `42883` on
-- a query Postgres answers. The gate now resolves qualifier-aware and declines a
-- qualifier that names no relation in the scope it is planning.
-- @setup
CREATE TABLE oc (x INT, tag TEXT);
CREATE TABLE ic (n INT, x TEXT);
INSERT INTO oc VALUES (1, 'a'), (2, 'b');
INSERT INTO ic VALUES (1, 'z');
-- @test
SELECT o.tag FROM oc o WHERE EXISTS (SELECT 1 FROM ic i WHERE i.n = o.x);
