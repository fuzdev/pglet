-- A `DELETE`'s `WHERE`: `false` from the left operand keeps the row without evaluating
-- the division.
-- @setup
CREATE TABLE sd (id INT);
INSERT INTO sd VALUES (1);
-- @test
DELETE FROM sd WHERE (id = 99 AND 1/(id-1) = 0) RETURNING id;
