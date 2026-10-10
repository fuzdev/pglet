-- A `WHERE` position. `build_call` is the one site a function call is built at, so every
-- expression position inherits the verdict; this and the `RETURNING`, `SET` and `DEFAULT`
-- fixtures are the positions where a dropped modifier used to decide which rows a
-- statement touched. 42809 on both.
-- @setup
CREATE TABLE sq_wq (c TEXT);
INSERT INTO sq_wq VALUES ('a');
-- @test
-- @expect-error: DISTINCT specified, but upper is not an aggregate function
SELECT c FROM sq_wq WHERE upper(DISTINCT c) = 'A';
