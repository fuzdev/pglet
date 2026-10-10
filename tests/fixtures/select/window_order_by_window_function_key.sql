-- @setup
CREATE TABLE wz (a INT, b INT);
INSERT INTO wz VALUES (1, 5), (1, 7), (2, 5);

-- A window function in the key is junk like any other non-output expression, and it
-- is the one kind that can only be evaluated *in* the Window — which is what the
-- junk column gives it. It was `0A000` from the row evaluator above the Window.
-- @test
SELECT b, row_number() OVER () AS rn FROM wz ORDER BY row_number() OVER () DESC;
