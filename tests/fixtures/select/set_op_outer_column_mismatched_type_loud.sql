-- Two enclosing columns of types with no common type are refused.
-- @setup
CREATE TABLE smt (i INT, d DATE);
INSERT INTO smt VALUES (7, '2020-01-01');
-- @expect-error: EXCEPT types integer and date cannot be matched
-- @test
SELECT count(*) FROM smt WHERE EXISTS (SELECT i EXCEPT SELECT d);
