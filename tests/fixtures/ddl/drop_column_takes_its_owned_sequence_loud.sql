-- The column half of the AUTO edge: dropping the SERIAL column takes the sequence it
-- owns, leaving no name behind.
-- @setup
CREATE TABLE dctos_t (a SERIAL, b INT);
ALTER TABLE dctos_t DROP COLUMN a;
-- @test
SELECT nextval('dctos_t_a_seq');
-- @expect-error: the owned sequence went with its column (42P01)
