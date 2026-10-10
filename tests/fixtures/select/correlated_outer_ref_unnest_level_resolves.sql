-- **An over-rejection removed, and the fixture that pinned it inverted.** A subquery
-- whose FROM is a set-returning `unnest(…)` used to *seal* its query level: the level
-- walker had no arm for it, so a bare name might have belonged to it and resolution
-- stopped there rather than walking outward. This query was `42883` here and a row on
-- 18.6.
--
-- An `unnest` item is an ordinary named relation — Postgres's `chooseScalarFunctionAlias`
-- gives it the explicit alias, else `unnest` — and its columns are exactly the plan's
-- output columns, which the walker can enumerate. Registering it that way makes the level
-- certain and the bare `v` bind to the unnest's own `float4`, which is what 18.6 does.
--
-- **The silent wrong the seal was protecting against does not come back**, and this is
-- the control that says so: with the unnest column named `w` instead of `v`, the bare `v`
-- has no claimant at the inner level and resolves outward to `cz_o.v` (`float8`),
-- answering the row on both engines. Registering the relation *exactly* is what
-- distinguishes those two — the seal could only refuse both.
--
-- Its sibling `select/correlated_outer_ref_unnest_qualified_resolves.sql` is the
-- qualified spelling, which the seal blocked for a different reason: it stopped the walk
-- for the whole level, including references the unnest item could not possibly shadow.
--
-- What still stops a walk is a relation the builder genuinely cannot enumerate: a
-- **set-op** node, whose output columns are not addressable by relation name at all, and
-- a plan node the walker has no model for.
-- @setup
CREATE TABLE cz_o (okey INT, v FLOAT8);
INSERT INTO cz_o VALUES (1, 1.1);

-- @test
SELECT okey FROM cz_o WHERE EXISTS (SELECT 1 FROM unnest(ARRAY[1.1::float4]) AS t(v) WHERE v = '1.1');
