-- `g.*` and a non-sole `*` over the series expand its one column — a base table of the
-- alias's name beside it notwithstanding.
-- @setup
CREATE TABLE g (q INT, r INT);
INSERT INTO g VALUES (7, 8);
-- @test
SELECT g.*, *, 1 AS one FROM generate_series(1, 2) g ORDER BY 1;
