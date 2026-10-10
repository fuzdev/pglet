-- The argument decides, not the quantifier: `row_number(DISTINCT 1)` is the same refusal.
-- @setup
CREATE TABLE wna (a INT);
INSERT INTO wna VALUES (1), (2);
-- @test
-- @expect-error: function row_number(integer) does not exist (42883)
SELECT row_number(DISTINCT 1) OVER () FROM wna;
