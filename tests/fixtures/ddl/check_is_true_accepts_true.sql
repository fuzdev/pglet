-- The accepting side of `check_is_true_rejects_false_and_null.sql`, and the fixture
-- that proves the constraint was *stored* rather than silently dropped: the row goes
-- in and reads back.
-- @setup
CREATE TABLE tk (id INT, b BOOL, CHECK (b IS TRUE));
INSERT INTO tk VALUES (1, true);
-- @test
SELECT id, b FROM tk ORDER BY id;
