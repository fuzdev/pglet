-- A `RETURNING`-less data-modifying body exposes **zero** columns, so any list is
-- `42P10` — `has 0 columns available`.
-- @expect-error: a RETURNING-less body has no columns to name
-- @expect-message
-- @setup
CREATE TABLE cldn (a INT);
INSERT INTO cldn VALUES (7);
-- @test
WITH dd(id) AS (DELETE FROM cldn WHERE false) SELECT 1 AS only_column;
