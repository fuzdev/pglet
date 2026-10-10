-- `row_number` has no one-argument signature: the argument is refused, not dropped.
-- @setup
CREATE TABLE wna (a INT);
INSERT INTO wna VALUES (1), (2);
-- @test
-- @expect-error: function row_number(integer) does not exist (42883)
SELECT row_number(1) OVER () FROM wna;
