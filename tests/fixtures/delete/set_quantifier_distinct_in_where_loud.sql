-- A `DELETE … WHERE` predicate, the shape where the dropped modifier **removed** a row
-- 18.6 keeps. 42809 on both.
-- @setup
CREATE TABLE sq_dl (c TEXT);
INSERT INTO sq_dl VALUES ('a');
-- @test
-- @expect-error: DISTINCT specified, but upper is not an aggregate function
DELETE FROM sq_dl WHERE upper(DISTINCT c) = 'A';
