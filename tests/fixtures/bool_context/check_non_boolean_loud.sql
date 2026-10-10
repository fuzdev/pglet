-- A `CHECK` over a non-boolean was accepted at DDL and then admitted every row —
-- a constraint that could never refuse anything, persisted into a catalog stock
-- Postgres refuses to load.
-- @expect-error: argument of CHECK must be type boolean
-- @test
CREATE TABLE bcckn (a INT CHECK (a));
