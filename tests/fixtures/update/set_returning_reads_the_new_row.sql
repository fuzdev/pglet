-- The accept-side companion of `set_swap_reads_the_old_row.sql`: the right-hand sides
-- read the old row and `RETURNING` reads the new one, so `a - b` is the swapped
-- difference.
-- @setup
CREATE TABLE sornr (a INT, b INT);
INSERT INTO sornr VALUES (10, 15);

-- @test
UPDATE sornr SET a = b, b = a RETURNING a, b, a - b AS d;
