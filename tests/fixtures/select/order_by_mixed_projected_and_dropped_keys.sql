-- @setup
CREATE TABLE sd (x INT, a INT, b INT);
INSERT INTO sd VALUES (1, 20, 300), (1, 10, 100), (2, 40, 200), (3, 30, 400);

-- Both projected columns and a dropped one in the same ORDER BY. The `x` tie is
-- broken by `a` ascending (10 before 20), which the projected `a` column shows, so
-- the ordering is observable and not just the absence of an error.
-- @test
SELECT x, a FROM sd ORDER BY x, a, b;
