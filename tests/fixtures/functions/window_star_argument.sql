-- The control: a `*` contributes no argument, so the zero-argument window functions answer.
-- @setup
CREATE TABLE wna (a INT);
INSERT INTO wna VALUES (1), (2), (2);
-- @test
SELECT a, row_number(*) OVER (ORDER BY a) AS rn, rank(*) OVER (ORDER BY a) AS r,
       dense_rank(*) OVER (ORDER BY a) AS dr
FROM wna ORDER BY a, rn;
