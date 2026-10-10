-- @setup
CREATE TABLE sd (x INT, a INT, b INT);
INSERT INTO sd VALUES (1, 20, 300), (1, 10, 100), (2, 40, 200), (3, 30, 400);

-- The mirror of `order_by_projected_then_dropped_key`: the dropped column leads and
-- the projected one tie-breaks. `a` is deliberately not correlated with `x`, so
-- ordering by `a` (1, 1, 3, 2) and by `x` (1, 1, 2, 3) differ — the result observes
-- which key actually drove the sort, not merely that the query ran.
-- @test
SELECT x FROM sd ORDER BY a, x;
