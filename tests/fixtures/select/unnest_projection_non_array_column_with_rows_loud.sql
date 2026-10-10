-- The row-bearing end of the row-count axis, and the half that was already loud under
-- the **wrong** code.
--
-- With a row the projection actually evaluated the leaf and pglet reached the
-- scalar-position refusal (`0A000 set-returning unnest() is only supported in a SELECT
-- projection`) — loud, and nothing to do with the missing signature. 18.6 answers the
-- same `42883` it answers over an empty table, so the SQLSTATE the harness compares is
-- what moves here, not the loudness.
-- @setup
CREATE TABLE unnest_na_rows (b BIGINT);
INSERT INTO unnest_na_rows VALUES (1), (2);
-- @test
-- @expect-error: no unnest(bigint)
SELECT unnest(b) FROM unnest_na_rows;
