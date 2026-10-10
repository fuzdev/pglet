-- A fractional LIMIT rounds to nearest (ties-to-even), matching Postgres's
-- numeric->int8 coercion (1.5 -> 2 rows), rather than silently dropping to unbounded.
-- @setup
CREATE TABLE lim_frac (n INT);
INSERT INTO lim_frac VALUES (1), (2), (3);
-- @test
SELECT n FROM lim_frac ORDER BY n LIMIT 1.5;
